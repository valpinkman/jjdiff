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

export interface RepoState {
  root: string;
  jjVersion: string;
  workingCopy: Change;
  stack: Change[];
  /** Recent history (ancestors of @ and all bookmarks) for the graph view. */
  graph: Change[];
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
const mockOutcome = (message: string): Promise<Outcome> =>
  Promise.resolve({ message, operation: 'mock-op' });

export const describeChange = (changeId: string, message: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('describe', { changeId, message }) : mockOutcome('Described.');
export const newChange = (parents: string[] = []): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('new_change', { parents }) : mockOutcome('New change created.');
/** jj edit — move the working copy onto an existing change. */
export const editChange = (revset: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('edit_change', { revset }) : mockOutcome('Working copy moved.');
/** Move paths from `from` into `into`: jj-native partial commit. */
export const squashPaths = (
  paths: string[],
  into?: string,
  from?: string,
): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('squash_paths', {
        paths,
        into: into ?? null,
        from: from ?? null,
      })
    : mockOutcome('Squashed.');
/** jj absorb — returns jj's summary of what moved where. */
export const absorb = (): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('absorb') : mockOutcome('Absorbed 2 hunks (mock).');
/** File-level split: `paths` stay put, the rest move to a new child change. */
export const splitPaths = (revset: string, paths: string[]): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('split_paths', { revset, paths }) : mockOutcome('Split.');
export const abandonChange = (revset: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('abandon_change', { revset }) : mockOutcome('Abandoned.');
export const duplicateChange = (revset: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('duplicate_change', { revset }) : mockOutcome('Duplicated.');
export const backoutChange = (revset: string): Promise<Outcome> =>
  IN_TAURI ? invoke<Outcome>('backout_change', { revset }) : mockOutcome('Backed out.');
/** mode: "revision" | "source" | "branch". */
export const rebaseChange = (
  mode: string,
  revset: string,
  destination: string,
): Promise<Outcome> =>
  IN_TAURI
    ? invoke<Outcome>('rebase_change', { mode, revset, destination })
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
}
export const onSecondInstance = (callback: (args: SecondInstanceArgs) => void): Promise<UnlistenFn> =>
  IN_TAURI ? listen<SecondInstanceArgs>('second-instance', (event) => callback(event.payload)) : Promise.resolve(() => {});

export const onRepoChanged = (callback: () => void): Promise<UnlistenFn> =>
  IN_TAURI ? listen('repo-changed', callback) : Promise.resolve(() => {});
