// Fixture backend for running the UI in a plain browser (`pnpm dev` without Tauri).
// Lets UI work be verified visually without a jj repo or the native shell.
import type {
  Activity,
  Change,
  ChangeVersion,
  Config,
  ConflictedFile,
  FilePatch,
  ForgeInfo,
  Interdiff,
  OpenedPullRequest,
  PullRequest,
  PullRequestSummary,
  RepoState,
  ReviewStatus,
  Walkthrough,
  WalkthroughStatus,
  Operation,
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

const retry = change({
  changeId: 'qpalzmwoskxn12345678',
  description: 'Add retry logic to the sync engine\n\nUses exponential backoff.',
  bookmarks: ['sync-retries'],
});
const pool = change({
  changeId: 'zxnvlqpwmrkd87654321',
  description: 'Refactor connection pool',
  conflict: true,
});
const trunk1 = change({
  changeId: 'trunktrunktrunk11111',
  description: 'Release 2.4',
  bookmarks: ['main'],
  immutable: true,
});
const trunk2 = change({
  changeId: 'trunktrunktrunk22222',
  description: 'Fix flaky CI on windows runners',
  immutable: true,
});
const sidebranch = change({
  changeId: 'featurexyzfeaturexyz',
  description: 'Experiment: streaming parser',
  bookmarks: ['streaming'],
});

// Parent wiring (commit ids) so the graph has a real fork: streaming branches off trunk1.
wc.parents = [retry.commitId];
retry.parents = [pool.commitId];
pool.parents = [trunk1.commitId];
sidebranch.parents = [trunk1.commitId];
trunk1.parents = [trunk2.commitId];
trunk2.parents = [];

export const mockRepoState: RepoState = {
  root: '/Users/dev/projects/example',
  jjVersion: '0.43.0 (mock)',
  workingCopy: wc,
  stack: [wc, retry, pool],
  graph: [wc, retry, sidebranch, pool, trunk1, trunk2],
  // One of each case so `pnpm dev` exercises all three renderings: ahead (and on
  // the mock proposal's head branch, so the banner's staleness warning shows),
  // behind, and in sync — which must render nothing at all.
  bookmarks: [
    { name: 'sync-retries', remote: 'origin', ahead: 2, behind: 0 },
    { name: 'streaming', remote: 'origin', ahead: 0, behind: 3 },
    { name: 'main', remote: 'origin', ahead: 0, behind: 0 },
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
  ui: { diffStyle: 'split', codeFontSize: 12.5, ignoreWhitespace: false, theme: 'system', wordWrap: false },
  keymap: { commandBar: 'Mod+k' },
  walkthrough: {
    backend: 'claude',
    claudeModel: '',
    codexModel: '',
    opencodeModel: '',
    piModel: '',
    prompt: '',
  },
  describe: { prompt: '' },
  editor: { command: 'zed {file}:{line}' },
};

/** A plausible generated message, slow enough to show the working state. */
export const mockGeneratedDescription = (): Promise<string> =>
  new Promise((resolve) =>
    setTimeout(
      () =>
        resolve(
          'Retry transient push failures in the sync engine\n\n' +
            'A flaky network aborted the whole sync rather than the one call that\n' +
            'failed. The push path now routes through an exponential-backoff helper,\n' +
            'so a transient failure costs a retry instead of the run.',
        ),
      1200,
    ),
  );

export const mockForgeInfo: ForgeInfo = { kind: 'github', noun: 'pull request' };

export const mockPullRequest: PullRequest = {
  number: 75,
  title: 'Add retry-with-backoff to the sync engine',
  body:
    '## What\n\nTransient failures no longer abort a sync. `retryWithBackoff` wraps the push\nand backs off exponentially.\n\n- caps at `MAX_ATTEMPTS`\n- base delay moved to `BASE_DELAY_MS`\n\n## Why\n\nThe old code aborted the whole sync on the first network blip. See [the issue](https://example.test/issues/12).\n',
  author: 'Ada Example',
  base: 'main',
  head: 'sync-retries',
  baseOid: 'b26222343117d49d65ee7a9222c924c702b7ed64',
  headOid: '70c32eeb155a5142074d34d860691ea9756b4522',
  state: 'OPEN',
  draft: false,
  mergeable: 'MERGEABLE',
  url: 'https://example.test/owner/repo/pull/75',
  additions: 8,
  deletions: 2,
  changedFiles: 2,
  reviewers: [
    { name: 'Grace Example', state: 'REQUESTED' },
    { name: 'Alan Example', state: 'APPROVED' },
  ],
  // One of each state, so the pills can be eyeballed in `pnpm dev`.
  checks: [
    { name: 'app', status: 'COMPLETED', conclusion: 'SUCCESS', url: 'https://example.test/1' },
    { name: 'crates', status: 'COMPLETED', conclusion: 'FAILURE', url: 'https://example.test/2' },
    { name: 'lint', status: 'IN_PROGRESS', conclusion: '', url: 'https://example.test/3' },
  ],
};

// One of each kind, so `pnpm dev` shows a discussion comment, both review
// verdicts and a line-anchored comment without needing a forge.
export const mockActivity: Activity[] = [
  {
    kind: 'comment',
    author: 'Grace Example',
    body: 'Nice — the backoff table is much easier to follow now.\n\nOne thought: should `BASE_DELAY_MS` be configurable?',
    createdAt: '2026-07-27T09:12:00Z',
    state: '',
    path: '',
    line: 0,
    url: 'https://example.test/owner/repo/pull/75#issuecomment-1',
  },
  {
    kind: 'inline',
    author: 'Alan Example',
    body: 'This retries forever if `task` always throws. Worth a cap?',
    createdAt: '2026-07-27T10:03:00Z',
    state: '',
    path: 'src/sync/backoff.ts',
    line: 3,
    url: 'https://example.test/owner/repo/pull/75#discussion_r1',
  },
  {
    kind: 'review',
    author: 'Alan Example',
    body: 'Looks good once the retry cap is in.',
    createdAt: '2026-07-27T10:05:00Z',
    state: 'CHANGES_REQUESTED',
    path: '',
    line: 0,
    url: 'https://example.test/owner/repo/pull/75#pullrequestreview-1',
  },
  {
    kind: 'review',
    author: 'Grace Example',
    body: '',
    createdAt: '2026-07-28T08:40:00Z',
    state: 'APPROVED',
    path: '',
    line: 0,
    url: 'https://example.test/owner/repo/pull/75#pullrequestreview-2',
  },
];

export const mockOpenedPullRequest: OpenedPullRequest = {
  ...mockPullRequest,
  bookmark: 'jjdiff-pr-75',
  revset: 'b26222343117d49d65ee7a9222c924c702b7ed64..jjdiff-pr-75',
};

export const mockPullRequestList: PullRequestSummary[] = [
  {
    number: 75,
    title: 'Add retry-with-backoff to the sync engine',
    author: 'Ada Example',
    state: 'OPEN',
    draft: false,
    head: 'sync-retries',
    updatedAt: '2026-07-27T10:00:00Z',
  },
  {
    number: 74,
    title: 'Experiment: streaming pulls',
    author: 'Grace Example',
    state: 'OPEN',
    draft: true,
    head: 'streaming',
    updatedAt: '2026-07-26T09:00:00Z',
  },
];

export const mockReviewStatus = (changeId: string): ReviewStatus =>
  changeId === 'qpalzmwoskxn12345678'
    ? { viewed: ['src/sync/backoff.ts'], reviewedCommit: 'anoldercommitid0000000000000000000000000' }
    : { viewed: [], reviewedCommit: null };

export const mockInterdiff: Interdiff = {
  files: [mockFiles[0]!],
  fromCommit: 'anoldercommitid0000000000000000000000000',
  toCommit: mockRepoState.stack[1]!.commitId,
};

/**
 * Three versions of any change: the current commit, plus two invented predecessors.
 * The drawer only needs plausible identities and timestamps — the interdiff it
 * requests comes back as `mockInterdiff` whichever pair is picked.
 */
export const mockChangeVersions = (changeId: string): ChangeVersion[] => {
  const current =
    mockRepoState.graph.find((entry) => entry.changeId === changeId)?.commitId ??
    changeId.repeat(2).slice(0, 40);
  return [
    {
      commitId: current,
      changeId,
      description: 'Add retry logic to the sync engine\n\nUses exponential backoff.',
      timestamp: '2026-07-26T10:05:00+02:00',
    },
    {
      commitId: 'b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5',
      changeId,
      description: 'Add retry logic to the sync engine',
      timestamp: '2026-07-26T09:41:00+02:00',
    },
    {
      commitId: 'anoldercommitid0000000000000000000000000',
      changeId,
      description: 'wip retries',
      timestamp: '2026-07-25T18:12:00+02:00',
    },
  ];
};

export const mockWalkthrough: Walkthrough = {
  summary:
    'This change adds retry-with-backoff to the sync engine: a new backoff helper, and the push path now routes through it so transient failures no longer abort a sync.',
  // Exercises everything the overview page renders: a mermaid diagram, a diff
  // fence, the clickable **Files:** line, and both tables.
  overview: `# Retry-with-backoff in the sync engine

Transient push failures abort a whole sync. This routes the push path through a new
exponential-backoff helper so a flaky network costs a retry instead of the run.

➕ addition · ✏️ modification · ➖ deletion

## Impacted Systems

\`\`\`mermaid
flowchart LR
  engine["Sync engine"]
  backoff["Backoff helper"]
  server["Remote sync server"]

  engine -->|"retryWithBackoff"| backoff
  backoff -->|"existing: push"| server
  engine -->|"existing: pull"| server
\`\`\`

## Changes to System Boundaries

### ➕ Sync engine ⇄ Backoff helper — \`retryWithBackoff\`

**Routing**

- The engine wraps only \`client.push\`; \`client.pull\` still calls the server directly.
- The helper rethrows the last error once attempts are exhausted, so a permanent failure
  still fails the sync.

**Files:** \`src/sync/backoff.ts\` · \`src/sync/engine.ts\`

**Contract changes**

\`\`\`diff
+export async function retryWithBackoff<T>(
+  operation: () => Promise<T>,
+): Promise<T>;

+const MAX_ATTEMPTS = 5;
-const DELAY_MS = 100;
+const BASE_DELAY_MS = 250;
\`\`\`

- Delay is \`BASE_DELAY_MS * 2 ** attempt\`; the caller cannot configure it yet.

## Changes to Mutable State

| State | Ownership, cardinality, lifecycle |
| ----- | --------------------------------- |
| ➕ Attempt counter | **Sync engine**<br>Closure local to \`retryWithBackoff\`.<br><br>**Cardinality:** one per in-flight push.<br>**Lifecycle:** created on call, discarded on success or on the final rethrow. |

## Changes to Effects

| Effect | Ownership and failure handling |
| ------ | ------------------------------ |
| ✏️ Push to the remote sync server | **Sync engine**<br>\`sync()\` → existing \`client.push\`<br><br>The network call itself is unchanged; it is now attempted up to five times. The last error propagates unwrapped, so existing callers still see the server's own failure. |
`,
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
  outline: false,
};

const mockPoolWalkthrough: Walkthrough = {
  summary: 'Connection pool refactor: ownership moves into the pool object.',
  // Deliberately absent, so the browser build exercises the fallback for a
  // walkthrough stored before overviews existed.
  overview: null,
  steps: [
    {
      title: 'Pool owns connections',
      narrative: 'The pool now tracks and reuses connections instead of the engine.',
      hunkIds: ['src/sync/engine.ts#0', 'src/sync/engine.ts#1'],
    },
    {
      title: 'Helper extraction',
      narrative: 'Backoff logic moves to its own module.',
      hunkIds: ['src/sync/backoff.ts#0'],
    },
  ],
  fingerprint: 'mockfingerprint001',
  outline: false,
};

/** Mirrors the real backend: generated walkthroughs persist for later fetches. */
const generatedStore = new Map<string, Walkthrough>();

export const mockWalkthroughStatus = (changeId: string): WalkthroughStatus => {
  const generated = generatedStore.get(changeId);
  if (generated) {
    return { walkthrough: generated, stale: false };
  }
  if (changeId === 'wcwcwcwcwcwcwcwcwcwc') {
    return { walkthrough: mockWalkthrough, stale: false };
  }
  if (changeId === 'qpalzmwoskxn12345678') {
    return { walkthrough: mockPoolWalkthrough, stale: false };
  }
  return { walkthrough: null, stale: false };
};

export const mockGenerateWalkthrough = (changeId: string): Promise<Walkthrough> =>
  new Promise((resolve) =>
    setTimeout(() => {
      generatedStore.set(changeId, mockWalkthrough);
      resolve(mockWalkthrough);
    }, 900),
  );

/** Synthetic full-file text so context expansion is exercisable in the browser. */
export const mockFileContent = (path: string): string => {
  if (path === 'src/sync/engine.ts') {
    const lines: string[] = [];
    for (let n = 1; n <= 60; n++) {
      if (n === 12) lines.push('export async function sync(options: SyncOptions) {');
      else if (n === 13) lines.push('  const client = await connect(options);');
      else if (n === 14) lines.push('  const outcome = await client.pull();');
      else if (n === 15) lines.push('  await retryWithBackoff(() => client.push(outcome));');
      else if (n === 16) lines.push('  return report(client);');
      else if (n === 17) lines.push('}');
      else if (n === 41) lines.push('const MAX_ATTEMPTS = 5;');
      else if (n === 42) lines.push('const BASE_DELAY_MS = 250;');
      else if (n === 43) lines.push('export { MAX_ATTEMPTS };');
      else lines.push(`// context line ${n}`);
    }
    return lines.join('\n');
  }
  return Array.from({ length: 20 }, (_, i) => `// line ${i + 1}`).join('\n');
};

/**
 * No conflicts in the fixture repo — the browser build has no jj to ask, and a
 * fabricated conflict would put a banner and a navigation control on screen
 * for a change whose diff contains no markers to navigate to.
 */
export const mockConflicts: ConflictedFile[] = [];

export const mockOperations: Operation[] = [
  {
    id: 'op1',
    description: 'describe commit 459309ebf873',
    args: 'jj describe -r @ -m "Plan Phase 2"',
    time: '2026-07-26T21:02:00+02:00',
    user: 'valpinkman',
    snapshot: false,
  },
  {
    id: 'op2',
    description: 'push all deleted bookmarks/tags to git remote origin',
    args: 'jj git push --remote origin --deleted',
    time: '2026-07-26T20:58:00+02:00',
    user: 'valpinkman',
    snapshot: false,
  },
  {
    id: 'op3',
    description: 'snapshot working copy',
    args: null,
    time: '2026-07-26T20:57:00+02:00',
    user: 'valpinkman',
    snapshot: true,
  },
  {
    id: 'op4',
    description: 'rebase commit 1da27fbb6a60 onto main',
    args: 'jj rebase -r worktree-correctness -d main',
    time: '2026-07-26T20:40:00+02:00',
    user: 'valpinkman',
    snapshot: false,
  },
];

/** Shaped like real `jj op diff --no-graph` output, which the UI renders verbatim. */
export const mockOperationDiff = (to: string, from: string | null): string => {
  const label = (id: string) => mockOperations.find((op) => op.id === id)?.description ?? id;
  const header = from
    ? `From operation: ${from} (2026-07-26 20:40:00) ${label(from)}\n  To operation: ${to} (2026-07-26 21:02:00) ${label(to)}`
    : `From operation: op2 (2026-07-26 20:58:00) ${label('op2')}\n  To operation: ${to} (2026-07-26 21:02:00) ${label(to)}`;
  return `${header}

Changed commits:
○  + qpalzmw 459309eb Plan Phase 2
   - qpalzmw hidden 1da27fbb (no description set)

Changed working copy default@:
+ qpalzmw 459309eb Plan Phase 2
- wcwcwcw 8bc80827 (empty) (no description set)
`;
};
