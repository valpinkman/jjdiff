// Typed IPC surface — mirrors src-tauri/src/lib.rs commands.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface Signature {
  name: string;
  email: string;
  timestamp: string;
}

export interface Change {
  changeId: string;
  commitId: string;
  parents: string[];
  description: string;
  author: Signature;
  committer: Signature;
  empty: boolean;
  conflict: boolean;
  immutable: boolean;
  workingCopy: boolean;
  bookmarks: string[];
}

/**
 * How a local bookmark stands against a remote it tracks. Stated from the local
 * bookmark's side — `jj bookmark list` phrases it from the remote's, and the
 * backend inverts it once so nothing downstream has to remember which is which.
 */
export interface BookmarkStatus {
  name: string;
  remote: string;
  /** Commits the local bookmark has that the remote does not — a push would send these. */
  ahead: number;
  /** Commits the remote has that the local does not — a fetch would bring these. */
  behind: number;
}

export interface RepoState {
  root: string;
  jjVersion: string;
  workingCopy: Change;
  stack: Change[];
  /** Recent history (ancestors of @ and all bookmarks) for the graph view. */
  graph: Change[];
  /** Tracking state per bookmark; empty when the repo has no remotes. */
  bookmarks: BookmarkStatus[];
}

export type FileStatus = 'added' | 'deleted' | 'modified' | 'renamed';
export type LineKind = 'context' | 'added' | 'removed';

export interface Line {
  kind: LineKind;
  text: string;
  oldLine: number | null;
  newLine: number | null;
  /** Intra-line emphasis ranges, [start, end) in UTF-16 code units. */
  spans: [number, number][];
}

export interface Hunk {
  /** Stable within one diff: `<path>#<index>`. Walkthrough steps reference these. */
  id: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  context: string;
  lines: Line[];
}

export interface FilePatch {
  path: string;
  oldPath: string | null;
  status: FileStatus;
  binary: boolean;
  /** Reason contents were not diffed (too large, conflicted, …). */
  skipped: string | null;
  added: number;
  removed: number;
  hunks: Hunk[];
}

export interface LaunchOptions {
  repoPath: string;
  revset: string | null;
  /** `-w`: generate a walkthrough for the launch target immediately. */
  walkthrough: boolean;
  /** `--walkthrough-file`: import an agent-authored walkthrough instead of generating. */
  walkthroughFile: string | null;
  /** `jjdiff pr 75`: open this forge proposal for review on launch. */
  pullRequest: number | null;
}

export interface WalkthroughStep {
  title: string;
  narrative: string;
  hunkIds: string[];
}

export interface Walkthrough {
  summary: string;
  steps: WalkthroughStep[];
  /** Fingerprint of the diff this was generated against. */
  fingerprint: string;
}

export interface WalkthroughStatus {
  walkthrough: Walkthrough | null;
  /** True when the change's diff no longer matches the walkthrough's fingerprint. */
  stale: boolean;
}

// -- Inline review comments --

export type CommentSide = 'old' | 'new';

export interface Comment {
  id: number;
  repo: string;
  changeId: string;
  path: string;
  /** `<path>#<index>` — the hunk the comment was written against. */
  hunkId: string;
  side: CommentSide;
  line: number;
  /** The text of the line when the comment was written (drift + outdated view). */
  lineText: string;
  /** Commit id the comment was written against. */
  commitId: string;
  author: string;
  body: string;
  /** ISO 8601 timestamp (UTC). */
  createdAt: string;
  /** Parent comment id for threading (null = top-level). */
  parentId: number | null;
  resolved: boolean;
  /** True when the anchor line no longer matches (drifted). */
  outdated: boolean;
}

export interface Config {
  ui: {
    diffStyle: string;
    codeFontSize: number;
    ignoreWhitespace: boolean;
    /** "system", "light", or "dark". */
    theme: string;
    wordWrap: boolean;
  };
  keymap: {
    /** E.g. "Mod+Shift+p" — Mod is Cmd on macOS, Ctrl elsewhere. */
    commandBar: string;
  };
  editor: {
    /** Template with {file}, {line}, {repo}; empty = no editor configured. */
    command: string;
  };
}

