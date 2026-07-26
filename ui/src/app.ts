import { html, LitElement, nothing } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { keyed } from 'lit/directives/keyed.js';
import { repeat } from 'lit/directives/repeat.js';

import './command-bar.js';
import type { Command } from './command-bar.js';
import './file-tree.js';
import './log-graph.js';
import {
  absorb,
  describeChange,
  generateWalkthrough,
  getConfig,
  getConflicts,
  getDiff,
  getInterdiffSinceReviewed,
  getLaunchOptions,
  getFileContent,
  getRecentRepos,
  getRepoState,
  getReviewStatus,
  getWalkthrough,
  importWalkthrough,
  markReviewed,
  newChange,
  onRepoChanged,
  openRepository,
  pickRepository,
  setViewed,
  squashPaths,
  type Change,
  type FilePatch,
  type RepoState,
  type Walkthrough,
} from './ipc.js';
import { folderIcon } from './file-icons.js';
import { matchesShortcut, parseShortcut, type Shortcut } from './keys.js';
import './patch-view.js';
import type { PatchView } from './patch-view.js';
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
  @state() private sidebarTab: 'stack' | 'files' | 'steps' = 'stack';
  @state() private walkthrough: Walkthrough | null = null;
  @state() private walkStale = false;
  @state() private walkActive = false;
  /** -1 = overview (summary + full diff); 0..n-1 = steps. */
  @state() private walkStep = -1;
  @state() private generating = false;
  /** Guided review across the whole stack: changes ordered oldest → newest. */
  @state() private stackReview: Change[] | null = null;
  @state() private repoMenuOpen = false;
  @state() private recentRepos: string[] = [];
  @state() private searchOpen = false;
  @state() private searchQuery = '';
  @state() private searchCount = 0;
  @state() private searchCurrent = -1;
  @state() private wordWrap = false;
  /** File the diff viewport is currently inside (sticky breadcrumb). */
  @state() private visibleFile: string | null = null;
  /** Full file text for context expansion, fetched on demand. */
  @state() private fileLines: ReadonlyMap<string, string[]> = new Map();
  @state() private expansions: ReadonlyMap<string, { up: number; down: number }> = new Map();

  private unlisten: (() => void) | null = null;
  /** The change id the description editor was last seeded from. */
  private seededFor: string | null = null;
  private commandBarShortcut: Shortcut = parseShortcut('Mod+Shift+p');

  override connectedCallback() {
    super.connectedCallback();
    void this.start();
    window.addEventListener('keydown', this.onGlobalKey);
    window.addEventListener('click', this.onWindowClick);
  }

  override disconnectedCallback() {
    this.unlisten?.();
    window.removeEventListener('keydown', this.onGlobalKey);
    window.removeEventListener('click', this.onWindowClick);
    super.disconnectedCallback();
  }

  /** Close the repo menu on any click outside it. */
  private onWindowClick = (event: MouseEvent) => {
    if (!this.repoMenuOpen) return;
    const path = event.composedPath();
    if (!path.some((node) => node instanceof HTMLElement && node.classList?.contains('repo-menu-root'))) {
      this.repoMenuOpen = false;
    }
  };

  private async toggleRepoMenu() {
    if (!this.repoMenuOpen) {
      this.recentRepos = await getRecentRepos();
    }
    this.repoMenuOpen = !this.repoMenuOpen;
  }

  /** Full reset after switching repos — nothing from the old repo may leak. */
  private async switchRepo(path: string) {
    this.repoMenuOpen = false;
    await this.run(async () => {
      await openRepository(path);
      this.selected = null;
      this.seededFor = null;
      this.focusPath = null;
      this.walkthrough = null;
      this.walkActive = false;
      this.walkStep = -1;
      this.stackReview = null;
      this.viewMode = 'full';
      this.sidebarTab = 'stack';
      await this.refresh();
    });
  }

  private async openFolder() {
    this.repoMenuOpen = false;
    const picked = await pickRepository();
    if (picked) {
      await this.switchRepo(picked);
    }
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
      this.wordWrap = config.ui.wordWrap;
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
      if (launch.revset) {
        // `jjdiff <revset>`: open on that change when it is in the loaded history.
        const target = this.repo?.graph.find(
          (change) =>
            change.changeId.startsWith(launch.revset!) ||
            change.commitId.startsWith(launch.revset!) ||
            change.bookmarks.includes(launch.revset!),
        );
        if (target) {
          this.select(target);
        }
      }
      if (launch.walkthroughFile) {
        // Agent-authored: import and enter guided review directly, no generation.
        const change = this.selectedChange;
        if (change) {
          await this.run(async () => {
            this.walkthrough = await importWalkthrough(
              change.changeId,
              this.isWorkingCopySelected ? null : change.changeId,
              this.ignoreWhitespace,
              launch.walkthroughFile!,
            );
            this.walkStale = false;
            this.walkActive = true;
            this.walkStep = -1;
            this.sidebarTab = 'steps';
          });
        }
      } else if (launch.walkthrough) {
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

  private get patchView(): PatchView | null {
    return this.querySelector('jj-patch-view');
  }

  private openSearch() {
    this.searchOpen = true;
    void this.updateComplete.then(() => {
      const input = this.querySelector<HTMLInputElement>('#diff-search');
      input?.focus();
      input?.select();
    });
  }

  private closeSearch() {
    this.searchOpen = false;
    this.searchQuery = '';
  }

  private onGlobalKey = (event: KeyboardEvent) => {
    if (matchesShortcut(event, this.commandBarShortcut)) {
      event.preventDefault();
      this.barOpen = !this.barOpen;
      return;
    }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f') {
      event.preventDefault();
      this.openSearch();
      return;
    }
    const typing = (event.target as HTMLElement | null)?.tagName === 'TEXTAREA'
      || (event.target as HTMLElement | null)?.tagName === 'INPUT';
    if (this.walkActive && !typing) {
      if (event.key === 'ArrowRight') {
        event.preventDefault();
        this.moveStep(1);
        return;
      }
      if (event.key === 'ArrowLeft') {
        event.preventDefault();
        this.moveStep(-1);
        return;
      }
    }
    if (event.key === 'Escape' && !typing) {
      if (this.searchOpen) {
        this.closeSearch();
        return;
      }
      if (this.walkActive) {
        this.exitWalkthrough();
        return;
      }
    }
    // Single-key review flow: j/k files, n/p hunks, v viewed.
    if (typing || this.barOpen || event.metaKey || event.ctrlKey || event.altKey) return;
    switch (event.key) {
      case 'j':
        event.preventDefault();
        this.patchView?.moveCursor('file', 1);
        break;
      case 'k':
        event.preventDefault();
        this.patchView?.moveCursor('file', -1);
        break;
      case 'n':
        event.preventDefault();
        this.patchView?.moveCursor('hunk', 1);
        break;
      case 'p':
        event.preventDefault();
        this.patchView?.moveCursor('hunk', -1);
        break;
      case 'v':
        event.preventDefault();
        this.patchView?.toggleViewedAtCursor();
        break;
    }
  };

  private get selectedChange(): Change | null {
    if (!this.repo) return null;
    const id = this.selected ?? this.repo.workingCopy.changeId;
    return (
      this.repo.stack.find((change) => change.changeId === id) ??
      this.repo.graph.find((change) => change.changeId === id) ??
      this.repo.workingCopy
    );
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
      // Expanded context belongs to the previous diff; drop it with the diff.
      this.fileLines = new Map();
      this.expansions = new Map();
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
    this.stackReview = null;
    this.sidebarTab = 'files';
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

  /** Index of the currently selected change within the stack-review order. */
  private get stackIndex(): number {
    if (!this.stackReview) return -1;
    const id = this.selected ?? this.repo?.workingCopy.changeId;
    return this.stackReview.findIndex((change) => change.changeId === id);
  }

  private revsetFor(change: Change): string | null {
    return change.workingCopy ? null : change.changeId;
  }

  /** Guided review of every reviewable change in the stack, oldest first (PR-style). */
  private reviewStack() {
    if (!this.repo || this.generating) return;
    const order = [...this.repo.stack]
      .filter((change) => !change.immutable && !change.empty)
      .reverse();
    if (order.length === 0) {
      this.actionInfo = 'Nothing to review in this stack.';
      return;
    }
    this.generating = true;
    void this.run(async () => {
      const ready: Change[] = [];
      for (const [index, change] of order.entries()) {
        const status = await getWalkthrough(
          change.changeId,
          this.revsetFor(change),
          this.ignoreWhitespace,
        );
        if (status.walkthrough && !status.stale) {
          ready.push(change);
          continue;
        }
        this.actionInfo = `Generating walkthrough ${index + 1}/${order.length} — ${
          change.description.split('\n')[0] || change.changeId.slice(0, 8)
        }…`;
        try {
          await generateWalkthrough(
            change.changeId,
            this.revsetFor(change),
            this.ignoreWhitespace,
            `change ${change.changeId.slice(0, 8)}: ${
              change.description.split('\n')[0] || '(no description)'
            }`,
          );
          ready.push(change);
        } catch (error) {
          // A change that can't be walked (e.g. only binary files) is skipped, not fatal.
          this.actionInfo = `Skipped ${change.changeId.slice(0, 8)}: ${String(error)}`;
        }
      }
      if (ready.length === 0) {
        throw new Error('no change in the stack produced a walkthrough');
      }
      this.stackReview = ready;
      this.actionInfo = null;
      await this.enterStackChange(ready[0]!, 'overview');
    }).finally(() => {
      this.generating = false;
    });
  }

  /** Move to a change within stack review, keeping guided mode on. */
  private async enterStackChange(change: Change, position: 'overview' | 'last') {
    this.selected = change.changeId;
    this.focusPath = null;
    this.viewMode = 'full';
    this.description = change.description;
    this.seededFor = change.changeId;
    await Promise.all([this.loadDiff(), this.loadReview(), this.loadConflicts()]);
    await this.loadWalkthrough();
    this.walkActive = true;
    this.walkStep =
      position === 'last' && this.walkthrough ? this.walkthrough.steps.length - 1 : -1;
    this.sidebarTab = 'steps';
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
      this.sidebarTab = 'steps';
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
    this.sidebarTab = 'steps';
  }

  private exitWalkthrough() {
    this.walkActive = false;
    this.walkStep = -1;
    this.stackReview = null;
    if (this.sidebarTab === 'steps') {
      this.sidebarTab = 'files';
    }
  }

  private moveStep(delta: number) {
    if (!this.walkthrough) return;
    const next = this.walkStep + delta;
    if (next >= -1 && next < this.walkthrough.steps.length) {
      this.walkStep = next;
      return;
    }
    // Past either end: in stack review, cross into the neighboring change.
    if (!this.stackReview) return;
    const index = this.stackIndex;
    if (delta > 0 && index >= 0 && index + 1 < this.stackReview.length) {
      void this.enterStackChange(this.stackReview[index + 1]!, 'overview');
    } else if (delta < 0 && index > 0) {
      void this.enterStackChange(this.stackReview[index - 1]!, 'last');
    }
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

  private toggleWordWrap() {
    this.wordWrap = !this.wordWrap;
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
      { id: 'find', label: 'Find in Diffs', hint: 'Mod+F', run: () => this.openSearch() },
      {
        id: 'wrap',
        label: this.wordWrap ? 'Disable Word Wrap' : 'Enable Word Wrap',
        run: () => this.toggleWordWrap(),
      },
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
        label: 'Refresh Walkthrough (Claude)',
        run: () => this.runGenerateWalkthrough(),
      });
    }
    if ((this.repo?.stack.filter((c) => !c.immutable && !c.empty).length ?? 0) > 1) {
      commands.push({
        id: 'stack-review',
        label: this.stackReview ? 'Exit Stack Review' : 'Review Stack (guided)',
        run: () => (this.stackReview ? this.exitWalkthrough() : this.reviewStack()),
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
      return html`<div class="fatal">
        <div class="card">
          <h2>jjdiff can't open this repository</h2>
          <pre>${this.error}</pre>
        </div>
      </div>`;
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
        <span class="repo-menu-root">
          <button class="repo-button" @click=${this.toggleRepoMenu} title=${this.repo.root}>
            <span class="repo-icon">${folderIcon(false)}</span>
            <span class="root">${basename(this.repo.root)}</span>
            <span class="caret">▾</span>
          </button>
          ${this.repoMenuOpen
            ? html`<div class="repo-menu">
                ${this.recentRepos.map(
                  (path) => html`
                    <button class="repo-item" @click=${() => void this.switchRepo(path)}>
                      <span class="repo-icon">${folderIcon(false)}</span>
                      <span class="repo-name">${basename(path)}</span>
                      <span class="repo-path">${path}</span>
                    </button>
                  `,
                )}
                <button class="repo-item open-folder" @click=${() => void this.openFolder()}>
                  Open Folder…
                </button>
              </div>`
            : nothing}
        </span>
        <span class="spacer"></span>
        ${this.repo.stack.filter((c) => !c.immutable && !c.empty).length > 1
          ? html`<button
              class="tool ${this.stackReview ? 'on' : ''} ${this.generating ? 'generating' : ''}"
              ?disabled=${this.generating}
              title="Guided review of every change in the stack, oldest first"
              @click=${() => (this.stackReview ? this.exitWalkthrough() : this.reviewStack())}
            >
              ${this.stackReview ? 'Exit Stack Review' : 'Review Stack'}
            </button>`
          : nothing}
        <button
          class="tool ${this.walkActive && !this.stackReview ? 'on' : ''} ${this.generating ? 'generating' : ''}"
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
        <button
          class="tool ${this.wordWrap ? 'on' : ''}"
          @click=${this.toggleWordWrap}
          title="Wrap long diff lines"
        >
          Wrap
        </button>
      </header>
      <aside>
        <nav class="tabs">
          <button
            class="tab ${this.sidebarTab === 'stack' ? 'active' : ''}"
            @click=${() => (this.sidebarTab = 'stack')}
          >
            Log
          </button>
          <button
            class="tab ${this.sidebarTab === 'files' ? 'active' : ''}"
            @click=${() => (this.sidebarTab = 'files')}
          >
            Files
            <span class="count">${this.files.length}</span>
          </button>
          ${this.walkActive && this.walkthrough
            ? html`<button
                class="tab ${this.sidebarTab === 'steps' ? 'active' : ''}"
                @click=${() => (this.sidebarTab = 'steps')}
              >
                Steps
                <span class="count">${this.walkthrough.steps.length}</span>
                ${this.walkStale ? html`<span class="stale-dot" title="Content changed"></span>` : nothing}
              </button>`
            : nothing}
        </nav>
        ${this.sidebarTab === 'stack'
          ? html`<div class="stack">
              <jj-log-graph
                .changes=${this.repo.graph}
                .selected=${selectedId}
                @change-selected=${(event: CustomEvent<Change>) => this.select(event.detail)}
              ></jj-log-graph>
            </div>`
          : html`
              ${change
                ? html`<button class="context-card" @click=${() => (this.sidebarTab = 'stack')}>
                    <span class="id">${change.changeId.slice(0, 8)}</span>
                    ${change.workingCopy ? html`<span class="badge">@</span>` : nothing}
                    <span class="desc ${change.description ? '' : 'empty-desc'}">
                      ${change.description.split('\n')[0] || '(no description)'}
                    </span>
                    ${this.viewedPaths.size
                      ? html`<span class="progress">${this.viewedPaths.size}/${this.files.length} viewed</span>`
                      : nothing}
                  </button>`
                : nothing}
              <div class="files">
                ${this.sidebarTab === 'steps' && this.walkthrough
                  ? html`<jj-walkthrough-panel
                      .walkthrough=${this.walkthrough}
                      .files=${this.files}
                      .viewed=${this.viewedPaths}
                      .current=${this.walkStep}
                      @step-selected=${(event: CustomEvent<number>) => {
                        this.walkStep = event.detail;
                      }}
                    ></jj-walkthrough-panel>`
                  : html`<jj-file-tree
                      .files=${this.files}
                      .selected=${this.focusPath}
                      .viewed=${this.viewedPaths}
                      @file-selected=${(event: CustomEvent<string | null>) => {
                        this.focusPath = event.detail;
                      }}
                    ></jj-file-tree>`}
              </div>
            `}
      </aside>
      <main
        @squash-file=${this.onSquashFile}
        @toggle-viewed=${this.onToggleViewed}
        @search-state=${(event: CustomEvent<{ count: number; current: number }>) => {
          this.searchCount = event.detail.count;
          this.searchCurrent = event.detail.current;
        }}
        @visible-file=${(event: CustomEvent<{ path: string }>) => {
          this.visibleFile = event.detail.path;
        }}
        @expand-context=${this.onExpandContext}
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
              ${keyed(
                this.walkStep,
                html`<div class="walk-content">
              <div class="walk-head">
                <span class="walk-progress">
                  ${this.stackReview
                    ? `Change ${this.stackIndex + 1}/${this.stackReview.length} · `
                    : ''}${this.walkStep < 0
                    ? 'Overview'
                    : `Step ${this.walkStep + 1} of ${this.walkthrough.steps.length}`}
                </span>
                <strong>
                  ${this.walkStep < 0
                    ? 'Guided review'
                    : this.walkthrough.steps[this.walkStep]?.title}
                </strong>
                <span class="spacer"></span>
                <button
                  class="tool"
                  ?disabled=${this.walkStep <= -1 && !(this.stackReview && this.stackIndex > 0)}
                  @click=${() => this.moveStep(-1)}
                >
                  ← Prev
                </button>
                <button
                  class="tool primary"
                  ?disabled=${this.walkStep >= this.walkthrough.steps.length - 1 &&
                  !(this.stackReview && this.stackIndex + 1 < this.stackReview.length)}
                  @click=${() => this.moveStep(1)}
                >
                  ${this.walkStep >= this.walkthrough.steps.length - 1 && this.stackReview
                    ? 'Next Change →'
                    : 'Next →'}
                </button>
              </div>
              <p class="walk-narrative">
                ${this.walkStep < 0
                  ? this.walkthrough.summary
                  : this.walkthrough.steps[this.walkStep]?.narrative}
              </p>
                </div>`,
              )}
            </div>`
          : nothing}
        ${this.walkthrough && this.walkStale && !this.generating
          ? html`<div class="banner">
              The walkthrough was generated for an older version of this change.
              <span class="spacer"></span>
              <button class="tool" @click=${this.runGenerateWalkthrough}>Refresh Walkthrough</button>
            </div>`
          : nothing}
        ${this.searchOpen
          ? html`<div class="search-bar">
              <input
                id="diff-search"
                placeholder="Find in diffs…"
                .value=${this.searchQuery}
                @input=${(event: Event) =>
                  (this.searchQuery = (event.target as HTMLInputElement).value)}
                @keydown=${(event: KeyboardEvent) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    this.patchView?.moveMatch(event.shiftKey ? -1 : 1);
                  } else if (event.key === 'Escape') {
                    event.preventDefault();
                    this.closeSearch();
                  }
                }}
              />
              <span class="matches">
                ${this.searchQuery.trim()
                  ? this.searchCount > 0
                    ? `${this.searchCurrent + 1}/${this.searchCount}`
                    : 'no matches'
                  : ''}
              </span>
              <button class="tool" @click=${() => this.patchView?.moveMatch(-1)}>↑</button>
              <button class="tool" @click=${() => this.patchView?.moveMatch(1)}>↓</button>
              <button class="tool" @click=${this.closeSearch}>Esc</button>
            </div>`
          : nothing}
        ${this.actionError
          ? html`<div class="status error">${this.actionError}</div>`
          : nothing}
        ${this.actionInfo ? html`<div class="status info">${this.actionInfo}</div>` : nothing}
        ${this.visibleFile && visible.length > 1
          ? html`<div class="crumb" title=${this.visibleFile}>
              <span class="crumb-dir">${dirname(this.visibleFile)}</span>
              <span class="crumb-name">${basename(this.visibleFile)}</span>
            </div>`
          : nothing}
        <jj-patch-view
          .files=${visible}
          .layout=${this.layout}
          .viewed=${this.viewedPaths}
          .canSquash=${isWc && this.viewMode === 'full' && !this.walkActive && this.squashTargets.length > 0}
          .canMarkViewed=${this.viewMode === 'full'}
          .squashTargets=${this.squashTargets}
          .conflicted=${this.conflictedPaths}
          .hunkFilter=${this.walkFilter}
          .searchQuery=${this.searchOpen ? this.searchQuery : null}
          .wordWrap=${this.wordWrap}
          .fileLines=${this.fileLines}
          .expansions=${this.expansions}
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

  /** Pull ~20 more lines of context around a hunk, fetching the file once. */
  private onExpandContext = async (
    event: CustomEvent<{ path: string; hunkId: string; direction: 'up' | 'down' }>,
  ) => {
    const { path, hunkId, direction } = event.detail;
    const STEP = 20;
    try {
      if (!this.fileLines.has(path)) {
        const text = await getFileContent(
          this.isWorkingCopySelected ? null : this.selected,
          path,
        );
        const next = new Map(this.fileLines);
        next.set(path, text.split('\n'));
        this.fileLines = next;
      }
      const current = this.expansions.get(hunkId) ?? { up: 0, down: 0 };
      const expanded = new Map(this.expansions);
      expanded.set(hunkId, {
        up: direction === 'up' ? current.up + STEP : current.up,
        down: direction === 'down' ? current.down + STEP : current.down,
      });
      this.expansions = expanded;
    } catch (error) {
      this.actionError = String(error);
    }
  };

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
const dirname = (path: string) => {
  const cut = path.lastIndexOf('/');
  return cut === -1 ? '' : `${path.slice(0, cut)}/`;
};

declare global {
  interface HTMLElementTagNameMap {
    'jj-app': App;
  }
}
