import { html, LitElement, nothing, type PropertyValues, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { virtualize, virtualizerRef } from '@lit-labs/virtualizer/virtualize.js';

import { HighlightStore } from './highlight.js';
import type { FilePatch, Line } from './ipc.js';
import { renderLineContent } from './render-line.js';
import { buildRows, type DiffLayout, type HlRef, type Row } from './rows.js';

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
  /** Paths with unresolved conflicts in the shown revision. */
  @property({ attribute: false }) conflicted: ReadonlySet<string> = new Set();
  /** Active walkthrough step's hunk ids; null = show everything. */
  @property({ attribute: false }) hunkFilter: ReadonlySet<string> | null = null;
  /** Find-in-diffs query; null when search is closed. */
  @property() searchQuery: string | null = null;
  /** Wrap long lines instead of scrolling horizontally. */
  @property({ type: Boolean }) wordWrap = false;

  @state() private cursor: number | null = null;
  @state() private searchCurrent = -1;

  private rows: Row[] = [];
  private visibleFile: string | null = null;
  private searchMatches: number[] = [];
  private highlights = new HighlightStore();

  protected override createRenderRoot() {
    return this; // light DOM
  }

  override connectedCallback() {
    super.connectedCallback();
    this.highlights.addEventListener('tokens', this.onTokens);
  }

  override disconnectedCallback() {
    this.highlights.removeEventListener('tokens', this.onTokens);
    super.disconnectedCallback();
  }

  private onTokens = () => this.requestUpdate();

  protected override willUpdate(changed: PropertyValues<this>) {
    const contentChanged =
      changed.has('files') ||
      changed.has('layout') ||
      changed.has('viewed') ||
      changed.has('hunkFilter');
    if (contentChanged) {
      this.rows = buildRows(this.files, this.layout, this.viewed, this.hunkFilter);
      if (this.cursor !== null && this.cursor >= this.rows.length) {
        this.cursor = null;
      }
    }
    if (changed.has('files')) {
      this.highlights.clear();
      for (const file of this.files) {
        this.highlights.request(file);
      }
    }
    if (contentChanged || changed.has('searchQuery')) {
      this.computeSearch(changed.has('searchQuery'));
    }
  }

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

  /** Scroll a file's header to the top of the viewport. */
  scrollToPath(path: string): void {
    const index = this.rows.findIndex((row) => row.kind === 'file' && row.file.path === path);
    if (index >= 0) {
      this.cursor = index;
      this.scrollToRow(index, 'start');
    }
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
    const host = this.querySelector('.jj-patch') as
      | (HTMLElement & { [virtualizerRef]?: { element(i: number): { scrollIntoView(o?: object): void } | undefined } })
      | null;
    host?.[virtualizerRef]?.element(index)?.scrollIntoView({ block });
  }

  /** Report which file the viewport is currently inside (sticky breadcrumb). */
  private onVisibilityChanged = (event: Event) => {
    const first = (event as CustomEvent<{ first: number }>).detail?.first ?? 0;
    // Walk back to the file header owning the topmost visible row.
    for (let index = Math.min(first, this.rows.length - 1); index >= 0; index--) {
      const row = this.rows[index];
      if (row?.kind === 'file') {
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
  };

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
        <div class="glyph">✓</div>
        <div class="title">Nothing to review</div>
        <div class="hint">Changes appear here live as files are edited or a revision is selected.</div>
      </div>`;
    }
    // The virtualize() DIRECTIVE, not the <lit-virtualizer> element: the element renders rows
    // into its shadow root, which would cut them off from theme.css and break cross-row text
    // selection — the whole reason this component is light DOM.
    return html`<div
      class="jj-patch ${this.layout} ${this.wordWrap ? 'wrap' : 'nowrap'}"
      @visibilityChanged=${this.onVisibilityChanged}
    >
      ${virtualize({
        items: this.rows,
        renderItem: (row: Row, index: number) => this.renderRow(row, index) as TemplateResult,
        scroller: true,
      })}
    </div>`;
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
          <span class="file-status ${file.status}">${file.status}</span>
          <span class="file-path"
            >${file.oldPath ? html`${file.oldPath} → ` : nothing}${file.path}</span
          >
          <span class="file-counts"
            >${file.added ? html`<span class="plus">+${file.added}</span>` : nothing}
            ${file.removed ? html`<span class="minus">−${file.removed}</span>` : nothing}</span
          >
          ${this.conflicted.has(file.path)
            ? html`<span class="conflict-badge">conflict</span>`
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
                  (target) =>
                    html`<option value=${target.changeId}>${target.label}</option>`,
                )}
              </select>`
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
        </div>`;
      }
      case 'hunk':
        return html`<div class="hunk-header ${extra}">${row.label}</div>`;
      case 'notice':
        return html`<div class="notice ${extra}">${row.text}</div>`;
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
    return html`<div class="line unified ${line.kind} ${markerClass(line.text)} ${extra}">
      <span class="num">${line.oldLine ?? ''}</span>
      <span class="num">${line.newLine ?? ''}</span>
      <span class="sign">${sign(line.kind)}</span>
      <span class="content">${renderLineContent(line.text, tokens, line.spans)}</span>
    </div>`;
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
    return html`<div class="cell ${side} ${kind} ${markerClass(line.text)}">
      <span class="num">${number ?? ''}</span>
      <span class="content">${renderLineContent(line.text, tokens, line.spans)}</span>
    </div>`;
  }
}

const sign = (kind: Line['kind']) => (kind === 'added' ? '+' : kind === 'removed' ? '−' : ' ');

/** jj materialized-conflict markers: `<<<<<<<`, `%%%%%%%`, `+++++++`, `|||||||`, `=======`, `>>>>>>>`, `\\\\\\\`. */
const CONFLICT_MARKER = /^(<{7}|>{7}|={7}|\|{7}|%{7}|\+{7}|\\{7})(\s|$)/;

const markerClass = (text: string) => (CONFLICT_MARKER.test(text) ? 'conflict-marker' : '');

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
  }
}