/** What a mutation did, plus the operation to undo it. */
export interface Outcome {
  message: string;
  operation: string;
}

export interface PushResult extends Outcome {
  /** Forge-provided "create a pull request" link, when the push produced one. */
  pullRequestUrl: string | null;
}

export interface Operation {
  id: string;
  description: string;
  /** Literal command line; snapshots have none. */
  args: string | null;
  time: string;
  user: string;
  snapshot: boolean;
}

export interface ReviewStatus {
  viewed: string[];
  /** Commit id stored when the change was last marked reviewed. */
  reviewedCommit: string | null;
}

export interface Interdiff {
  files: FilePatch[];
  fromCommit: string;
  toCommit: string;
}

/** One recorded version of a change, from jj's evolog. Newest first; entry 0 is current. */
export interface ChangeVersion {
  commitId: string;
  changeId: string;
  description: string;
  /** Committer timestamp (RFC 3339) — when this version was written. */
  timestamp: string;
}

/** True inside the Tauri shell; false in a plain browser (`pnpm dev`), where fixtures serve. */
const IN_TAURI = '__TAURI_INTERNALS__' in window;

const mock = async <T>(load: (m: typeof import('./mock.js')) => T): Promise<T> =>
  load(await import('./mock.js'));

export const getLaunchOptions = (): Promise<LaunchOptions> =>
  IN_TAURI
    ? invoke<LaunchOptions>('launch_options')
    : Promise.resolve({
        repoPath: '/mock',
        revset: null,
        walkthrough: false,
        walkthroughFile: null,
        pullRequest: null,
      });
export const getWalkthrough = (
  changeId: string,
  revset: string | null,
  ignoreWhitespace: boolean,
): Promise<WalkthroughStatus> =>
  IN_TAURI
    ? invoke<WalkthroughStatus>('get_walkthrough', { changeId, revset, ignoreWhitespace })
    : mock((m) => m.mockWalkthroughStatus(changeId));
export const generateWalkthrough = (
  changeId: string,
  revset: string | null,
  ignoreWhitespace: boolean,
  context: string,
): Promise<Walkthrough> =>
  IN_TAURI
    ? invoke<Walkthrough>('generate_walkthrough', {
        changeId,
        revset,
        ignoreWhitespace,
        context,
      })
    : mock((m) => m.mockGenerateWalkthrough(changeId)).then((walkthrough) => walkthrough);
export const getConfig = (): Promise<Config> =>
  IN_TAURI ? invoke<Config>('get_config') : mock((m) => m.mockConfig);
export const getRepoState = (revset?: string): Promise<RepoState> =>
  IN_TAURI
    ? invoke<RepoState>('repo_state', { revset: revset ?? null })
    : mock((m) => m.mockRepoState);
export const getDiff = (revset: string | null, ignoreWhitespace: boolean): Promise<FilePatch[]> =>
  IN_TAURI
    ? invoke<FilePatch[]>('diff', { revset, ignoreWhitespace })
    : mock((m) => m.mockFiles);
/** How a change's diff evolved since it was last marked reviewed. */
export const getInterdiffSinceReviewed = (
  changeId: string,
  ignoreWhitespace: boolean,
): Promise<Interdiff> =>
  IN_TAURI
    ? invoke<Interdiff>('interdiff_since_reviewed', { changeId, ignoreWhitespace })
    : mock((m) => m.mockInterdiff);
/** Every recorded version of a change — the evolog drawer's list. */
export const getChangeVersions = (changeId: string): Promise<ChangeVersion[]> =>
  IN_TAURI
    ? invoke<ChangeVersion[]>('change_versions', { changeId })
    : mock((m) => m.mockChangeVersions(changeId));
/** Interdiff between two arbitrary versions of a change, addressed by commit id. */
export const getInterdiff = (
  fromCommit: string,
  toCommit: string,
  ignoreWhitespace: boolean,
): Promise<Interdiff> =>
  IN_TAURI
    ? invoke<Interdiff>('interdiff', { fromCommit, toCommit, ignoreWhitespace })
    : mock((m) => ({ ...m.mockInterdiff, fromCommit, toCommit }));
const mockOutcome = (message: string): Promise<Outcome> =>
  Promise.resolve({ message, operation: 'mock-op' });

