import { css, html, LitElement, nothing } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { repeat } from 'lit/directives/repeat.js';

import './file-tree.js';
import {
  getDiff,
  getRepoState,
  onRepoChanged,
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
    header button {
      border: 1px solid var(--jj-border);
      background: var(--jj-bg);
      color: var(--jj-fg);
      font: inherit;
      font-size: 12px;
      border-radius: 6px;
      padding: 3px 10px;
      cursor: pointer;
    }
    header button:hover {
      border-color: var(--jj-accent);
    }
    header button.on {
      border-color: var(--jj-accent);
      color: var(--jj-accent);
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
  @state() private selected: string | null = null; // change id; null = working copy
  @state() private files: FilePatch[] = [];
  @state() private layout: DiffLayout = 'split';
  @state() private ignoreWhitespace = false;
  @state() private focusPath: string | null = null;

  private unlisten: (() => void) | null = null;

  override connectedCallback() {
    super.connectedCallback();
    void this.refresh();
    void onRepoChanged(() => void this.refresh()).then((unlisten) => {
      this.unlisten = unlisten;
    });
  }

  override disconnectedCallback() {
    this.unlisten?.();
    super.disconnectedCallback();
  }

  private async refresh() {
    try {
      this.repo = await getRepoState();
      this.error = null;
      await this.loadDiff();
    } catch (error) {
      this.error = String(error);
    }
  }

  private async loadDiff() {
    const isWorkingCopy =
      this.selected === null || this.selected === this.repo?.workingCopy.changeId;
    try {
      this.files = await getDiff(isWorkingCopy ? null : this.selected, this.ignoreWhitespace);
      if (this.focusPath && !this.files.some((f) => f.path === this.focusPath)) {
        this.focusPath = null;
      }
    } catch (error) {
      this.error = String(error);
    }
  }

  private select(change: Change) {
    this.selected = change.changeId;
    this.focusPath = null;
    void this.loadDiff();
  }

  private toggleLayout() {
    this.layout = this.layout === 'split' ? 'unified' : 'split';
  }

  private toggleWhitespace() {
    this.ignoreWhitespace = !this.ignoreWhitespace;
    void this.loadDiff();
  }

  protected override render() {
    if (this.error) {
      return html`<div class="error">${this.error}</div>`;
    }
    if (!this.repo) {
      return nothing;
    }
    const selectedId = this.selected ?? this.repo.workingCopy.changeId;
    const visible = this.focusPath
      ? this.files.filter((file) => file.path === this.focusPath)
      : this.files;
    return html`
      <header>
        <span class="root">${basename(this.repo.root)}</span>
        <span class="version">jj ${this.repo.jjVersion}</span>
        <span class="spacer"></span>
        <button @click=${this.toggleLayout} title="Toggle diff layout">
          ${this.layout === 'split' ? 'Split' : 'Unified'}
        </button>
        <button
          class=${this.ignoreWhitespace ? 'on' : ''}
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
            (change) => change.changeId,
            (change) => html`
              <button
                class="change ${change.changeId === selectedId ? 'selected' : ''}"
                @click=${() => this.select(change)}
              >
                <span class="id">${change.changeId.slice(0, 8)}</span>
                ${change.workingCopy ? html`<span class="badge">@</span>` : nothing}
                ${change.conflict ? html`<span class="badge">conflict</span>` : nothing}
                ${change.bookmarks.map((b) => html`<span class="badge">${b}</span>`)}
                <span class="desc ${change.description ? '' : 'empty-desc'}">
                  ${change.description.split('\n')[0] || '(no description)'}
                </span>
              </button>
            `,
          )}
        </div>
        <div class="section-title">Files (${this.files.length})</div>
        <div class="files">
          <jj-file-tree
            .files=${this.files}
            .selected=${this.focusPath}
            @file-selected=${(event: CustomEvent<string | null>) => {
              this.focusPath = event.detail;
            }}
          ></jj-file-tree>
        </div>
      </aside>
      <main>
        <jj-patch-view .files=${visible} .layout=${this.layout}></jj-patch-view>
      </main>
    `;
  }
}

const basename = (path: string) => path.slice(path.lastIndexOf('/') + 1) || path;

declare global {
  interface HTMLElementTagNameMap {
    'jj-app': App;
  }
}
