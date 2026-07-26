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
    : Promise.resolve({ repoPath: '/mock', revset: null, walkthrough: false });
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
export const getRepoState = (): Promise<RepoState> =>
  IN_TAURI ? invoke<RepoState>('repo_state') : mock((m) => m.mockRepoState);
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
export const describeChange = (changeId: string, message: string) =>
  IN_TAURI ? invoke<void>('describe', { changeId, message }) : Promise.resolve();
export const newChange = () => (IN_TAURI ? invoke<void>('new_change') : Promise.resolve());
/** Move working-copy paths into `into` (parent when omitted): jj-native partial commit. */
export const squashPaths = (paths: string[], into?: string) =>
  IN_TAURI ? invoke<void>('squash_paths', { paths, into: into ?? null }) : Promise.resolve();
/** jj absorb — returns jj's summary of what moved where. */
export const absorb = (): Promise<string> =>
  IN_TAURI ? invoke<string>('absorb') : Promise.resolve('Absorbed 2 hunks (mock).');
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

export const onRepoChanged = (callback: () => void): Promise<UnlistenFn> =>
  IN_TAURI ? listen('repo-changed', callback) : Promise.resolve(() => {});