/**
 * `allowImmutable` passes jj's `--ignore-immutable` for this one call. It is a
 * per-call argument rather than app state on purpose: jj marks commits immutable
 * to stop them being rewritten by accident, and a mode would surrender that for
 * a whole session instead of a single, confirmed command.
 */
export const describeChange = (
  changeId: string,
  message: string,
  allowImmutable = false,
): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('describe', { changeId, message, allowImmutable })
    : mockOutcome('Described.');
export const newChange = (parents: string[] = []): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('new_change', { parents }) : mockOutcome('New change created.');
/** jj edit — move the working copy onto an existing change. */
export const editChange = (revset: string, allowImmutable = false): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('edit_change', { revset, allowImmutable })
    : mockOutcome('Working copy moved.');
/** Move paths from `from` into `into`: jj-native partial commit. */
export const squashPaths = (
  paths: string[],
  into?: string,
  from?: string,
  allowImmutable = false,
): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('squash_paths', {
        paths,
        into: into ?? null,
        from: from ?? null,
        allowImmutable,
      })
    : mockOutcome('Squashed.');
/** jj absorb — returns jj's summary of what moved where. */
export const absorb = (): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('absorb') : mockOutcome('Absorbed 2 hunks (mock).');
/** File-level split: `paths` stay put, the rest move to a new child change. */
export const splitPaths = (
  revset: string,
  paths: string[],
  allowImmutable = false,
): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('split_paths', { revset, paths, allowImmutable })
    : mockOutcome('Split.');
export const abandonChange = (revset: string, allowImmutable = false): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('abandon_change', { revset, allowImmutable })
    : mockOutcome('Abandoned.');
