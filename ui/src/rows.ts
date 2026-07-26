// Flat row model: the virtualizer renders one flat list of rows across all files,
// so huge diffs stay cheap regardless of where the changes live.
import type { FilePatch, Line } from './ipc.js';

export type DiffLayout = 'split' | 'unified';

export type Row =
  | { kind: 'file'; fileIndex: number; file: FilePatch }
  | { kind: 'hunk'; fileIndex: number; label: string }
  | { kind: 'notice'; fileIndex: number; text: string }
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
    };

export interface HlRef {
  side: 'old' | 'new';
  index: number;
}

/** Per-file, per-side reconstructed text used for syntax highlighting. */
export interface SideTexts {
  old: string[];
  new: string[];
}

export function buildRows(
  files: FilePatch[],
  layout: DiffLayout,
  viewed: ReadonlySet<string> = new Set(),
): Row[] {
  const rows: Row[] = [];
  files.forEach((file, fileIndex) => {
    rows.push({ kind: 'file', fileIndex, file });
    if (viewed.has(file.path)) {
      // Viewed files collapse — that is the point of the flag.
      return;
    }
    if (file.binary) {
      rows.push({ kind: 'notice', fileIndex, text: 'Binary file' });
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

    for (const hunk of file.hunks) {
      rows.push({
        kind: 'hunk',
        fileIndex,
        label: `@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@${
          hunk.context ? ' ' + hunk.context : ''
        }`,
      });
      if (layout === 'unified') {
        for (const line of hunk.lines) {
          rows.push({ kind: 'unified', fileIndex, line, hl: refFor(line) });
        }
      } else {
        pushSplitRows(rows, fileIndex, hunk.lines, refFor);
      }
    }
  });
  return rows;
}

/** Pair removed/added runs side-by-side; context spans both sides. */
function pushSplitRows(
  rows: Row[],
  fileIndex: number,
  lines: Line[],
  refFor: (line: Line) => HlRef,
) {
  let removed: { line: Line; hl: HlRef }[] = [];
  let added: { line: Line; hl: HlRef }[] = [];

  const flush = () => {
    const count = Math.max(removed.length, added.length);
    for (let i = 0; i < count; i++) {
      rows.push({
        kind: 'split',
        fileIndex,
        left: removed[i]?.line ?? null,
        right: added[i]?.line ?? null,
        hlLeft: removed[i]?.hl ?? null,
        hlRight: added[i]?.hl ?? null,
      });
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
      rows.push({ kind: 'split', fileIndex, left: line, right: line, hlLeft: hl, hlRight: hl });
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
