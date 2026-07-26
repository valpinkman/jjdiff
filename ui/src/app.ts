import { html, LitElement, nothing } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { repeat } from 'lit/directives/repeat.js';

import './command-bar.js';
import type { Command } from './command-bar.js';
import './file-tree.js';
import {
  absorb,
  describeChange,
  generateWalkthrough,
  getConfig,
  getConflicts,
  getDiff,
  getInterdiffSinceReviewed,
  getLaunchOptions,
  getRepoState,
  getReviewStatus,
  getWalkthrough,
  markReviewed,
  newChange,
  onRepoChanged,
  setViewed,
  squashPaths,
  type Change,
  type FilePatch,
  type RepoState,
  type Walkthrough,
} from './ipc.js';
import { matchesShortcut, parseShortcut, type Shortcut } from './keys.js';
import './patch-view.js';
import './walkthrough-panel.js';
import type { DiffLayout } from './rows.js';

/** What the main pane shows for the selected change. */
type ViewMode = 'full' | 'interdiff';

/**
 * App shell. LIGHT DOM, non-negotiably: the diff pane (jj-patch-view) is a descendant, and
 * document stylesheets cannot cross a shadow boundary — a shadow root here would sever
 * theme.css from every diff row below it (exactly the bug that shipped in M1–M4). Chrome
 * styles live in theme.css under the `jj-app` prefix; leaf widgets with no cross-boundary
 * text selection (file tree, command bar) keep their own shadow styles.
 */
@customElement('jj-app')
export class App extends LitElement {
  protected override createRenderRoot() {
    return this; // light DOM
  }

  @state() private repo: RepoState | null = null;
  @state() private error: string | null = null;
  @state() private actionError: string | null = null;
  @state() private actionInfo: string | null = null;
  @state() private selected: string | null = null; // change id; null = working copy
  @state() private files: FilePatch[] = [];
  @state() private layout: DiffLayout = 'split';
  @state() private ignoreWhitespace = false;
  @state() private focusPath: string | null = null;
  @state() private viewedPaths: ReadonlySet<string> = new Set();
  @state() private reviewedCommit: string | null = null;
  @state() private conflictedPaths: ReadonlySet<string> = new Set();
  @state() private description = '';
  @state() private barOpen = false;
  @state() private viewMode: ViewMode = 'full';
  @state() private walkthrough: Walkthrough | null = null;
  @state() private walkStale = false;
  @state() private walkActive = false;
  /** -1 = overview (summary + full diff); 0..n-1 = steps. */
  @state() private walkStep = -1;
  @state() private generating = false;

