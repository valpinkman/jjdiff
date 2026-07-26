import { css, html, LitElement, nothing, svg, type TemplateResult } from 'lit';
import { customElement, property } from 'lit/decorators.js';

import { layoutGraph, type GraphRow } from './graph.js';
import type { Change } from './ipc.js';

const LANE_W = 12;
const ROW_H = 30;
const DOT_R = 3.5;

/**
 * jj-log-style commit graph. Message-first rows: change ids stay out of the list (the
 * context card shows them once selected), immutable history is dimmed so active work
 * pops, and the rails — not row borders — carry the vertical structure.
 * Dot glyphs follow jj's vocabulary: @ = working copy (signal), ◆ = immutable (filled),
 * ○ = mutable (hollow).
 */
@customElement('jj-log-graph')
export class LogGraph extends LitElement {
  static override styles = css`
    :host {
      display: block;
      font-size: 12.5px;
      user-select: none;
      padding: 4px 0;
    }
    .row {
      transition: background-color 0.13s ease;
      display: flex;
      align-items: center;
      gap: 7px;
      width: 100%;
      height: ${ROW_H}px;
      border: 0;
      border-radius: var(--jj-r-sm, 7px);
      background: none;
      color: var(--jj-fg);
      font: inherit;
      text-align: left;
      padding: 0 10px 0 0;
      cursor: pointer;
      box-sizing: border-box;
    }
    .row:hover {
      background: var(--jj-wash);
    }
    .row:focus-visible {
      outline: 2px solid var(--jj-accent);
      outline-offset: -2px;
    }
    .row.selected {
      background: var(--jj-accent-soft);
    }
    svg {
      flex: none;
      display: block;
    }
    .rail {
      stroke: var(--jj-fg-faint);
      stroke-width: 1.4;
      fill: none;
      stroke-linecap: round;
    }
    .dot-mutable {
      fill: var(--jj-bg-panel);
      stroke: var(--jj-fg-muted);
      stroke-width: 1.6;
    }
    .dot-immutable {
      fill: var(--jj-fg-faint);
      stroke: none;
    }
    .dot-wc {
      fill: var(--jj-accent);
      stroke: var(--jj-bg-panel);
      stroke-width: 2.5;
    }
    .dot-conflict {
      fill: var(--jj-removed-fg);
      stroke: var(--jj-bg-panel);
      stroke-width: 2;
    }
    .tag {
      flex: none;
      font-family: var(--jj-sans);
      font-size: 10px;
      font-weight: 600;
      border-radius: 999px;
      background: var(--jj-ref-soft);
      color: var(--jj-ref);
      padding: 1px 7px;
      line-height: 15px;
    }
    .tag.warn {
      background: var(--jj-removed-bg);
      color: var(--jj-removed-fg);
    }
    .desc {
      flex: 1;
      min-width: 0;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .row.wc .desc {
      font-weight: 600;
    }
    .row.immutable .desc {
      color: var(--jj-fg-muted);
    }
    .row.selected .desc {
      color: var(--jj-fg);
    }
    .desc.empty-desc {
      color: var(--jj-fg-muted);
      font-style: italic;
    }
    .when {
      flex: none;
      font-size: 10.5px;
      color: var(--jj-fg-faint);
    }
  `;

  @property({ attribute: false }) changes: Change[] = [];
  @property() selected: string | null = null;

  private pick(change: Change) {
    this.dispatchEvent(
      new CustomEvent<Change>('change-selected', { detail: change, bubbles: true, composed: true }),
    );
  }

  protected override render() {
    const rows = layoutGraph(this.changes);
    const maxWidth = Math.max(1, ...rows.map((row) => row.width));
    const gutter = maxWidth * LANE_W + 2;
    return html`${rows.map((row) => this.renderRow(row, gutter))}`;
  }

  private renderRow(row: GraphRow, gutter: number): TemplateResult {
    const { change } = row;
    const x = (lane: number) => lane * LANE_W + LANE_W / 2 + 2;
    const mid = ROW_H / 2;
    const cx = x(row.lane);

    const rails = [];
    for (const lane of row.through) {
      rails.push(svg`<line class="rail" x1=${x(lane)} y1="0" x2=${x(lane)} y2=${ROW_H} />`);
    }
    rails.push(svg`<line class="rail" x1=${cx} y1="0" x2=${cx} y2=${mid} />`);
    if (row.continues) {
      rails.push(svg`<line class="rail" x1=${cx} y1=${mid} x2=${cx} y2=${ROW_H} />`);
    }
    for (const lane of row.joins) {
      rails.push(
        svg`<path class="rail" d="M ${x(lane)} 0 C ${x(lane)} ${mid * 0.7}, ${cx} ${mid * 0.4}, ${cx} ${mid}" />`,
      );
    }
    for (const lane of row.forks) {
      rails.push(
        svg`<path class="rail" d="M ${cx} ${mid} C ${cx} ${mid + mid * 0.6}, ${x(lane)} ${mid + mid * 0.3}, ${x(lane)} ${ROW_H}" />`,
      );
    }

    const dotClass = change.workingCopy
      ? 'dot-wc'
      : change.conflict
        ? 'dot-conflict'
        : change.immutable
          ? 'dot-immutable'
          : 'dot-mutable';
    const rowClass = [
      'row',
      change.changeId === this.selected ? 'selected' : '',
      change.workingCopy ? 'wc' : '',
      change.immutable ? 'immutable' : '',
    ].join(' ');
    const summary = change.description.split('\n')[0] ?? '';

    return html`
      <button
        class=${rowClass}
        title="${change.changeId.slice(0, 12)} — ${summary || '(no description)'}"
        @click=${() => this.pick(change)}
      >
        <svg width=${gutter} height=${ROW_H}>
          ${rails}
          <circle class=${dotClass} cx=${cx} cy=${mid} r=${DOT_R} />
        </svg>
        ${change.bookmarks.map((bookmark) => html`<span class="tag">${bookmark}</span>`)}
        ${change.conflict ? html`<span class="tag warn">×</span>` : nothing}
        <span class="desc ${summary ? '' : 'empty-desc'}">
          ${summary || (change.workingCopy ? 'working copy' : '(no description)')}
        </span>
        <span class="when">${relativeTime(change.committer.timestamp)}</span>
      </button>
    `;
  }
}

/** Compact relative age: now, 5m, 3h, 2d, 4w, 7mo. */
function relativeTime(timestamp: string): string {
  const then = Date.parse(timestamp);
  if (Number.isNaN(then)) return '';
  const seconds = Math.max(0, (Date.now() - then) / 1000);
  if (seconds < 60) return 'now';
  const minutes = seconds / 60;
  if (minutes < 60) return `${Math.floor(minutes)}m`;
  const hours = minutes / 60;
  if (hours < 24) return `${Math.floor(hours)}h`;
  const days = hours / 24;
  if (days < 7) return `${Math.floor(days)}d`;
  const weeks = days / 7;
  if (weeks < 9) return `${Math.floor(weeks)}w`;
  return `${Math.floor(days / 30)}mo`;
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-log-graph': LogGraph;
  }
}
