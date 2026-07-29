// Flat row model: the virtualizer renders one flat list of rows across all files,
// so huge diffs stay cheap regardless of where the changes live.
import type { Comment, FilePatch, Line } from './ipc.js';

export type DiffLayout = 'split' | 'unified';

export type Row =
  /** Transparent space between two file cards. See `buildRows`. */
  | { kind: 'gap'; fileIndex: number }
  | { kind: 'file'; fileIndex: number; file: FilePatch }
  | { kind: 'hunk'; fileIndex: number; label: string; path: string; hunkId: string }
  /** Clickable gap between hunks: pulls more context from the full file. */
  | {
      kind: 'expander';
      fileIndex: number;
      path: string;
      hunkId: string;
      direction: 'up' | 'down';
      hidden: number;
    }
  | { kind: 'notice'; fileIndex: number; text: string }
  /** Binary image file — the view fetches and renders it. */
  | { kind: 'image'; fileIndex: number; path: string; oldPath: string | null }
  /** Rendered markdown preview (replaces the diff rows for `.md` files). */
  | { kind: 'markdown'; fileIndex: number; path: string; html: string }
  /** Closes a file card: the rounded, bordered bottom edge. */
  | { kind: 'file-end'; fileIndex: number }
  | {
      kind: 'unified';
      fileIndex: number;
      line: Line;
      /** Highlight lookup: which side/index carries this line's tokens. */
      hl: HlRef | null;
    }
  | {
      kind: 'split';
      fileIndex: number;
      left: Line | null;
      right: Line | null;
      hlLeft: HlRef | null;
      hlRight: HlRef | null;
    }
  /** Inline review comments rendered under their anchor line. */
  | { kind: 'comments'; fileIndex: number; path: string; comments: Comment[] };

export interface HlRef {
  side: 'old' | 'new';
  index: number;
}

/** Per-file, per-side reconstructed text used for syntax highlighting. */
export interface SideTexts {
  old: string[];
  new: string[];
}

/** Extra context lines pulled in around a hunk, keyed by hunk id. */
export interface Expansion {
  up: number;
  down: number;
}

