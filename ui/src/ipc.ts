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
}

export interface Config {
  ui: {
    diffStyle: string;
    codeFontSize: number;
    ignoreWhitespace: boolean;
  };
}

export const getLaunchOptions = () => invoke<LaunchOptions>('launch_options');
export const getConfig = () => invoke<Config>('get_config');
export const getRepoState = () => invoke<RepoState>('repo_state');
export const getDiff = (revset: string | null, ignoreWhitespace: boolean) =>
  invoke<FilePatch[]>('diff', { revset, ignoreWhitespace });
export const describeChange = (changeId: string, message: string) =>
  invoke<void>('describe', { changeId, message });
export const newChange = () => invoke<void>('new_change');
/** Move working-copy paths into the parent change (jj-native partial commit). */
export const squashPaths = (paths: string[]) => invoke<void>('squash_paths', { paths });
export const getViewedFiles = (changeId: string) =>
  invoke<string[]>('viewed_files', { changeId });
export const setViewed = (changeId: string, path: string, viewed: boolean) =>
  invoke<void>('set_viewed', { changeId, path, viewed });

export const onRepoChanged = (callback: () => void): Promise<UnlistenFn> =>
  listen('repo-changed', callback);
