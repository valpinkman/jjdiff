import { html, LitElement, nothing, type TemplateResult } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import '@lit-labs/virtualizer';

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
      return html`<p class="jj-empty">No changes.</p>`;
    }
    const rows = buildRows(this.files, this.layout);
    return html`<lit-virtualizer
      class="jj-patch ${this.layout}"
      scroller
      .items=${rows}
      .renderItem=${(row: Row) => this.renderRow(row) as TemplateResult}
    ></lit-virtualizer>`;
  }

  private renderRow(row: Row): TemplateResult {
    switch (row.kind) {
      case 'file': {
        const { file } = row;
        return html`<div class="file-header" data-path=${file.path}>
          <span class="file-status ${file.status}">${file.status}</span>
          <span class="file-path"
            >${file.oldPath ? html`${file.oldPath} → ` : nothing}${file.path}</span
          >
          <span class="file-counts"
            >${file.added ? html`<span class="plus">+${file.added}</span>` : nothing}
            ${file.removed ? html`<span class="minus">−${file.removed}</span>` : nothing}</span
          >
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
    return html`<div class="line unified ${line.kind}">
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
    return html`<div class="cell ${side} ${kind}">
      <span class="num">${number ?? ''}</span>
      <span class="content">${renderLineContent(line.text, tokens, line.spans)}</span>
    </div>`;
  }
}

const sign = (kind: Line['kind']) => (kind === 'added' ? '+' : kind === 'removed' ? '−' : ' ');

declare global {
  interface HTMLElementTagNameMap {
    'jj-patch-view': PatchView;
  }
}
