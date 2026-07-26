// Fixture backend for running the UI in a plain browser (`pnpm dev` without Tauri).
// Lets UI work be verified visually without a jj repo or the native shell.
import type {
  Change,
  Config,
  FilePatch,
  Interdiff,
  RepoState,
  ReviewStatus,
  Walkthrough,
  WalkthroughStatus,
} from './ipc.js';

const change = (partial: Partial<Change> & Pick<Change, 'changeId' | 'description'>): Change => ({
  commitId: partial.changeId.repeat(2).slice(0, 40),
  parents: [],
  author: { name: 'Ada Example', email: 'ada@example.com', timestamp: '2026-07-26T10:00:00+02:00' },
  committer: { name: 'Ada Example', email: 'ada@example.com', timestamp: '2026-07-26T10:05:00+02:00' },
  empty: false,
  conflict: false,
  immutable: false,
  workingCopy: false,
  bookmarks: [],
  ...partial,
});

const wc = change({ changeId: 'wcwcwcwcwcwcwcwcwcwc', description: '', workingCopy: true, empty: false });

export const mockRepoState: RepoState = {
  root: '/Users/dev/projects/example',
  jjVersion: '0.43.0 (mock)',
  workingCopy: wc,
  stack: [
    wc,
    change({
      changeId: 'qpalzmwoskxn12345678',
      description: 'Add retry logic to the sync engine\n\nUses exponential backoff.',
      bookmarks: ['sync-retries'],
    }),
    change({
      changeId: 'zxnvlqpwmrkd87654321',
      description: 'Refactor connection pool',
      conflict: true,
    }),
  ],
};

export const mockFiles: FilePatch[] = [
  {
    path: 'src/sync/engine.ts',
    oldPath: null,
    status: 'modified',
    binary: false,
    skipped: null,
    added: 3,
    removed: 2,
    hunks: [
      {
        id: 'src/sync/engine.ts#0',
        oldStart: 12,
        oldLines: 7,
        newStart: 12,
        newLines: 8,
        context: 'function sync()',
        lines: [
          { kind: 'context', text: 'export async function sync(options: SyncOptions) {', oldLine: 12, newLine: 12, spans: [] },
          { kind: 'context', text: '  const client = await connect(options);', oldLine: 13, newLine: 13, spans: [] },
          { kind: 'removed', text: '  const result = await client.pull();', oldLine: 14, newLine: null, spans: [[8, 14]] },
          { kind: 'added', text: '  const outcome = await client.pull();', oldLine: null, newLine: 14, spans: [[8, 15]] },
          { kind: 'added', text: '  await retryWithBackoff(() => client.push(outcome));', oldLine: null, newLine: 15, spans: [] },
          { kind: 'context', text: '  return report(client);', oldLine: 15, newLine: 16, spans: [] },
          { kind: 'context', text: '}', oldLine: 16, newLine: 17, spans: [] },
        ],
      },
      {
        id: 'src/sync/engine.ts#1',
        oldStart: 40,
        oldLines: 4,
        newStart: 41,
        newLines: 4,
        context: '',
        lines: [
          { kind: 'context', text: 'const MAX_ATTEMPTS = 5;', oldLine: 40, newLine: 41, spans: [] },
          { kind: 'removed', text: 'const DELAY_MS = 100;', oldLine: 41, newLine: null, spans: [[6, 14]] },
          { kind: 'added', text: 'const BASE_DELAY_MS = 250;', oldLine: null, newLine: 42, spans: [[6, 19]] },
          { kind: 'context', text: 'export { MAX_ATTEMPTS };', oldLine: 42, newLine: 43, spans: [] },
        ],
      },
    ],
  },
  {
    path: 'src/sync/backoff.ts',
    oldPath: null,
    status: 'added',
    binary: false,
    skipped: null,
    added: 5,
    removed: 0,
    hunks: [
      {
        id: 'src/sync/backoff.ts#0',
        oldStart: 0,
        oldLines: 0,
        newStart: 1,
        newLines: 5,
        context: '',
        lines: [
          { kind: 'added', text: 'export async function retryWithBackoff<T>(task: () => Promise<T>) {', oldLine: null, newLine: 1, spans: [] },
          { kind: 'added', text: '  for (let attempt = 0; ; attempt++) {', oldLine: null, newLine: 2, spans: [] },
          { kind: 'added', text: '    try { return await task(); } catch (e) { await delay(attempt); }', oldLine: null, newLine: 3, spans: [] },
          { kind: 'added', text: '  }', oldLine: null, newLine: 4, spans: [] },
          { kind: 'added', text: '}', oldLine: null, newLine: 5, spans: [] },
        ],
      },
    ],
  },
  {
    path: 'assets/logo.png',
    oldPath: null,
    status: 'modified',
    binary: true,
    skipped: null,
    added: 0,
    removed: 0,
    hunks: [],
  },
  {
    path: 'vendor/blob.dat',
    oldPath: null,
    status: 'modified',
    binary: false,
    skipped: 'file too large',
    added: 0,
    removed: 0,
    hunks: [],
  },
];

export const mockConfig: Config = {
  ui: { diffStyle: 'split', codeFontSize: 12.5, ignoreWhitespace: false, theme: 'system' },
  keymap: { commandBar: 'Mod+Shift+p' },
};

export const mockReviewStatus = (changeId: string): ReviewStatus =>
  changeId === 'qpalzmwoskxn12345678'
    ? { viewed: ['src/sync/backoff.ts'], reviewedCommit: 'anoldercommitid0000000000000000000000000' }
    : { viewed: [], reviewedCommit: null };

export const mockInterdiff: Interdiff = {
  files: [mockFiles[0]!],
  fromCommit: 'anoldercommitid0000000000000000000000000',
  toCommit: mockRepoState.stack[1]!.commitId,
};

export const mockWalkthrough: Walkthrough = {
  summary:
    'This change adds retry-with-backoff to the sync engine: a new backoff helper, and the push path now routes through it so transient failures no longer abort a sync.',
  steps: [
    {
      title: 'New backoff helper',
      narrative:
        'A small retry loop with exponential delay. Read this first — everything else builds on it.',
      hunkIds: ['src/sync/backoff.ts#0'],
    },
    {
      title: 'Sync engine uses the helper',
      narrative:
        'The pull result is renamed to outcome and pushes are wrapped in retryWithBackoff, so a flaky network no longer fails the whole sync.',
      hunkIds: ['src/sync/engine.ts#0'],
    },
    {
      title: 'Tuning constants',
      narrative: 'DELAY_MS becomes BASE_DELAY_MS at 250ms to match the exponential schedule.',
      hunkIds: ['src/sync/engine.ts#1'],
    },
  ],
  fingerprint: 'mockfingerprint000',
};

export const mockWalkthroughStatus = (changeId: string): WalkthroughStatus =>
  changeId === 'wcwcwcwcwcwcwcwcwcwc'
    ? { walkthrough: mockWalkthrough, stale: false }
    : { walkthrough: null, stale: false };

export const mockGenerateWalkthrough = (): Promise<Walkthrough> =>
  new Promise((resolve) => setTimeout(() => resolve(mockWalkthrough), 900));