  private unlisten: (() => void) | null = null;
  /** The change id the description editor was last seeded from. */
  private seededFor: string | null = null;
  private commandBarShortcut: Shortcut = parseShortcut('Mod+Shift+p');

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
      if (config.ui.theme === 'light' || config.ui.theme === 'dark') {
        document.documentElement.dataset['jjTheme'] = config.ui.theme;
        document.documentElement.style.colorScheme = config.ui.theme;
      }
      this.commandBarShortcut = parseShortcut(config.keymap.commandBar);
    } catch {
      // Config is best-effort; defaults are fine.
    }
    await this.refresh();
    void onRepoChanged(() => void this.refresh()).then((unlisten) => {
      this.unlisten = unlisten;
    });
    try {
      const launch = await getLaunchOptions();
      if (launch.walkthrough) {
        if (this.walkthrough && !this.walkStale) {
          this.walkActive = true;
        } else {
          this.runGenerateWalkthrough();
        }
      }
    } catch {
      // Launch options are best-effort.
    }
  }

  private onGlobalKey = (event: KeyboardEvent) => {
    if (matchesShortcut(event, this.commandBarShortcut)) {
      event.preventDefault();
      this.barOpen = !this.barOpen;
      return;
    }
    const typing = (event.target as HTMLElement | null)?.tagName === 'TEXTAREA'
      || (event.target as HTMLElement | null)?.tagName === 'INPUT';
    if (this.walkActive && !typing) {
      if (event.key === 'ArrowRight') {
        event.preventDefault();
        this.moveStep(1);
      } else if (event.key === 'ArrowLeft') {
        event.preventDefault();
        this.moveStep(-1);
      } else if (event.key === 'Escape') {
        this.exitWalkthrough();
      }
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

  /** True when the selected change moved since it was last marked reviewed. */
  private get changedSinceReview(): boolean {
    const change = this.selectedChange;
    return (
      change !== null &&
      this.reviewedCommit !== null &&
      this.reviewedCommit !== change.commitId
    );
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
      await Promise.all([this.loadDiff(), this.loadReview(), this.loadConflicts(), this.loadWalkthrough()]);
    } catch (error) {
      this.error = String(error);
    }
  }

  private async loadDiff() {
    try {
      if (this.viewMode === 'interdiff' && this.selectedChange && this.changedSinceReview) {
        const interdiff = await getInterdiffSinceReviewed(
          this.selectedChange.changeId,
          this.ignoreWhitespace,
        );
        this.files = interdiff.files;
      } else {
        this.viewMode = 'full';
        this.files = await getDiff(
          this.isWorkingCopySelected ? null : this.selected,
          this.ignoreWhitespace,
        );
      }
      this.actionError = null;
      if (this.focusPath && !this.files.some((f) => f.path === this.focusPath)) {
        this.focusPath = null;
      }
    } catch (error) {
      this.actionError = String(error);
    }
  }

  private async loadReview() {
    const change = this.selectedChange;
    if (!change) return;
    try {
      const status = await getReviewStatus(change.changeId);
      this.viewedPaths = new Set(status.viewed);
      this.reviewedCommit = status.reviewedCommit;
    } catch {
      this.viewedPaths = new Set();
      this.reviewedCommit = null;
    }
  }

  private async loadConflicts() {
    const change = this.selectedChange;
    if (!change || !change.conflict) {
      this.conflictedPaths = new Set();
      return;
    }
    try {
      this.conflictedPaths = new Set(await getConflicts(change.changeId));
    } catch {
      this.conflictedPaths = new Set();
    }
  }

  private select(change: Change) {
    this.selected = change.changeId;
    this.focusPath = null;
    this.viewMode = 'full';
    this.walkActive = false;
    this.walkStep = -1;
    const target = this.repo?.stack.find((c) => c.changeId === change.changeId);
    this.description = target?.description ?? '';
    this.seededFor = change.changeId;
    void this.loadDiff();
    void this.loadReview();
    void this.loadConflicts();
    void this.loadWalkthrough();
  }

  private async loadWalkthrough() {
    const change = this.selectedChange;
    if (!change) return;
    try {
      const status = await getWalkthrough(
        change.changeId,
        this.isWorkingCopySelected ? null : change.changeId,
        this.ignoreWhitespace,
      );
      this.walkthrough = status.walkthrough;
      this.walkStale = status.stale;
      if (!status.walkthrough) {
        this.walkActive = false;
      }
    } catch {
      this.walkthrough = null;
      this.walkStale = false;
      this.walkActive = false;
    }
  }

  private runGenerateWalkthrough() {
    const change = this.selectedChange;
    if (!change || this.generating) return;
    this.generating = true;
    void this.run(async () => {
      const label = change.description.split('\n')[0] || '(no description)';
      this.walkthrough = await generateWalkthrough(
        change.changeId,
        this.isWorkingCopySelected ? null : change.changeId,
        this.ignoreWhitespace,
        `change ${change.changeId.slice(0, 8)}: ${label}`,
      );
      this.walkStale = false;
      this.walkActive = true;
      this.walkStep = -1;
    }).finally(() => {
      this.generating = false;
    });
  }

  private startWalkthrough() {
    if (!this.walkthrough) {
      this.runGenerateWalkthrough();
      return;
    }
    this.walkActive = true;
    this.walkStep = -1;
  }

  private exitWalkthrough() {
    this.walkActive = false;
    this.walkStep = -1;
  }

  private moveStep(delta: number) {
    if (!this.walkthrough) return;
    const next = this.walkStep + delta;
    if (next < -1 || next >= this.walkthrough.steps.length) return;
    this.walkStep = next;
  }

  /** Hunks visible in the current walkthrough step, or null for everything. */
  private get walkFilter(): ReadonlySet<string> | null {
    if (!this.walkActive || !this.walkthrough || this.walkStep < 0) return null;
    return new Set(this.walkthrough.steps[this.walkStep]?.hunkIds ?? []);
  }

  private async run(action: () => Promise<void>) {
    try {
      await action();
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
      this.actionInfo = null;
    }
  }

  private saveDescription() {
    const change = this.selectedChange;
    if (!change) return;
    void this.run(async () => {
      await describeChange(change.changeId, this.description);
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

  private runAbsorb() {
    void this.run(async () => {
      const summary = await absorb();
      this.actionInfo = summary || 'Nothing to absorb.';
      await this.refresh();
    });
  }

  private markCurrentReviewed() {
    const change = this.selectedChange;
    if (!change) return;
    void this.run(async () => {
      await markReviewed(change.changeId, change.commitId);
      this.reviewedCommit = change.commitId;
      this.viewMode = 'full';
      await this.loadDiff();
    });
  }

  private showInterdiff() {
    this.viewMode = 'interdiff';
    void this.loadDiff();
  }

  private showFullDiff() {
    this.viewMode = 'full';
    void this.loadDiff();
  }

  private toggleLayout() {
    this.layout = this.layout === 'split' ? 'unified' : 'split';
  }

  private toggleWhitespace() {
    this.ignoreWhitespace = !this.ignoreWhitespace;
    void this.loadDiff();
  }

  /** Mutable non-@ stack changes a file can be squashed into. */
  private get squashTargets(): { changeId: string; label: string }[] {
    if (!this.repo) return [];
    return this.repo.stack
      .filter((change) => !change.workingCopy && !change.immutable)
      .map((change) => ({
        changeId: change.changeId,
        label: `${change.changeId.slice(0, 8)} ${
          change.description.split('\n')[0] || '(no description)'
        }`,
      }));
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
      this.walkthrough
        ? {
            id: 'walkthrough',
            label: this.walkActive ? 'Exit Walkthrough' : 'Start Walkthrough',
            run: () => (this.walkActive ? this.exitWalkthrough() : this.startWalkthrough()),
          }
        : {
            id: 'walkthrough',
            label: 'Generate Walkthrough (Claude)',
            run: () => this.runGenerateWalkthrough(),
          },
    ];
    if (this.walkthrough) {
      commands.push({
        id: 'regen-walkthrough',
        label: 'Regenerate Walkthrough (Claude)',
        run: () => this.runGenerateWalkthrough(),
      });
    }
    if (this.isWorkingCopySelected) {
      commands.push(
        {
          id: 'new',
          label: 'New Change (jj new)',
          run: () =>
            void this.run(async () => {
              await newChange();
              this.selected = null;
              this.seededFor = null;
              await this.refresh();
            }),
        },
        {
          id: 'absorb',
          label: 'Absorb Into Ancestors (jj absorb)',
          run: () => this.runAbsorb(),
        },
      );
    } else {
      commands.push({
        id: 'reviewed',
        label: 'Mark Change Reviewed',
        run: () => this.markCurrentReviewed(),
      });
      if (this.changedSinceReview) {
        commands.push({
          id: 'interdiff',
          label: 'Show Changes Since Last Review',
          run: () => this.showInterdiff(),
        });
      }
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
      return html`<div class="fatal">${this.error}</div>`;
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
        <button
          class="tool ${this.walkActive ? 'on' : ''}"
          ?disabled=${this.generating || this.files.length === 0}
          title=${this.walkthrough
            ? 'Guided review of this change'
            : 'Generate a guided review with Claude'}
          @click=${() => (this.walkActive ? this.exitWalkthrough() : this.startWalkthrough())}
        >
          ${this.generating
            ? 'Generating…'
            : this.walkActive
              ? 'Exit Walkthrough'
              : this.walkthrough
                ? 'Walkthrough'
                : 'Generate Walkthrough'}
        </button>
        ${isWc
          ? html`<button
              class="tool"
              title="Route working-copy hunks into the ancestors that last touched them"
              @click=${this.runAbsorb}
            >
              Absorb
            </button>`
          : nothing}
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
                ${item.conflict ? html`<span class="badge warn">conflict</span>` : nothing}
                ${item.immutable ? html`<span class="badge">immutable</span>` : nothing}
                ${item.bookmarks.map((b) => html`<span class="badge">${b}</span>`)}
                <span class="desc ${item.description ? '' : 'empty-desc'}">
                  ${item.description.split('\n')[0] || '(no description)'}
                </span>
              </button>
            `,
          )}
        </div>
        ${this.walkActive && this.walkthrough
          ? html`
              <div class="section-title">
                Steps (${this.walkthrough.steps.length})
              </div>
              <div class="files">
                <jj-walkthrough-panel
                  .walkthrough=${this.walkthrough}
                  .files=${this.files}
                  .viewed=${this.viewedPaths}
                  .current=${this.walkStep}
                  @step-selected=${(event: CustomEvent<number>) => {
                    this.walkStep = event.detail;
                  }}
                ></jj-walkthrough-panel>
              </div>
            `
          : html`
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
            `}
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
                : html`<button
                    class="tool ${this.changedSinceReview ? '' : 'on'}"
                    title="Record the current commit as reviewed"
                    @click=${this.markCurrentReviewed}
                  >
                    ${this.reviewedCommit && !this.changedSinceReview
                      ? 'Reviewed ✓'
                      : 'Mark Reviewed'}
                  </button>`}
            </div>`
          : nothing}
        ${change?.conflict
          ? html`<div class="banner conflict">
              ⚠ This change has unresolved conflicts
              (${this.conflictedPaths.size || '?'} file${this.conflictedPaths.size === 1
                ? ''
                : 's'}) — resolve with <code>jj resolve</code> in a terminal.
            </div>`
          : nothing}
        ${this.changedSinceReview
          ? html`<div class="banner">
              This change evolved since you reviewed it.
              <span class="spacer"></span>
              ${this.viewMode === 'interdiff'
                ? html`<button class="tool" @click=${this.showFullDiff}>Full Diff</button>`
                : html`<button class="tool" @click=${this.showInterdiff}>
                    What Changed Since Review
                  </button>`}
              <button class="tool" @click=${this.markCurrentReviewed}>Mark Reviewed</button>
            </div>`
          : nothing}
        ${this.walkActive && this.walkthrough
          ? html`<div class="walk-banner">
              <div class="walk-head">
                <span class="walk-progress">
                  ${this.walkStep < 0
                    ? 'Overview'
                    : `Step ${this.walkStep + 1} of ${this.walkthrough.steps.length}`}
                </span>
                <strong>
                  ${this.walkStep < 0
                    ? 'Guided review'
                    : this.walkthrough.steps[this.walkStep]?.title}
                </strong>
                <span class="spacer"></span>
                <button class="tool" ?disabled=${this.walkStep <= -1} @click=${() => this.moveStep(-1)}>
                  ← Prev
                </button>
                <button
                  class="tool primary"
                  ?disabled=${this.walkStep >= this.walkthrough.steps.length - 1}
                  @click=${() => this.moveStep(1)}
                >
                  Next →
                </button>
              </div>
              <p class="walk-narrative">
                ${this.walkStep < 0
                  ? this.walkthrough.summary
                  : this.walkthrough.steps[this.walkStep]?.narrative}
              </p>
            </div>`
          : nothing}
        ${this.walkthrough && this.walkStale && !this.generating
          ? html`<div class="banner">
              The walkthrough was generated for an older version of this change.
              <span class="spacer"></span>
              <button class="tool" @click=${this.runGenerateWalkthrough}>Regenerate</button>
            </div>`
          : nothing}
        ${this.actionError
          ? html`<div class="status error">${this.actionError}</div>`
          : nothing}
        ${this.actionInfo ? html`<div class="status info">${this.actionInfo}</div>` : nothing}
        <jj-patch-view
          .files=${visible}
          .layout=${this.layout}
          .viewed=${this.viewedPaths}
          .canSquash=${isWc && this.viewMode === 'full' && !this.walkActive && this.squashTargets.length > 0}
          .canMarkViewed=${this.viewMode === 'full'}
          .squashTargets=${this.squashTargets}
          .conflicted=${this.conflictedPaths}
          .hunkFilter=${this.walkFilter}
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

  private onSquashFile = (event: CustomEvent<{ path: string; into: string }>) => {
    void this.run(async () => {
      await squashPaths([event.detail.path], event.detail.into);
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
    void setViewed(change.changeId, path, viewed).catch(() => void this.loadReview());
  };
}

const basename = (path: string) => path.slice(path.lastIndexOf('/') + 1) || path;

declare global {
  interface HTMLElementTagNameMap {
    'jj-app': App;
  }
}
