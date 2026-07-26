import { css, html, LitElement, nothing } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { repeat } from 'lit/directives/repeat.js';

import './command-bar.js';
import type { Command } from './command-bar.js';
import './file-tree.js';
import {
  describeChange,
  getConfig,
  getDiff,
  getRepoState,
  getViewedFiles,
  newChange,
  onRepoChanged,
  setViewed,
  squashPaths,
  type Change,
  type FilePatch,
  type RepoState,
} from './ipc.js';
import './patch-view.js';
import type { DiffLayout } from './rows.js';

/** App chrome (shadow DOM is fine here — no text the user selects across boundaries). */
@customElement('jj-app')
export class App extends LitElement {
  static override styles = css`
    :host {
      display: grid;
      grid-template-columns: 280px 1fr;
      grid-template-rows: auto 1fr;
      height: 100%;
    }
    header {
      grid-column: 1 / -1;
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 6px 12px;
      border-bottom: 1px solid var(--jj-border);
      background: var(--jj-bg-panel);
    }
    header .root {
      font-weight: 600;
    }
    header .version {
      color: var(--jj-fg-muted);
      font-size: 11px;
    }
    header .spacer {
      flex: 1;
    }
    button.tool {
      border: 1px solid var(--jj-border);
      background: var(--jj-bg);
      color: var(--jj-fg);
      font: inherit;
      font-size: 12px;
      border-radius: 6px;
      padding: 3px 10px;
      cursor: pointer;
    }
    button.tool:hover {
      border-color: var(--jj-accent);
    }
    button.tool.on {
      border-color: var(--jj-accent);
      color: var(--jj-accent);
    }
    button.tool.primary {
      background: var(--jj-accent);
      border-color: var(--jj-accent);
      color: #fff;
    }
    button.tool:disabled {
      opacity: 0.5;
      cursor: default;
    }
    aside {
      border-right: 1px solid var(--jj-border);
      overflow-y: auto;
      display: flex;
      flex-direction: column;
      min-height: 0;
    }
    .section-title {
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.08em;
      text-transform: uppercase;
      color: var(--jj-fg-muted);
      padding: 10px 10px 4px;
    }
    .stack {
      padding: 0 6px;
    }
    .change {
      display: block;
      width: 100%;
      text-align: left;
      border: 1px solid transparent;
      border-radius: 6px;
      background: none;
      color: var(--jj-fg);
      font: inherit;
      padding: 5px 8px;
      cursor: pointer;
    }
    .change:hover {
      background: var(--jj-bg-panel);
    }
    .change.selected {
      border-color: var(--jj-accent);
      background: var(--jj-bg-panel);
    }
    .change .id {
      font-family: var(--jj-mono);
      color: var(--jj-accent);
      font-size: 11px;
    }
    .change .desc {
      display: block;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      font-size: 12px;
    }
    .change .desc.empty-desc {
      color: var(--jj-fg-muted);
      font-style: italic;
    }
    .badge {
      font-size: 10px;
      border-radius: 4px;
      padding: 1px 5px;
      margin-left: 6px;
      background: var(--jj-hunk-bg);
      color: var(--jj-fg-muted);
    }
    .files {
      flex: 1;
      overflow-y: auto;
      padding: 0 6px 10px;
    }
    main {
      min-height: 0;
      display: flex;
      flex-direction: column;
    }
    .describe {
      display: flex;
      gap: 8px;
      align-items: flex-start;
      padding: 8px 12px;
      border-bottom: 1px solid var(--jj-border);
    }
    .describe textarea {
      flex: 1;
      resize: vertical;
      min-height: 34px;
      max-height: 140px;
      border: 1px solid var(--jj-border);
      border-radius: 6px;
      background: var(--jj-bg);
      color: var(--jj-fg);
      font: inherit;
      font-size: 12.5px;
      padding: 6px 8px;
      box-sizing: border-box;
    }
    .describe textarea:focus {
      outline: none;
      border-color: var(--jj-accent);
    }
    .status {
      padding: 4px 12px;
      font-size: 11px;
      color: var(--jj-removed-fg);
      white-space: pre-wrap;
    }
    .error {
      grid-column: 1 / -1;
      padding: 24px;
      color: var(--jj-removed-fg);
      font-family: var(--jj-mono);
      white-space: pre-wrap;
    }
  `;

  @state() private repo: RepoState | null = null;
  @state() private error: string | null = null;
  @state() private actionError: string | null = null;
  @state() private selected: string | null = null; // change id; null = working copy
  @state() private files: FilePatch[] = [];
  @state() private layout: DiffLayout = 'split';
  @state() private ignoreWhitespace = false;
  @state() private focusPath: string | null = null;
  @state() private viewedPaths: ReadonlySet<string> = new Set();
  @state() private description = '';
  @state() private barOpen = false;

