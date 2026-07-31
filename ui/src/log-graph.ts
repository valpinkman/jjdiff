import { css, html, LitElement, nothing, svg, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

import { layoutGraph, type GraphRow } from './graph.js';
import type { BookmarkStatus, Change } from './ipc.js';
import { relativeTime } from './time.js';
import { unpushedAndUnnamed, worstTracking } from './tracking.js';

const LANE_W = 12;
/* Taller than the text needs. A commit list is scanned, not read, and rows this
   dense read as a table dump; the extra 6px is what lets each row register as a
   separate thing at a glance. */
const ROW_H = 36;
const DOT_R = 3.5;
/**
 * Left/right margin inside the graph SVG, in px.
 *
 * An `<svg>` clips to its own box, and the widest thing drawn on lane 0 is the
 * working copy's halo: `DOT_R * 2.4` at rest and 1.18× that at the top of its
 * breath, so ~9.9px of radius. At the old inset of 2 the lane centre sat at 8
 * and the halo lost its left edge to the viewport — a circle with a flat side.
 * The inset has to clear the largest *animated* radius, not the resting one.
 */
const RAIL_INSET = 11;

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
      position: relative;
      transition:
        background-color var(--jj-t-2, 180ms) var(--jj-ease-out, ease),
        color var(--jj-t-2, 180ms) var(--jj-ease-out, ease);
      display: flex;
      align-items: center;
      gap: 7px;
      width: 100%;
      height: ${ROW_H}px;
      border: 0;
      /* Square and full-bleed. A rounded row inside a padded list reads as a
         card in a stack of cards; a full-width band reads as a row in a list,
         which is what it is. */
      border-radius: 0;
      background: none;
      color: var(--jj-fg);
      font: inherit;
      text-align: left;
      /* No left padding: the graph SVG carries its own RAIL_INSET, which is
         what keeps the rail clear of the 2px selection bar *and* gives the
         working copy's halo room to breathe without being clipped. Padding here
         as well would just push the graph twice as far from the edge. */
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
    /* The same cursor bar as the command palette: selection in this app is one
       shape wherever it appears, a soft fill with a bar on its leading edge.
       Flush to the pane's left edge and the full height of the row — a cursor
       on the list, not a pill floating inside it. */
    .row.selected::before {
      content: '';
      position: absolute;
      left: 0;
      top: 0;
      bottom: 0;
      width: 2px;
      background: var(--jj-accent);
      animation: row-cursor var(--jj-t-2, 180ms) var(--jj-ease-out, ease-out);
    }
    @keyframes row-cursor {
      from {
        transform: scaleY(0.2);
        opacity: 0;
      }
    }
    svg {
      flex: none;
      display: block;
    }
    /* Rails and mutable dots take their lane's hue (set per element); everything else
       keeps its meaning — immutable recedes, the working copy is the accent, conflicts
       are red. Slightly transparent so a busy graph tints rather than shouts. */
    .rail {
      stroke: var(--jj-fg-faint);
      stroke-width: 1.4;
      stroke-opacity: 0.85;
      fill: none;
      stroke-linecap: round;
    }
    /* Immutable history keeps its lane's hue but steps back, so colour never undoes the
       point of dimming it. */
    .row.immutable .rail {
      stroke-opacity: 0.4;
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
    /* Halo under the working-copy dot: "you are here", visible without a label.
       It breathes, slowly — the working copy is the one row in the graph that is
       still moving, and a stationary marker on it says the opposite. Slow enough
       (4s) to be noticed only when the eye rests there, which is the point:
       peripheral vision reads it as alive without it ever asking for attention. */
    .dot-halo {
      fill: var(--jj-accent);
      opacity: 0.18;
      transform-origin: center;
      transform-box: fill-box;
      animation: halo 4s ease-in-out infinite;
    }
    @keyframes halo {
      50% {
        opacity: 0.3;
        transform: scale(1.18);
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .dot-halo {
        animation: none;
      }
      .row.selected::before {
        animation: none;
      }
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
      border-radius: var(--jj-r-pill, 999px);
      background: var(--jj-ref-soft);
      color: var(--jj-ref);
      padding: 2px 8px;
      line-height: 15px;
    }
    .tag.warn {
      background: var(--jj-removed-bg);
      color: var(--jj-removed-fg);
    }
    /* Inside the bookmark's own pill rather than beside it: the count belongs to
       that ref, and a second free-standing tag would compete with the name for
       the width the description needs. Held off the name by a hairline for the
       same reason the tool-group draws its own seams. */
    .tag .ahead {
      margin-left: 5px;
      padding-left: 5px;
      border-left: 1px solid color-mix(in srgb, var(--jj-ref) 35%, transparent);
      font-family: var(--jj-mono);
    }
    /* Not the bookmark colour: this is the absence of a ref, not one. Neutral,
       like the workspace tag, so a row of them does not read as a row of things
       that have names. */
    .tag.unpushed {
      background: var(--jj-surface-2);
      color: var(--jj-fg-muted);
      font-family: var(--jj-mono);
      padding: 2px 6px;
    }
    /* A workspace is a place, not a ref — neutral rather than the bookmark colour, so the
       eye does not read a name@ tag as something publishable. */
    .tag.workspace {
      background: var(--jj-surface-2);
      color: var(--jj-fg-muted);
      font-family: var(--jj-mono);
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
    /* Dragging: the row being moved recedes, the row under the pointer shows
       where it would land. A line rather than a fill, because the destination
       becomes the *parent* — the change lands under it, and a band across the
       row would say it lands on it. */
    /* WebKit does not start a drag from a button element on the draggable
       attribute alone, and every row here is one. Scoped to the attribute so
       turning dragging off actually turns it off. */
    .row[draggable='true'] {
      -webkit-user-drag: element;
    }
    .row.dragging {
      opacity: 0.4;
    }
    .row.drop-target {
      background: var(--jj-accent-soft);
      box-shadow: inset 0 -2px 0 var(--jj-accent);
    }
    /* While a drag is in flight, every row that cannot take it says so — a
       destination that silently does nothing on drop is worse than one that
       looks unavailable. */
    .row.barred {
      opacity: 0.35;
      cursor: no-drop;
    }
  `;

  @property({ attribute: false }) changes: Change[] = [];
  @property() selected: string | null = null;
  /** The workspace this window is showing — the one whose `name@` tag is left off. */
  @property() workspace: string | null = null;
  /** Tracking state per bookmark, for the `↑n` on a tag. Empty without remotes. */
  @property({ attribute: false }) bookmarks: readonly BookmarkStatus[] = [];
  /**
   * Change ids that are on no remote. The graph is the pane you *scan*, so this
   * is where "none of this is published" has to be legible without selecting
   * anything — the detail card can only ever answer for one change at a time.
   */
  @property({ attribute: false }) unpushed: ReadonlySet<string> = new Set();
  /**
   * Whether rows can be dragged onto each other to rebase. Off while the graph
   * is showing something a rebase makes no sense against.
   */
  @property({ type: Boolean }) canRebase = false;

  /** The change id being dragged; null when no drag is in flight. */
  @state() private draggingId: string | null = null;
  /** The change id currently under the pointer, when it can accept the drop. */
  @state() private overId: string | null = null;

  private pick(change: Change) {
    this.dispatchEvent(
      new CustomEvent<Change>('change-selected', { detail: change, bubbles: true, composed: true }),
    );
  }

  /**
   * Right-click a row: the verbs for that change, at the pointer.
   *
   * Escapes this shadow root like the file tree's does, because the sidebar is a
   * scroll container and a menu positioned inside it is clipped by it.
   */
  private openMenu(event: MouseEvent, change: Change) {
    event.preventDefault();
    this.dispatchEvent(
      new CustomEvent<ChangeMenuRequest>('change-menu', {
        detail: { change, x: event.clientX, y: event.clientY },
        bubbles: true,
        composed: true,
      }),
    );
  }

  /**
   * Commit ids the dragged change cannot land on: itself and its descendants.
   *
   * The same exclusion the rebase picker makes, for the same reason — a commit
   * cannot be its own ancestor — and found the same way, by walking the log
   * backwards. jj's log order is reverse-topological, children before parents,
   * so one pass backwards visits every parent before its children and reaches
   * the whole descendant set.
   */
  private get barred(): Set<string> {
    const source = this.changes.find((change) => change.changeId === this.draggingId);
    if (!source) return new Set();
    const barred = new Set<string>([source.commitId]);
    for (let index = this.changes.length - 1; index >= 0; index--) {
      const change = this.changes[index]!;
      if (change.parents.some((parent) => barred.has(parent))) barred.add(change.commitId);
    }
    return barred;
  }

  private onDragStart(event: DragEvent, change: Change) {
    if (!this.canRebase) return;
    this.draggingId = change.changeId;
    this.overId = null;
    // WebKit will not start a drag at all without data on the transfer, and the
    // id is the honest thing to carry: a jjdiff row dropped into a text field
    // should paste the change it names.
    event.dataTransfer?.setData('text/plain', change.changeId);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
  }

  private onDragEnd = () => {
    this.draggingId = null;
    this.overId = null;
  };

  private onDragOver(event: DragEvent, change: Change) {
    if (!this.draggingId || this.barred.has(change.commitId)) return;
    // Without this the drop never fires: the default action for dragover is to
    // refuse the drop.
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
    this.overId = change.changeId;
  }

  private onDrop(event: DragEvent, destination: Change) {
    event.preventDefault();
    const source = this.changes.find((change) => change.changeId === this.draggingId);
    // Read the exclusions before clearing the drag: they are derived from it.
    const barred = this.barred;
    this.draggingId = null;
    this.overId = null;
    if (!source || source.changeId === destination.changeId) return;
    if (barred.has(destination.commitId)) return;
    this.dispatchEvent(
      new CustomEvent('rebase-drop', {
        detail: { source, destination },
        bubbles: true,
        composed: true,
      }),
    );
  }

  protected override render() {
    const rows = layoutGraph(this.changes);
    const maxWidth = Math.max(1, ...rows.map((row) => row.width));
    const gutter = maxWidth * LANE_W + RAIL_INSET * 2;
    return html`${rows.map((row) => this.renderRow(row, gutter))}`;
  }

  private renderRow(row: GraphRow, gutter: number): TemplateResult {
    const { change } = row;
    const x = (lane: number) => lane * LANE_W + LANE_W / 2 + RAIL_INSET;
    const mid = ROW_H / 2;
    const cx = x(row.lane);

    // A rail wears the colour of the lane it belongs to — a merge curve is painted by
    // the lane merging in, a fork by the lane branching out, so a branch keeps one
    // colour from the row it leaves to the row it rejoins.
    const rails = [];
    for (const lane of row.through) {
      rails.push(
        svg`<line class="rail" style=${laneStroke(lane)} x1=${x(lane)} y1="0" x2=${x(lane)} y2=${ROW_H} />`,
      );
    }
    rails.push(svg`<line class="rail" style=${laneStroke(row.lane)} x1=${cx} y1="0" x2=${cx} y2=${mid} />`);
    if (row.continues) {
      rails.push(
        svg`<line class="rail" style=${laneStroke(row.lane)} x1=${cx} y1=${mid} x2=${cx} y2=${ROW_H} />`,
      );
    }
    for (const lane of row.joins) {
      rails.push(
        svg`<path class="rail" style=${laneStroke(lane)} d="M ${x(lane)} 0 C ${x(lane)} ${mid * 0.7}, ${cx} ${mid * 0.4}, ${cx} ${mid}" />`,
      );
    }
    for (const lane of row.forks) {
      rails.push(
        svg`<path class="rail" style=${laneStroke(lane)} d="M ${cx} ${mid} C ${cx} ${mid + mid * 0.6}, ${x(lane)} ${mid + mid * 0.3}, ${x(lane)} ${ROW_H}" />`,
      );
    }

    const dotClass = change.workingCopy
      ? 'dot-wc'
      : change.conflict
        ? 'dot-conflict'
        : change.immutable
          ? 'dot-immutable'
          : 'dot-mutable';
    const dragging = this.draggingId !== null;
    const barred = dragging && this.barred.has(change.commitId);
    const rowClass = [
      'row',
      change.changeId === this.selected ? 'selected' : '',
      change.workingCopy ? 'wc' : '',
      change.immutable ? 'immutable' : '',
      change.changeId === this.draggingId ? 'dragging' : '',
      change.changeId === this.overId ? 'drop-target' : '',
      barred && change.changeId !== this.draggingId ? 'barred' : '',
    ].join(' ');
    const summary = change.description.split('\n')[0] ?? '';

    return html`
      <button
        class=${rowClass}
        draggable=${this.canRebase ? 'true' : 'false'}
        title=${
          dragging
            ? barred
              ? `${change.changeId.slice(0, 12)} — cannot hold this change; it is descended from it`
              : `Drop here to rebase onto ${change.changeId.slice(0, 12)}`
            : `${change.changeId.slice(0, 12)} — ${summary || '(no description)'}${
                this.canRebase ? '\nDrag onto another change to rebase it there.' : ''
              }`
        }
        @click=${() => this.pick(change)}
        @contextmenu=${(event: MouseEvent) => this.openMenu(event, change)}
        @dragstart=${(event: DragEvent) => this.onDragStart(event, change)}
        @dragend=${this.onDragEnd}
        @dragover=${(event: DragEvent) => this.onDragOver(event, change)}
        @dragleave=${() => {
          if (this.overId === change.changeId) this.overId = null;
        }}
        @drop=${(event: DragEvent) => this.onDrop(event, change)}
      >
        <svg width=${gutter} height=${ROW_H}>
          ${rails}
          ${change.workingCopy
            ? svg`<circle class="dot-halo" cx=${cx} cy=${mid} r=${DOT_R * 2.4} />`
            : nothing}
          <circle
            class=${dotClass}
            style=${dotClass === 'dot-mutable' ? laneStroke(row.lane) : nothing}
            cx=${cx}
            cy=${mid}
            r=${DOT_R}
          />
        </svg>
        ${change.bookmarks.map((bookmark) => {
          const status = worstTracking(this.bookmarks, bookmark);
          return html`<span
            class="tag"
            title=${
              status?.ahead
                ? `${bookmark} is ${status.ahead} commit${
                    status.ahead === 1 ? '' : 's'
                  } ahead of ${status.remote} — a push would send ${
                    status.ahead === 1 ? 'it' : 'them'
                  }`
                : bookmark
            }
            >${bookmark}${
              // Only ahead. `behind` is on the detail card, because it is not a
              // property of *this* row: the commits the remote has and we do not
              // are somewhere else entirely, and a `↓` here points at nothing on
              // screen.
              status?.ahead ? html`<span class="ahead">↑${status.ahead}</span>` : nothing
            }</span
          >`;
        })}
        ${
          // Unpushed work with no bookmark to hang a count on. Deliberately not a
          // number: there is no remote ref to be ahead *of*, so the honest claim
          // is the binary one — this exists here and nowhere else.
          unpushedAndUnnamed(change, this.unpushed, this.bookmarks)
            ? html`<span
                class="tag unpushed"
                title="Not on any remote — this change has never been pushed, and has no bookmark to push it under"
                >↑</span
              >`
            : nothing
        }
        ${
          // `name@`, as jj writes it in its own log — for every workspace holding this
          // commit *except* this window's own. That one is already marked by the dot and
          // its halo, and labelling it too would put a tag on one row of every repo,
          // including the overwhelming majority that have a single workspace.
          change.workspaces
            .filter((name) => name !== this.workspace)
            .map(
              (name) => html`<span class="tag workspace" title="Checked out in the ${name} workspace"
                >${name}@</span
              >`,
            )
        }
        ${change.conflict ? html`<span class="tag warn">×</span>` : nothing}
        <span class="desc ${summary ? '' : 'empty-desc'}">
          ${summary || (change.workingCopy ? 'working copy' : '(no description)')}
        </span>
        <span class="when">${relativeTime(change.committer.timestamp)}</span>
      </button>
    `;
  }
}

/** How many lane hues theme.css defines; lanes past it wrap around. */
const LANE_COLOURS = 6;

const laneStroke = (lane: number) => `stroke: var(--jj-lane-${lane % LANE_COLOURS})`;

/** A right-click on a graph row, and where to put the menu. */
export interface ChangeMenuRequest {
  change: Change;
  /** Viewport coordinates to anchor the menu at. */
  x: number;
  y: number;
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-log-graph': LogGraph;
  }
  interface HTMLElementEventMap {
    'rebase-drop': CustomEvent<{ source: Change; destination: Change }>;
    'change-menu': CustomEvent<ChangeMenuRequest>;
  }
}
