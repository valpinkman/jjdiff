import { html, LitElement, nothing, type TemplateResult } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import { virtualize } from '@lit-labs/virtualizer/virtualize.js';

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

  protected override willUpdate(changed: Map<string, unknown>) {
    if (changed.has('files')) {
      this.highlights.clear();
      for (const file of this.files) {
        this.highlights.request(file);
      }
    }
  }

  protected override render() {
    if (this.files.length === 0) {
      return html`<div class="jj-empty">
        <div class="glyph">✓</div>
        <div class="title">Nothing to review</div>
        <div class="hint">Changes appear here live as files are edited or a revision is selected.</div>
      </div>`;
    }
    const rows = buildRows(this.files, this.layout, this.viewed, this.hunkFilter);
    // The virtualize() DIRECTIVE, not the <lit-virtualizer> element: the element renders rows
    // into its shadow root, which would cut them off from theme.css and break cross-row text
    // selection — the whole reason this component is light DOM.
    return html`<div class="jj-patch ${this.layout}">
      ${virtualize({
        items: rows,
        renderItem: (row: Row) => this.renderRow(row) as TemplateResult,
        scroller: true,
      })}
    </div>`;
  }

  private renderRow(row: Row): TemplateResult {
    switch (row.kind) {
      case 'file': {
        const { file } = row;
        const isViewed = this.viewed.has(file.path);
        return html`<div class="file-header ${isViewed ? 'viewed' : ''}" data-path=${file.path}>
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
        return html`<div class="hunk-header">${row.label}</div>`;
      case 'notice':
        return html`<div class="notice">${row.text}</div>`;
      case 'unified':
        return this.renderUnified(row.fileIndex, row.line, row.hl);
      case 'split':
        return html`<div class="split-row">
          ${this.renderCell(row.fileIndex, row.left, row.hlLeft, 'left')}
          ${this.renderCell(row.fileIndex, row.right, row.hlRight, 'right')}
        </div>`;
    }
  }

  private renderUnified(fileIndex: number, line: Line, hl: HlRef | null): TemplateResult {
    const tokens = this.highlights.tokensFor(this.files[fileIndex]!, hl);
    return html`<div class="line unified ${line.kind} ${markerClass(line.text)}">
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

declare global {
  interface HTMLElementEventMap {
    'squash-file': CustomEvent<{ path: string; into: string }>;
    'toggle-viewed': CustomEvent<{ path: string; viewed: boolean }>;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-patch-view': PatchView;
  }
}
