import { html, LitElement, nothing, type TemplateResult } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { keyed } from 'lit/directives/keyed.js';
import { repeat } from 'lit/directives/repeat.js';

import './command-bar.js';
import './evolog-drawer.js';
import type { Command } from './command-bar.js';
import './file-tree.js';
import './log-graph.js';
import { renderMarkdown } from './markdown.js';
import './orbs.js';
import {
  abandonChange,
  absorb,
  backoutChange,
  deleteBookmark,
  duplicateChange,
  editChange,
  getChangeVersions,
  getInterdiff,
  getOperationDiff,
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
  getForgeInfo,
  getPullRequest,
  getPullRequestActivity,
  type Activity,
  listPullRequests,
  onMenuCommand,
  onRepoChanged,
  openInEditor,
  openPullRequest,
  openUrl,
  setEditorCommand,
  setUiTheme,
  openRepoWindow,
  setMenu,
  submitReview,
  type ForgeInfo,
  type MenuGroup,
  type PullRequest,
  type PullRequestSummary,
  type ReviewVerdict,
  onSecondInstance,
  openRepository,
  pickRepository,
  setViewed,
  squashPaths,
  addComment,
  type BookmarkStatus,
  type Change,
  type ChangeVersion,
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
import type { FileMenuRequest } from './file-tree.js';
import {
  iconAbsorb,
  iconChevron,
  iconCommit,
  iconComment,
  iconFetch,
  iconFiles,
  iconGraph,
  iconInfo,
  iconSearch,
  iconSparkle,
  iconSplit,
  iconUndo,
  iconUnified,
  iconWarn,
} from './icons.js';
import { formatShortcut, matchesShortcut, parseShortcut, type Shortcut } from './keys.js';
import { relativeTime as age } from './time.js';
import { askConfirm, askText } from './prompt.js';
import './patch-view.js';
import type { PatchView } from './patch-view.js';
import './shortcuts-help.js';
import './theme-picker.js';
import { applyThemeTokens, isKnownTheme, THEMES } from './themes.js';
import './walkthrough-panel.js';
import type { DiffLayout } from './rows.js';

/** Ages in the shell read as prose ("3h ago"); the log graph uses the compact form. */
const relativeTime = (timestamp: string): string => age(timestamp, true);

/** What the main pane shows for the selected change. */
type ViewMode = 'full' | 'interdiff' | 'ops' | 'pr';

/** Revsets people actually reach for; the empty one restores the default view. */
const REVSET_PRESETS: { label: string; revset: string }[] = [
  { label: 'All', revset: '' },
  { label: 'Stack', revset: 'trunk()..@ | @' },
  { label: 'Recent', revset: 'ancestors(@, 50)' },
  { label: 'Mine', revset: 'mine()' },
  { label: 'Conflicts', revset: 'conflicts()' },
  { label: 'Bookmarks', revset: 'ancestors(bookmarks(), 5)' },
];

/**
 * Everything after a jj description's subject line.
 *
 * The subject is the detail card's title, so rendering it again here would put
 * the same sentence on screen twice, one line apart.
 */
function descriptionBody(description: string): string {
  const [, ...rest] = description.split('\n');
  return rest.join('\n').trim();
}

/** Compact relative age: now, 5m, 3h, 2d. */
function activityKey(entry: { createdAt: string; url: string }, index: number): string {
  return `${index}:${entry.url || entry.createdAt}`;
}

/**
 * App shell. LIGHT DOM, non-negotiably: the diff pane (jj-patch-view) is a descendant, and
 * document stylesheets cannot cross a shadow boundary — a shadow root here would sever
 * theme.css from every diff row below it (exactly the bug that shipped in M1–M4). Chrome
 * styles live in theme.css under the `jj-app` prefix; leaf widgets with no cross-boundary
 * text selection (file tree, command bar) keep their own shadow styles.
 */
/**
 * How stale the proposal index may be before a window focus reloads it. Long
 * enough that alt-tabbing between two windows does not spawn a `gh` process
 * per switch, short enough that stepping out to open a proposal and coming
 * straight back finds it.
 */
const PROPOSAL_REFRESH_MS = 15_000;

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
  @state() private sidebarTab: SidebarTab = 'stack';
  /**
   * Whether the sidebar panel is folded away, leaving only the rail.
   *
   * The rail stays: it is how you get the panel back, and collapsing the pane
   * switcher along with the pane would make the app look like it had lost a
   * feature. Review and guided steps are the reason this exists — both are
   * about reading the diff, and the sidebar is 292px the diff could have.
   */
  @state() private sidebarCollapsed = false;
  /** Description editing is opt-in. Reading a change is the common case, and a
   *  textarea permanently sitting there reads as an input to fill in rather
   *  than a message to read. */
  @state() private editingDescription = false;
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
  /**
   * Whether the guided-review narrative is showing in full.
   *
   * A step's narrative is prose of no fixed length, and the overview's is the
   * longest of them — left unclamped it pushed the diff, which is the thing
   * being reviewed, off the bottom of the window. It is clamped to three lines
   * with a toggle, and the toggle is deliberately *not* reset per step: someone
   * who wants the long form wants it for the whole walkthrough, not once.
   */
  @state() private walkExpanded = false;
  /** Set from layout — see `measureNarrative`. */
  @state() private walkOverflow = false;
  @state() private scopeOpen = false;
  @state() private repoMenuOpen = false;
  /**
   * Where the change's More menu is anchored, or null when it is closed.
   *
   * Nine buttons in a row is nine decisions of equal weight, and the two people
   * actually reach for — work on this, push — were the same size as the one
   * that erases a commit. What stays out is what gets used; the rest is one
   * click further away and the destructive verb is at the bottom, alone.
   *
   * Viewport coordinates, and the panel is `position: fixed`, because the card
   * it opens from scrolls (`max-height: 44%`) and an absolutely-positioned menu
   * is clipped by that — the last entry, which is Abandon, was the one cut off.
   * Same arrangement as the file context menu.
   */
  /**
   * Anchor for the change's overflow menu, as a distance from the *right* edge.
   *
   * Left-anchored, it ran off the window: the button sits at the right of the
   * detail card, and a 190px menu opening rightwards put most of every item
   * outside the viewport, where a click hits nothing and the outside-click
   * handler closes the menu instead. Every entry behind More was unreachable.
   */
  @state() private moreAt: { right: number; y: number } | null = null;
  @state() private recentRepos: string[] = [];
  @state() private searchOpen = false;
  @state() private searchQuery = '';
  @state() private searchCount = 0;
  @state() private searchCurrent = -1;
  @state() private wordWrap = false;
  /** Last mutation's narration + the operation that would undo it. */
  @state() private lastOutcome: (Outcome & { pullRequestUrl?: string | null }) | null = null;
  @state() private operations: Operation[] = [];
  /**
   * jj's narration of one operation, keyed so a stale answer cannot land under
   * the wrong row: `to` alone for "what did this do", `from..to` for a range.
   */
  @state() private opDiff: { key: string; text: string } | null = null;
  /** The operation pinned as the older end of a comparison, if any. */
  @state() private opCompareFrom: Operation | null = null;
  /** Evolog drawer: every recorded version of the selected change. */
  @state() private versionsOpen = false;
  @state() private versions: ChangeVersion[] = [];
  @state() private versionsLoading = false;
  /**
   * The two commits an interdiff is showing, when it came from the drawer rather
   * than from "since I reviewed". Both modes render as `viewMode === 'interdiff'`;
   * this is what tells them apart, so it must clear whenever the diff goes back
   * to showing a change whole.
   */
  @state() private versionPair: { from: string; to: string } | null = null;
  @state() private busy: string | null = null;
  /** Revset scoping the Log graph; null = the default. */
  @state() private graphRevset: string | null = null;
  @state() private revsetSearch = '';
  /** "system" | "light" | "dark" — runtime override of the config value. */
  /** A `THEMES` id — 'system', 'light', 'dark' or a named palette. */
  @state() private theme = 'system';
  @state() private themePickerOpen = false;
  /** Bumped on theme change so the diff re-tokenizes (shiki tokens carry colours). */
  @state() private themeVersion = 0;
  /** File the diff viewport is currently inside; drives the pinned file header. */
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
  /** Sidebar width in px (resizable via drag handle). */
  @state() private sidebarWidth = 292;
  /** Open file-tree context menu, anchored at viewport coordinates. */
  @state() private fileMenu: FileMenuRequest | null = null;
  /** The `?` shortcut sheet. */
  @state() private shortcutsOpen = false;
  /** Which forge this repo is on, or null when it is on none we can drive. */
  @state() private forge: ForgeInfo | null = null;
  /** The proposal under review, when one is open. */
  @state() private pullRequest: PullRequest | null = null;
  /**
   * Set only while the diff pane is showing the *whole* proposal rather than
   * the selected change. Null is the normal state — the banner is context on a
   * change you are already looking at, not a mode.
   */
  @state() private prRevset: string | null = null;
  /** Open proposals indexed by head branch, for matching against bookmarks. */
  @state() private proposalsByBranch: ReadonlyMap<string, PullRequestSummary> = new Map();
  /** When `proposalsByBranch` was last (re)loaded; drives the focus throttle. */
  private proposalIndexAt = 0;

  /** Rendered proposal body, and its conversation. Both are markdown from the
   *  forge, so both go through the sanitising renderer. */
  @state() private prBody: TemplateResult | null = null;
  @state() private prActivity: Activity[] = [];
  @state() private prActivityBodies: ReadonlyMap<string, TemplateResult> = new Map();
  /** Which proposal the loaded body and conversation belong to — a late
   *  response for a proposal you have already navigated away from is dropped
   *  rather than rendered under the wrong banner. */
  private prDetailsFor: number | null = null;
  /** Review composer state; null when closed. */
  @state() private reviewDraft: { verdict: ReviewVerdict; body: string } | null = null;
  /**
   * When set, the command bar shows these instead of the app commands — the
   * proposal picker borrows the palette rather than duplicating its filtering
   * and keyboard handling. Cleared when the bar closes.
   */
  @state() private proposalPicker: Command[] | null = null;

  private unlisten: (() => void) | null = null;
  private unlistenMenu: (() => void) | null = null;
  /** Serialized shape of the last menu pushed, so identical renders are free. */
  private menuSignature = '';
  /** The change id the description editor was last seeded from. */
  private seededFor: string | null = null;
  private commandBarShortcut: Shortcut = parseShortcut('Mod+k');
  /** The raw binding string, for display in the shortcut sheet and palette hints. */
  @state() private commandBarBinding = 'Mod+k';

  override connectedCallback() {
    super.connectedCallback();
    void this.start();
    window.addEventListener('keydown', this.onGlobalKey);
    window.addEventListener('click', this.onWindowClick);
    window.addEventListener('focus', this.onWindowFocus);
    void onMenuCommand(this.onMenuCommand).then((stop) => (this.unlistenMenu = stop));
  }

  override disconnectedCallback() {
    this.unlisten?.();
    this.unlistenMenu?.();
    window.removeEventListener('keydown', this.onGlobalKey);
    window.removeEventListener('click', this.onWindowClick);
    window.removeEventListener('focus', this.onWindowFocus);
    super.disconnectedCallback();
  }

  /**
   * The menu bar is app-global on macOS, so it must follow focus: whichever
   * window the user is in re-pushes its own commands on the way in.
   */
  private onWindowFocus = () => {
    this.menuSignature = '';
    this.syncMenu();
    // Coming back to the window is the signal that something may have happened
    // elsewhere — a proposal opened from a terminal or a browser, which touches
    // the repo not at all and so trips no watcher.
    void this.refreshProposals(PROPOSAL_REFRESH_MS);
  };

  /** Run a command the native menu dispatched — unless another window owns focus. */
  private onMenuCommand = (id: string) => {
    if (!document.hasFocus()) return;
    this.commands.find((command) => command.id === id)?.run();
  };

  /**
   * Push the command list to the native menu whenever it changes shape. The
   * palette is the single source of truth; this only reshapes it into groups.
   * Diffed by signature because `updated()` runs on every render and rebuilding
   * a native menu per keystroke would be absurd.
   */
  private syncMenu() {
    const groups: MenuGroup[] = [];
    for (const command of this.commands) {
      const title = command.group ?? 'Commands';
      const last = groups[groups.length - 1];
      const group = last?.title === title ? last : (groups.push({ title, items: [] }), groups[groups.length - 1]!);
      group.items.push({ id: command.id, label: command.label });
    }
    const signature = JSON.stringify(groups);
    if (signature === this.menuSignature) return;
    this.menuSignature = signature;
    void setMenu(groups).catch(() => {
      // A menu that fails to build must not take the window down with it.
      this.menuSignature = '';
    });
  }

  protected override updated() {
    if (document.hasFocus()) this.syncMenu();
    this.measureNarrative();
  }

  /**
   * Whether the guided-review narrative is longer than its clamp.
   *
   * The clamp is CSS, but "is there more to show" is only answerable after
   * layout, so the toggle has to be driven by a measurement. Once expanded the
   * element no longer overflows — it is showing everything — so the flag is
   * held rather than recomputed, or the control that expanded it would vanish
   * the moment it worked.
   */
  private measureNarrative() {
    const narrative = this.querySelector<HTMLElement>('.walk-narrative');
    if (!narrative) {
      if (this.walkOverflow) this.walkOverflow = false;
      return;
    }
    const overflows = this.walkExpanded
      ? this.walkOverflow
      : narrative.scrollHeight > narrative.clientHeight + 1;
    if (overflows !== this.walkOverflow) this.walkOverflow = overflows;
  }

  /** Close the repo menu and the file context menu on any click outside them. */
  private onWindowClick = (event: MouseEvent) => {
    const path = event.composedPath();
    const inside = (className: string) =>
      path.some((node) => node instanceof HTMLElement && node.classList?.contains(className));
    if (this.repoMenuOpen && !inside('repo-menu-root')) {
      this.repoMenuOpen = false;
    }
    if (this.moreAt && !inside('more-root')) {
      this.moreAt = null;
    }
    if (this.scopeOpen && !inside('scope-root')) {
      this.scopeOpen = false;
    }
    if (this.fileMenu && !inside('file-menu')) {
      this.fileMenu = null;
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
      this.editingDescription = false;
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

  /** Same picker, but the repo lands in its own window (or focuses its existing one). */
  private async openFolderInNewWindow() {
    this.repoMenuOpen = false;
    const picked = await pickRepository();
    if (!picked) return;
    await this.run(() => openRepoWindow(picked));
  }

  /**
   * `jjdiff` launched again while the app is running. The backend routes this:
   * a repo with no window gets a fresh one, and only the window already bound
   * to that repo receives the event — so by the time we see it, this window is
   * the right one and all that is left is to reload and apply the flags.
   */
  private async handleSecondInstance(args: SecondInstanceArgs) {
    if (args.repoPath) {
      await this.refresh();
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

  /**
   * Open a file in the configured editor. With no explicit path, uses the diff
   * cursor (so `o` works mid-review), falling back to the focused file.
   */
  private async openFileInEditor(path?: string, line?: number) {
    const target =
      path !== undefined
        ? { path, line }
        : this.patchView?.cursorLocation() ??
          (this.focusPath ? { path: this.focusPath, line: undefined } : null);
    if (!target) {
      this.actionError = 'No file selected — move the cursor to a file first (j/k).';
      return;
    }
    this.fileMenu = null;
    try {
      await openInEditor(target.path, target.line);
      this.actionError = null;
    } catch (error) {
      // An unconfigured editor is a setup step, not a failure — offer the
      // setting rather than printing the config key and leaving them to it.
      if (String(error).includes('no editor configured')) {
        await this.configureEditor();
        return;
      }
      this.actionError = String(error);
    }
  }

  /** Set `[editor] command`, seeded with whatever is configured now. */
  private async configureEditor() {
    const current = await getConfig()
      .then((config) => config.editor.command)
      .catch(() => '');
    const command = await askText({
      heading: 'Editor command',
      detail:
        'Placeholders: {file} (absolute path), {line}, {repo}.\n' +
        'Split on spaces and run directly — no shell.\n\n' +
        'Examples:  zed {file}:{line}   ·   code -g {file}:{line}   ·   idea --line {line} {file}',
      value: current,
      placeholder: 'zed {file}:{line}',
      confirmLabel: 'Save',
    });
    if (command === null) return;
    try {
      const path = await setEditorCommand(command);
      this.lastOutcome = {
        message: command.trim() ? `Editor saved to ${path}.` : `Editor cleared in ${path}.`,
        operation: '',
      };
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
    }
  }

  // ---- Forge review ----

  /**
   * Index the open proposals by their head branch, so a change that carries a
   * matching bookmark can show its proposal without being asked to.
   *
   * One `gh pr list` per refresh, not per selection — it is a network call.
   * Failure is silent: no forge, no auth or no network simply means no banner.
   */
  private async loadProposalIndex() {
    if (!this.forge) return;
    // Stamped before the attempt, not after, so a forge that is down throttles
    // its own retries instead of one per focus.
    this.proposalIndexAt = Date.now();
    try {
      const proposals = await listPullRequests();
      const index = new Map<string, PullRequestSummary>();
      for (const proposal of proposals) {
        index.set(proposal.head, proposal);
        // A proposal fetched by number lands on its own namespaced bookmark
        // rather than the author's branch name; match that too, so the explicit
        // and automatic paths converge on the same banner.
        index.set(`jjdiff-pr-${proposal.number}`, proposal);
        index.set(`jjdiff-mr-${proposal.number}`, proposal);
      }
      this.proposalsByBranch = index;
    } catch {
      // Keep whatever we had. This used to clear, which was harmless when the
      // index loaded once per repo — as a background refresh it would make a
      // visible banner vanish on one flaky network call.
      if (this.proposalsByBranch.size === 0) this.proposalsByBranch = new Map();
    }
  }

  /**
   * Re-index proposals and re-match the selection against them.
   *
   * A proposal can appear or move while jjdiff is not the focused app — `gh pr
   * create` in a terminal, or the "create a pull request" link a push prints.
   * Neither touches the repo, so no watcher fires and nothing would bring the
   * banner in until a manual reload.
   *
   * `maxAgeMs` skips the work when the index is already fresher than that;
   * focus fires on every alt-tab and each call is a `gh` subprocess plus a
   * network round trip. Omit it to force.
   */
  private async refreshProposals(maxAgeMs = 0) {
    if (!this.forge) return;
    if (maxAgeMs && Date.now() - this.proposalIndexAt < maxAgeMs) return;
    await this.loadProposalIndex();
    // Force the conversation to reload next time the view is opened: a push
    // moved the head, and comments may have arrived with it.
    this.prDetailsFor = null;
    await this.syncMatchedProposal(true);
  }

  /**
   * Tracking state for one bookmark. A bookmark can track several remotes; the
   * one that has drifted is the one worth reporting, so a diverged remote wins
   * over a synced one rather than whichever happened to be listed first.
   */
  private tracking(bookmark: string): BookmarkStatus | null {
    const all = this.repo?.bookmarks.filter((entry) => entry.name === bookmark) ?? [];
    return all.find((entry) => entry.ahead || entry.behind) ?? all[0] ?? null;
  }

  /**
   * `↑2 ↓1` beside a bookmark. Absent when the bookmark is in sync — a badge
   * that is always on screen stops being read, and "in sync" is the state you
   * do not need telling about.
   */
  private renderTracking(bookmark: string) {
    const status = this.tracking(bookmark);
    if (!status || (!status.ahead && !status.behind)) return nothing;
    const parts = [
      status.ahead ? `${status.ahead} to push` : '',
      status.behind ? `${status.behind} to pull` : '',
    ].filter(Boolean);
    return html`<span
      class="tag-track"
      title=${`${bookmark} vs ${status.remote}: ${parts.join(', ')}`}
      >${status.ahead ? html`<span class="ahead">↑${status.ahead}</span>` : nothing}${status.behind
        ? html`<span class="behind">↓${status.behind}</span>`
        : nothing}</span
    >`;
  }

  /**
   * Load the proposal's body and conversation.
   *
   * Deliberately after the banner is already on screen: this is two more `gh`
   * calls, and state, checks and reviewers are what a reviewer needs first.
   * A failure leaves the banner intact and simply shows no conversation.
   */
  private async loadProposalDetails(pr: PullRequest) {
    const number = pr.number;
    this.prDetailsFor = number;
    this.prBody = null;
    this.prActivity = [];
    this.prActivityBodies = new Map();

    if (pr.body.trim()) {
      const body = await renderMarkdown(pr.body);
      if (this.prDetailsFor !== number) return;
      this.prBody = body;
    }

    let entries: Activity[];
    try {
      entries = await getPullRequestActivity(number);
    } catch {
      return;
    }
    if (this.prDetailsFor !== number) return;
    const bodies = new Map<string, TemplateResult>();
    await Promise.all(
      entries.map(async (entry, index) => {
        if (entry.body.trim()) bodies.set(activityKey(entry, index), await renderMarkdown(entry.body));
      }),
    );
    if (this.prDetailsFor !== number) return;
    this.prActivity = entries;
    this.prActivityBodies = bodies;
  }

  /** The open proposal for the selected change, matched on its bookmarks. */
  private get matchedProposal(): PullRequestSummary | null {
    const change = this.selectedChange;
    if (!change || this.proposalsByBranch.size === 0) return null;
    for (const bookmark of change.bookmarks) {
      const found = this.proposalsByBranch.get(bookmark);
      if (found) return found;
    }
    return null;
  }

  /**
   * Load the full proposal (checks, reviewers, merge state) for whatever the
   * selection matched. The list call carries none of that, so this fills it in
   * once per number and is skipped when it is already loaded.
   */
  private async syncMatchedProposal(refetch = false) {
    const matched = this.matchedProposal;
    if (!matched) {
      // Only clear a banner we inferred; an explicitly opened proposal owns the
      // view until it is closed.
      if (!this.prRevset) {
        this.pullRequest = null;
        this.prDetailsFor = null;
        this.prBody = null;
        this.prActivity = [];
      }
      return;
    }
    // `refetch` is for the case where the number has not changed but its
    // contents have — after a push, the head moved and the checks it reports
    // are about the previous one.
    if (!refetch && this.pullRequest?.number === matched.number) return;
    const previous = this.pullRequest;
    try {
      this.pullRequest = await getPullRequest(matched.number);
    } catch {
      // A failed refresh must not take a banner that is already on screen down
      // with it; only a first load has nothing to fall back to.
      this.pullRequest = previous?.number === matched.number ? previous : null;
    }
  }

  /**
   * Fetch a proposal by number — for one whose branch is not local, which is
   * the case whenever you are reviewing someone else's work. Afterwards its
   * head is an ordinary bookmark, so the banner arrives through the same path
   * as a proposal that was already there.
   */
  private async openProposal(number: number) {
    this.busy = 'pull-request';
    try {
      const opened = await openPullRequest(number);
      this.pullRequest = opened;
      this.focusPath = null;
      this.viewMode = 'full';
      await this.refresh();
      // Select the fetched head so the change, its diff and the banner agree.
      const head = this.repo?.graph.find((change) =>
        change.bookmarks.includes(opened.bookmark),
      );
      if (head) {
        this.selected = head.changeId;
      }
      // Someone else's proposal is usually several commits, so default to the
      // whole thing rather than whichever commit happens to be the tip.
      this.prRevset = opened.revset;
      await this.loadDiff();
      void this.loadProposalIndex();
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
      this.pullRequest = null;
      this.prRevset = null;
    } finally {
      this.busy = null;
    }
  }

  /** Toggle between the whole proposal's diff and the selected change's own. */
  private async toggleProposalDiff() {
    const pr = this.pullRequest;
    if (!pr) return;
    // The forge's own view of the proposal, not base..selection — selecting a
    // commit in the middle of a stack must still diff the whole thing.
    this.prRevset = this.prRevset
      ? null
      : `${pr.baseOid || pr.base}..${pr.headOid || pr.head}`;
    this.focusPath = null;
    await this.loadDiff();
  }

  /** Hand a URL to the system browser; the WebView has nowhere to open it. */
  private async openExternal(url: string) {
    try {
      await openUrl(url);
      this.actionError = null;
    } catch (error) {
      this.actionError = String(error);
    }
  }

  /**
   * Pick from the open proposals. Uses the command bar rather than a bespoke
   * list: it already does filtering, keyboard selection and grouping, and a
   * proposal is just another thing to run.
   */
  private async showProposalList() {
    this.busy = 'pull-request';
    try {
      const proposals = await listPullRequests();
      this.actionError = null;
      if (proposals.length === 0) {
        this.lastOutcome = { message: `No open ${this.forge?.noun ?? 'pull request'}s.`, operation: '' };
        return;
      }
      this.proposalPicker = proposals.map((proposal) => ({
        id: `pr-${proposal.number}`,
        label: `#${proposal.number} ${proposal.title}`,
        hint: `${proposal.author}${proposal.draft ? ' · draft' : ''}`,
        group: 'Open for review',
        run: () => void this.openProposal(proposal.number),
      }));
      this.barOpen = true;
    } catch (error) {
      this.actionError = String(error);
    } finally {
      this.busy = null;
    }
  }

  private async promptForProposal() {
    const noun = this.forge?.noun ?? 'pull request';
    const answer = await askText({
      heading: `Review which ${noun}?`,
      detail: 'Its number on the forge.',
      placeholder: '75',
      confirmLabel: 'Open',
    });
    const number = Number(answer?.trim());
    if (!answer || !Number.isInteger(number) || number <= 0) return;
    await this.openProposal(number);
  }

  /** Seed the composer from the pending inline comments, if any. */
  private async openReviewComposer() {
    const change = this.selectedChange;
    let body = '';
    if (change) {
      try {
        body = this.pendingComments.length ? await exportReviewMarkdown(change.changeId) : '';
      } catch {
        // A comment store that will not export must not block a plain review.
        body = '';
      }
    }
    this.reviewDraft = { verdict: 'comment', body };
  }

  /**
   * Submit the review. Outward-facing and effectively irreversible, so it
   * confirms first, naming the verdict and the proposal.
   */
  private async sendReview() {
    const draft = this.reviewDraft;
    const pr = this.pullRequest;
    if (!draft || !pr) return;
    const verdictLabel = {
      approve: 'Approve',
      requestChanges: 'Request changes on',
      comment: 'Comment on',
    }[draft.verdict];
    const noun = this.forge?.noun ?? 'pull request';
    const ok = await askConfirm({
      heading: `${verdictLabel} ${noun} #${pr.number}?`,
      detail: 'This is posted publicly on the forge.',
      confirmLabel: verdictLabel.split(' ')[0],
      danger: draft.verdict === 'requestChanges',
    });
    if (!ok) return;
    this.busy = 'submit-review';
    try {
      // Outdated comments have no line on the current diff, so the forge would
      // reject the whole review. They ride along in the body instead — the
      // backend does the same for anything else it cannot anchor.
      const anchored = this.pendingComments.filter((comment) => !comment.outdated);
      const result = await submitReview(
        pr.number,
        draft.verdict,
        draft.body,
        anchored.map((comment) => ({
          path: comment.path,
          line: comment.line,
          side: comment.side,
          body: comment.body,
        })),
      );
      this.reviewDraft = null;
      this.lastOutcome = {
        message: result.fellBack
          ? `Review submitted on #${pr.number}, but the comments went into the body: ${result.fellBack}`
          : result.inline
            ? `Review submitted on #${pr.number} with ${result.inline} inline comment${result.inline === 1 ? '' : 's'}.`
            : `Review submitted on #${pr.number}.`,
        operation: '',
      };
      this.actionError = null;
      // Reviewer state and merge status just changed.
      this.pullRequest = await getPullRequest(pr.number);
    } catch (error) {
      this.actionError = String(error);
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
      // An unknown theme name — a typo, or one removed since it was written —
      // falls through to `system` rather than leaving the app unstyled.
      if (isKnownTheme(config.ui.theme)) this.applyTheme(config.ui.theme);
      this.commandBarShortcut = parseShortcut(config.keymap.commandBar);
      this.commandBarBinding = config.keymap.commandBar;
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
    // Which forge this repo is on, if any. Best-effort: forge affordances are
    // simply absent on a repo we cannot drive, never broken.
    void getForgeInfo()
      .then((info) => {
        this.forge = info;
        return this.loadProposalIndex();
      })
      .catch(() => (this.forge = null));
    try {
      const launch = await getLaunchOptions();
      if (launch.pullRequest !== null) {
        // `jjdiff pr 75` — straight into reviewing the proposal.
        await this.openProposal(launch.pullRequest);
      }
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
            this.sidebarTab = 'walkthrough';
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

  /**
   * The change's less-used verbs.
   *
   * Ordered by how much of history they touch: rearrange, then copy, then the
   * two that undo. Abandon sits below a rule and keeps its danger styling —
   * it is the only entry here that can lose work, and distance is what stops a
   * misaimed click from being the one that costs something.
   */
  private toggleMore = (event: MouseEvent) => {
    if (this.moreAt) {
      this.moreAt = null;
      return;
    }
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    this.moreAt = { right: Math.max(8, window.innerWidth - rect.right), y: rect.bottom + 6 };
  };

  private renderMoreMenu(change: Change, at: { right: number; y: number }) {
    const pick = (run: () => void) => () => {
      this.moreAt = null;
      run();
    };
    return html`<div class="more-menu" role="menu" style="right: ${at.right}px; top: ${at.y}px">
      <button
        role="menuitem"
        title=${
          change.immutable
            ? 'jj rebase -s --ignore-immutable — this change is immutable (at or below trunk); you will be asked to confirm first.'
            : 'jj rebase -s — move this change and everything built on top of it onto a different parent. Conflicts are recorded, not fatal.'
        }
        @click=${pick(() => void this.rebaseSelected())}
      >
        Rebase…
      </button>
      <button
        role="menuitem"
        title=${
          this.files.length < 2
            ? 'jj split — needs at least two files; there is nothing to separate.'
            : change.immutable
              ? 'jj split --ignore-immutable — this change is immutable; you will be asked to confirm first.'
              : 'jj split — pull the focused file out into its own change, leaving the rest here. File-level, no hunk picking.'
        }
        ?disabled=${this.files.length < 2}
        @click=${pick(() => void this.splitSelectedFiles())}
      >
        Split file
      </button>
      <button
        role="menuitem"
        title="jj duplicate — copy this change to a second, independent change with the same content. The original stays put."
        @click=${pick(() => void this.duplicateSelected())}
      >
        Duplicate
      </button>
      <!-- A read, sitting among rewrites, because it answers the question the
           rewrites raise: what this change used to be before one of them. -->
      <button
        role="menuitem"
        title="jj evolog — every version this change has been, and an interdiff between any two of them."
        @click=${pick(() => this.openVersions())}
      >
        Versions…
      </button>
      <button
        role="menuitem"
        title="jj backout — add a NEW change that undoes this one, keeping this one in history. Use for already-pushed work; use Abandon for work only you have."
        @click=${pick(() => void this.backoutSelected())}
      >
        Back out
      </button>
      <button
        class="danger"
        role="menuitem"
        title=${
          change.immutable
            ? 'jj abandon --ignore-immutable — this change is immutable; you will be asked to confirm first. Back out is usually what you want for published work.'
            : 'jj abandon — remove this change from history entirely, as if it never existed. Undoable from the Ops tab. To reverse already-pushed work instead, use Back out.'
        }
        @click=${pick(() => void this.abandonSelected())}
      >
        Abandon
      </button>
    </div>`;
  }

  /** The Walkthrough tab: shows the generate button or the walkthrough panel. */
  private renderWalkthroughTab() {
    const change = this.selectedChange;
    if (this.generating) {
      // An agent CLI is running and cannot report progress, so the indicator
      // says "thinking" rather than pretending to a percentage. The beam on the
      // card is the second half of that: it closes a lap, which reads as time
      // passing even though nothing here knows how much is left.
      return html`<div class="walkthrough-working beam">
        <jj-orbs .size=${52} label="Generating walkthrough"></jj-orbs>
        <p class="working-title">Reading the change</p>
        <p class="working-hint">
          An agent is working through the diff. This usually takes under a minute.
        </p>
      </div>`;
    }
    if (!change) {
      return html`<div class="walkthrough-empty">Select a change to generate a walkthrough.</div>`;
    }
    if (!this.walkthrough) {
      return html`<div class="walkthrough-generate">
        <p>No walkthrough for this change yet.</p>
        <button class="tool primary" ?disabled=${this.files.length === 0} @click=${() => this.runGenerateWalkthrough()}>
          Generate Walkthrough
        </button>
        ${this.walkStale ? html`<p class="stale-note">The change has evolved — regenerate to update.</p>` : nothing}
      </div>`;
    }
    return html`<jj-walkthrough-panel
      .walkthrough=${this.walkthrough}
      .files=${this.files}
      .viewed=${this.viewedPaths}
      .current=${this.walkStep}
      @step-selected=${(event: CustomEvent<number>) => {
        this.walkStep = event.detail;
      }}
    ></jj-walkthrough-panel>
    <!-- One button. "Regenerate" and "Refresh Walkthrough" ran the same handler
         and sat one line apart, so the stale warning came with a choice that
         was not a choice. The warning is now a caption on the single action
         under it. -->
    <div class="pane-footer">
      ${this.walkStale
        ? html`<p class="stale-note">The change has evolved since this was generated.</p>`
        : nothing}
      <button class="tool block" @click=${() => this.runGenerateWalkthrough()}>
        Refresh Walkthrough
      </button>
    </div>`;
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
      // Always the command palette, never a stale proposal picker.
      this.proposalPicker = null;
      this.barOpen = !this.barOpen;
      return;
    }
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f') {
      event.preventDefault();
      this.openSearch();
      return;
    }
    // Mod+B, the convention every editor uses for this.
    if ((event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === 'b') {
      event.preventDefault();
      this.toggleSidebar();
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
      if (this.shortcutsOpen) {
        this.shortcutsOpen = false;
        return;
      }
      if (this.fileMenu) {
        this.fileMenu = null;
        return;
      }
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
      case 'o':
        event.preventDefault();
        void this.openFileInEditor();
        break;
      case '?':
        event.preventDefault();
        this.shortcutsOpen = !this.shortcutsOpen;
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

  /** The log graph filtered by the search bar (change id, commit id, description). */
  private get filteredGraph(): Change[] {
    if (!this.repo) return [];
    const needle = this.revsetSearch.trim().toLowerCase();
    if (!needle) return this.repo.graph;
    return this.repo.graph.filter((change) =>
      change.changeId.toLowerCase().includes(needle) ||
      change.commitId.toLowerCase().includes(needle) ||
      change.description.toLowerCase().includes(needle) ||
      change.bookmarks.some((b) => b.toLowerCase().includes(needle)),
    );
  }

  /** Scroll the diff to the file owning a comment and focus it. */
  private scrollToComment(comment: Comment) {
    this.focusPath = comment.path;
    this.sidebarTab = 'files';
    this.patchView?.scrollToPath(comment.path);
  }

  // ---- Sidebar resize ----

  private onSidebarResizeStart = (event: MouseEvent) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = this.sidebarWidth;
    const onMove = (e: MouseEvent) => {
      const width = Math.max(200, Math.min(600, startWidth + (e.clientX - startX)));
      this.sidebarWidth = width;
    };
    const onUp = () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  };

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
        this.syncMatchedProposal(),
      ]);
    } catch (error) {
      this.error = String(error);
    }
  }

  private async loadDiff() {
    try {
      if (this.viewMode === 'interdiff' && this.versionPair) {
        const interdiff = await getInterdiff(
          this.versionPair.from,
          this.versionPair.to,
          this.ignoreWhitespace,
        );
        this.files = interdiff.files;
      } else if (this.viewMode === 'interdiff' && this.selectedChange && this.changedSinceReview) {
        const interdiff = await getInterdiffSinceReviewed(
          this.selectedChange.changeId,
          this.ignoreWhitespace,
        );
        this.files = interdiff.files;
      } else if (this.prRevset) {
        // Reviewing a proposal: the revset is the forge's own comparison
        // (merge base .. head), not a change the local repo selected.
        this.viewMode = 'full';
        this.versionPair = null;
        this.files = await getDiff(this.prRevset, this.ignoreWhitespace);
      } else {
        this.viewMode = 'full';
        this.versionPair = null;
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
    // Back to reading. Carrying an open editor across a selection would offer a
    // half-typed message as though it belonged to the change now on screen.
    this.editingDescription = false;
    // Selecting a change leaves whole-proposal mode: the diff should follow
    // what was clicked.
    this.prRevset = null;
    void this.loadDiff();
    void this.loadReview();
    void this.loadConflicts();
    void this.loadWalkthrough();
    void this.syncMatchedProposal();
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
      // Having a walkthrough is not the same as wanting to be *in* one.
      //
      // This used to switch guided review on for any change that happened to
      // have one cached, so clicking down the log dropped you into a
      // walkthrough you had not asked for — hiding the describe box and
      // filtering the diff to one step. The mode belongs to the Steps pane
      // (`selectTab`): entering review is a thing you do, not a thing a
      // selection does to you.
      //
      // Staying on Steps while selecting another change *does* keep review on,
      // because that pane is the mode, and it restarts at the overview since
      // step 4 of the previous change means nothing here.
      if (this.sidebarTab === 'walkthrough' && status.walkthrough) {
        this.walkActive = true;
        this.walkStep = -1;
      } else if (!status.walkthrough) {
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
    this.sidebarTab = 'walkthrough';
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
      this.sidebarTab = 'walkthrough';
    }).finally(() => {
      this.generating = false;
    });
  }

  /**
   * Switch sidebar pane, and let the Steps tab drive guided review.
   *
   * `startWalkthrough` already selected Steps when review began, so the tab and
   * the mode were coupled in one direction only — you could enter review and
   * land on Steps, but selecting Steps did nothing. Closing the loop makes the
   * tab the visible switch for the mode.
   *
   * Leaving Steps deliberately does *not* exit: you often jump to Files or Log
   * mid-review to check something, and losing your position for it would be a
   * punishment for looking around. Exit is explicit, from the banner.
   *
   * Selecting Steps with no walkthrough yet does not start one either — that
   * shells out to an agent, which is not what clicking a tab should do. The
   * pane shows its Generate button instead.
   */
  /**
   * The count on a rail icon, or nothing when there is nothing to count.
   *
   * A badge reading `0` is worse than no badge: it draws the eye to a pane to
   * tell you it is empty.
   */
  private railBadge(pane: SidebarTab) {
    const count =
      pane === 'files'
        ? this.files.length
        : pane === 'walkthrough'
          ? (this.walkthrough?.steps.length ?? 0)
          : pane === 'review'
            ? this.pendingComments.length
            : 0;
    return count > 0 ? html`<span class="rail-count">${count}</span>` : nothing;
  }

  private selectTab(tab: SidebarTab) {
    // Clicking the pane you are already on folds the sidebar away; clicking any
    // other one brings it back on that pane. One control, and no extra chevron
    // hanging off the panel edge.
    if (tab === this.sidebarTab && !this.sidebarCollapsed) {
      this.sidebarCollapsed = true;
      return;
    }
    this.sidebarCollapsed = false;
    this.sidebarTab = tab;
    if (tab === 'walkthrough' && this.walkthrough && !this.walkActive) {
      this.walkActive = true;
      this.walkStep = -1;
    }
  }

  /** The keyboard/palette route, which does not care which pane is showing. */
  private toggleSidebar() {
    this.sidebarCollapsed = !this.sidebarCollapsed;
  }

  private startWalkthrough() {
    if (!this.walkthrough) {
      this.runGenerateWalkthrough();
      return;
    }
    this.walkActive = true;
    this.walkStep = -1;
    this.sidebarTab = 'walkthrough';
  }

  private exitWalkthrough() {
    this.walkActive = false;
    this.walkStep = -1;
    this.stackReview = null;
    if (this.sidebarTab === 'walkthrough') {
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
      if (this.viewMode === 'ops') {
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
    void this.refresh();
  }

  private async loadOperations() {
    try {
      this.operations = await getOperationLog(100);
      // An undo or a restore rewrites the log out from under a pinned anchor.
      // Left in place it would name an operation that is no longer there, and
      // no row would offer to compare to it.
      if (this.opCompareFrom && !this.operations.some((op) => op.id === this.opCompareFrom?.id)) {
        this.opCompareFrom = null;
        this.opDiff = null;
      }
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

  /**
   * Gate anything that rewrites a commit jj has marked immutable.
   *
   * jjdiff used to disable these actions outright, which is the safe default and
   * the wrong one for the case that actually comes up: fixing your own already-
   * pushed commit. So the affordance exists, and the guarantee is preserved by
   * making the user say yes to a description of what they are about to break —
   * not by a mode, a setting, or a checkbox that stays ticked.
   *
   * Returns true for mutable changes without asking anything.
   */
  private confirmImmutableRewrite(change: Change, verb: string): Promise<boolean> {
    if (!change.immutable) return Promise.resolve(true);
    const label = change.description.split('\n')[0] || change.changeId.slice(0, 8);
    // Naming the bookmark makes the consequence concrete instead of theoretical:
    // "you will need to force-push main" lands differently than "may require a
    // force push".
    // Abandon deletes rather than rewrites, and saying "gives it a new commit id"
    // about a commit that is about to stop existing is the kind of small
    // inaccuracy that teaches people to stop reading these dialogs.
    const effect =
      verb === 'Abandon'
        ? 'Abandoning it drops the commit from history entirely.'
        : 'Rewriting it gives it a new commit id; the old one stays where it already is.';
    const shared = change.bookmarks.length
      ? `It is published as ${change.bookmarks.join(', ')}. The remote still has the original, so pushing over it needs a force — and anyone who already pulled it has to reconcile.`
      : 'Anyone who already has this commit — the remote, CI, a teammate — keeps the original; they do not follow along.';
    return askConfirm({
      heading: `${verb} "${label}"? This change is immutable.`,
      detail:
        'jj marks a commit immutable once it is published — part of trunk, tagged, or already on a remote — precisely to stop this happening by accident.\n\n' +
        `${effect} ${shared}\n\n` +
        'Everything built on top is rebased onto the result. jjdiff passes --ignore-immutable for this one command only, and the whole thing is undoable from the Ops tab.',
      confirmLabel: `${verb} anyway`,
      danger: true,
    });
  }

  private startDescriptionEdit() {
    const change = this.selectedChange;
    if (!change) return;
    this.description = change.description;
    this.editingDescription = true;
  }

  private cancelDescriptionEdit() {
    this.description = this.selectedChange?.description ?? '';
    this.editingDescription = false;
  }

  private async saveDescription() {
    const change = this.selectedChange;
    if (!change) return;
    if (!(await this.confirmImmutableRewrite(change, 'Rewrite'))) return;
    await this.command('describe', () =>
      describeChange(change.changeId, this.description, change.immutable),
    );
    // Back to reading on success only. A failed describe keeps the box open
    // with the text still in it, rather than discarding what was typed.
    if (!this.actionError) this.editingDescription = false;
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
  private async editSelected() {
    const change = this.selectedChange;
    if (!change) return;
    // `jj edit` is how you change an immutable commit's *code* — the working copy
    // lands on it and every subsequent save rewrites it.
    if (!(await this.confirmImmutableRewrite(change, 'Work on'))) return;
    void this.command('edit', async () => {
      const outcome = await editChange(change.changeId, change.immutable);
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

  private async abandonSelected() {
    const change = this.selectedChange;
    if (!change) return;
    const label = change.description.split('\n')[0] || change.changeId.slice(0, 8);
    // One dialog, not two: the immutable warning already says everything the
    // ordinary abandon confirmation would, and more.
    const ok = change.immutable
      ? await this.confirmImmutableRewrite(change, 'Abandon')
      : await askConfirm({
          heading: `Abandon "${label}"?`,
          detail: 'Undoable from the Ops tab.',
          confirmLabel: 'Abandon',
          danger: true,
        });
    if (!ok) return;
    void this.command('abandon', async () => {
      const outcome = await abandonChange(change.changeId, change.immutable);
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

  private async rebaseSelected() {
    const change = this.selectedChange;
    if (!change || !this.repo) return;
    // Warn before the destination prompt — nobody should fill in a form for an
    // operation they are then told they probably do not want.
    if (!(await this.confirmImmutableRewrite(change, 'Rebase'))) return;
    const destination = await askText({
      heading: 'Rebase onto which revision?',
      detail: 'A change id, bookmark, or revset (e.g. main, @-).',
      value: 'main',
      confirmLabel: 'Rebase',
    });
    if (!destination?.trim()) return;
    void this.command('rebase', () =>
      rebaseChange('source', change.changeId, destination.trim(), change.immutable),
    );
  }

  private async splitSelectedFiles() {
    const change = this.selectedChange;
    if (!change) return;
    const paths = this.focusPath ? [this.focusPath] : [...this.viewedPaths];
    if (paths.length === 0) {
      this.actionError =
        'Select a file (or mark the files to keep as viewed) before splitting.';
      return;
    }
    if (!(await this.confirmImmutableRewrite(change, 'Split'))) return;
    void this.command('split', () => splitPaths(change.changeId, paths, change.immutable));
  }

  private async restoreSelectedFile() {
    if (!this.isWorkingCopySelected) return;
    const paths = this.focusPath ? [this.focusPath] : [];
    const what = paths.length ? paths[0] : 'all working-copy changes';
    const ok = await askConfirm({
      heading: `Discard ${what}?`,
      detail: 'Undoable from the Ops tab.',
      confirmLabel: 'Discard',
      danger: true,
    });
    if (!ok) return;
    void this.command('restore', () => restorePaths(paths));
  }

  private async createBookmark() {
    const change = this.selectedChange;
    if (!change) return;
    const name = await askText({ heading: 'Bookmark name', confirmLabel: 'Create' });
    if (!name?.trim()) return;
    void this.command('bookmark', () => setBookmark(name.trim(), change.changeId));
  }

  private async removeBookmark(name: string) {
    const ok = await askConfirm({
      heading: `Delete bookmark "${name}"?`,
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
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
      // A push either creates the branch a proposal will attach to, or moves an
      // existing proposal's head — in which case its checks are now running
      // against something other than what they last reported.
      void this.refreshProposals();
      return result;
    });
  }

  private runUndo() {
    void this.command('undo', () => undo());
  }

  private async restoreTo(operation: Operation) {
    const ok = await askConfirm({
      heading: 'Restore the repository?',
      detail: `Back to just after:\n${operation.description}\n\nThis rewrites the working copy. It is itself undoable.`,
      confirmLabel: 'Restore',
      danger: true,
    });
    if (!ok) return;
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
    this.versionPair = null;
    void this.loadDiff();
  }

  private showFullDiff() {
    this.viewMode = 'full';
    this.versionPair = null;
    void this.loadDiff();
  }

  /**
   * Open the evolog drawer for the selected change.
   *
   * The list is fetched on open rather than with the change: most selections
   * never ask for it, and it is another `jj` process each time.
   */
  private openVersions() {
    const change = this.selectedChange;
    if (!change) return;
    this.versionsOpen = true;
    this.versions = [];
    this.versionsLoading = true;
    void (async () => {
      try {
        this.versions = await getChangeVersions(change.changeId);
      } catch (error) {
        this.actionError = String(error);
        this.versionsOpen = false;
      } finally {
        this.versionsLoading = false;
      }
    })();
  }

  private compareVersions(from: string, to: string) {
    this.versionsOpen = false;
    this.versionPair = { from, to };
    this.viewMode = 'interdiff';
    void this.loadDiff();
  }

  /**
   * Load jj's account of what one operation (or a span of them) changed.
   *
   * Toggling: asking for the row already open closes it, so the button is its
   * own dismiss and the log does not fill up with expanded prose.
   */
  private showOpDiff(to: Operation, from: Operation | null) {
    const key = from ? `${from.id}..${to.id}` : to.id;
    if (this.opDiff?.key === key) {
      this.opDiff = null;
      return;
    }
    this.opDiff = { key, text: 'Loading…' };
    void (async () => {
      try {
        const text = await getOperationDiff(to.id, from?.id ?? null);
        // A second request may have started while this one was in flight; only
        // the row still asking for this key should receive it.
        if (this.opDiff?.key === key) {
          this.opDiff = { key, text: text.trim() || 'This operation changed nothing.' };
        }
      } catch (error) {
        if (this.opDiff?.key === key) this.opDiff = { key, text: String(error) };
      }
    })();
  }

  private toggleLayout() {
    this.layout = this.layout === 'split' ? 'unified' : 'split';
  }

  /**
   * Switch palette at runtime.
   *
   * Shiki tokens carry their theme's colours, so `themeVersion` is bumped to
   * force re-tokenization; the CSS custom properties handle everything else.
   * Preview and commit are separate: hovering a swatch calls this and nothing
   * else, so nothing is written until a theme is actually chosen.
   */
  private applyTheme(theme: string) {
    this.theme = theme;
    applyThemeTokens(theme);
    this.themeVersion += 1;
  }

  /** Chosen for real — apply it and remember it. */
  private async chooseTheme(theme: string) {
    this.applyTheme(theme);
    try {
      await setUiTheme(theme);
    } catch (error) {
      // A theme that cannot be written is still a theme that works for this
      // session; say so rather than reverting what the user just picked.
      this.actionError = `Theme applied, but not saved: ${String(error)}`;
    }
  }

  private toggleWordWrap() {
    this.wordWrap = !this.wordWrap;
  }

  private toggleWhitespace() {
    this.ignoreWhitespace = !this.ignoreWhitespace;
    void this.loadDiff();
  }

  /** The preset the log is currently filtered by; falls back to the first ("All"). */
  private get activeScope(): { label: string; revset: string } {
    const current = this.graphRevset ?? '';
    return REVSET_PRESETS.find((preset) => preset.revset === current) ?? REVSET_PRESETS[0]!;
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
    const stackSize = this.repo?.stack.filter((c) => !c.immutable && !c.empty).length ?? 0;
    const commands: Command[] = [];
    const add = (group: string, entries: (Command | false)[]) => {
      for (const entry of entries) {
        if (entry) commands.push({ ...entry, group });
      }
    };

    // Ordered by how often a hand reaches for them. The view toggles used to be
    // first and are the least-used thing here — five switches sitting above the
    // verbs that actually do something. "Review" also used to hold both the
    // review workflow *and* find/editor/shortcuts, which share nothing but the
    // fact that nothing else fit; those are Tools now.

    add('Change', [
      !!change && { id: 'jj-edit', label: 'Work on This Change (jj edit)', run: () => this.editSelected() },
      { id: 'jj-new', label: 'New Change on Top (jj new)', run: () => this.newOnSelected() },
      isWc && { id: 'jj-absorb', label: 'Absorb Into Ancestors (jj absorb)', run: () => this.runAbsorb() },
      !!change && { id: 'jj-rebase', label: 'Rebase…  (jj rebase)', run: () => void this.rebaseSelected() },
      !!change && { id: 'jj-split', label: 'Split File Out (jj split)', run: () => this.splitSelectedFiles() },
      { id: 'jj-duplicate', label: 'Duplicate Change (jj duplicate)', run: () => this.duplicateSelected() },
      { id: 'jj-backout', label: 'Back Out Change (jj backout)', run: () => this.backoutSelected() },
      !!change && { id: 'jj-abandon', label: 'Abandon Change (jj abandon)', run: () => void this.abandonSelected() },
      isWc && {
        id: 'jj-restore',
        label: 'Discard Working-Copy Changes (jj restore)',
        run: () => void this.restoreSelectedFile(),
      },
    ]);

    add('Repository', [
      { id: 'jj-fetch', label: 'Fetch (jj git fetch)', run: () => this.runFetch() },
      { id: 'jj-push', label: 'Push (jj git push)', run: () => this.runPush() },
      { id: 'jj-bookmark', label: 'Create Bookmark…', run: () => void this.createBookmark() },
      {
        id: 'refresh',
        label: 'Reload Repository',
        run: () => {
          void this.refresh();
          void this.refreshProposals();
        },
      },
      { id: 'open-repo', label: 'Open Repository…', run: () => void this.openFolder() },
      {
        id: 'open-repo-window',
        label: 'Open Repository in New Window…',
        run: () => void this.openFolderInNewWindow(),
      },
    ]);

    add('Review', [
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

    // Only on a forge we can actually drive — an affordance that always fails
    // is worse than one that is absent.
    if (this.forge) {
      const noun = this.forge.noun;
      const Noun = noun.replace(/\b\w/g, (c) => c.toUpperCase());
      add('Forge', [
        {
          id: 'pr-open',
          label: `Review ${Noun}…`,
          hint: 'by number',
          run: () => void this.promptForProposal(),
        },
        {
          id: 'pr-list',
          label: `List Open ${Noun}s`,
          run: () => void this.showProposalList(),
        },
        !!this.pullRequest && {
          id: 'pr-view',
          label: `Show ${Noun}`,
          hint: `#${this.pullRequest.number}`,
          run: () => this.showProposalView(),
        },
        !!this.pullRequest && {
          id: 'pr-review',
          label: 'Submit Review…',
          run: () => void this.openReviewComposer(),
        },
        !!this.pullRequest && {
          id: 'pr-whole',
          label: this.prRevset ? `Diff This Change Only` : `Diff Whole ${Noun}`,
          run: () => void this.toggleProposalDiff(),
        },
      ]);
    }

    add('History', [
      { id: 'jj-undo', label: 'Undo Last Operation (jj undo)', run: () => this.runUndo() },
      {
        id: 'ops',
        label: 'Show Operation Log',
        run: () => {
          this.viewMode = 'ops';
          void this.loadOperations();
        },
      },
      !!change && {
        id: 'versions',
        label: 'Compare Versions of This Change…',
        hint: 'evolog',
        run: () => this.openVersions(),
      },
      !!this.versionPair && {
        id: 'versions-exit',
        label: 'Stop Comparing Versions',
        run: () => this.showFullDiff(),
      },
    ]);

    add('Tools', [
      {
        id: 'find',
        label: 'Find in Diffs',
        hint: formatShortcut('Mod+f'),
        run: () => this.openSearch(),
      },
      {
        id: 'open-in-editor',
        label: 'Open File in Editor',
        hint: 'o',
        run: () => void this.openFileInEditor(),
      },
      {
        id: 'shortcuts',
        label: 'Keyboard Shortcuts',
        hint: '?',
        run: () => (this.shortcutsOpen = true),
      },
      {
        id: 'set-editor',
        label: 'Set Editor Command…',
        hint: 'for o',
        run: () => void this.configureEditor(),
      },
      {
        id: 'install-terminal-helper',
        label: 'Install Terminal Helper…',
        hint: 'add `jjdiff` to PATH',
        run: () => void this.runInstallTerminalHelper(),
      },
    ]);

    add('View', [
      {
        id: 'toggle-sidebar',
        label: this.sidebarCollapsed ? 'Show Sidebar' : 'Hide Sidebar',
        hint: formatShortcut('Mod+b'),
        run: () => this.toggleSidebar(),
      },
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
      // One entry, not twenty. Choosing a palette is a visual decision and the
      // picker shows the colours; a list of names here would be the same
      // decision made blind.
      {
        id: 'theme',
        label: 'Theme…',
        hint: THEMES.find((entry) => entry.id === this.theme)?.label ?? this.theme,
        run: () => (this.themePickerOpen = true),
      },
      {
        id: 'theme-cycle-mode',
        label: 'Toggle Light / Dark',
        hint: 'base themes',
        run: () => void this.chooseTheme(this.theme === 'dark' ? 'light' : 'dark'),
      },
    ]);

    return commands;
  }

  protected override render() {
    this.style.setProperty('--jj-sidebar-w', `${this.sidebarWidth}px`);
    // A class on the host rather than a width of 0: the grid rule owns the
    // collapsed geometry, so the resize handle's stored width survives a fold
    // and the panel comes back exactly as wide as it was.
    this.classList.toggle('sidebar-collapsed', this.sidebarCollapsed);
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
      <!-- The window's drag handle. With titleBarStyle: Overlay the WebView
           covers the title bar, so the OS no longer gets the mousedown that
           moves the window — this row has to offer it back.

           "deep", not a bare attribute. Bare means "only a direct click on this
           exact element", which the header almost never is: the spacer, the
           tool-group wrappers and every label span sit on top of it, so most of
           the bar was dead. Tauri's own walk stops at the first clickable
           element it meets on the way up, so buttons, inputs and menu items
           still click rather than drag. -->
      <header data-tauri-drag-region="deep">
        <span class="repo-menu-root">
          <button class="repo-button" @click=${this.toggleRepoMenu} title=${this.repo.root}>
            <span class="chip accent">${folderIcon(false)}</span>
            <span class="root">${basename(this.repo.root)}</span>
            <span class="fold-chevron ${this.repoMenuOpen ? 'up' : ''}">${iconChevron}</span>
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
        <!-- Ordered by what the verbs do to the repository, left to right:
             bring work in (fetch), rearrange the work you have (absorb), take a
             step back (undo). The old order put absorb first, which read as
             "the main thing you do here" — it is the rarest of the three.
             The view toggle changes nothing and lives in its own group. -->
        <span class="tool-group">
          <button
            class="tool icon"
            title="jj git fetch — update remote-tracking state"
            ?disabled=${!!this.busy}
            @click=${this.runFetch}
          >
            ${iconFetch}
          </button>
          ${isWc
            ? html`<button
                class="tool icon"
                title="jj absorb — auto-distribute working-copy changes into the relevant ancestors"
                ?disabled=${!!this.busy || this.files.length === 0}
                @click=${this.runAbsorb}
              >
                ${iconAbsorb}
              </button>`
            : nothing}
          <button
            class="tool icon"
            title="jj undo — reverse the last operation"
            ?disabled=${!!this.busy}
            @click=${this.runUndo}
          >
            ${iconUndo}
          </button>
        </span>
        <span class="tool-group">
          <button
            class="tool icon"
            title="Switch between side-by-side and unified diffs"
            @click=${this.toggleLayout}
          >
            ${this.layout === 'split' ? iconSplit : iconUnified}
          </button>
        </span>
        <button
          class="tool palette-key"
          title="Everything else lives here (Mod+K)"
          @click=${() => (this.barOpen = true)}
        >
          <kbd>⌘</kbd><kbd>K</kbd>
        </button>
      </header>
      <!-- The pane switcher is a rail of icons, not a row of tabs.

           Four labels plus two count badges never fit a 292px sidebar at a
           readable size — the segmented control was already truncating, and the
           indicator looked wrong because equal quarters give "Log" the same
           width as "Files 17". A rail costs one fixed column, scales to any
           number of panes, and hands the whole sidebar width back to the
           content. The pane's name is not hidden: it is the sidebar's title. -->
      <nav class="rail" aria-label="Sidebar panes">
        ${RAIL_PANES.map(
          (pane) => html`<button
            class="rail-item ${this.sidebarTab === pane.id ? 'active' : ''}"
            title=${
              this.sidebarTab === pane.id && !this.sidebarCollapsed
                ? `Hide ${pane.label}`
                : pane.label
            }
            aria-label=${pane.label}
            aria-expanded=${this.sidebarTab === pane.id && !this.sidebarCollapsed}
            aria-current=${this.sidebarTab === pane.id ? 'page' : nothing}
            @click=${() => this.selectTab(pane.id)}
          >
            ${pane.icon}
            ${this.railBadge(pane.id)}
          </button>`,
        )}
      </nav>
      <aside>
        <div class="pane-head">
          <h2 class="pane-title">${RAIL_PANES.find((p) => p.id === this.sidebarTab)?.label}</h2>
          ${this.sidebarTab === 'walkthrough' && this.walkStale
            ? html`<span class="stale-dot" title="The change moved since this was generated"></span>`
            : nothing}
        </div>
        ${this.sidebarTab === 'stack'
          ? html`<div class="revset-bar">
                <!-- Scope, then search: which commits are in the list, then
                     which of those you are looking for. The scope used to be a
                     deck of pills that collapsed to initials — clever, and
                     unreadable: it hid five of six options behind a hover, so
                     the only way to learn what the filters were was to sweep
                     the mouse across them one at a time. -->
                <span class="scope-root">
                  <button
                    class="scope-button"
                    title="Which commits the log shows"
                    aria-expanded=${this.scopeOpen}
                    @click=${() => (this.scopeOpen = !this.scopeOpen)}
                  >
                    <span class="scope-label">${this.activeScope.label}</span>
                    <span class="fold-chevron ${this.scopeOpen ? 'up' : ''}">${iconChevron}</span>
                  </button>
                  ${this.scopeOpen
                    ? html`<div class="scope-menu" role="menu">
                        ${REVSET_PRESETS.map((preset) => {
                          const on = (this.graphRevset ?? '') === preset.revset;
                          return html`<button
                            class="scope-item ${on ? 'on' : ''}"
                            role="menuitem"
                            @click=${() => {
                              this.scopeOpen = false;
                              this.applyRevset(preset.revset);
                            }}
                          >
                            <span class="scope-item-label">${preset.label}</span>
                            <!-- The revset itself, because this app is for
                                 people who write them and the preset is a
                                 shortcut, not a replacement. -->
                            <code>${preset.revset || 'all()'}</code>
                          </button>`;
                        })}
                      </div>`
                    : nothing}
                </span>
                <label class="field">
                  <span class="field-icon">${iconSearch}</span>
                  <input
                    class="revset-input"
                    placeholder="Search commits…"
                    .value=${this.revsetSearch}
                    @input=${(event: Event) =>
                      (this.revsetSearch = (event.target as HTMLInputElement).value)}
                  />
                </label>
              </div>
              <div class="stack">
              <jj-log-graph
                .changes=${this.filteredGraph}
                .selected=${selectedId}
                @change-selected=${(event: CustomEvent<Change>) => this.select(event.detail)}
              ></jj-log-graph>
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
                ? html`<button class="context-card" @click=${() => this.selectTab('stack')}>
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
                ${this.sidebarTab === 'walkthrough'
                  ? this.renderWalkthroughTab()
                  : html`<jj-file-tree
                      .files=${this.files}
                      .selected=${this.focusPath}
                      .viewed=${this.viewedPaths}
                      @file-selected=${(event: CustomEvent<string | null>) => {
                        this.focusPath = event.detail;
                      }}
                      @file-menu=${(event: CustomEvent<FileMenuRequest>) => {
                        this.fileMenu = event.detail;
                      }}
                    ></jj-file-tree>`}
              </div>
            `}
      </aside>
      <div
        class="sidebar-resize"
        @mousedown=${this.onSidebarResizeStart}
      ></div>
      <main
        class=${this.viewMode === 'pr'
          ? 'showing-pr'
          : this.viewMode === 'ops'
            ? 'showing-ops'
            : ''}
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
        ${this.viewMode === 'pr' && this.pullRequest
          ? this.renderProposalView(this.pullRequest)
          : nothing}
        ${this.viewMode === 'ops'
          ? this.renderOperationLog()
          : change && this.detailView
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
                <!-- The card's own header: mark, title, then the identity line
                     under it. The subject used to live in the folded body, so
                     collapsing the card hid the one thing that names it and a
                     truncated copy had to be rendered up here instead. It is
                     the title now, and folding hides only the detail. -->
                <span class="chip accent detail-mark">${iconCommit}</span>
                <span class="detail-headings">
                  <h2 class="detail-title">
                    ${change.description.split('\n')[0] || '(no description)'}
                  </h2>
                  <span class="detail-meta">
                    <span class="detail-id">${change.changeId.slice(0, 12)}</span>
                    ${change.bookmarks.map(
                      (bookmark) => html`<span class="tag"
                        >${bookmark}${this.renderTracking(bookmark)}
                        <button
                          class="tag-x"
                          title="Delete bookmark"
                          @click=${(event: Event) => {
                            event.stopPropagation();
                            void this.removeBookmark(bookmark);
                          }}
                        >
                          ×
                        </button></span
                      >`,
                    )}
                    ${change.immutable ? html`<span class="tag muted">immutable</span>` : nothing}
                    ${change.conflict ? html`<span class="tag warn">conflict</span>` : nothing}
                    ${change.empty ? html`<span class="tag muted">empty</span>` : nothing}
                  </span>
                </span>
                <span class="spacer"></span>
                <span class="detail-when"
                  >${change.author.name} · ${relativeTime(change.committer.timestamp)}</span
                >
                <span class="fold-chevron ${this.detailCollapsed ? 'closed' : ''}"
                  >${iconChevron}</span
                >
              </header>

              <!-- Kept mounted and folded rather than unmounted: an element that
                   does not exist cannot animate out, and a card that snaps shut
                   loses the one cue that says where the content went. -->
              <div class="fold ${this.detailCollapsed ? 'closed' : ''}">
                <div>
              <!-- Prose left, actions right. The description is capped at a
                   readable measure, so on a wide window the right half of the
                   card sat empty while the buttons queued up underneath it.
                   The file list stays full width, below both. -->
              <div class="detail-main">
                <div class="detail-prose">
              <!-- Guided review is a *reading* mode: you are being walked
                   through someone's change, step by step. Offering to rewrite
                   its message mid-walkthrough is an invitation to edit the
                   thing you are in the middle of reviewing, so the control is
                   gone until the walkthrough is exited. The message itself
                   still shows. -->
              ${this.editingDescription && !this.walkActive
                ? html`<textarea
                      class="detail-edit"
                      .value=${this.description}
                      @input=${(event: Event) =>
                        (this.description = (event.target as HTMLTextAreaElement).value)}
                    ></textarea>
                    <div class="detail-actions">
                      <button
                        class="tool ${change.immutable ? 'danger' : ''}"
                        title=${
                          change.immutable
                            ? 'jj describe --ignore-immutable — rewrite the message of a published commit. You will be asked to confirm.'
                            : 'jj describe — save this message onto the change.'
                        }
                        ?disabled=${this.description === change.description}
                        @click=${this.saveDescription}
                      >
                        Save description
                      </button>
                      <button class="tool" @click=${this.cancelDescriptionEdit}>Cancel</button>
                    </div>`
                : html`${descriptionBody(change.description)
                      ? html`<div class="detail-desc">${descriptionBody(change.description)}</div>`
                      : nothing}`}
                </div>

              <!-- Four verbs out, the rest behind "More". The split is by how
                   often a hand reaches for them, not by what jj calls them. -->
              <div class="detail-actions detail-verbs">
                <button
                  class="tool primary"
                  title=${`jj edit — move the working copy onto this change so your edits land in it.${
                    change.immutable
                      ? ' This change is immutable; jjdiff will explain what rewriting it costs before doing anything.'
                      : ''
                  }`}
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
                <!-- Third, because the row is grouped by what each verb touches:
                     the working copy, then this change's own message, then the
                     refs pointing at it. Hidden while the editor is open — Save
                     and Cancel are down in the prose column with the text they
                     act on — and during guided review, which is a reading mode. -->
                ${this.editingDescription || this.walkActive
                  ? nothing
                  : html`<button
                      class="tool"
                      title=${
                        change.immutable
                          ? 'jj describe --ignore-immutable — this change is immutable, so you will be asked to confirm before the message is rewritten.'
                          : 'jj describe — edit this change\'s message.'
                      }
                      @click=${this.startDescriptionEdit}
                    >
                      Edit description
                    </button>`}
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

                <span class="more-root">
                  <button
                    class="tool"
                    title="Rebase, split, duplicate, back out, abandon"
                    aria-expanded=${this.moreAt !== null}
                    @click=${this.toggleMore}
                  >
                    More
                    <span class="fold-chevron ${this.moreAt ? 'up' : ''}">${iconChevron}</span>
                  </button>
                  ${this.moreAt ? this.renderMoreMenu(change, this.moreAt) : nothing}
                </span>
              </div>
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
              </div>
                </div>
              </div>
            </section>`
          : change && !this.walkActive
            ? html`<div class="describe">
                <!-- Hidden during guided review for the same reason the detail
                     card's edit button is: a walkthrough is for reading a
                     change, and a compose box at the top of it is an invitation
                     to start writing instead.

                     Header then content, the same shape as the change detail
                     card. The textarea carries no frame of its own: a bordered
                     box inside a bordered card is two frames around one field. -->
                <div class="describe-head">
                  <span class="chip accent">${iconCommit}</span>
                  <span class="describe-headings">
                    <span class="describe-title">Working copy</span>
                    <span class="describe-count">
                      ${this.files.length
                        ? `${this.files.length} file${this.files.length === 1 ? '' : 's'} changed`
                        : 'No changes yet'}
                    </span>
                  </span>
                  <span class="spacer"></span>
                  <button
                    class="tool"
                    ?disabled=${this.description === change.description}
                    @click=${this.saveDescription}
                  >
                    Describe
                  </button>
                  <button
                    class="tool"
                    ?disabled=${this.files.length === 0}
                    title="Discard the focused file's changes (or all when none is focused)"
                    @click=${this.restoreSelectedFile}
                  >
                    Discard…
                  </button>
                  <button
                    class="tool primary"
                    ?disabled=${this.files.length === 0 || !this.description.trim()}
                    title="Describe @ and start a new change on top (jj describe + jj new)"
                    @click=${this.commitAndNew}
                  >
                    Commit &amp; New
                  </button>
                </div>
                <textarea
                  placeholder="Describe this change…"
                  .value=${this.description}
                  @input=${(event: Event) =>
                    (this.description = (event.target as HTMLTextAreaElement).value)}
                ></textarea>
              </div>`
            : nothing}
        ${change?.conflict
          ? html`<div class="banner conflict">
              <span class="chip warn">${iconWarn}</span>
              <span
                >This change has unresolved conflicts
                (${this.conflictedPaths.size || '?'} file${this.conflictedPaths.size === 1
                  ? ''
                  : 's'}) — resolve with <code>jj resolve</code> in a terminal.</span
              >
            </div>`
          : nothing}
        ${this.versionPair
          ? html`<div class="banner">
              <span class="chip">${iconInfo}</span>
              <span>
                Comparing two versions of this change —
                <code>${this.versionPair.from.slice(0, 12)}</code> →
                <code>${this.versionPair.to.slice(0, 12)}</code>. Rebase noise is excluded.
              </span>
              <span class="spacer"></span>
              <button class="tool" @click=${this.openVersions}>Pick Versions</button>
              <button class="tool" @click=${this.showFullDiff}>Full Diff</button>
            </div>`
          : nothing}
        ${this.changedSinceReview && !this.versionPair
          ? html`<div class="banner">
              <span class="chip">${iconInfo}</span>
              <span>This change evolved since you reviewed it.</span>
              <span class="spacer"></span>
              ${this.viewMode === 'interdiff'
                ? html`<button class="tool" @click=${this.showFullDiff}>Full Diff</button>`
                : html`<button class="tool" @click=${this.showInterdiff}>
                    What Changed Since Review
                  </button>`}
              <button class="tool" @click=${this.markCurrentReviewed}>Mark Reviewed</button>
            </div>`
          : nothing}
        ${this.renderPullRequestBanner()}
        <!-- Above the step block, not below it. "This was generated for an
             older version" changes how much you should trust everything under
             it, so it has to be read before that, not after. -->
        ${this.walkthrough && this.walkStale && !this.generating
          ? html`<div class="banner">
              <span class="chip">${iconInfo}</span>
              <span>The walkthrough was generated for an older version of this change.</span>
              <span class="spacer"></span>
              <button class="tool" @click=${this.runGenerateWalkthrough}>Refresh Walkthrough</button>
            </div>`
          : nothing}
        ${this.walkActive && this.walkthrough
          ? html`<div class="walk-banner">
              ${keyed(
                this.walkStep,
                html`<div class="walk-content">
              <div class="walk-head">
                <span class="chip accent">${iconSparkle}</span>
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
                <!-- The way out, and it has to be here.

                     Guided review hides the describe box and the edit-message
                     button, because a walkthrough is for reading. That made the
                     mode a trap: the only way to leave it was a command palette
                     entry, so the describe box simply vanished with nothing on
                     screen explaining where it went or how to get it back.

                     Separated from Prev/Next by a rule: those two move *within*
                     the review, this one ends it. -->
                <button
                  class="tool walk-exit"
                  title="Leave guided review and go back to the full change"
                  @click=${this.exitWalkthrough}
                >
                  Exit review
                </button>
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
              <p class="walk-narrative ${this.walkExpanded ? 'expanded' : ''}">
                ${this.walkStep < 0
                  ? this.walkthrough.summary
                  : this.walkthrough.steps[this.walkStep]?.narrative}
              </p>
              ${this.walkOverflow
                ? html`<button
                    class="walk-more"
                    @click=${() => (this.walkExpanded = !this.walkExpanded)}
                  >
                    ${this.walkExpanded ? 'Show less' : 'Show more'}
                    <span class="fold-chevron ${this.walkExpanded ? 'up' : ''}"
                      >${iconChevron}</span
                    >
                  </button>`
                : nothing}
                </div>`,
              )}
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
        <!-- No breadcrumb: the diff pane pins the current file's own header,
             which names the file *and* carries its actions. A separate crumb
             put the same path on screen twice, one line apart. -->

        ${this.viewMode === 'pr'
          ? nothing
          : html`<jj-patch-view
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
              .canComment=${!this.walkActive && this.selectedChange !== null}
              .revset=${this.isWorkingCopySelected ? null : this.selected}
              .markdownPreviews=${this.markdownPreviews}
              @add-comment=${(e: CustomEvent) => this.onAddComment(e.detail)}
              @resolve-comment=${(e: CustomEvent<{ id: number; value: boolean }>) =>
                this.onResolveComment(e.detail.id, e.detail.value)}
              @delete-comment=${(e: CustomEvent<{ id: number; value: boolean }>) =>
                this.onDeleteComment(e.detail.id)}
            ></jj-patch-view>`}
      </main>
      ${this.barOpen
        ? html`<jj-command-bar
            .commands=${this.proposalPicker ?? this.commands}
            @close=${() => {
              this.barOpen = false;
              this.proposalPicker = null;
            }}
          ></jj-command-bar>`
        : nothing}
      ${this.themePickerOpen
        ? html`<jj-theme-picker
            .current=${this.theme}
            @preview-theme=${(event: CustomEvent<string>) => this.applyTheme(event.detail)}
            @pick-theme=${(event: CustomEvent<string>) => void this.chooseTheme(event.detail)}
            @close=${() => (this.themePickerOpen = false)}
          ></jj-theme-picker>`
        : nothing}
      ${this.shortcutsOpen
        ? html`<jj-shortcuts-help
            .commandBar=${this.commandBarBinding}
            @close=${() => (this.shortcutsOpen = false)}
          ></jj-shortcuts-help>`
        : nothing}
      ${this.versionsOpen
        ? html`<jj-evolog-drawer
            .versions=${this.versions}
            .changeId=${this.selectedChange?.changeId ?? ''}
            ?loading=${this.versionsLoading}
            @compare-versions=${(event: CustomEvent<{ from: string; to: string }>) =>
              this.compareVersions(event.detail.from, event.detail.to)}
            @close=${() => (this.versionsOpen = false)}
          ></jj-evolog-drawer>`
        : nothing}
      ${this.renderFileMenu()}
    `;
  }

  /**
   * The proposal the selected change belongs to: identity, merge state, checks
   * and reviewers, above the diff it describes.
   *
   * This is *context*, not a mode. It appears because a bookmark on the change
   * matched an open proposal, so working on your own branch shows its CI and
   * reviewers without asking. Reviewing the whole proposal rather than the one
   * commit is a toggle, not a different screen.
   *
   * Colour follows DESIGN.md — no new hue. Check and review outcomes reuse the
   * added/removed semantics (they are pass/fail), everything else is neutral.
   */
  private renderPullRequestBanner() {
    const pr = this.pullRequest;
    // The indicator is an entry point to the proposal view; inside that view it
    // would just repeat the title immediately above itself.
    if (!pr || this.viewMode === 'pr') return nothing;
    const whole = this.prRevset !== null;
    const conflicting = pr.mergeable === 'CONFLICTING';
    return html`
      <div class="pr-banner">
        <div class="pr-head">
          ${proposalState(pr)}
          <button
            class="pr-open"
            title=${`Open #${pr.number} in jjdiff`}
            @click=${() => this.showProposalView()}
          >
            <span class="pr-number">#${pr.number}</span>
            <strong class="pr-title">${pr.title}</strong>
          </button>
          <span class="spacer"></span>
          ${whole
            ? html`<span class="pr-scope">whole ${this.forge?.noun ?? 'PR'}</span>`
            : nothing}
          <button class="tool" @click=${() => void this.toggleProposalDiff()}>
            ${whole ? 'This change only' : 'Diff whole PR'}
          </button>
          <button class="tool" @click=${() => void this.openReviewComposer()}>Review…</button>
        </div>
        <div class="pr-meta">
          <span>${pr.author}</span>
          <span class="pr-branches">
            <code>${pr.base}</code> ← <code>${pr.head}</code>
          </span>
          ${pr.additions || pr.deletions
            ? html`<span class="pr-stat">
                <span class="plus">+${pr.additions}</span>
                <span class="minus">−${pr.deletions}</span>
              </span>`
            : nothing}
          ${conflicting
            ? html`<span class="pr-conflict" title="This ${this.forge?.noun ?? 'pull request'} has conflicts with its base branch"
                >⚠ conflicts</span
              >`
            : nothing}
          ${this.renderChecks(pr)} ${this.renderHeadDrift(pr)}
          ${pr.reviewers.length
            ? html`<span class="pr-reviewers">
                ${pr.reviewers.map(
                  (reviewer) => html`<span
                    class="tag muted ${reviewer.state === 'APPROVED'
                      ? 'approved'
                      : reviewer.state === 'CHANGES_REQUESTED'
                        ? 'changes'
                        : ''}"
                    title=${reviewerTitle(reviewer.state)}
                    >${reviewer.state === 'APPROVED'
                      ? '✓ '
                      : reviewer.state === 'CHANGES_REQUESTED'
                        ? '✕ '
                        : ''}${reviewer.name}</span
                  >`,
                )}
              </span>`
            : nothing}
        </div>
      </div>
      ${this.reviewDraft ? this.renderReviewComposer(pr) : nothing}
    `;
  }

  /**
   * Switch the main pane to the proposal.
   *
   * The description and conversation load here rather than with the banner:
   * they cost two more `gh` calls, and most selections never open this view.
   * Already-loaded content is reused, so coming back is instant.
   */
  private showProposalView() {
    const pr = this.pullRequest;
    if (!pr) return;
    this.viewMode = 'pr';
    if (this.prDetailsFor !== pr.number) void this.loadProposalDetails(pr);
  }

  /**
   * The operation log, with jj's own account of what each operation did.
   *
   * "What changed" answers the question the log itself cannot: a row says
   * `rebase commit 1da27fbb onto main`, and the diff says which commits moved,
   * where the working copy went and which bookmarks followed. Comparing a pair
   * covers the other case — several operations ago something went wrong, and
   * what matters is the span, not any one step.
   *
   * Snapshots are filtered out here rather than in the query: they are jj's
   * bookkeeping, they outnumber real operations, and none of them is a place
   * you would want to restore to. `index` is therefore the position in the
   * *filtered* list, which is what makes index 0 the current operation.
   */
  private renderOperationLog() {
    const visible = this.operations.filter((operation) => !operation.snapshot);
    const anchor = this.opCompareFrom;
    return html`<div class="ops-view">
      <div class="ops-header">
        <h2>Operation Log</h2>
        <span class="spacer"></span>
        ${anchor
          ? html`<span class="op-anchor"
              >Comparing from <strong>${anchor.description}</strong>
              <button class="tool" @click=${() => (this.opCompareFrom = null)}>Cancel</button>
            </span>`
          : nothing}
        <button class="tool" @click=${() => (this.viewMode = 'full')}>Back to Diff</button>
      </div>
      ${visible.length === 0
        ? html`<div class="ops-empty">No operations recorded yet.</div>`
        : visible.map((operation, index) => {
            const single = this.opDiff?.key === operation.id;
            const spanKey = anchor ? `${anchor.id}..${operation.id}` : null;
            const span = spanKey !== null && this.opDiff?.key === spanKey;
            // An operation can only be compared *to* something newer than the
            // anchor, and the log is newest first — so anything at or below the
            // anchor's own row is not a comparison, it is the same point twice.
            const anchorIndex = anchor ? visible.findIndex((entry) => entry.id === anchor.id) : -1;
            const comparable = anchor !== null && index < anchorIndex;
            return html`<div class="op ${single || span ? 'expanded' : ''}">
              <div class="op-head">
                <span class="op-when">${relativeTime(operation.time)}</span>
                ${index === 0 ? html`<span class="op-current">current</span>` : nothing}
                ${anchor?.id === operation.id
                  ? html`<span class="op-current">comparing from</span>`
                  : nothing}
              </div>
              <div class="op-desc">${operation.description}</div>
              ${operation.args ? html`<code class="op-args">${operation.args}</code>` : nothing}
              <div class="op-actions">
                ${index === 0
                  ? html`<button class="tool" @click=${this.runUndo}>Undo</button>`
                  : html`<button class="tool" @click=${() => void this.restoreTo(operation)}>
                      Restore here
                    </button>`}
                <button class="tool" @click=${() => this.showOpDiff(operation, null)}>
                  ${single ? 'Hide changes' : 'What changed'}
                </button>
                ${comparable
                  ? html`<button class="tool" @click=${() => this.showOpDiff(operation, anchor)}>
                      ${span ? 'Hide comparison' : 'Compare to here'}
                    </button>`
                  : anchor?.id === operation.id
                    ? nothing
                    : html`<button
                        class="tool"
                        title="Pin this operation as the older end of a comparison"
                        @click=${() => (this.opCompareFrom = operation)}
                      >
                        Compare from here
                      </button>`}
              </div>
              ${single || span
                ? html`<pre class="op-diff">${this.opDiff?.text}</pre>`
                : nothing}
            </div>`;
          })}
    </div>`;
  }

  /**
   * The proposal as its own view: description, then the conversation.
   *
   * A view rather than a panel above the diff. Everything here is prose of
   * unbounded length, and hanging it over the diff meant the code — the thing
   * being reviewed — started halfway down the window.
   *
   * Links go through a delegated handler; the WebView has no tabs, so an
   * ordinary `<a>` click does nothing at all.
   */
  private renderProposalView(pr: PullRequest) {
    return html`<div class="pr-view" @click=${this.onProposalLinkClick}>
      <div class="pr-view-head">
        ${proposalState(pr)}
        <span class="pr-number">#${pr.number}</span>
        <h2>${pr.title}</h2>
        <span class="spacer"></span>
        <button class="tool" @click=${() => void this.toggleProposalDiff()}>
          ${this.prRevset ? 'This change only' : 'Diff whole PR'}
        </button>
        <button class="tool" @click=${() => void this.openReviewComposer()}>Review…</button>
        <button class="tool" @click=${() => (this.viewMode = 'full')}>Back to Diff</button>
      </div>
      <div class="pr-view-meta">
        <span>${pr.author}</span>
        <span class="pr-branches"><code>${pr.base}</code> ← <code>${pr.head}</code></span>
        ${pr.additions || pr.deletions
          ? html`<span class="pr-stat">
              <span class="plus">+${pr.additions}</span>
              <span class="minus">−${pr.deletions}</span>
            </span>`
          : nothing}
        ${pr.mergeable === 'CONFLICTING'
          ? html`<span class="pr-conflict">⚠ conflicts</span>`
          : nothing}
        ${this.renderChecks(pr)} ${this.renderHeadDrift(pr)}
        ${pr.reviewers.length
          ? html`<span class="pr-reviewers">
              ${pr.reviewers.map(
                (reviewer) => html`<span
                  class="tag muted ${reviewer.state === 'APPROVED'
                    ? 'approved'
                    : reviewer.state === 'CHANGES_REQUESTED'
                      ? 'changes'
                      : ''}"
                  title=${reviewerTitle(reviewer.state)}
                  >${reviewer.state === 'APPROVED'
                    ? '✓ '
                    : reviewer.state === 'CHANGES_REQUESTED'
                      ? '✕ '
                      : ''}${reviewer.name}</span
                >`,
              )}
            </span>`
          : nothing}
        <span class="spacer"></span>
        <a class="pr-more" href=${pr.url}>Open on GitHub →</a>
      </div>
      ${this.renderProposalDetails(pr)}
    </div>`;
  }

  /** Description + conversation, shared by the proposal view. */
  private renderProposalDetails(pr: PullRequest) {
    const hasBody = this.prBody !== null;
    if (!hasBody && this.prActivity.length === 0) {
      return html`<div class="pr-empty">
        <div class="pr-empty-glyph">💬</div>
        <div class="pr-empty-title">No description or comments yet</div>
        <div class="pr-empty-hint">
          Anything written on #${pr.number} shows up here.
        </div>
      </div>`;
    }
    return html`<div class="pr-details">
      ${hasBody ? html`<div class="pr-body markdown-preview">${this.prBody}</div>` : nothing}
      ${this.prActivity.map((entry, index) => {
        const body = this.prActivityBodies.get(activityKey(entry, index));
        return html`<article class="pr-event ${entry.kind}">
          <header>
            <strong>${entry.author}</strong>
            ${entry.kind === 'review' && entry.state !== 'COMMENTED'
              ? html`<span
                  class="pr-verdict ${entry.state === 'APPROVED' ? 'approved' : 'changes'}"
                  >${entry.state === 'APPROVED' ? '✓ approved' : '✕ changes requested'}</span
                >`
              : nothing}
            ${entry.kind === 'inline'
              ? html`<code class="pr-anchor" title="Comment anchored to this line"
                  >${entry.path}${entry.line ? `:${entry.line}` : ''}</code
                >`
              : nothing}
            <span class="spacer"></span>
            ${entry.url
              ? html`<a class="pr-event-link" href=${entry.url} title="Open on the forge"
                  >${relativeTime(entry.createdAt)}</a
                >`
              : html`<span class="pr-event-link">${relativeTime(entry.createdAt)}</span>`}
          </header>
          ${body ? html`<div class="markdown-preview">${body}</div>` : nothing}
        </article>`;
      })}
    </div>`;
  }

  /**
   * Send any link inside the proposal panel to the OS browser. Delegated
   * because the markdown is generated: there is nowhere to attach a handler
   * per anchor, and `target="_blank"` is a silent no-op in this WebView.
   */
  private onProposalLinkClick = (event: MouseEvent) => {
    const anchor = (event.target as HTMLElement | null)?.closest?.('a[href]');
    const href = anchor?.getAttribute('href');
    if (!href) return;
    event.preventDefault();
    void this.openExternal(href);
  };

  /**
   * Unpushed commits on the proposal's head branch.
   *
   * This is the fact that decides whether anything else on the banner can be
   * trusted. Reviewers, merge state and above all CI describe the head the forge
   * has; if the local branch has moved since, every one of them is about
   * different code than the diff on screen — and a green "checks passed" next to
   * unpushed work reads as approval of what you are looking at.
   */
  private renderHeadDrift(pr: PullRequest) {
    const ahead = this.tracking(pr.head)?.ahead ?? 0;
    if (!ahead) return nothing;
    const noun = this.forge?.noun ?? 'pull request';
    return html`<span
      class="pr-drift"
      title=${`${ahead} local commit${ahead === 1 ? '' : 's'} on ${pr.head} ${
        ahead === 1 ? 'has' : 'have'
      } not been pushed. Everything this ${noun} reports — checks, reviews, merge state — is about the head the forge has, not the code shown here.`}
      >⚠ ${ahead} unpushed</span
    >`;
  }

  /**
   * CI summary. Reads as one verdict, not a row of counters: what a reviewer
   * needs is "can I trust this build", and only the failures are worth naming.
   * Failed checks are clickable — a red name with no way to reach the log is
   * an invitation to go hunting in a browser.
   */
  private renderChecks(pr: PullRequest) {
    if (pr.checks.length === 0) return nothing;
    const failed = pr.checks.filter((check) => check.conclusion === 'FAILURE');
    const running = pr.checks.filter((check) => check.status !== 'COMPLETED');
    const passed = pr.checks.filter((check) => check.conclusion === 'SUCCESS');
    if (failed.length) {
      return html`<span class="pr-checks bad">
        <span>✕ ${failed.length} of ${pr.checks.length} failed</span>
        ${failed.map(
          (check) => html`<button
            class="pr-check-name"
            title="Open ${check.name} on the forge"
            @click=${() => void this.openExternal(check.url)}
          >
            ${check.name}
          </button>`,
        )}
      </span>`;
    }
    if (running.length) {
      // Neutral, and pulsing rather than spinning (DESIGN.md §6): in progress
      // is not an outcome, so it must not read as one.
      return html`<span class="pr-checks pending">
        <span class="dot"></span>
        ${running.length} check${running.length === 1 ? '' : 's'} running
      </span>`;
    }
    if (passed.length) {
      return html`<span class="pr-checks ok"
        >✓ ${passed.length} check${passed.length === 1 ? '' : 's'} passed</span
      >`;
    }
    return nothing;
  }

  /** Review composer, seeded from the change's pending inline comments. */
  private renderReviewComposer(pr: PullRequest) {
    const draft = this.reviewDraft!;
    const verdicts: { id: ReviewVerdict; label: string }[] = [
      { id: 'comment', label: 'Comment' },
      { id: 'approve', label: 'Approve' },
      { id: 'requestChanges', label: 'Request changes' },
    ];
    return html`
      <div class="pr-review">
        <div class="pr-review-head">
          <span class="section-label">Submit review</span>
          ${this.pendingComments.length
            ? html`<span class="pr-review-hint"
                >seeded from ${this.pendingComments.length} pending comment${this.pendingComments
                  .length === 1
                  ? ''
                  : 's'}</span
              >`
            : nothing}
          <span class="spacer"></span>
          ${verdicts.map(
            (verdict) => html`<button
              class="tool ${draft.verdict === verdict.id ? 'primary' : ''}"
              @click=${() => (this.reviewDraft = { ...draft, verdict: verdict.id })}
            >
              ${verdict.label}
            </button>`,
          )}
        </div>
        <textarea
          class="pr-review-body"
          placeholder="Leave a comment…"
          .value=${draft.body}
          @input=${(event: Event) =>
            (this.reviewDraft = {
              ...draft,
              body: (event.target as HTMLTextAreaElement).value,
            })}
        ></textarea>
        <div class="pr-review-actions">
          <span class="pr-review-hint">Posted publicly on #${pr.number}.</span>
          <span class="spacer"></span>
          <button class="tool" @click=${() => (this.reviewDraft = null)}>Cancel</button>
          <button
            class="tool primary"
            ?disabled=${this.busy === 'submit-review'}
            @click=${() => void this.sendReview()}
          >
            ${this.busy === 'submit-review' ? 'Submitting…' : 'Submit'}
          </button>
        </div>
      </div>
    `;
  }

  /**
   * File-tree context menu. Rendered at the app root rather than inside the
   * tree so the sidebar's `overflow` cannot clip it; positioned from the click
   * and nudged back inside the viewport when it would overhang.
   */
  private renderFileMenu() {
    const menu = this.fileMenu;
    if (!menu) return nothing;
    const WIDTH = 210;
    const HEIGHT = 116;
    const left = Math.min(menu.x, window.innerWidth - WIDTH - 8);
    const top = Math.min(menu.y, window.innerHeight - HEIGHT - 8);
    const isViewed = this.viewedPaths.has(menu.path);
    const close = () => (this.fileMenu = null);
    return html`
      <div class="file-menu" style="left:${left}px; top:${top}px; width:${WIDTH}px">
        <button @click=${() => void this.openFileInEditor(menu.path)}>Open in Editor</button>
        <button
          @click=${() => {
            this.focusPath = this.focusPath === menu.path ? null : menu.path;
            close();
          }}
        >
          ${this.focusPath === menu.path ? 'Clear File Focus' : 'Focus on This File'}
        </button>
        <button
          @click=${() => {
            this.setPathViewed(menu.path, !isViewed);
            close();
          }}
        >
          ${isViewed ? 'Mark as Not Viewed' : 'Mark as Viewed'}
        </button>
        <button
          @click=${() => {
            void navigator.clipboard.writeText(menu.path);
            close();
          }}
        >
          Copy Path
        </button>
      </div>
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
    this.setPathViewed(event.detail.path, event.detail.viewed);
  };

  /** Optimistic update; persistence follows, and a failure reloads the truth. */
  private setPathViewed(path: string, viewed: boolean) {
    const change = this.selectedChange;
    if (!change) return;
    const next = new Set(this.viewedPaths);
    if (viewed) next.add(path);
    else next.delete(path);
    this.viewedPaths = next;
    void setViewed(change.changeId, path, viewed).catch(() => void this.loadReview());
  }
}

/**
 * The proposal's state as a glyph + word.
 *
 * Only *outcomes* take colour (DESIGN.md §2): merged succeeded, closed did not.
 * Open and draft are neutral, because they are a status rather than a verdict —
 * which is what leaves the coloured ones worth noticing. GitHub's purple for
 * merged would be a third hue, so it stays out.
 */
function proposalState(pr: PullRequest) {
  const state = pr.draft ? 'draft' : pr.state.toLowerCase();
  const glyph = { merged: '✓', closed: '✕', draft: '◌', open: '●' }[state] ?? '●';
  return html`<span class="pr-state ${state}" title=${`${state} · ${pr.mergeable.toLowerCase()}`}>
    <span class="pr-state-glyph">${glyph}</span>${state}
  </span>`;
}

const reviewerTitle = (state: string) =>
  ({
    APPROVED: 'approved',
    CHANGES_REQUESTED: 'requested changes',
    COMMENTED: 'commented',
    REQUESTED: 'review requested',
  })[state] ?? state.toLowerCase().replace(/_/g, ' ');

/** Sidebar panes. `walkthrough` doubles as the guided-review switch — see `selectTab`. */
type SidebarTab = 'stack' | 'files' | 'walkthrough' | 'review';

/**
 * The rail, in order. Ordered by how far from the code each pane sits: the
 * commit graph, then that commit's files, then the guided reading of them, then
 * the comments left on them.
 */
const RAIL_PANES: { id: SidebarTab; label: string; icon: TemplateResult }[] = [
  { id: 'stack', label: 'Log', icon: iconGraph },
  { id: 'files', label: 'Files', icon: iconFiles },
  { id: 'walkthrough', label: 'Steps', icon: iconSparkle },
  { id: 'review', label: 'Review', icon: iconComment },
];

const basename = (path: string) => path.slice(path.lastIndexOf('/') + 1) || path;

declare global {
  interface HTMLElementTagNameMap {
    'jj-app': App;
  }
}