  private unlisten: (() => void) | null = null;
  /** The change id the description editor was last seeded from. */
  private seededFor: string | null = null;

  override connectedCallback() {
    super.connectedCallback();
    void this.start();
    window.addEventListener('keydown', this.onGlobalKey);
  }

  override disconnectedCallback() {
    this.unlisten?.();
    window.removeEventListener('keydown', this.onGlobalKey);
    super.disconnectedCallback();
  }

  private async start() {
    try {
      const config = await getConfig();
      this.layout = config.ui.diffStyle === 'unified' ? 'unified' : 'split';
      this.ignoreWhitespace = config.ui.ignoreWhitespace;
      document.documentElement.style.setProperty(
        '--jj-code-size',
        `${config.ui.codeFontSize}px`,
      );
    } catch {
      // Config is best-effort; defaults are fine.
    }
    await this.refresh();
    void onRepoChanged(() => void this.refresh()).then((unlisten) => {
      this.unlisten = unlisten;
    });
  }

  private onGlobalKey = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === 'p') {
      event.preventDefault();
      this.barOpen = !this.barOpen;
    }
  };

  private get selectedChange(): Change | null {
    if (!this.repo) return null;
    const id = this.selected ?? this.repo.workingCopy.changeId;
    return this.repo.stack.find((change) => change.changeId === id) ?? this.repo.workingCopy;
  }

  private get isWorkingCopySelected(): boolean {
    return this.selected === null || this.selected === this.repo?.workingCopy.changeId;
  }

  private async refresh() {
    try {
      this.repo = await getRepoState();
      this.error = null;
      // Seed the description editor only when the selection target changed — a background
      // refresh must not clobber what the user is typing.
      const current = this.selectedChange;
      if (current && this.seededFor !== current.changeId) {
        this.description = current.description;
        this.seededFor = current.changeId;
      }
      await this.loadDiff();
      await this.loadViewed();
    } catch (error) {
      this.error = String(error);
    }
  }

  private async loadDiff() {
    try {
      this.files = await getDiff(
        this.isWorkingCopySelected ? null : this.selected,
        this.ignoreWhitespace,
      );
      this.actionError = null;
      if (this.focusPath && !this.files.some((f) => f.path === this.focusPath)) {
        this.focusPath = null;
      }
    } catch (error) {
      this.actionError = String(error);
    }
  }

  private async loadViewed() {
    const change = this.selectedChange;
    if (!change) return;
    try {
      this.viewedPaths = new Set(await getViewedFiles(change.changeId));
    } catch {
      this.viewedPaths = new Set();
    }
  }

  private select(change: Change) {
    this.selected = change.changeId;
    this.focusPath = null;
    const target = this.repo?.stack.find((c) => c.changeId === change.changeId);
    this.description = target?.description ?? '';
    this.seededFor = change.changeId;
    void this.loadDiff();
    void this.loadViewed();
  }

  private async run(action: () => Promise<void>) {
    try {
      await action();
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
    }
  }

  private saveDescription() {
    const change = this.selectedChange;
    if (!change) return;
    void this.run(async () => {
      await describeChange(change.changeId, this.description);
      // The op watcher also fires, but refresh immediately for snappy feedback.
      await this.refresh();
    });
  }

  private commitAndNew() {
    const change = this.selectedChange;
    if (!change) return;
    void this.run(async () => {
      await describeChange(change.changeId, this.description);
      await newChange();
      this.selected = null;
      this.seededFor = null;
      await this.refresh();
    });
  }

  private toggleLayout() {
    this.layout = this.layout === 'split' ? 'unified' : 'split';
  }

  private toggleWhitespace() {
    this.ignoreWhitespace = !this.ignoreWhitespace;
    void this.loadDiff();
  }

  private get commands(): Command[] {
    const commands: Command[] = [
      {
        id: 'layout',
        label: 'Toggle Diff Layout',
        hint: this.layout === 'split' ? 'unified' : 'split',
        run: () => this.toggleLayout(),
      },
      {
        id: 'whitespace',
        label: this.ignoreWhitespace ? 'Show Whitespace Changes' : 'Hide Whitespace Changes',
        run: () => this.toggleWhitespace(),
      },
      { id: 'refresh', label: 'Refresh', run: () => void this.refresh() },
    ];
    if (this.isWorkingCopySelected) {
      commands.push({
        id: 'new',
        label: 'New Change (jj new)',
        run: () => void this.run(async () => {
          await newChange();
          this.selected = null;
          this.seededFor = null;
          await this.refresh();
        }),
      });
    }
    if (this.focusPath) {
      commands.push({
        id: 'unfocus',
        label: 'Show All Files',
        run: () => (this.focusPath = null),
      });
    }
    return commands;
  }

  protected override render() {
    if (this.error) {
      return html`<div class="error">${this.error}</div>`;
    }
    if (!this.repo) {
      return nothing;
    }
    const selectedId = this.selected ?? this.repo.workingCopy.changeId;
    const change = this.selectedChange;
    const isWc = this.isWorkingCopySelected;
    const visible = this.focusPath
      ? this.files.filter((file) => file.path === this.focusPath)
      : this.files;
    return html`
      <header>
        <span class="root">${basename(this.repo.root)}</span>
        <span class="version">jj ${this.repo.jjVersion}</span>
        <span class="spacer"></span>
        <button class="tool" @click=${this.toggleLayout} title="Toggle diff layout">
          ${this.layout === 'split' ? 'Split' : 'Unified'}
        </button>
        <button
          class="tool ${this.ignoreWhitespace ? 'on' : ''}"
          @click=${this.toggleWhitespace}
          title="Hide whitespace-only changes"
        >
          W/S
        </button>
      </header>
      <aside>
        <div class="section-title">Stack</div>
        <div class="stack">
          ${repeat(
            this.repo.stack,
            (item) => item.changeId,
            (item) => html`
              <button
                class="change ${item.changeId === selectedId ? 'selected' : ''}"
                @click=${() => this.select(item)}
              >
                <span class="id">${item.changeId.slice(0, 8)}</span>
                ${item.workingCopy ? html`<span class="badge">@</span>` : nothing}
                ${item.conflict ? html`<span class="badge">conflict</span>` : nothing}
                ${item.immutable ? html`<span class="badge">immutable</span>` : nothing}
                ${item.bookmarks.map((b) => html`<span class="badge">${b}</span>`)}
                <span class="desc ${item.description ? '' : 'empty-desc'}">
                  ${item.description.split('\n')[0] || '(no description)'}
                </span>
              </button>
            `,
          )}
        </div>
        <div class="section-title">
          Files (${this.viewedPaths.size ? `${this.viewedPaths.size}/` : ''}${this.files.length})
        </div>
        <div class="files">
          <jj-file-tree
            .files=${this.files}
            .selected=${this.focusPath}
            .viewed=${this.viewedPaths}
            @file-selected=${(event: CustomEvent<string | null>) => {
              this.focusPath = event.detail;
            }}
          ></jj-file-tree>
        </div>
      </aside>
      <main
        @squash-file=${this.onSquashFile}
        @toggle-viewed=${this.onToggleViewed}
      >
        ${change
          ? html`<div class="describe">
              <textarea
                placeholder=${isWc ? 'Describe this change…' : 'Description'}
                .value=${this.description}
                ?disabled=${change.immutable}
                @input=${(event: Event) =>
                  (this.description = (event.target as HTMLTextAreaElement).value)}
              ></textarea>
              <button
                class="tool"
                ?disabled=${change.immutable || this.description === change.description}
                @click=${this.saveDescription}
              >
                Describe
              </button>
              ${isWc
                ? html`<button
                    class="tool primary"
                    ?disabled=${this.files.length === 0 || !this.description.trim()}
                    title="Describe @ and start a new change on top (jj describe + jj new)"
                    @click=${this.commitAndNew}
                  >
                    Commit & New
                  </button>`
                : nothing}
            </div>`
          : nothing}
        ${this.actionError ? html`<div class="status">${this.actionError}</div>` : nothing}
        <jj-patch-view
          .files=${visible}
          .layout=${this.layout}
          .viewed=${this.viewedPaths}
          .canSquash=${isWc && !this.parentImmutable()}
          .canMarkViewed=${true}
        ></jj-patch-view>
      </main>
      ${this.barOpen
        ? html`<jj-command-bar
            .commands=${this.commands}
            @close=${() => (this.barOpen = false)}
          ></jj-command-bar>`
        : nothing}
    `;
  }

  private parentImmutable(): boolean {
    if (!this.repo) return true;
    const parentId = this.repo.workingCopy.parents[0];
    const parent = this.repo.stack.find((change) => change.commitId === parentId);
    // Parent outside the stack (e.g. trunk) is immutable by definition of `trunk()..@`.
    return parent ? parent.immutable : true;
  }

  private onSquashFile = (event: CustomEvent<{ path: string }>) => {
    void this.run(async () => {
      await squashPaths([event.detail.path]);
      await this.refresh();
    });
  };

  private onToggleViewed = (event: CustomEvent<{ path: string; viewed: boolean }>) => {
    const change = this.selectedChange;
    if (!change) return;
    const { path, viewed } = event.detail;
    // Optimistic update; persistence follows.
    const next = new Set(this.viewedPaths);
    if (viewed) next.add(path);
    else next.delete(path);
    this.viewedPaths = next;
    void setViewed(change.changeId, path, viewed).catch(() => void this.loadViewed());
  };
}

const basename = (path: string) => path.slice(path.lastIndexOf('/') + 1) || path;

declare global {
  interface HTMLElementTagNameMap {
    'jj-app': App;
  }
}