export function buildRows(
  files: FilePatch[],
  layout: DiffLayout,
  viewed: ReadonlySet<string> = new Set(),
  /** When set (walkthrough step), only these hunks render — files without any are skipped. */
  hunkFilter: ReadonlySet<string> | null = null,
  /** Full new-side file text, split into lines, for context expansion. */
  fileLines: ReadonlyMap<string, string[]> = new Map(),
  expansions: ReadonlyMap<string, Expansion> = new Map(),
  /** Inline review comments, keyed `${path}:${side}:${line}`. */
  comments: ReadonlyMap<string, Comment[]> = new Map(),
  /** Paths in markdown-preview mode → rendered HTML. Replaces the diff rows. */
  markdownPreviews: ReadonlyMap<string, string> = new Map(),
): Row[] {
  const rows: Row[] = [];
  const commentsFor = (path: string, side: 'old' | 'new', line: number | null): Comment[] => {
    if (line === null) return [];
    return comments.get(`${path}:${side}:${line}`) ?? [];
  };
  const pushCommentRow = (rows: Row[], fileIndex: number, path: string, side: 'old' | 'new', line: number | null) => {
    const list = commentsFor(path, side, line);
    if (list.length > 0) {
      rows.push({ kind: 'comments', fileIndex, path, comments: list });
    }
  };
  files.forEach((file, fileIndex) => {
    if (hunkFilter && !file.hunks.some((hunk) => hunkFilter.has(hunk.id))) {
      return;
    }
    // Space between cards is a row of its own, not a margin and not a
    // transparent border on the header. Margins are invisible to the
    // virtualizer (it measures `offsetHeight`), and a transparent border on the
    // header cannot be rounded — `background-clip: padding-box` clips the
    // surface to a box whose corner radius is the border radius *minus* the
    // border width, so an 18px strip flattens any radius under it. A spacer row
    // leaves the header free to round its own top.
    if (rows.length > 0) {
      rows.push({ kind: 'gap', fileIndex });
    }
    rows.push({ kind: 'file', fileIndex, file });
    if (viewed.has(file.path)) {
      // Viewed files collapse to the header alone — that is the point of the flag. The
      // header carries the closing radius itself in that case (.file-header.viewed).
      return;
    }
    // Markdown preview: replaces the diff rows with a rendered view.
    if (markdownPreviews.has(file.path)) {
      rows.push({ kind: 'markdown', fileIndex, path: file.path, html: markdownPreviews.get(file.path)! });
      rows.push({ kind: 'file-end', fileIndex });
      return;
    }
    if (file.binary) {
      if (isImagePath(file.path) || isImagePath(file.oldPath)) {
        rows.push({ kind: 'image', fileIndex, path: file.path, oldPath: file.oldPath });
      } else {
        rows.push({ kind: 'notice', fileIndex, text: 'Binary file' });
      }
      return;
    }
    if (file.skipped) {
      rows.push({ kind: 'notice', fileIndex, text: file.skipped });
      return;
    }
    if (file.hunks.length === 0) {
      rows.push({ kind: 'notice', fileIndex, text: 'Empty file' });
      return;
    }
    // Side line counters for highlight lookup — must mirror sideTexts().
    let oldIndex = 0;
    let newIndex = 0;
    const refFor = (line: Line): HlRef => {
      if (line.kind === 'removed') return { side: 'old', index: oldIndex++ };
      if (line.kind === 'added') return { side: 'new', index: newIndex++ };
      oldIndex += 1;
      return { side: 'new', index: newIndex++ };
    };

    const lines = fileLines.get(file.path);
    let previousEnd = 0; // last new-side line already shown, for gap sizing
    for (const hunk of file.hunks) {
      // Filtered-out hunks still consume highlight indices (refFor) so lookups stay in
      // sync with sideTexts(), which always walks every hunk.
      const included = !hunkFilter || hunkFilter.has(hunk.id);
      const expansion = expansions.get(hunk.id) ?? { up: 0, down: 0 };
      if (included) {
        // The gap above this hunk is known from hunk boundaries alone, so the expander
        // renders before any file content has been fetched — clicking it is what loads
        // the file. Expanded lines only appear once `lines` is present.
        const gapStart = previousEnd + 1;
        const shownFrom = lines
          ? Math.max(gapStart, hunk.newStart - expansion.up)
          : hunk.newStart;
        const stillHidden = shownFrom - gapStart;
        if (stillHidden > 0) {
          rows.push({
            kind: 'expander',
            fileIndex,
            path: file.path,
            hunkId: hunk.id,
            direction: 'up',
            hidden: stillHidden,
          });
        }
        rows.push({
          kind: 'hunk',
          fileIndex,
          path: file.path,
          hunkId: hunk.id,
          label: `@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@${
            hunk.context ? ' ' + hunk.context : ''
          }`,
        });
        if (lines) {
          for (let n = shownFrom; n < hunk.newStart; n++) {
            pushContextLine(rows, fileIndex, layout, lines[n - 1] ?? '', n);
          }
        }
      }
      if (layout === 'unified') {
        for (const line of hunk.lines) {
          const hl = refFor(line);
          if (included) {
            rows.push({ kind: 'unified', fileIndex, line, hl });
            // Comments anchor to the side matching the line kind.
            const side = line.kind === 'removed' ? 'old' : 'new';
            const lineNum = side === 'old' ? line.oldLine : line.newLine;
            pushCommentRow(rows, fileIndex, file.path, side, lineNum);
          }
        }
      } else {
        pushSplitRows(rows, fileIndex, hunk.lines, refFor, included, file.path, pushCommentRow);
      }

      previousEnd = hunk.newStart + hunk.newLines - 1;
      if (included && lines && expansion.down > 0) {
        const until = Math.min(previousEnd + expansion.down, lines.length);
        for (let n = previousEnd + 1; n <= until; n++) {
          pushContextLine(rows, fileIndex, layout, lines[n - 1] ?? '', n);
        }
        previousEnd = until;
      }
    }

    // Trailing gap. Without the file loaded the remaining length is unknown, so the
    // expander is offered optimistically and disappears once the end is reached.
    const lastHunk = file.hunks[file.hunks.length - 1];
    const remaining = lines ? lines.length - previousEnd : -1;
    if (lastHunk && remaining !== 0 && (!hunkFilter || hunkFilter.has(lastHunk.id))) {
      rows.push({
        kind: 'expander',
        fileIndex,
        path: file.path,
        hunkId: lastHunk.id,
        direction: 'down',
        hidden: remaining > 0 ? remaining : 0,
      });
    }
    rows.push({ kind: 'file-end', fileIndex });
  });
  return rows;
}