export const duplicateChange = (revset: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('duplicate_change', { revset }) : mockOutcome('Duplicated.');
export const backoutChange = (revset: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('backout_change', { revset }) : mockOutcome('Backed out.');
/** mode: "revision" | "source" | "branch". */
export const rebaseChange = (
  mode: string,
  revset: string,
  destination: string,
  allowImmutable = false,
): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('rebase_change', { mode, revset, destination, allowImmutable })
    : mockOutcome('Rebased.');
/** Discard working-copy changes to `paths` (all when empty). */
export const restorePaths = (paths: string[]): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('restore_paths', { paths }) : mockOutcome('Restored.');
export const setBookmark = (name: string, revset: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('set_bookmark', { name, revset }) : mockOutcome('Bookmark set.');
export const deleteBookmark = (name: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('delete_bookmark', { name }) : mockOutcome('Bookmark deleted.');
export const gitFetch = (remote?: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('git_fetch', { remote: remote ?? null }) : mockOutcome('Fetched.');
export const gitPush = (options: {
  remote?: string;
  bookmark?: string;
  change?: string;
}): Promise<PushResult> =>
  IN_TAURI
    ? invoke<PushResult>('git_push', {
        remote: options.remote ?? null,
        bookmark: options.bookmark ?? null,
        change: options.change ?? null,
      })
    : Promise.resolve({
        message: 'Pushed (mock).',
        operation: 'mock-op',
        pullRequestUrl: 'https://example.test/pulls/new?sourceBranch=demo',
      });
export const getRemotes = (): Promise<string[]> =>
  IN_TAURI ? invoke<string[]>('remotes') : Promise.resolve(['origin']);

// -- Operation log / undo --
export const getOperationLog = (limit = 100): Promise<Operation[]> =>
  IN_TAURI ? invoke<Operation[]>('operation_log', { limit }) : mock((m) => m.mockOperations);
/**
 * jj's narration of what an operation changed. `from` null compares the operation
 * against its own parent. Text, not structure: `jj op diff` has no `json()` form,
 * so this is displayed verbatim rather than parsed.
 */
export const getOperationDiff = (to: string, from: string | null = null): Promise<string> =>
  IN_TAURI
    ? invoke<string>('operation_diff', { from, to })
    : mock((m) => m.mockOperationDiff(to, from));
export const undo = (): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('undo') : mockOutcome('Undid the last operation.');
export const restoreOperation = (operation: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('restore_operation', { operation }) : mockOutcome('Restored.');
export const revertOperation = (operation: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('revert_operation', { operation }) : mockOutcome('Reverted.');
export const getConflicts = (revset: string): Promise<string[]> =>
  IN_TAURI ? invoke<string[]>('conflicts', { revset }) : Promise.resolve([]);
export const getReviewStatus = (changeId: string): Promise<ReviewStatus> =>
  IN_TAURI
    ? invoke<ReviewStatus>('review_status', { changeId })
    : mock((m) => m.mockReviewStatus(changeId));
export const setViewed = (changeId: string, path: string, viewed: boolean) =>
  IN_TAURI ? invoke<void>('set_viewed', { changeId, path, viewed }) : Promise.resolve();
export const markReviewed = (changeId: string, commitId: string) =>
  IN_TAURI ? invoke<void>('mark_reviewed', { changeId, commitId }) : Promise.resolve();

/** Add a comment anchored to a line in a change's diff. */
export const addComment = (
  changeId: string,
  path: string,
  hunkId: string,
  side: CommentSide,
  line: number,
  lineText: string,
  commitId: string,
  author: string,
  body: string,
  parentId: number | null,
): Promise<Comment> =>
  IN_TAURI
    ? invoke<Comment>('add_comment', {
        changeId, path, hunkId, side, line, lineText, commitId, author, body, parentId,
      })
    : Promise.resolve({
        id: Math.floor(Math.random() * 1e6),
        repo: '', changeId, path, hunkId, side, line, lineText, commitId,
        author, body, createdAt: new Date().toISOString(), parentId, resolved: false, outdated: false,
      });

/** All comments for a change, ordered by path then line then time. */
export const listComments = (changeId: string): Promise<Comment[]> =>
  IN_TAURI ? invoke<Comment[]>('list_comments', { changeId }) : Promise.resolve([]);

export const setCommentResolved = (id: number, resolved: boolean) =>
  IN_TAURI ? invoke<void>('set_comment_resolved', { id, resolved }) : Promise.resolve();

export const deleteComment = (id: number) =>
  IN_TAURI ? invoke<void>('delete_comment', { id }) : Promise.resolve();

export const updateComment = (id: number, body: string) =>
  IN_TAURI ? invoke<void>('update_comment', { id, body }) : Promise.resolve();

/** Re-anchor comments against the current diff; returns how many moved. */
export const refreshCommentAnchors = (
  changeId: string,
  currentCommitId: string,
  revset: string | null,
  ignoreWhitespace: boolean,
): Promise<number> =>
  IN_TAURI
    ? invoke<number>('refresh_comment_anchors', { changeId, currentCommitId, revset, ignoreWhitespace })
    : Promise.resolve(0);

/** Pending (unresolved) comments as a paste-ready Markdown review. */
export const exportReviewMarkdown = (changeId: string): Promise<string> =>
  IN_TAURI ? invoke<string>('export_review_markdown', { changeId }) : Promise.resolve('No pending comments.');

/** Import an agent-authored walkthrough JSON file for a change. */
export const importWalkthrough = (
  changeId: string,
  revset: string | null,
  ignoreWhitespace: boolean,
  path: string,
): Promise<Walkthrough> =>
  IN_TAURI
    ? invoke<Walkthrough>('import_walkthrough', { changeId, revset, ignoreWhitespace, path })
    : mock((m) => m.mockWalkthrough);

/** Full text of a file for expanding diff context; null revset = working copy. */
export const getFileContent = (revset: string | null, path: string): Promise<string> =>
  IN_TAURI
    ? invoke<string>('file_content', { revset, path })
    : mock((m) => m.mockFileContent(path));

/** Raw bytes of a file as base64 + mime, for rendering images. */
export interface FileBytes {
  data: string;
  mime: string;
}
export const getFileBytes = (revset: string | null, path: string): Promise<FileBytes> =>
  IN_TAURI
    ? invoke<FileBytes>('file_bytes', { revset, path })
    : Promise.resolve({ data: '', mime: 'application/octet-stream' });

/** Switch the app to another repository (path must be a colocated jj repo). */
export const openRepository = (path: string) =>
  IN_TAURI ? invoke<void>('open_repository', { path }) : Promise.resolve();
/** Native folder picker; resolves null on cancel (and always in the browser mock). */
export const pickRepository = (): Promise<string | null> =>
  IN_TAURI ? invoke<string | null>('pick_repository') : Promise.resolve(null);
export const getRecentRepos = (): Promise<string[]> =>
  IN_TAURI
    ? invoke<string[]>('recent_repos')
    : Promise.resolve(['/Users/dev/projects/example', '/Users/dev/projects/other-app', '/Users/dev/oss/jj']);

/**
 * Write the `jjdiff` shim on PATH so the bundle is reachable from a shell.
 * Returns a human-readable report (the installed path, or the command to run
 * manually if no writable dir was found).
 */
export const installTerminalHelper = (): Promise<string> =>
  IN_TAURI ? invoke<string>('install_terminal_helper') : Promise.resolve('(mock) would install jjdiff on PATH');

// -- Forge review (gh) --

export interface Reviewer {
  name: string;
  /** REQUESTED / APPROVED / CHANGES_REQUESTED / COMMENTED. */
  state: string;
}

export interface Check {
  name: string;
  /** QUEUED / IN_PROGRESS / COMPLETED. */
  status: string;
  /** SUCCESS / FAILURE / SKIPPED / …; empty while still running. */
  conclusion: string;
  url: string;
}

export interface PullRequest {
  number: number;
  title: string;
  body: string;
  author: string;
  base: string;
  head: string;
  /** The forge's own merge base — what a merged proposal must be diffed from. */
  baseOid: string;
  headOid: string;
  /** OPEN / MERGED / CLOSED. */
  state: string;
  draft: boolean;
  mergeable: string;
  url: string;
  additions: number;
  deletions: number;
  changedFiles: number;
  reviewers: Reviewer[];
  checks: Check[];
}

export interface PullRequestSummary {
  number: number;
  title: string;
  author: string;
  state: string;
  draft: boolean;
  head: string;
  updatedAt: string;
}

/**
 * One entry in a proposal's conversation. GitHub keeps these in three places
 * that only read as one thread once merged and sorted by time.
 */
export interface Activity {
  /** `comment` (discussion), `review` (a verdict) or `inline` (anchored to a line). */
  kind: 'comment' | 'review' | 'inline';
  author: string;
  body: string;
  createdAt: string;
  /** Review verdict; empty for anything that is not a review. */
  state: string;
  /** Inline only. */
  path: string;
  line: number;
  url: string;
}

/** A fetched proposal plus the revsets that make it reviewable. */
export interface OpenedPullRequest extends PullRequest {
  /** Local bookmark the head landed on. */
  bookmark: string;
  /** Revset for the proposal's own commits. */
  revset: string;
}

export interface ForgeInfo {
  kind: 'github';
  /** What the forge calls a proposal ("pull request"). */
  noun: string;
}

export type ReviewVerdict = 'approve' | 'requestChanges' | 'comment';

/** One inline comment to post against a line of the proposal's diff. */
export interface ReviewComment {
  path: string;
  line: number;
  side: CommentSide;
  body: string;
}

/** What a submitted review actually did. */
export interface Submitted {
  /** How many comments landed as real inline comments. */
  inline: number;
  /** Set when inline posting failed and they went into the body instead. */
  fellBack: string | null;
}

/** Null when the repo is on no forge jjdiff can drive — not an error. */
export const getForgeInfo = (): Promise<ForgeInfo | null> =>
  IN_TAURI ? invoke<ForgeInfo | null>('forge_info') : mock((m) => m.mockForgeInfo);

export const listPullRequests = (limit = 30): Promise<PullRequestSummary[]> =>
  IN_TAURI
    ? invoke<PullRequestSummary[]>('list_pull_requests', { limit })
    : mock((m) => m.mockPullRequestList);

export const getPullRequest = (number: number): Promise<PullRequest> =>
  IN_TAURI ? invoke<PullRequest>('pull_request', { number }) : mock((m) => m.mockPullRequest);

/**
 * The proposal's conversation, oldest first. Separate from `getPullRequest`
 * because it costs two more `gh` calls and the banner should not wait on them.
 */
export const getPullRequestActivity = (number: number): Promise<Activity[]> =>
  IN_TAURI
    ? invoke<Activity[]>('pull_request_activity', { number })
    : mock((m) => m.mockActivity);

/** Fetch the proposal's head so it can be reviewed as an ordinary revset. */
export const openPullRequest = (number: number): Promise<OpenedPullRequest> =>
  IN_TAURI
    ? invoke<OpenedPullRequest>('open_pull_request', { number })
    : mock((m) => m.mockOpenedPullRequest);

/**
 * Outward-facing and effectively irreversible — confirm before calling.
 * `comments` land as real inline comments where the forge allows it.
 */
export const submitReview = (
  number: number,
  verdict: ReviewVerdict,
  body: string,
  comments: ReviewComment[] = [],
): Promise<Submitted> =>
  IN_TAURI
    ? invoke<Submitted>('submit_review', { number, verdict, body, comments })
    : Promise.resolve({ inline: comments.length, fellBack: null });

/**
 * Open a URL in the system browser. The WebView has no tabs, so `target=_blank`
 * silently does nothing — every outbound link has to go through the host OS.
 */
export const openUrl = (url: string): Promise<void> =>
  IN_TAURI ? invoke<void>('open_url', { url }) : Promise.resolve(void window.open(url, '_blank'));

/** One native submenu, mirroring a command-palette group. */
export interface MenuGroup {
  title: string;
  items: { id: string; label: string; enabled?: boolean }[];
}

/**
 * Rebuild the native menu bar. The backend ignores pushes from unfocused
 * windows — on macOS the menu bar is app-global.
 */
export const setMenu = (groups: MenuGroup[]): Promise<void> =>
  IN_TAURI ? invoke<void>('set_menu', { groups }) : Promise.resolve();

/** A native menu item was clicked; the payload is the palette command id. */
export const onMenuCommand = (callback: (id: string) => void): Promise<UnlistenFn> =>
  IN_TAURI ? listen<string>('menu-command', (event) => callback(event.payload)) : Promise.resolve(() => {});

/**
 * Open a repository in its own window, or focus the window already showing it.
 * Windows own their repo, so this is how you get two repos side by side.
 */
export const openRepoWindow = (path: string): Promise<void> =>
  IN_TAURI ? invoke<void>('open_repo_window', { path }) : Promise.resolve();

/**
 * Persist `[editor] command`, resolving to the config path written. Only that
 * one value is touched — comments and other settings survive.
 */
export const setEditorCommand = (command: string): Promise<string> =>
  IN_TAURI
    ? invoke<string>('set_editor_command', { command })
    : Promise.resolve('(mock) ~/.config/jjdiff/config.toml');

/**
 * Persist `[ui] theme`. Same surgical edit as the editor command — a theme is
 * picked once and expected to still be there next launch.
 */
export const setUiTheme = (theme: string): Promise<string> =>
  IN_TAURI
    ? invoke<string>('set_ui_theme', { theme })
    : Promise.resolve('(mock) ~/.config/jjdiff/config.toml');

/**
 * Open a repo-relative path in the configured editor. Rejects with a message
 * naming the config key when `[editor] command` is unset.
 */
export const openInEditor = (path: string, line?: number): Promise<void> =>
  IN_TAURI
    ? invoke<void>('open_in_editor', { path, line: line ?? null })
    : Promise.reject(new Error('(mock) no editor in the browser'));

/**
 * Second-instance event: emitted by `tauri-plugin-single-instance` when
 * `jjdiff` is launched again while the app is running. The payload is the
 * parsed argv (`{ repoPath?, revset?, walkthrough, walkthroughFile? }`),
 * so the existing window can open the newly requested repo.
 */
export interface SecondInstanceArgs {
  repoPath: string | null;
  revset: string | null;
  walkthrough: boolean;
  walkthroughFile: string | null;
  pullRequest: number | null;
}
export const onSecondInstance = (callback: (args: SecondInstanceArgs) => void): Promise<UnlistenFn> =>
  IN_TAURI ? listen<SecondInstanceArgs>('second-instance', (event) => callback(event.payload)) : Promise.resolve(() => {});

export const onRepoChanged = (callback: () => void): Promise<UnlistenFn> =>
  IN_TAURI ? listen('repo-changed', callback) : Promise.resolve(() => {});
