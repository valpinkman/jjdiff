import { html, LitElement, nothing, type PropertyValues, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { virtualize, virtualizerRef } from '@lit-labs/virtualizer/virtualize.js';

import { HighlightStore } from './highlight.js';
import type { Comment, CommentSide, FilePatch, Line } from './ipc.js';
import { renderLineContent } from './render-line.js';
import {
  buildRows,
  selectionUnits,
  supportsHunkSelection,
  type DiffLayout,
  type Expansion,
  type HlRef,
  type Row,
} from './rows.js';
import { relativeTime } from './time.js';

/**
 * Virtualized diff renderer.
 *
 * Light DOM on purpose (PLAN.md): native text selection, copy, and find must work across
 * lines without crossing shadow boundaries. All rows across all files form ONE flat
 * virtualized list, so a 10k-line diff costs what the viewport shows. Styles: theme.css.
 *
 * Keyboard review surface: the app shell drives a row cursor (j/k files, n/p hunks,
 * v viewed) and a text search via public methods; scrolling goes through the virtualizer.
 */
@customElement('jj-patch-view')
export class PatchView extends LitElement {
  @property({ attribute: false }) files: FilePatch[] = [];
  @property() layout: DiffLayout = 'split';
  /** Paths marked viewed — their content collapses. */
  @property({ attribute: false }) viewed: ReadonlySet<string> = new Set();
  /** Whether per-file actions (viewed toggle, squash) apply to this diff. */
  @property({ type: Boolean }) canSquash = false;
  @property({ type: Boolean }) canMarkViewed = false;
  /** Mutable changes a working-copy file can be squashed into ("move to change"). */
  @property({ attribute: false }) squashTargets: { changeId: string; label: string }[] = [];
  /** Conflicted paths → jj's description of the conflict ("2-sided conflict"). */
  @property({ attribute: false }) conflicted: ReadonlyMap<string, string> = new Map();
  /**
   * Hunk selection: the picked units, or null when not selecting.
   *
   * Null rather than an empty set, because "no checkboxes on screen" and "no
   * boxes ticked" are different states and the second is a legitimate place to
   * be while deciding.
   */
  @property({ attribute: false }) selection: ReadonlySet<string> | null = null;
  /**
   * What the selection is for. Split and squash pick from the same diff with
   * the same checkboxes; only what the tick promises differs, and a checkbox
   * that says "split" while a squash is in progress is worse than no tooltip.
   */
  @property() selectionVerb: 'split' | 'squash' = 'split';
  /** Active walkthrough step's hunk ids; null = show everything. */
  @property({ attribute: false }) hunkFilter: ReadonlySet<string> | null = null;
  /** Find-in-diffs query; null when search is closed. */
  @property() searchQuery: string | null = null;
  /** Wrap long lines instead of scrolling horizontally. */
  @property({ type: Boolean }) wordWrap = false;
  /** Full new-side file text (split into lines) for context expansion. */
  @property({ attribute: false }) fileLines: ReadonlyMap<string, string[]> = new Map();
  /** Extra context pulled in per hunk. */
  @property({ attribute: false }) expansions: ReadonlyMap<string, Expansion> = new Map();
  /** Bumped when the colour theme changes: shiki tokens are theme-specific. */
  @property({ type: Number }) themeVersion = 0;
  /** Inline review comments, keyed `${path}:${side}:${line}`. */
  @property({ attribute: false }) comments: ReadonlyMap<string, Comment[]> = new Map();
  /** Whether comments can be added (working-copy + mutable change). */
  @property({ type: Boolean }) canComment = false;
  /** Revset for the diff being shown; null = working copy. Used for image fetching. */
  @property({ attribute: false }) revset: string | null = null;
  /** Paths in markdown-preview mode → rendered HTML. */
  @property({ attribute: false }) markdownPreviews: ReadonlyMap<string, string> = new Map();

  @state() private cursor: number | null = null;
  @state() private searchCurrent = -1;
  /** Active inline composer, if any. */
  @state() private composer: { path: string; side: CommentSide; line: number; lineText: string } | null = null;

  private rows: Row[] = [];
  /** Widest rendered line in monospace columns — see `codeColumns`. */
  private codeCols = 0;
  /** How far each side's code is panned, in px. The pane itself never scrolls sideways. */
  private codeScroll = { old: 0, new: 0 };
  private visibleFile: string | null = null;
  /** The file whose header is currently pinned, or null when it is on screen. */
  @state() private stuck: FilePatch | null = null;
  /** Whether the pinned header is all that is left of its card — see `updateStuckClosing`. */
  @state() private stuckClosing = false;
  /** Row index of the pinned card's closing row; -1 when it has none. */
  private stuckFoot = -1;
  /** Row index of the next card's header — what pushes the pinned bar out. */
  private stuckNext = -1;
  /** Rows the virtualizer currently has mounted, from `visibilityChanged`. */
  private range = { first: 0, last: 0 };
  private searchMatches: number[] = [];
  private highlights = new HighlightStore();

  protected override createRenderRoot() {
    return this; // light DOM
  }

  override connectedCallback() {
    super.connectedCallback();
    this.highlights.addEventListener('tokens', this.onTokens);
    // Not passive: a horizontal gesture over the code is ours to consume.
    this.addEventListener('wheel', this.onWheel, { passive: false });
    this.resizeObserver.observe(this);
  }

  override disconnectedCallback() {
    this.highlights.removeEventListener('tokens', this.onTokens);
    this.removeEventListener('wheel', this.onWheel);
    this.resizeObserver.disconnect();
    super.disconnectedCallback();
  }

  private onTokens = () => this.requestUpdate();

  protected override willUpdate(changed: PropertyValues<this>) {
    const contentChanged =
      changed.has('files') ||
      changed.has('layout') ||
      changed.has('viewed') ||
      changed.has('hunkFilter') ||
      changed.has('fileLines') ||
      changed.has('expansions') ||
      changed.has('comments') ||
      changed.has('markdownPreviews');
    if (contentChanged) {
      this.rows = buildRows(
        this.files,
        this.layout,
        this.viewed,
        this.hunkFilter,
        this.fileLines,
        this.expansions,
        this.comments,
        this.markdownPreviews,
      );
      this.codeCols = widestLine(this.rows);
      if (this.cursor !== null && this.cursor >= this.rows.length) {
        this.cursor = null;
      }
      // The pinned header holds a `FilePatch` captured by a *scroll* event, and
      // nothing else ever reassigns it. When the diff changes underneath it —
      // a file discarded, squashed away, split out, or filtered out by a
      // walkthrough step — it would go on naming a file that is no longer in
      // the diff until the next scroll, which for a short diff may never come.
      // Re-resolving by path also refreshes its +/− counts, which were pinned
      // to the old object too.
      if (this.stuck) {
        const path = this.stuck.path;
        this.stuck = this.files.find((file) => file.path === path) ?? null;
      }
    }
    if (changed.has('files') || changed.has('layout') || changed.has('wordWrap')) {
      this.codeScroll = { old: 0, new: 0 };
    }
    if (changed.has('files') || changed.has('themeVersion')) {
      this.highlights.clear();
      for (const file of this.files) {
        this.highlights.request(file);
      }
    }
    if (contentChanged || changed.has('searchQuery')) {
      this.computeSearch(changed.has('searchQuery'));
    }
  }

  protected override updated() {
    this.applyPaneVars();
  }

  // ---- Horizontal panning ----
  //
  // Long lines scroll within their own column rather than scrolling the pane, so the
  // file card keeps its gutters and never drifts sideways. Every row is a separate
  // virtualizer child, so the offset lives in two CSS custom properties on the pane and
  // the rows translate off it — rows recycled into view mid-pan come in already in step.

  private get pane(): HTMLElement | null {
    return this.querySelector('.jj-patch');
  }

  private applyPaneVars() {
    const pane = this.pane;
    if (!pane) return;
    pane.style.setProperty('--jj-code-cols', String(this.codeCols));
    pane.style.setProperty('--jj-scroll-old', `${this.codeScroll.old}px`);
    pane.style.setProperty('--jj-scroll-new', `${this.codeScroll.new}px`);
  }

  /** How far a column can pan: what the widest line needs, minus what a row shows. */
  private maxCodeScroll(pane: HTMLElement): number {
    const styles = getComputedStyle(pane);
    const natural = parseFloat(styles.getPropertyValue('--jj-row-width'));
    const gutter = parseFloat(styles.getPropertyValue('--jj-diff-gutter'));
    if (!Number.isFinite(natural) || !Number.isFinite(gutter)) return 0;
    const shown = pane.clientWidth - gutter * 2;
    const overflow = Math.max(0, natural - shown);
    // Split cells are equal halves, so the overflow is shared evenly between them.
    return this.layout === 'split' ? overflow / 2 : overflow;
  }

  private onWheel = (event: WheelEvent) => {
    if (this.wordWrap) return;
    const pane = this.pane;
    if (!pane) return;
    // Trackpads report sideways gestures as deltaX; shift+wheel is the mouse equivalent.
    const delta =
      Math.abs(event.deltaX) > Math.abs(event.deltaY)
        ? event.deltaX
        : event.shiftKey
          ? event.deltaY
          : 0;
    if (delta === 0) return;
    const side = this.sideAt(event.clientX, pane);
    const next = clamp(this.codeScroll[side] + delta, 0, this.maxCodeScroll(pane));
    // At either end the gesture belongs to whatever is behind us (or nothing).
    if (next === this.codeScroll[side]) return;
    event.preventDefault();
    this.codeScroll = { ...this.codeScroll, [side]: next };
    this.applyPaneVars();
  };

  /** Which side the pointer is over. The gutters are symmetric, so the pane's midpoint
      is exactly the split divider; unified has one column and always pans the new side. */
  private sideAt(clientX: number, pane: HTMLElement): 'old' | 'new' {
    if (this.layout !== 'split') return 'new';
    const box = pane.getBoundingClientRect();
    return clientX < box.left + box.width / 2 ? 'old' : 'new';
  }

  private resizeObserver = new ResizeObserver(() => {
    const pane = this.pane;
    if (!pane) return;
    const max = this.maxCodeScroll(pane);
    const old = Math.min(this.codeScroll.old, max);
    const fresh = Math.min(this.codeScroll.new, max);
    if (old === this.codeScroll.old && fresh === this.codeScroll.new) return;
    this.codeScroll = { old, new: fresh };
    this.applyPaneVars();
  });

  // ---- Keyboard review surface (called by the app shell) ----

  /** Move the cursor to the next/previous row of `kind` and scroll it into view. */
  moveCursor(kind: 'file' | 'hunk', direction: 1 | -1): void {
    const candidates: number[] = [];
    this.rows.forEach((row, index) => {
      if (row.kind === kind) candidates.push(index);
    });
    if (candidates.length === 0) return;
    const from = this.cursor ?? -1;
    let next: number | undefined;
    if (direction === 1) {
      next = candidates.find((index) => index > from) ?? candidates[candidates.length - 1];
    } else {
      next = [...candidates].reverse().find((index) => index < from) ?? candidates[0];
    }
    if (next !== undefined) {
      this.cursor = next;
      this.scrollToRow(next);
    }
  }

  /** Toggle viewed on the file owning the cursor row. */
  toggleViewedAtCursor(): void {
    if (this.cursor === null || !this.canMarkViewed) return;
    for (let index = this.cursor; index >= 0; index--) {
      const row = this.rows[index];
      if (row?.kind === 'file') {
        const path = row.file.path;
        this.emit('toggle-viewed', path, !this.viewed.has(path));
        return;
      }
    }
  }

  /**
   * Where the cursor is, for "open in editor": the file owning the cursor row,
   * plus a line number when the cursor sits on a code row. Prefers the new-side
   * number — that is what exists on disk; a removed line has only an old one,
   * and pointing an editor at it would land on the wrong place.
   */
  cursorLocation(): { path: string; line?: number } | null {
    if (this.cursor === null) return null;
    const row = this.rows[this.cursor];
    let line: number | undefined;
    if (row?.kind === 'unified') {
      line = row.line.newLine ?? undefined;
    } else if (row?.kind === 'split') {
      line = row.right?.newLine ?? undefined;
    }
    for (let index = this.cursor; index >= 0; index--) {
      const owner = this.rows[index];
      if (owner?.kind === 'file') return { path: owner.file.path, line };
    }
    return null;
  }

  /** Scroll a file's header to the top of the viewport. */
  scrollToPath(path: string): void {
    const index = this.rows.findIndex((row) => row.kind === 'file' && row.file.path === path);
    if (index >= 0) {
      this.cursor = index;
      this.scrollToRow(index, 'start');
    }
  }

  /**
   * Move to the next or previous conflict and report where it landed.
   *
   * The target is the `<<<<<<<` line that opens each conflict region, not the
   * file — a file can hold several, and stopping at its header would leave the
   * reviewer to find them by scrolling, which is the job being automated. Files
   * jj flagged but whose contents were not diffed (too large, binary) have no
   * marker to land on, so their header stands in; missing them out entirely
   * would make the count in the banner disagree with what the button can reach.
   */
  moveToConflict(direction: 1 | -1): { path: string; index: number; total: number } | null {
    const stops: number[] = [];
    let inConflictedFile = false;
    // The header standing in for the file, until a real marker inside it turns
    // up and takes its place. Held per file, and only the first marker spends it.
    let placeholder: number | null = null;
    this.rows.forEach((row, index) => {
      if (row.kind === 'file') {
        inConflictedFile = this.conflicted.has(row.file.path);
        placeholder = inConflictedFile ? index : null;
        if (inConflictedFile) stops.push(index);
        return;
      }
      if (!inConflictedFile || !conflictOpener(row)) return;
      if (placeholder !== null) {
        stops.splice(stops.indexOf(placeholder), 1);
        placeholder = null;
      }
      stops.push(index);
    });
    if (stops.length === 0) return null;

    const from = this.cursor ?? -1;
    const next =
      direction === 1
        ? (stops.find((index) => index > from) ?? stops[0]!)
        : ([...stops].reverse().find((index) => index < from) ?? stops[stops.length - 1]!);
    this.cursor = next;
    this.scrollToRow(next);
    for (let index = next; index >= 0; index--) {
      const owner = this.rows[index];
      if (owner?.kind === 'file') {
        return { path: owner.file.path, index: stops.indexOf(next) + 1, total: stops.length };
      }
    }
    return null;
  }

  // ---- Split selection ----

  /** Whether every / any selectable unit of a file is picked. */
  private fileSelection(file: FilePatch): { all: boolean; some: boolean } {
    const units = selectionUnits(file);
    const picked = units.filter((unit) => this.selection?.has(unit)).length;
    return { all: picked === units.length, some: picked > 0 };
  }

  private emitSelect(units: string[], selected: boolean) {
    this.dispatchEvent(
      new CustomEvent('select-units', {
        bubbles: true,
        composed: true,
        detail: { units, selected },
      }),
    );
  }

  /** The revset to fetch file bytes at; null = working copy (on-disk). */
  private revsetForFile(_fileIndex: number): string | null {
    return this.revset;
  }

  private isMarkdown(path: string): boolean {
    return path.toLowerCase().endsWith('.md');
  }

  private emitToggleMarkdown(path: string) {
    this.dispatchEvent(
      new CustomEvent('toggle-markdown', {
        bubbles: true,
        composed: true,
        detail: { path },
      }),
    );
  }

  /** Advance the current search match (wraps). */
  moveMatch(direction: 1 | -1): void {
    if (this.searchMatches.length === 0) return;
    const count = this.searchMatches.length;
    this.searchCurrent = (this.searchCurrent + direction + count) % count;
    this.scrollToRow(this.searchMatches[this.searchCurrent]!);
    this.emitSearchState();
  }

  private computeSearch(isNewQuery: boolean) {
    const query = this.searchQuery?.trim().toLowerCase() ?? '';
    if (!query) {
      this.searchMatches = [];
      this.searchCurrent = -1;
      this.emitSearchState();
      return;
    }
    const matches: number[] = [];
    this.rows.forEach((row, index) => {
      if (rowMatches(row, query)) matches.push(index);
    });
    this.searchMatches = matches;
    this.searchCurrent = matches.length > 0 ? 0 : -1;
    if (isNewQuery && matches.length > 0) {
      this.scrollToRow(matches[0]!);
    }
    this.emitSearchState();
  }

  private emitSearchState() {
    this.dispatchEvent(
      new CustomEvent('search-state', {
        bubbles: true,
        composed: true,
        detail: { count: this.searchMatches.length, current: this.searchCurrent },
      }),
    );
  }

  private scrollToRow(index: number, block: 'center' | 'start' = 'center') {
    // `element(i)` is NOT a DOM node — it is a scroll handle the virtualizer
    // hands out so you can target rows it has not rendered, and the only thing
    // on it is `scrollIntoView`. Anything needing real geometry has to find the
    // rendered node itself; see `cardEdge`.
    const host = this.querySelector('.jj-patch') as
      | (HTMLElement & { [virtualizerRef]?: { element(i: number): { scrollIntoView(o?: object): void } | undefined } })
      | null;
    host?.[virtualizerRef]?.element(index)?.scrollIntoView({ block });
  }

  /**
   * A rendered card edge, found by path rather than by row index.
   *
   * Row index cannot be mapped to a child element: the virtualizer renders a
   * window of rows, and some row kinds emit more than one top-level node (a
   * code line and the comment composer under it), so the nth child is not the
   * nth row. Both edges carry `data-path`, which is unambiguous.
   */
  private cardEdge(kind: '.file-header' | '.file-end', path: string): HTMLElement | null {
    return this.querySelector<HTMLElement>(
      `.jj-patch > ${kind}[data-path="${CSS.escape(path)}"]`,
    );
  }

  /**
   * Which file the viewport is inside, and whether its header has scrolled off.
   *
   * `position: sticky` is not available here: the virtualizer positions every
   * row absolutely, and an absolutely-positioned element cannot stick. So the
   * pinned header is a separate element overlaying the top of the pane, shown
   * only while the real one is above the fold — which is also what stops the
   * path appearing twice on screen at once.
   */
  private onVisibilityChanged = (event: Event) => {
    // `visibilityChanged` carries `first`/`last` as own properties on the event
    // object — it is not a CustomEvent and has no `detail`. Reading `.detail`
    // yielded undefined and fell back to 0, so this always reported the first
    // file in the diff no matter where the viewport was.
    const first = (event as Event & { first?: number }).first ?? 0;
    this.range = { first, last: (event as Event & { last?: number }).last ?? first };
    // Walk back to the file header owning the topmost visible row.
    for (let index = Math.min(first, this.rows.length - 1); index >= 0; index--) {
      const row = this.rows[index];
      if (row?.kind === 'file') {
        // `index === first` means the header itself is the topmost row, so it is
        // still on screen and nothing needs pinning.
        this.stuck = index < first ? row.file : null;
        this.stuckFoot = this.stuck ? this.footRow(index) : -1;
        this.stuckNext = this.stuck ? this.nextCardRow(index) : -1;
        this.updateStuckGeometry();
        if (this.visibleFile !== row.file.path) {
          this.visibleFile = row.file.path;
          this.dispatchEvent(
            new CustomEvent('visible-file', {
              bubbles: true,
              composed: true,
              detail: { path: row.file.path },
            }),
          );
        }
        return;
      }
    }
    this.stuck = null;
    this.stuckFoot = -1;
    this.stuckNext = -1;
    this.stuckClosing = false;
  };

  /** The row index of a card's closing row, or -1 when it has none (collapsed). */
  private footRow(fileRow: number): number {
    for (let index = fileRow + 1; index < this.rows.length; index++) {
      const kind = this.rows[index]?.kind;
      if (kind === 'file-end') return index;
      // A collapsed (viewed) file has no foot — its header is already the whole
      // card, and `.viewed` rounds it all round without any of this.
      if (kind === 'file' || kind === 'gap') return -1;
    }
    return -1;
  }

  /** The row index of the next card's header, or -1 when this is the last. */
  private nextCardRow(fileRow: number): number {
    for (let index = fileRow + 1; index < this.rows.length; index++) {
      if (this.rows[index]?.kind === 'file') return index;
    }
    return -1;
  }

  /**
   * Where the pinned bar sits and whether it has to close itself.
   *
   * Two separate things, measured rather than inferred from row indices — the
   * bar *overlays* the top of the scroller, so anything phrased in terms of the
   * first visible row is a bar-height out of date by construction.
   *
   * **Push.** A pinned header must be shoved out of the way by the card coming
   * up behind it, not left hovering over it. Once the next card's top edge
   * reaches the bar's bottom, the bar rides up with it and off the top — the
   * standard sticky-header handover, and the thing that makes the pinned bar
   * feel attached to the list rather than painted on the window.
   *
   * **Closing.** Rounding the bottom corners happens earlier, when the card's
   * own foot passes under the bar: at that point the bar is the last piece of
   * that card on screen, so it stops being a lid and becomes the whole card.
   * The two moments are one gap-row apart, which is why they are measured
   * against different things.
   *
   * Rows outside the rendered range have no element to measure, so the visible
   * range answers those: above it has certainly passed, below it certainly has
   * not.
   */
  private updateStuckGeometry() {
    const bar = this.querySelector('.stuck-holder') as HTMLElement | null;
    const pane = this.pane;
    if (!this.stuck || !bar || !pane) {
      if (this.stuckClosing) this.stuckClosing = false;
      return;
    }
    const barBox = bar.getBoundingClientRect();
    const paneTop = pane.getBoundingClientRect().top;
    // `offsetHeight`, not the rect: the rect is the *pushed* box, and feeding
    // that back in would make the push chase its own tail.
    const barHeight = bar.offsetHeight;

    // The next card is what shoves this one out. Not rendered yet means it is
    // still far below, so nothing is pushing.
    //
    // The gap counts: the bar has to come to rest one gap-row *above* the next
    // card, not flush against it. Landing flush makes two cards touch, which is
    // the one thing the gap row exists to prevent — and it is the pinned bar,
    // not the card behind it, that has to give way. Measured off a real gap
    // rather than hard-coded, since the height is a CSS decision.
    // Clamped at bar + gap: that is the point where the bar has cleared the top
    // entirely, and stopping at the bar's height alone would let the last few
    // pixels of travel eat the gap it just spent the whole handover holding.
    const gap = this.querySelector<HTMLElement>('.jj-patch > .file-gap')?.offsetHeight ?? 0;
    const next = this.stuckNext >= 0 ? this.rows[this.stuckNext] : undefined;
    const nextHeader =
      next?.kind === 'file' ? this.cardEdge('.file-header', next.file.path) : null;
    const push = nextHeader
      ? Math.max(
          -(barHeight + gap),
          Math.min(0, nextHeader.getBoundingClientRect().top - paneTop - barHeight - gap),
        )
      : 0;
    bar.style.setProperty('--jj-stuck-push', `${push}px`);

    // Rounding is the card's own foot passing under the bar, one gap-row before
    // the push begins. An unrendered foot is answered by the row range: above
    // it has certainly passed, below it certainly has not.
    const foot = this.stuck ? this.cardEdge('.file-end', this.stuck.path) : null;
    let closing: boolean;
    if (this.stuckFoot < 0) {
      closing = false;
    } else if (foot) {
      closing = foot.getBoundingClientRect().bottom <= barBox.bottom;
    } else {
      closing = this.stuckFoot < this.range.first;
    }
    if (closing !== this.stuckClosing) this.stuckClosing = closing;
  }

  /** Scrolling slides rows under the bar without changing the rendered range. */
  private onScroll = () => this.updateStuckGeometry();

  private rowClasses(index: number): string {
    const classes: string[] = [];
    if (index === this.cursor) classes.push('kbd-cursor');
    if (this.searchMatches.includes(index)) {
      classes.push(
        index === this.searchMatches[this.searchCurrent] ? 'search-current' : 'search-match',
      );
    }
    return classes.join(' ');
  }

  private emit(name: 'toggle-viewed', path: string, viewed: boolean): void;
  private emit(name: string, path: string, viewed?: boolean) {
    this.dispatchEvent(
      new CustomEvent(name, {
        bubbles: true,
        composed: true,
        detail: viewed === undefined ? { path } : { path, viewed },
      }),
    );
  }

  private emitSquash(path: string, into: string) {
    this.dispatchEvent(
      new CustomEvent('squash-file', {
        bubbles: true,
        composed: true,
        detail: { path, into },
      }),
    );
  }

  protected override render() {
    if (this.files.length === 0) {
      return html`<div class="jj-empty">
        <div class="jj-empty-copy">
          <div class="glyph">✓</div>
          <div class="title">Nothing to review</div>
          <div class="hint">
            Changes appear here live as files are edited or a revision is selected.
          </div>
        </div>
      </div>`;
    }
    // The virtualize() DIRECTIVE, not the <lit-virtualizer> element: the element renders rows
    // into its shadow root, which would cut them off from theme.css and break cross-row text
    // selection — the whole reason this component is light DOM.
    return html`${this.stuck
        ? html`<div class="stuck-holder">
            <div
              class="file-header stuck ${this.stuckClosing ? 'closing' : ''}"
              data-path=${this.stuck.path}
            >
              ${this.renderFileHeaderBody(this.stuck, this.viewed.has(this.stuck.path))}
            </div>
          </div>`
        : nothing}
      <div
        class="jj-patch ${this.layout} ${this.wordWrap ? 'wrap' : 'nowrap'}"
        @scroll=${this.onScroll}
        @visibilityChanged=${this.onVisibilityChanged}
      >
        ${virtualize({
          items: this.rows,
          renderItem: (row: Row, index: number) => this.renderRow(row, index) as TemplateResult,
          scroller: true,
        })}
      </div>`;
  }

  /**
   * The contents of a file header, shared by the row and the sticky bar.
   *
   * One template, two mounts: the header that scrolls with the card and the one
   * pinned to the top of the pane are the same control, so "viewed" is in the
   * same place with the same shape whether or not you have scrolled past it.
   */
  private renderFileHeaderBody(file: FilePatch, isViewed: boolean) {
    const selection = this.selection ? this.fileSelection(file) : null;
    return html`
      ${selection
        ? html`<input
            class="split-check file"
            type="checkbox"
            title=${
              supportsHunkSelection(file)
                ? `Include every hunk of this file in the ${this.selectionVerb}`
                : `Include this file in the ${this.selectionVerb} (it cannot be divided further)`
            }
            aria-label=${`Include ${file.path} in the ${this.selectionVerb}`}
            .checked=${selection.all}
            .indeterminate=${selection.some && !selection.all}
            @click=${(event: Event) => event.stopPropagation()}
            @change=${(event: Event) =>
              this.emitSelect(selectionUnits(file), (event.target as HTMLInputElement).checked)}
          />`
        : nothing}
      <span class="file-status ${file.status}">${file.status}</span>
      <span class="file-path"
        >${file.oldPath ? html`${file.oldPath} → ` : nothing}${file.path}</span
      >
      <span class="file-counts"
        >${file.added ? html`<span class="plus">+${file.added}</span>` : nothing}
        ${file.removed ? html`<span class="minus">−${file.removed}</span>` : nothing}</span
      >
      ${this.conflicted.has(file.path)
        ? html`<span class="conflict-badge" title=${this.conflicted.get(file.path) || 'Unresolved conflict'}
            >${this.conflicted.get(file.path) || 'conflict'}</span
          >`
        : nothing}
      ${this.canSquash
        ? html`<select
            class="file-action"
            title="Squash this file into another change (jj squash)"
            @click=${(event: Event) => event.stopPropagation()}
            @change=${(event: Event) => {
              const select = event.target as HTMLSelectElement;
              if (select.value) {
                this.emitSquash(file.path, select.value);
                select.value = '';
              }
            }}
          >
            <option value="">⇩ move to…</option>
            ${this.squashTargets.map(
              (target) => html`<option value=${target.changeId}>${target.label}</option>`,
            )}
          </select>`
        : nothing}
      ${this.isMarkdown(file.path)
        ? html`<button
            class="file-action md-toggle"
            title="Toggle between diff and rendered preview"
            @click=${(event: Event) => {
              event.stopPropagation();
              this.emitToggleMarkdown(file.path);
            }}
          >
            ${this.markdownPreviews.has(file.path) ? 'diff' : 'preview'}
          </button>`
        : nothing}
      ${this.canMarkViewed
        ? html`<label class="file-action viewed-toggle" title="Mark as viewed">
            <input
              type="checkbox"
              .checked=${isViewed}
              @change=${(event: Event) =>
                this.emit('toggle-viewed', file.path, (event.target as HTMLInputElement).checked)}
            />
            viewed
          </label>`
        : nothing}
    `;
  }

  private renderRow(row: Row, index: number): TemplateResult {
    const extra = this.rowClasses(index);
    switch (row.kind) {
      case 'file': {
        const { file } = row;
        const isViewed = this.viewed.has(file.path);
        return html`<div
          class="file-header ${isViewed ? 'viewed' : ''} ${extra}"
          data-path=${file.path}
        >
          ${this.renderFileHeaderBody(file, isViewed)}
        </div>`;
      }
      case 'hunk': {
        const file = this.files[row.fileIndex];
        // Only where the file can actually be divided: elsewhere the header's
        // box already decides for the whole file, and a second control saying
        // the same thing invites the belief that they differ.
        const pickable = this.selection !== null && !!file && supportsHunkSelection(file);
        return html`<div class="hunk-header ${extra}">
          ${pickable
            ? html`<input
                class="split-check"
                type="checkbox"
                title=${
                  this.selectionVerb === 'squash'
                    ? 'Move this hunk into the destination change'
                    : 'Move this hunk into the split-off change'
                }
                aria-label=${`Include hunk ${row.hunkId} in the ${this.selectionVerb}`}
                .checked=${this.selection!.has(row.hunkId)}
                @change=${(event: Event) =>
                  this.emitSelect([row.hunkId], (event.target as HTMLInputElement).checked)}
              />`
            : nothing}
          <span class="hunk-label">${row.label}</span>
        </div>`;
      }
      case 'expander':
        return html`<button
          class="expander ${extra}"
          title="Show more context"
          @click=${() =>
            this.dispatchEvent(
              new CustomEvent('expand-context', {
                bubbles: true,
                composed: true,
                detail: { path: row.path, hunkId: row.hunkId, direction: row.direction },
              }),
            )}
        >
          <span class="chev">${row.direction === 'up' ? '↑' : '↓'}</span>
          <span class="count"
            >${row.hidden > 0
              ? `${row.hidden} hidden line${row.hidden === 1 ? '' : 's'}`
              : 'more context'}</span
          >
        </button>`;
      case 'notice':
        return html`<div class="notice ${extra}">${row.text}</div>`;
      case 'image':
        return html`<jj-image-view
          .path=${row.path}
          .oldPath=${row.oldPath}
          .revset=${this.revsetForFile(row.fileIndex)}
        ></jj-image-view>`;
      case 'gap':
        return html`<div class="file-gap"></div>`;
      case 'file-end':
        // `data-path` so the pinned bar can measure this edge — see `cardEdge`.
        return html`<div class="file-end" data-path=${this.files[row.fileIndex]?.path ?? ''}></div>`;
      case 'markdown':
        return html`<div class="markdown-preview" .innerHTML=${row.html}></div>`;
      case 'comments':
        return this.renderComments(row.comments, extra);
      case 'unified':
        return this.renderUnified(row.fileIndex, row.line, row.hl, extra);
      case 'split':
        return html`<div class="split-row ${extra}">
          ${this.renderCell(row.fileIndex, row.left, row.hlLeft, 'left')}
          ${this.renderCell(row.fileIndex, row.right, row.hlRight, 'right')}
        </div>`;
    }
  }

  private renderUnified(
    fileIndex: number,
    line: Line,
    hl: HlRef | null,
    extra: string,
  ): TemplateResult {
    const tokens = this.highlights.tokensFor(this.files[fileIndex]!, hl);
    const file = this.files[fileIndex]!;
    // A unified row is one line of code however many numbers precede it, so it
    // has one comment anchor — and it must be the one `buildRows` files
    // existing comments under, or a saved comment renders nowhere: a removed
    // line belongs to the old side, everything else to the new.
    //
    // Both numbers open that one anchor. They used to open their own side
    // each, which put every comment on a context line's *left* number under a
    // key unified never reads back. The composer under the row was separately
    // unreachable on the new side: it was picked with `??` over the two sides,
    // and the miss value is lit's `nothing` — a symbol, not null, so the first
    // call always won and every added line silently swallowed the composer.
    const side: CommentSide = line.kind === 'removed' ? 'old' : 'new';
    const anchor = side === 'old' ? line.oldLine : line.newLine;
    const clickable = this.canComment && anchor !== null;
    const numClick = (e: MouseEvent) => {
      e.stopPropagation();
      if (!clickable) return;
      this.openComposer(file.path, side, anchor!, line.text);
    };
    return html`<div class="line unified ${line.kind} ${markerClass(line.text)} ${extra}">
      <span class="num ${clickable ? 'clickable' : ''}" @click=${numClick}>${line.oldLine ?? ''}</span>
      <span class="num ${clickable ? 'clickable' : ''}" @click=${numClick}>${line.newLine ?? ''}</span>
      <span class="sign">${sign(line.kind)}</span>
      <span class="content"
        ><span class="pan">${renderLineContent(line.text, tokens, line.spans)}</span></span
      >
    </div>${this.renderComposerForLine(file.path, side, anchor)}`;
  }

  private renderCell(
    fileIndex: number,
    line: Line | null,
    hl: HlRef | null,
    side: 'left' | 'right',
  ): TemplateResult {
    if (line === null) {
      return html`<div class="cell ${side} filler"></div>`;
    }
    // In split view a context line shows on both sides; removed only left, added only right.
    const kind = line.kind === 'context' ? 'context' : line.kind;
    const number = side === 'left' ? line.oldLine : line.newLine;
    const tokens = this.highlights.tokensFor(this.files[fileIndex]!, hl);
    const file = this.files[fileIndex]!;
    const commentSide: CommentSide = side === 'left' ? 'old' : 'new';
    const numClick = (e: MouseEvent) => {
      e.stopPropagation();
      if (number === null || !this.canComment) return;
      this.openComposer(file.path, commentSide, number, line.text);
    };
    return html`<div class="cell ${side} ${kind} ${markerClass(line.text)}">
      <span class="num ${this.canComment ? 'clickable' : ''}" @click=${numClick}>${number ?? ''}</span>
      <span class="content"
        ><span class="pan">${renderLineContent(line.text, tokens, line.spans)}</span></span
      >
    </div>${this.renderComposerForLine(file.path, commentSide, number) ?? nothing}`;
  }

  /**
   * Render the composer after a line when it targets that line and there are
   * no existing comments there (existing comments render their own composer
   * slot via the `comments` row). Returns `nothing` when inactive.
   */
  private renderComposerForLine(path: string, side: CommentSide, line: number | null): TemplateResult | typeof nothing {
    if (!this.composer || line === null) return nothing;
    if (this.composer.path !== path || this.composer.side !== side || this.composer.line !== line) {
      return nothing;
    }
    // If there are already comments for this line, the `comments` row handles
    // the composer slot — don't double-render.
    const key = `${path}:${side}:${line}`;
    if (this.comments.has(key)) return nothing;
    return this.renderComposer(null);
  }

  private openComposer(path: string, side: CommentSide, line: number, lineText: string) {
    this.composer = { path, side, line, lineText };
  }

  private closeComposer() {
    this.composer = null;
  }

  private submitComposer(body: string, parentId: number | null) {
    if (!this.composer || !body.trim()) {
      this.composer = null;
      return;
    }
    const c = this.composer;
    this.dispatchEvent(
      new CustomEvent('add-comment', {
        bubbles: true,
        composed: true,
        detail: { ...c, body, parentId },
      }),
    );
    this.composer = null;
  }

  private renderComments(comments: Comment[], extra: string): TemplateResult {
    const top: Comment[] = comments.filter((c) => c.parentId === null);
    return html`<div class="comment-row ${extra}">
      ${top.map((comment) => this.renderCommentThread(comment, comments))}
      ${this.composer && this.commentsMatchActiveRow(comments)
        ? this.renderComposer(null)
        : nothing}
    </div>`;
  }

  /** Whether the active composer targets the same (path, side, line) as this row. */
  private commentsMatchActiveRow(comments: Comment[]): boolean {
    if (!this.composer) return false;
    const key = `${this.composer.path}:${this.composer.side}:${this.composer.line}`;
    return comments.some((c) => `${c.path}:${c.side}:${c.line}` === key);
  }

  private renderCommentThread(top: Comment, all: Comment[]): TemplateResult {
    const replies = all.filter((c) => c.parentId === top.id);
    return html`<div class="comment-thread">
      ${this.renderComment(top)}
      ${replies.map((reply) => html`<div class="comment-reply">${this.renderComment(reply)}</div>`)}
      ${this.composer && this.composer.path === top.path &&
      this.composer.side === top.side &&
      this.composer.line === top.line
        ? this.renderComposer(top.id)
        : html`<button class="comment-reply-btn" @click=${() => this.openComposer(top.path, top.side, top.line, top.lineText)}>
            reply
          </button>`}
    </div>`;
  }

  private renderComment(comment: Comment): TemplateResult {
    return html`<div class="comment ${comment.resolved ? 'resolved' : ''} ${comment.outdated ? 'outdated' : ''}">
      <div class="comment-head">
        <span class="comment-author">${comment.author}</span>
        <span class="comment-time">${relativeTime(comment.createdAt)}</span>
        ${comment.outdated ? html`<span class="comment-badge" title="The anchored line no longer exists">outdated</span>` : nothing}
        <span class="comment-actions">
          <button title="Resolve" @click=${() => this.emitCommentAction('resolve-comment', comment.id, !comment.resolved)}>
            ${comment.resolved ? 'unresolve' : 'resolve'}
          </button>
          <button title="Delete" @click=${() => this.emitCommentAction('delete-comment', comment.id, true)}>delete</button>
        </span>
      </div>
      <div class="comment-body">${comment.body}</div>
    </div>`;
  }

  private renderComposer(parentId: number | null): TemplateResult {
    return html`<div class="comment-composer">
      <textarea
        placeholder="Write a comment…"
        rows="2"
        @keydown=${(e: KeyboardEvent) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            const area = e.target as HTMLTextAreaElement;
            this.submitComposer(area.value, parentId);
          }
          if (e.key === 'Escape') {
            e.preventDefault();
            this.closeComposer();
          }
        }}
      ></textarea>
      <div class="composer-actions">
        <span class="hint">Mod+Enter to post</span>
        <button @click=${() => this.closeComposer()}>Cancel</button>
        <button class="primary" @click=${(e: Event) => {
          const area = (e.currentTarget as HTMLElement)
            .closest('.comment-composer')!
            .querySelector('textarea')!;
          this.submitComposer(area.value, parentId);
        }}>Comment</button>
      </div>
    </div>`;
  }

  private emitCommentAction(name: 'resolve-comment' | 'delete-comment', id: number, value: boolean) {
    this.dispatchEvent(
      new CustomEvent(name, {
        bubbles: true,
        composed: true,
        detail: { id, value },
      }),
    );
  }
}

const sign = (kind: Line['kind']) => (kind === 'added' ? '+' : kind === 'removed' ? '−' : ' ');

const clamp = (value: number, low: number, high: number) =>
  Math.min(high, Math.max(low, value));

/** Must match `tab-size` on .jj-patch, or wide lines mismeasure. */
const TAB_COLUMNS = 8;

/**
 * Width of one line in monospace columns, tabs expanded to the next tab stop.
 *
 * Rows are sized off the widest line so every row in the diff is the same width:
 * they are separate virtualizer children, so left to `max-content` each row would
 * end wherever its own text does and a horizontally scrolled file card would fray
 * into a ragged right edge. Columns (not pixels) because the code font is
 * monospace — `ch` in CSS finishes the conversion, no DOM measurement needed.
 */
function codeColumns(text: string): number {
  let columns = 0;
  for (const char of text) {
    columns += char === '\t' ? TAB_COLUMNS - (columns % TAB_COLUMNS) : 1;
  }
  return columns;
}

function widestLine(rows: Row[]): number {
  let widest = 0;
  for (const row of rows) {
    if (row.kind === 'unified') {
      widest = Math.max(widest, codeColumns(row.line.text));
    } else if (row.kind === 'split') {
      if (row.left) widest = Math.max(widest, codeColumns(row.left.text));
      if (row.right) widest = Math.max(widest, codeColumns(row.right.text));
    }
  }
  return widest;
}

/**
 * jj materialized-conflict markers, and which part of a conflict each opens.
 *
 * jj already names its sides in the marker text — `+++++++ <commit> "<desc>"`
 * and `%%%%%%% diff from: … / to: …`, where git writes a bare `<<<<<<< HEAD` —
 * so the job here is not to invent labels but to stop seven kinds of fence
 * reading as one undifferentiated colour. The fences bound the region, a
 * `+++++++` line introduces a side given verbatim, and `%%%%%%%` with its
 * `\\\\\\\` continuation introduces one given as a diff from the base.
 */
const CONFLICT_MARKERS: { pattern: RegExp; role: string }[] = [
  { pattern: /^<{7}(\s|$)/, role: 'start' },
  { pattern: /^>{7}(\s|$)/, role: 'end' },
  { pattern: /^\+{7}(\s|$)/, role: 'side' },
  { pattern: /^%{7}(\s|$)/, role: 'base' },
  { pattern: /^\\{7}(\s|$)/, role: 'base' },
  // git-style markers: jj does not emit these, but a merge tool run over a jj
  // tree can leave them behind, and they are still a conflict to read.
  { pattern: /^\|{7}(\s|$)/, role: 'base' },
  { pattern: /^={7}(\s|$)/, role: 'side' },
];

const markerClass = (text: string): string => {
  const marker = CONFLICT_MARKERS.find(({ pattern }) => pattern.test(text));
  return marker ? `conflict-marker ${marker.role}` : '';
};

/** Whether a row is the `<<<<<<<` line opening a conflict region. */
function conflictOpener(row: Row): boolean {
  const text =
    row.kind === 'unified'
      ? row.line.text
      : row.kind === 'split'
        ? (row.left?.text ?? row.right?.text ?? null)
        : null;
  return text !== null && /^<{7}(\s|$)/.test(text);
}

function rowMatches(row: Row, query: string): boolean {
  switch (row.kind) {
    case 'file':
      return row.file.path.toLowerCase().includes(query);
    case 'unified':
      return row.line.text.toLowerCase().includes(query);
    case 'split':
      return (
        (row.left?.text.toLowerCase().includes(query) ?? false) ||
        (row.right?.text.toLowerCase().includes(query) ?? false)
      );
    default:
      return false;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-patch-view': PatchView;
  }
  interface HTMLElementEventMap {
    'squash-file': CustomEvent<{ path: string; into: string }>;
    'toggle-viewed': CustomEvent<{ path: string; viewed: boolean }>;
    'search-state': CustomEvent<{ count: number; current: number }>;
    'expand-context': CustomEvent<{ path: string; hunkId: string; direction: 'up' | 'down' }>;
    'visible-file': CustomEvent<{ path: string }>;
    'toggle-markdown': CustomEvent<{ path: string }>;
    'select-units': CustomEvent<{ units: string[]; selected: boolean }>;
    'add-comment': CustomEvent<{ path: string; side: CommentSide; line: number; lineText: string; body: string; parentId: number | null }>;
    'resolve-comment': CustomEvent<{ id: number; value: boolean }>;
    'delete-comment': CustomEvent<{ id: number; value: boolean }>;
  }
}