/** Emit one expanded-context line in whichever layout is active. */
function pushContextLine(
  rows: Row[],
  fileIndex: number,
  layout: DiffLayout,
  text: string,
  lineNumber: number,
) {
  // Expanded context is unchanged code: same number both sides, no highlight tokens
  // (they are keyed to hunk lines) and no word spans.
  const line: Line = {
    kind: 'context',
    text,
    oldLine: lineNumber,
    newLine: lineNumber,
    spans: [],
  };
  if (layout === 'unified') {
    rows.push({ kind: 'unified', fileIndex, line, hl: null });
  } else {
    rows.push({ kind: 'split', fileIndex, left: line, right: line, hlLeft: null, hlRight: null });
  }
}

/** Pair removed/added runs side-by-side; context spans both sides. */
type CommentPusher = (rows: Row[], fileIndex: number, path: string, side: 'old' | 'new', line: number | null) => void;

function pushSplitRows(
  rows: Row[],
  fileIndex: number,
  lines: Line[],
  refFor: (line: Line) => HlRef,
  emit: boolean,
  path: string,
  pushCommentRow: CommentPusher,
) {
  let removed: { line: Line; hl: HlRef }[] = [];
  let added: { line: Line; hl: HlRef }[] = [];

  const flush = () => {
    const count = Math.max(removed.length, added.length);
    for (let i = 0; i < count; i++) {
      if (emit) {
        rows.push({
          kind: 'split',
          fileIndex,
          left: removed[i]?.line ?? null,
          right: added[i]?.line ?? null,
          hlLeft: removed[i]?.hl ?? null,
          hlRight: added[i]?.hl ?? null,
        });
        // Comments under each side that has a line number.
        const r = removed[i];
        const a = added[i];
        if (r) pushCommentRow(rows, fileIndex, path, 'old', r.line.oldLine);
        if (a) pushCommentRow(rows, fileIndex, path, 'new', a.line.newLine);
      }
    }
    removed = [];
    added = [];
  };

  for (const line of lines) {
    if (line.kind === 'removed') {
      if (added.length > 0) flush();
      removed.push({ line, hl: refFor(line) });
    } else if (line.kind === 'added') {
      added.push({ line, hl: refFor(line) });
    } else {
      flush();
      const hl = refFor(line);
      if (emit) {
        rows.push({ kind: 'split', fileIndex, left: line, right: line, hlLeft: hl, hlRight: hl });
        pushCommentRow(rows, fileIndex, path, 'old', line.oldLine);
        pushCommentRow(rows, fileIndex, path, 'new', line.newLine);
      }
    }
  }
  flush();
}

/**
 * Reconstruct each side's text for highlighting. Old side = context + removed lines,
 * new side = context + added lines — token grammar state flows across hunks well enough
 * for review purposes (a full-file fetch arrives with M2 file contents).
 */
export function sideTexts(file: FilePatch): SideTexts {
  const oldLines: string[] = [];
  const newLines: string[] = [];
  for (const hunk of file.hunks) {
    for (const line of hunk.lines) {
      if (line.kind === 'removed') oldLines.push(line.text);
      else if (line.kind === 'added') newLines.push(line.text);
      else {
        oldLines.push(line.text);
        newLines.push(line.text);
      }
    }
  }
  return { old: oldLines, new: newLines };
}

// ---- Selection units (hunk-level split) ----
//
// A split selection is a set of strings, and a file contributes either its
// hunk ids or a single id standing for the whole file. One set rather than two
// because every consumer — the checkboxes, the counts, the plan — asks the same
// question of both kinds: is this in.

/**
 * The id standing for a whole file. `#*` cannot collide with a hunk id, which
 * is always `<path>#<index>`.
 */
export const fileUnitId = (path: string): string => `${path}#*`;

/**
 * Whether a file's change can be divided at all.
 *
 * Binary content has no hunks to pick between. A rename is one act — keeping
 * the move but dropping the edits is not a thing `jj split` can express, since
 * both halves would have to claim the same path. And a single hunk is already
 * the whole file, so offering a second checkbox for it would be two controls
 * for one decision.
 */
export const supportsHunkSelection = (file: FilePatch): boolean =>
  !file.binary && !file.skipped && !file.oldPath && file.hunks.length > 1;

/** Everything in `file` that can be independently selected. */
export function selectionUnits(file: FilePatch): string[] {
  return supportsHunkSelection(file)
    ? file.hunks.map((hunk) => hunk.id)
    : [fileUnitId(file.path)];
}

const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico']);

export function isImagePath(path: string | null | undefined): boolean {
  if (!path) return false;
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  return IMAGE_EXTENSIONS.has(ext);
}
