import { html, LitElement, nothing } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { keyed } from 'lit/directives/keyed.js';
import { repeat } from 'lit/directives/repeat.js';

import './command-bar.js';
import type { Command } from './command-bar.js';
import './file-tree.js';
import './log-graph.js';
import {
  abandonChange,
  absorb,
  backoutChange,
  deleteBookmark,
  duplicateChange,
  editChange,
  getOperationLog,
  getRemotes,
  gitFetch,
  gitPush,
  rebaseChange,
  restoreOperation,
  restorePaths,
  setBookmark,
  splitPaths,
  undo,
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
  installTerminalHelper,
  markReviewed,
  newChange,
  onRepoChanged,
  onSecondInstance,
  openRepository,
  pickRepository,
  setViewed,
  squashPaths,
  addComment,
  type Change,
  type Comment,
  type CommentSide,
  deleteComment,
  exportReviewMarkdown,
  type FilePatch,
  type RepoState,
  listComments,
  refreshCommentAnchors,
  type Operation,
  type Outcome,
  type SecondInstanceArgs,
  setCommentResolved,
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

/** Revsets people actually reach for; the empty one restores the default view. */
const REVSET_PRESETS: { label: string; revset: string }[] = [
  { label: 'All', revset: '' },
  { label: 'Stack', revset: 'trunk()..@ | @' },
  { label: 'Recent', revset: 'ancestors(@, 50)' },
  { label: 'Mine', revset: 'mine()' },
  { label: 'Conflicts', revset: 'conflicts()' },
  { label: 'Bookmarks', revset: 'ancestors(bookmarks(), 5)' },
];

/** Split a jj description into subject + body, mail-client style. */
function descriptionParts(description: string) {
  const [subject = '', ...rest] = description.split('\n');
  const body = rest.join('\n').trim();
  return html`<span class="detail-subject">${subject || '(no description)'}</span
    >${body ? html`<span class="detail-body">${body}</span>` : nothing}`;
}

/** Compact relative age: now, 5m, 3h, 2d. */
function relativeTime(timestamp: string): string {
  const then = Date.parse(timestamp);
  if (Number.isNaN(then)) return '';
  const seconds = Math.max(0, (Date.now() - then) / 1000);
  if (seconds < 60) return 'now';
  const minutes = seconds / 60;
  if (minutes < 60) return `${Math.floor(minutes)}m ago`;
  const hours = minutes / 60;
  if (hours < 24) return `${Math.floor(hours)}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

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
  @state() private sidebarTab: 'stack' | 'files' | 'steps' | 'ops' | 'review' = 'stack';
  /** Non-working-copy selection opens the detail view instead of jumping to Files. */
  @state() private detailView = false;
  /** Collapsed detail block: sticks across selections, so "hide it" means hide it. */
  @state() private detailCollapsed = false;
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
  /** Last mutation's narration + the operation that would undo it. */
  @state() private lastOutcome: (Outcome & { pullRequestUrl?: string | null }) | null = null;
  @state() private operations: Operation[] = [];
  @state() private busy: string | null = null;
  /** Revset scoping the Log graph; null = the default. */
  @state() private graphRevset: string | null = null;
  @state() private revsetDraft = '';
  /** "system" | "light" | "dark" — runtime override of the config value. */
  @state() private theme: 'system' | 'light' | 'dark' = 'system';
  /** Bumped on theme change so the diff re-tokenizes (shiki tokens carry colours). */
  @state() private themeVersion = 0;
  /** File the diff viewport is currently inside (sticky breadcrumb). */
  @state() private visibleFile: string | null = null;
  /** Full file text for context expansion, fetched on demand. */
  @state() private fileLines: ReadonlyMap<string, string[]> = new Map();
  @state() private expansions: ReadonlyMap<string, { up: number; down: number }> = new Map();
  /** Inline review comments keyed `${path}:${side}:${line}`. */
  @state() private comments: ReadonlyMap<string, Comment[]> = new Map();
  /** All comments for the selected change (for the Review tab). */
  @state() private allComments: Comment[] = [];
  /** Paths in markdown-preview mode → rendered HTML. */
  @state() private markdownPreviews: ReadonlyMap<string, string> = new Map();

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

  /**
   * `jjdiff` launched again while the app is running: open the repo in the
   * existing window. If the second invocation pointed at the same repo, a
   * plain refresh keeps it simple; otherwise we switch as `openFolder` does.
   * Revset/walkthrough flags reapply through the same launch-options path.
   */
  private async handleSecondInstance(args: SecondInstanceArgs) {
    if (args.repoPath) {
      const current = this.repo?.root;
      if (current && current !== args.repoPath) {
        await this.switchRepo(args.repoPath);
      } else {
        await this.refresh();
      }
    }
    if (args.revset) {
      const target = this.repo?.graph.find(
        (change) =>
          change.changeId.startsWith(args.revset!) ||
          change.commitId.startsWith(args.revset!) ||
          change.bookmarks.includes(args.revset!),
      );
      if (target) this.select(target);
    }
    if (args.walkthrough) {
      this.runGenerateWalkthrough();
    }
  }

  /** Write the `jjdiff` shim on PATH; surface the report in the status bar. */
  private async runInstallTerminalHelper() {
    this.busy = 'install-terminal-helper';
    try {
      const report = await installTerminalHelper();
      // `lastOutcome` is the success toast; reuse it even though this isn't a
      // jj mutation — the report reads like one ("Installed `jjdiff` on PATH").
      this.lastOutcome = { message: report, operation: '' };
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
      this.lastOutcome = null;
    } finally {
      this.busy = null;
    }
  }

  // ---- Inline review comments ----

  private async onAddComment(
    detail: { path: string; side: CommentSide; line: number; lineText: string; body: string; parentId: number | null },
  ) {
    const change = this.selectedChange;
    if (!change || !detail.body.trim()) return;
    try {
      await addComment(
        change.changeId,
        detail.path,
        `${detail.path}#0`, // hunk id is approximate; the store keys by change+path+line anyway
        detail.side,
        detail.line,
        detail.lineText,
        change.commitId,
        'you',
        detail.body,
        detail.parentId,
      );
      await this.loadComments();
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
    }
  }

  private async onResolveComment(id: number, resolved: boolean) {
    try {
      await setCommentResolved(id, resolved);
      await this.loadComments();
    } catch (error) {
      this.actionError = String(error);
    }
  }

  private async onDeleteComment(id: number) {
    try {
      await deleteComment(id);
      await this.loadComments();
    } catch (error) {
      this.actionError = String(error);
    }
  }

  /** Copy pending comments as a Markdown review to the clipboard. */
  private async copyReviewMarkdown() {
    const change = this.selectedChange;
    if (!change) return;
    try {
      const md = await exportReviewMarkdown(change.changeId);
      await navigator.clipboard.writeText(md);
      this.lastOutcome = { message: 'Copied review as Markdown to clipboard.', operation: '' };
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
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
        this.applyTheme(config.ui.theme);
      }
      this.commandBarShortcut = parseShortcut(config.keymap.commandBar);
    } catch {
      // Config is best-effort; defaults are fine.
    }
    await this.refresh();
    void onRepoChanged(() => void this.refresh()).then((unlisten) => {
      this.unlisten = unlisten;
    });
    // Single instance: launching `jjdiff` from a second repo while the app is
    // running forwards its parsed argv here. Open the repo in the existing
    // window rather than starting a rival process.
    void onSecondInstance((args) => void this.handleSecondInstance(args));
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

  /** Unresolved comments for the selected change (Review tab badge + list). */
  private get pendingComments(): Comment[] {
    return this.allComments.filter((c) => !c.resolved);
  }

  /** Scroll the diff to the file owning a comment and focus it. */
  private scrollToComment(comment: Comment) {
    this.focusPath = comment.path;
    this.sidebarTab = 'files';
    this.patchView?.scrollToPath(comment.path);
  }

  private async refresh() {
    try {
      this.repo = await getRepoState(this.graphRevset ?? undefined);
      this.error = null;
      // Seed the description editor only when the selection target changed — a background
      // refresh must not clobber what the user is typing.
      const current = this.selectedChange;
      if (current && this.seededFor !== current.changeId) {
        this.description = current.description;
        this.seededFor = current.changeId;
      }
      await Promise.all([
        this.loadDiff(),
        this.loadReview(),
        this.loadConflicts(),
        this.loadWalkthrough(),
        this.loadComments(),
      ]);
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

  /** Load comments for the selected change + re-anchor against the current diff. */
  private async loadComments() {
    const change = this.selectedChange;
    if (!change) {
      this.comments = new Map();
      this.allComments = [];
      return;
    }
    try {
      // Re-anchor if the change has evolved since comments were written.
      await refreshCommentAnchors(
        change.changeId,
        change.commitId,
        this.isWorkingCopySelected ? null : change.changeId,
        this.ignoreWhitespace,
      );
      const list = await listComments(change.changeId);
      this.allComments = list;
      // Index by `${path}:${side}:${line}` for the diff view.
      const map = new Map<string, Comment[]>();
      for (const comment of list) {
        const key = `${comment.path}:${comment.side}:${comment.line}`;
        const existing = map.get(key);
        if (existing) existing.push(comment);
        else map.set(key, [comment]);
      }
      this.comments = map;
    } catch {
      this.comments = new Map();
      this.allComments = [];
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
    // The working copy keeps the edit-first layout; anything else opens the detail view
    // rather than jumping to Files, which used to throw away the change's identity.
    this.detailView = !change.workingCopy;
    // Seed from the clicked change itself: older changes live in `graph`, not `stack`,
    // so a stack-only lookup silently blanked their description.
    this.description = change.description;
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

  /** Run a jj mutation: capture its narration for the toast, refresh, surface errors. */
  private async command(label: string, action: () => Promise<Outcome>) {
    if (this.busy) return;
    this.busy = label;
    try {
      const outcome = await action();
      this.lastOutcome = outcome;
      this.actionError = null;
      await this.refresh();
      if (this.sidebarTab === 'ops') {
        await this.loadOperations();
      }
    } catch (error) {
      this.actionError = String(error);
      this.lastOutcome = null;
    } finally {
      this.busy = null;
    }
  }

  /** Scope the Log graph. jj validates the revset; its error is surfaced verbatim. */
  private applyRevset(revset: string) {
    const next = revset.trim();
    this.graphRevset = next === '' ? null : next;
    this.revsetDraft = next;
    void this.refresh();
  }

  private async loadOperations() {
    try {
      this.operations = await getOperationLog(100);
    } catch (error) {
      this.actionError = String(error);
    }
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
    if (!change || change.immutable) return;
    void this.command('describe', () => describeChange(change.changeId, this.description));
  }

  private commitAndNew() {
    const change = this.selectedChange;
    if (!change) return;
    void this.command('commit', async () => {
      await describeChange(change.changeId, this.description);
      const outcome = await newChange();
      this.selected = null;
      this.seededFor = null;
      this.detailView = false;
      return outcome;
    });
  }

  private runAbsorb() {
    void this.command('absorb', () => absorb());
  }

  /** Actions available on the selected change, gated by jj's own rules. */
  private editSelected() {
    const change = this.selectedChange;
    if (!change) return;
    void this.command('edit', async () => {
      const outcome = await editChange(change.changeId);
      this.selected = null;
      this.seededFor = null;
      this.detailView = false;
      return outcome;
    });
  }

  private newOnSelected() {
    const change = this.selectedChange;
    if (!change) return;
    void this.command('new', async () => {
      const outcome = await newChange([change.changeId]);
      this.selected = null;
      this.seededFor = null;
      this.detailView = false;
      return outcome;
    });
  }

  private abandonSelected() {
    const change = this.selectedChange;
    if (!change || change.immutable) return;
    const label = change.description.split('\n')[0] || change.changeId.slice(0, 8);
    if (!confirm(`Abandon "${label}"?\n\nUndoable from the Ops tab.`)) return;
    void this.command('abandon', async () => {
      const outcome = await abandonChange(change.changeId);
      this.selected = null;
      this.detailView = false;
      return outcome;
    });
  }

  private duplicateSelected() {
    const change = this.selectedChange;
    if (!change) return;
    void this.command('duplicate', () => duplicateChange(change.changeId));
  }

  private backoutSelected() {
    const change = this.selectedChange;
    if (!change) return;
    void this.command('backout', () => backoutChange(change.changeId));
  }

  private rebaseSelected() {
    const change = this.selectedChange;
    if (!change || change.immutable || !this.repo) return;
    const destination = prompt(
      'Rebase onto which revision?\n\nA change id, bookmark, or revset (e.g. main, @-).',
      'main',
    );
    if (!destination) return;
    void this.command('rebase', () =>
      rebaseChange('source', change.changeId, destination.trim()),
    );
  }

  private splitSelectedFiles() {
    const change = this.selectedChange;
    if (!change || change.immutable) return;
    const paths = this.focusPath ? [this.focusPath] : [...this.viewedPaths];
    if (paths.length === 0) {
      this.actionError =
        'Select a file (or mark the files to keep as viewed) before splitting.';
      return;
    }
    void this.command('split', () => splitPaths(change.changeId, paths));
  }

  private restoreSelectedFile() {
    if (!this.isWorkingCopySelected) return;
    const paths = this.focusPath ? [this.focusPath] : [];
    const what = paths.length ? paths[0] : 'ALL working-copy changes';
    if (!confirm(`Discard ${what}?\n\nUndoable from the Ops tab.`)) return;
    void this.command('restore', () => restorePaths(paths));
  }

  private createBookmark() {
    const change = this.selectedChange;
    if (!change) return;
    const name = prompt('Bookmark name:');
    if (!name?.trim()) return;
    void this.command('bookmark', () => setBookmark(name.trim(), change.changeId));
  }

  private removeBookmark(name: string) {
    if (!confirm(`Delete bookmark "${name}"?`)) return;
    void this.command('bookmark', () => deleteBookmark(name));
  }

  private runFetch() {
    void this.command('fetch', () => gitFetch());
  }

  /** Push the selected change: an existing bookmark if it has one, else --change. */
  private runPush() {
    const change = this.selectedChange;
    if (!change) return;
    const bookmark = change.bookmarks[0];
    void this.command('push', async () => {
      const result = await gitPush(
        bookmark ? { bookmark } : { change: change.changeId },
      );
      this.lastOutcome = result;
      return result;
    });
  }

  private runUndo() {
    void this.command('undo', () => undo());
  }

  private restoreTo(operation: Operation) {
    if (
      !confirm(
        `Restore the repository to just after:\n\n${operation.description}\n\n` +
          'This rewrites the working copy. It is itself undoable.',
      )
    ) {
      return;
    }
    void this.command('op restore', () => restoreOperation(operation.id));
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

  /** Switch palette at runtime. Shiki tokens are theme-specific, so bump themeVersion
   *  to force re-tokenization; CSS variables handle the rest. */
  private applyTheme(theme: 'system' | 'light' | 'dark') {
    this.theme = theme;
    const root = document.documentElement;
    if (theme === 'system') {
      delete root.dataset['jjTheme'];
      root.style.colorScheme = '';
    } else {
      root.dataset['jjTheme'] = theme;
      root.style.colorScheme = theme;
    }
    this.themeVersion += 1;
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
    const change = this.selectedChange;
    const isWc = this.isWorkingCopySelected;
    const mutable = !!change && !change.immutable;
    const stackSize = this.repo?.stack.filter((c) => !c.immutable && !c.empty).length ?? 0;
    const commands: Command[] = [];
    const add = (group: string, entries: (Command | false)[]) => {
      for (const entry of entries) {
        if (entry) commands.push({ ...entry, group });
      }
    };

    add('Presentation', [
      {
        id: 'layout',
        label: `Diff Layout: ${this.layout === 'split' ? 'Split' : 'Unified'}`,
        hint: 'switch',
        run: () => this.toggleLayout(),
      },
      {
        id: 'wrap',
        label: this.wordWrap ? 'Word Wrap: On' : 'Word Wrap: Off',
        hint: 'toggle',
        run: () => this.toggleWordWrap(),
      },
      {
        id: 'whitespace',
        label: this.ignoreWhitespace ? 'Whitespace: Hidden' : 'Whitespace: Shown',
        hint: 'toggle',
        run: () => this.toggleWhitespace(),
      },
      {
        id: 'theme-system',
        label: 'Theme: System',
        hint: this.theme === 'system' ? 'current' : undefined,
        run: () => this.applyTheme('system'),
      },
      {
        id: 'theme-light',
        label: 'Theme: Light',
        hint: this.theme === 'light' ? 'current' : undefined,
        run: () => this.applyTheme('light'),
      },
      {
        id: 'theme-dark',
        label: 'Theme: Dark',
        hint: this.theme === 'dark' ? 'current' : undefined,
        run: () => this.applyTheme('dark'),
      },
    ]);

    add('Review', [
      { id: 'find', label: 'Find in Diffs', hint: 'Mod+F', run: () => this.openSearch() },
      this.walkthrough
        ? {
            id: 'walkthrough',
            label: this.walkActive ? 'Exit Walkthrough' : 'Start Walkthrough',
            run: () => (this.walkActive ? this.exitWalkthrough() : this.startWalkthrough()),
          }
        : {
            id: 'walkthrough',
            label: 'Generate Walkthrough',
            run: () => this.runGenerateWalkthrough(),
          },
      !!this.walkthrough && {
        id: 'regen-walkthrough',
        label: 'Refresh Walkthrough',
        run: () => this.runGenerateWalkthrough(),
      },
      stackSize > 1 && {
        id: 'stack-review',
        label: this.stackReview ? 'Exit Stack Review' : 'Review Stack (guided)',
        run: () => (this.stackReview ? this.exitWalkthrough() : this.reviewStack()),
      },
      !isWc && {
        id: 'reviewed',
        label: 'Mark Change Reviewed',
        run: () => this.markCurrentReviewed(),
      },
      this.changedSinceReview && {
        id: 'interdiff',
        label: 'Show Changes Since Last Review',
        run: () => this.showInterdiff(),
      },
      !!this.focusPath && {
        id: 'unfocus',
        label: 'Clear File Focus',
        run: () => (this.focusPath = null),
      },
      !!change && {
        id: 'review-tab',
        label: 'Open Review Tab',
        hint: 'comments',
        run: () => (this.sidebarTab = 'review'),
      },
      !!change && {
        id: 'copy-review-md',
        label: 'Copy Review as Markdown',
        run: () => void this.copyReviewMarkdown(),
      },
    ]);

    add('Change', [
      mutable && { id: 'jj-edit', label: 'Work on This Change (jj edit)', run: () => this.editSelected() },
      { id: 'jj-new', label: 'New Change on Top (jj new)', run: () => this.newOnSelected() },
      isWc && { id: 'jj-absorb', label: 'Absorb Into Ancestors (jj absorb)', run: () => this.runAbsorb() },
      mutable && { id: 'jj-rebase', label: 'Rebase…  (jj rebase)', run: () => this.rebaseSelected() },
      mutable && { id: 'jj-split', label: 'Split File Out (jj split)', run: () => this.splitSelectedFiles() },
      { id: 'jj-duplicate', label: 'Duplicate Change (jj duplicate)', run: () => this.duplicateSelected() },
      { id: 'jj-backout', label: 'Back Out Change (jj backout)', run: () => this.backoutSelected() },
      mutable && { id: 'jj-abandon', label: 'Abandon Change (jj abandon)', run: () => this.abandonSelected() },
      isWc && {
        id: 'jj-restore',
        label: 'Discard Working-Copy Changes (jj restore)',
        run: () => this.restoreSelectedFile(),
      },
    ]);

    add('Repository', [
      { id: 'jj-fetch', label: 'Fetch (jj git fetch)', run: () => this.runFetch() },
      { id: 'jj-push', label: 'Push (jj git push)', run: () => this.runPush() },
      { id: 'jj-bookmark', label: 'Create Bookmark…', run: () => this.createBookmark() },
      { id: 'refresh', label: 'Reload Repository', run: () => void this.refresh() },
      { id: 'open-repo', label: 'Open Repository…', run: () => void this.openFolder() },
      {
        id: 'install-terminal-helper',
        label: 'Install Terminal Helper…',
        hint: 'add `jjdiff` to PATH',
        run: () => void this.runInstallTerminalHelper(),
      },
    ]);

    add('History', [
      { id: 'jj-undo', label: 'Undo Last Operation (jj undo)', run: () => this.runUndo() },
      {
        id: 'ops',
        label: 'Show Operation Log',
        run: () => {
          this.sidebarTab = 'ops';
          void this.loadOperations();
        },
      },
    ]);

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
        <button
          class="tool ${this.walkActive ? 'on' : ''} ${this.generating ? 'generating' : ''}"
          ?disabled=${this.generating || this.files.length === 0}
          title=${
            this.walkthrough
              ? 'Guided review of this change'
              : 'Generate a guided review with an agent CLI'
          }
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
        <button
          class="tool"
          title="jj git fetch — update remote-tracking state"
          ?disabled=${!!this.busy}
          @click=${this.runFetch}
        >
          Fetch
        </button>
        <button
          class="tool"
          title="jj undo — reverse the last operation"
          ?disabled=${!!this.busy}
          @click=${this.runUndo}
        >
          Undo
        </button>
        <button
          class="tool"
          title="Switch between side-by-side and unified diffs"
          @click=${this.toggleLayout}
        >
          ${this.layout === 'split' ? 'Split' : 'Unified'}
        </button>
        <button
          class="tool"
          title="Everything else lives here (Mod+Shift+P)"
          @click=${() => (this.barOpen = true)}
        >
          ⌘K
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
          <button
            class="tab ${this.sidebarTab === 'ops' ? 'active' : ''}"
            @click=${() => {
              this.sidebarTab = 'ops';
              void this.loadOperations();
            }}
          >
            Ops
          </button>
          <button
            class="tab ${this.sidebarTab === 'review' ? 'active' : ''}"
            @click=${() => (this.sidebarTab = 'review')}
          >
            Review
            ${this.pendingComments.length > 0
              ? html`<span class="count">${this.pendingComments.length}</span>`
              : nothing}
          </button>
        </nav>
        ${this.sidebarTab === 'stack'
          ? html`<div class="revset-bar">
                ${REVSET_PRESETS.map(
                  (preset) => html`<button
                    class="preset ${(this.graphRevset ?? '') === preset.revset ? 'on' : ''}"
                    title=${preset.revset}
                    @click=${() => this.applyRevset(preset.revset)}
                  >
                    ${preset.label}
                  </button>`,
                )}
                <input
                  class="revset-input"
                  placeholder="revset…"
                  .value=${this.revsetDraft}
                  @input=${(event: Event) =>
                    (this.revsetDraft = (event.target as HTMLInputElement).value)}
                  @keydown=${(event: KeyboardEvent) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      this.applyRevset(this.revsetDraft);
                    }
                  }}
                />
              </div>
              <div class="stack">
              <jj-log-graph
                .changes=${this.repo.graph}
                .selected=${selectedId}
                @change-selected=${(event: CustomEvent<Change>) => this.select(event.detail)}
              ></jj-log-graph>
              </div>`
          : this.sidebarTab === 'ops'
            ? html`<div class="files">
                ${this.operations.length === 0
                  ? html`<div class="ops-empty">No operations recorded yet.</div>`
                  : this.operations
                      .filter((operation) => !operation.snapshot)
                      .map(
                        (operation, index) => html`<div class="op">
                          <div class="op-head">
                            <span class="op-when">${relativeTime(operation.time)}</span>
                            ${index === 0
                              ? html`<span class="op-current">current</span>`
                              : nothing}
                          </div>
                          <div class="op-desc">${operation.description}</div>
                          ${operation.args
                            ? html`<code class="op-args">${operation.args}</code>`
                            : nothing}
                          <div class="op-actions">
                            ${index === 0
                              ? html`<button class="tool" @click=${this.runUndo}>Undo</button>`
                              : html`<button
                                  class="tool"
                                  @click=${() => this.restoreTo(operation)}
                                >
                                  Restore here
                                </button>`}
                          </div>
                        </div>`,
                      )}
              </div>`
          : this.sidebarTab === 'review'
            ? html`<div class="review-list">
                <button class="tool review-export" @click=${() => void this.copyReviewMarkdown()}>
                  Copy as Markdown
                </button>
                ${this.pendingComments.length === 0
                  ? html`<div class="ops-empty">No pending comments.</div>`
                  : this.pendingComments.map(
                      (comment) => html`<div
                        class="review-item ${comment.outdated ? 'outdated' : ''}"
                        @click=${() => this.scrollToComment(comment)}
                      >
                        <span class="review-path">${comment.path}</span>
                        <span class="review-line">line ${comment.line}${comment.outdated ? ' (outdated)' : ''}</span>
                        <div class="review-snippet">${comment.body.split('\n')[0]}</div>
                      </div>`,
                    )}
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
        @toggle-markdown=${(e: CustomEvent<{ path: string }>) => this.onToggleMarkdown(e.detail.path)}
      >
        ${change && this.detailView
          ? html`<section class="detail ${this.detailCollapsed ? 'collapsed' : ''}">
              <header
                class="detail-head"
                title=${this.detailCollapsed ? 'Show change details' : 'Hide change details'}
                @click=${(event: Event) => {
                  // The row is the hit target — a 10px chevron was not clickable in
                  // practice. Nested controls (bookmark delete) opt out via stopPropagation.
                  if ((event.target as HTMLElement).closest('.tag-x')) return;
                  this.detailCollapsed = !this.detailCollapsed;
                }}
              >
                <span class="detail-toggle">${this.detailCollapsed ? '▸' : '▾'}</span>
                <span class="detail-id">${change.changeId.slice(0, 12)}</span>
                ${change.bookmarks.map(
                  (bookmark) => html`<span class="tag"
                    >${bookmark}
                    <button
                      class="tag-x"
                      title="Delete bookmark"
                      @click=${(event: Event) => {
                        event.stopPropagation();
                        this.removeBookmark(bookmark);
                      }}
                    >
                      ×
                    </button></span
                  >`,
                )}
                ${change.immutable ? html`<span class="tag muted">immutable</span>` : nothing}
                ${change.conflict ? html`<span class="tag warn">conflict</span>` : nothing}
                ${change.empty ? html`<span class="tag muted">empty</span>` : nothing}
                ${this.detailCollapsed
                  ? html`<span class="detail-summary"
                      >${change.description.split('\n')[0] || '(no description)'}</span
                    >`
                  : nothing}
                <span class="spacer"></span>
                <span class="detail-when"
                  >${change.author.name} · ${relativeTime(change.committer.timestamp)}</span
                >
              </header>

              ${this.detailCollapsed
                ? nothing
                : html`
              ${change.immutable
                ? html`<div class="detail-desc">${descriptionParts(change.description)}</div>`
                : html`<textarea
                      class="detail-edit"
                      .value=${this.description}
                      @input=${(event: Event) =>
                        (this.description = (event.target as HTMLTextAreaElement).value)}
                    ></textarea>
                    <div class="detail-actions">
                      <button
                        class="tool"
                        title="jj describe — save this message onto the change."
                        ?disabled=${this.description === change.description}
                        @click=${this.saveDescription}
                      >
                        Save description
                      </button>
                    </div>`}

              <div class="detail-actions">
                <span class="action-group">
                  <button
                    class="tool primary"
                    title=${`jj edit — move the working copy onto this change so your edits land in it.${
                      change.immutable ? ' Blocked: this change is immutable.' : ''
                    }`}
                    ?disabled=${change.immutable}
                    @click=${this.editSelected}
                  >
                    Work on this
                  </button>
                  <button
                    class="tool"
                    title="jj new — start a fresh empty change with this one as its parent. Leaves this change untouched."
                    @click=${this.newOnSelected}
                  >
                    New on top
                  </button>
                </span>

                <span class="action-group">
                  <button
                    class="tool"
                    title=${
                      change.immutable
                        ? 'jj rebase — blocked: this change is immutable (at or below trunk).'
                        : 'jj rebase -s — move this change and everything built on top of it onto a different parent. Conflicts are recorded, not fatal.'
                    }
                    ?disabled=${change.immutable}
                    @click=${this.rebaseSelected}
                  >
                    Rebase…
                  </button>
                  <button
                    class="tool"
                    title=${
                      change.immutable
                        ? 'jj split — blocked: this change is immutable.'
                        : this.files.length < 2
                          ? 'jj split — needs at least two files; there is nothing to separate.'
                          : 'jj split — pull the focused file out into its own change, leaving the rest here. File-level, no hunk picking.'
                    }
                    ?disabled=${change.immutable || this.files.length < 2}
                    @click=${this.splitSelectedFiles}
                  >
                    Split file
                  </button>
                  <button
                    class="tool"
                    title="jj duplicate — copy this change to a second, independent change with the same content. The original stays put."
                    @click=${this.duplicateSelected}
                  >
                    Duplicate
                  </button>
                  <button
                    class="tool"
                    title="jj backout — add a NEW change that undoes this one, keeping this one in history. Use for already-pushed work; use Abandon for work only you have."
                    @click=${this.backoutSelected}
                  >
                    Back out
                  </button>
                </span>

                <span class="action-group">
                  <button
                    class="tool"
                    title="jj bookmark set — name this change so it can be pushed and referenced (jj's equivalent of a git branch)."
                    @click=${this.createBookmark}
                  >
                    Bookmark…
                  </button>
                  <button
                    class="tool"
                    title=${
                      change.bookmarks.length
                        ? `jj git push -b ${change.bookmarks[0]} — push this bookmark to the remote.`
                        : 'jj git push --change — push this change, auto-naming a bookmark from its change id.'
                    }
                    @click=${this.runPush}
                  >
                    Push
                  </button>
                </span>

                <button
                  class="tool danger"
                  title=${
                    change.immutable
                      ? 'jj abandon — blocked: this change is immutable.'
                      : 'jj abandon — remove this change from history entirely, as if it never existed. Undoable from the Ops tab. To reverse already-pushed work instead, use Back out.'
                  }
                  ?disabled=${change.immutable}
                  @click=${this.abandonSelected}
                >
                  Abandon
                </button>
              </div>

              <div class="detail-files">
                <span class="detail-label">${this.files.length} file${
                  this.files.length === 1 ? '' : 's'
                }</span>
                ${this.files.map(
                  (file) => html`<button
                    class="detail-file"
                    @click=${() => this.patchView?.scrollToPath(file.path)}
                  >
                    <span class="detail-file-status ${file.status}">${file.status[0]}</span>
                    <span class="detail-file-path">${file.path}</span>
                    <span class="detail-file-counts">
                      ${file.added ? html`<span class="plus">+${file.added}</span>` : nothing}
                      ${file.removed ? html`<span class="minus">−${file.removed}</span>` : nothing}
                    </span>
                  </button>`,
                )}
              </div>`}
            </section>`
          : change
            ? html`<div class="describe">
                <textarea
                  placeholder="Describe this change…"
                  .value=${this.description}
                  @input=${(event: Event) =>
                    (this.description = (event.target as HTMLTextAreaElement).value)}
                ></textarea>
                <button
                  class="tool"
                  ?disabled=${this.description === change.description}
                  @click=${this.saveDescription}
                >
                  Describe
                </button>
                <button
                  class="tool primary"
                  ?disabled=${this.files.length === 0 || !this.description.trim()}
                  title="Describe @ and start a new change on top (jj describe + jj new)"
                  @click=${this.commitAndNew}
                >
                  Commit & New
                </button>
                <button
                  class="tool"
                  ?disabled=${this.files.length === 0}
                  title="Discard the focused file's changes (or all when none is focused)"
                  @click=${this.restoreSelectedFile}
                >
                  Discard…
                </button>
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
          .themeVersion=${this.themeVersion}
          .comments=${this.comments}
          .canComment=${this.viewMode === 'full' && !this.walkActive}
          .revset=${this.isWorkingCopySelected ? null : this.selected}
          .markdownPreviews=${this.markdownPreviews}
          @add-comment=${(e: CustomEvent) => this.onAddComment(e.detail)}
          @resolve-comment=${(e: CustomEvent<{ id: number; value: boolean }>) =>
            this.onResolveComment(e.detail.id, e.detail.value)}
          @delete-comment=${(e: CustomEvent<{ id: number; value: boolean }>) =>
            this.onDeleteComment(e.detail.id)}
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

  /** Toggle a `.md` file between diff view and rendered preview. */
  private async onToggleMarkdown(path: string) {
    const next = new Map(this.markdownPreviews);
    if (next.has(path)) {
      next.delete(path);
      this.markdownPreviews = next;
      return;
    }
    try {
      const text = await getFileContent(
        this.isWorkingCopySelected ? null : this.selected,
        path,
      );
      const { marked } = await import('marked');
      const html = marked.parse(text, { async: false }) as string;
      next.set(path, html);
      this.markdownPreviews = next;
    } catch (error) {
      this.actionError = String(error);
    }
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
const dirname = (path: string) => {
  const cut = path.lastIndexOf('/');
  return cut === -1 ? '' : `${path.slice(0, cut)}/`;
};

declare global {
  interface HTMLElementTagNameMap {
    'jj-app': App;
  }
}
