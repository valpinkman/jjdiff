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
  hunks: Hunk[];
}

export interface LaunchOptions {
  repoPath: string;
  revset: string | null;
}

export const getLaunchOptions = () => invoke<LaunchOptions>('launch_options');
export const getRepoState = () => invoke<RepoState>('repo_state');
export const getDiff = (revset?: string) =>
  invoke<FilePatch[]>('diff', { revset: revset ?? null });
export const describeChange = (changeId: string, message: string) =>
  invoke<void>('describe', { changeId, message });
export const newChange = () => invoke<void>('new_change');

export const onRepoChanged = (callback: () => void): Promise<UnlistenFn> =>
  listen('repo-changed', callback);
