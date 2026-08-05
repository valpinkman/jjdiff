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
      display: block;
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
      padding: 0;
      cursor: pointer;
      box-sizing: border-box;
      overflow: hidden;
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
      position: absolute;
      inset: 0 auto auto 0;
      display: block;
      pointer-events: none;
    }
    .row-content {
      position: absolute;
      top: 0;
      right: 10px;
      bottom: 0;
      left: var(--content-left);
      display: flex;
      align-items: center;
      gap: 7px;
      min-width: 0;
      box-sizing: border-box;
      padding-left: 4px;
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
    /* Which side of a divergent change this row is, in the same shape: the
       commit id is the only thing distinguishing two rows that otherwise carry
       the same id, the same description and the same badge. Off currentColor
       rather than the bookmark hue, since this pill is a warning, not a ref. */
    .tag .which {
      margin-left: 5px;
      padding-left: 5px;
      border-left: 1px solid color-mix(in srgb, currentColor 35%, transparent);
      font-family: var(--jj-mono);
      font-weight: 500;
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
    /* A bookmark is a thing you can pick up, and grab is the cursor that says
       so. Only on hover: shown always it would claim the whole row is draggable
       when the row's own gesture is a different one.

       No draggable attribute on the chip. The row is the drag source for both
       gestures — a chip that was its own source never received dragstart,
       because a draggable ancestor wins — and the chip is identified by
       data-bookmark instead, read on pointerdown. */
    .tag[data-bookmark]:hover {
      cursor: grab;
      border-color: var(--jj-accent);
    }
    /* The tag being moved recedes where it came from, the way a dragged row
       does — so the graph shows the name leaving one change for another rather
       than appearing to be in both. */
    .tag.moving {
      opacity: 0.35;
    }
  `;

  @property({ attribute: false }) changes: Change[] = [];
  /**
   * The selected row, by **commit** id.
   *
   * A change id here highlighted both sides of a divergent change at once and
   * gave the pane no way to tell which had been clicked. The app resolves its
   * selection before handing it over, so this is always a commit currently in
   * `changes` — one row, or none.
   */
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
  /**
   * The bookmark being dragged off its change, when the drag started on a tag
   * rather than on the row.
   *
   * A separate piece of state, not a mode on `draggingId`, because the two
   * gestures differ in what they mean and in what can accept them: a rebase
   * moves the change and is barred from its own descendants, a bookmark move
   * moves only a name and can land anywhere except where it already is. They
   * are told apart at `dragstart`, which is the only moment the distinction is
   * available — by `drop` both look like a row under a pointer.
   */
  @state() private draggingBookmark: { name: string; from: Change } | null = null;

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
    const source = this.changes.find((change) => change.commitId === this.draggingId);
    if (!source) return new Set();
    const barred = new Set<string>([source.commitId]);
    for (let index = this.changes.length - 1; index >= 0; index--) {
      const change = this.changes[index]!;
      if (change.parents.some((parent) => barred.has(parent))) barred.add(change.commitId);
    }
    return barred;
  }

  private onDragStart(event: DragEvent, change: Change) {
    // A press that landed on a chip is a bookmark move, whatever element the
    // browser then chose as the drag source. Unconditional, unlike the rebase
    // below: `canRebase` is off when the graph shows something a rebase makes
    // no sense against — a proposal's commits, say — and a bookmark still means
    // the same thing there.
    const pending = this.pendingBookmark;
    if (pending && pending.from.commitId === change.commitId) {
      this.draggingBookmark = pending;
      this.draggingId = null;
      this.overId = null;
      event.dataTransfer?.setData('text/plain', pending.name);
      if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
      return;
    }
    if (!this.canRebase) return;
    this.draggingId = change.commitId;
    this.overId = null;
    // WebKit will not start a drag at all without data on the transfer, and the
    // id is the honest thing to carry: a jjdiff row dropped into a text field
    // should paste the change it names.
    event.dataTransfer?.setData('text/plain', change.changeId);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
  }

  /**
   * Which bookmark the pointer went down on, if any — read by the *row's*
   * `dragstart` to tell a bookmark move from a rebase.
   *
   * The two gestures start on different elements and cannot be told apart by
   * the drag event alone. A chip carrying its own `dragstart` was the obvious
   * shape and does not work: the chip sits inside a row that is itself
   * draggable, WebKit resolves nested draggables to the *ancestor*, so the
   * chip's handler never ran. The drag started (the browser drags a draggable
   * ancestor regardless) and nothing could accept it, because the state that
   * makes `dragover` call `preventDefault` was never set. A drag you can begin
   * and cannot finish, with no error anywhere.
   *
   * `pointerdown` runs before any of that and reports the element actually
   * under the finger, so the row's one handler can ask what was grabbed. Plain
   * field, not `@state`: nothing renders from it, and a re-render between
   * pointerdown and dragstart is exactly what we do not want.
   */
  private pendingBookmark: { name: string; from: Change } | null = null;

  /** Record whether this press landed on a bookmark chip or on the row itself. */
  private onPointerDown(event: PointerEvent, change: Change) {
    const chip = (event.target as HTMLElement | null)?.closest<HTMLElement>('.tag[data-bookmark]');
    const name = chip?.dataset.bookmark;
    this.pendingBookmark = name ? { name, from: change } : null;
  }

  private onDragEnd = () => {
    this.draggingId = null;
    this.draggingBookmark = null;
    this.pendingBookmark = null;
    this.overId = null;
  };

  /** Rows this drag cannot land on, whichever drag it is. */
  private refuses(change: Change): boolean {
    // A bookmark can go anywhere but where it already is — moving a name is not
    // constrained by ancestry the way moving a commit is.
    if (this.draggingBookmark) return this.draggingBookmark.from.commitId === change.commitId;
    return this.barred.has(change.commitId);
  }

  private onDragOver(event: DragEvent, change: Change) {
    if (!this.draggingId && !this.draggingBookmark) return;
    if (this.refuses(change)) return;
    // Without this the drop never fires: the default action for dragover is to
    // refuse the drop.
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
    this.overId = change.commitId;
  }

  private onDrop(event: DragEvent, destination: Change) {
    event.preventDefault();
    const source = this.changes.find((change) => change.commitId === this.draggingId);
    const bookmark = this.draggingBookmark;
    // Read the exclusions before clearing the drag: they are derived from it.
    const refused = this.refuses(destination);
    this.draggingId = null;
    this.draggingBookmark = null;
    this.overId = null;
    if (refused) return;
    if (bookmark) {
      this.dispatchEvent(
        new CustomEvent<BookmarkDrop>('bookmark-drop', {
          detail: { name: bookmark.name, from: bookmark.from, to: destination },
          bubbles: true,
          composed: true,
        }),
      );
      return;
    }
    if (!source || source.commitId === destination.commitId) return;
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
    const dragging = this.draggingId !== null || this.draggingBookmark !== null;
    const barred = dragging && this.refuses(change);
    const rowClass = [
      'row',
      change.commitId === this.selected ? 'selected' : '',
      change.workingCopy ? 'wc' : '',
      change.immutable ? 'immutable' : '',
      change.commitId === this.draggingId ? 'dragging' : '',
      change.commitId === this.overId ? 'drop-target' : '',
      barred && change.commitId !== this.draggingId ? 'barred' : '',
    ].join(' ');
    const summary = change.description.split('\n')[0] ?? '';
    const connectedLane = Math.max(row.lane, ...row.joins, ...row.forks);
    const contentLeft = x(connectedLane) + DOT_R + 9;

    return html`
      <button
        class=${rowClass}
        draggable=${this.canRebase || change.bookmarks.length ? 'true' : 'false'}
        title=${
          this.draggingBookmark
            ? barred
              ? `${this.draggingBookmark.name} is already here`
              : `Drop here to move ${this.draggingBookmark.name} to ${change.changeId.slice(0, 12)}`
            : dragging
              ? barred
                ? `${change.changeId.slice(0, 12)} — cannot hold this change; it is descended from it`
                : `Drop here to rebase onto ${change.changeId.slice(0, 12)}`
              : `${change.changeId.slice(0, 12)} — ${summary || '(no description)'}${
                  this.canRebase ? '\nDrag onto another change to rebase it there.' : ''
                }`
        }
        @click=${() => this.pick(change)}
        @contextmenu=${(event: MouseEvent) => this.openMenu(event, change)}
        @pointerdown=${(event: PointerEvent) => this.onPointerDown(event, change)}
        @dragstart=${(event: DragEvent) => this.onDragStart(event, change)}
        @dragend=${this.onDragEnd}
        @dragover=${(event: DragEvent) => this.onDragOver(event, change)}
        @dragleave=${() => {
          if (this.overId === change.commitId) this.overId = null;
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
        <span class="row-content" style=${`--content-left: ${contentLeft}px`}>
          ${change.bookmarks.map((bookmark) => {
            const status = worstTracking(this.bookmarks, bookmark);
            const moving = this.draggingBookmark?.name === bookmark;
            return html`<span
              class="tag ${moving ? 'moving' : ''}"
              data-bookmark=${bookmark}
              title=${
                status?.ahead
                  ? `${bookmark} is ${status.ahead} commit${
                      status.ahead === 1 ? '' : 's'
                    } ahead of ${status.remote} — a push would send ${
                      status.ahead === 1 ? 'it' : 'them'
                    }\nDrag onto another change to move the bookmark there.`
                  : `${bookmark}\nDrag onto another change to move the bookmark there.`
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
          ${
            // Spelled out, unlike the conflict glyph beside it. A conflict has a
            // red × everywhere in this app and in jj, so the mark is the word; a
            // divergent change has no such vocabulary, and two rows carrying the
            // same unexplained symbol would read as one more thing they share.
            change.divergent
              ? html`<span
                  class="tag warn"
                  title="Two visible commits share this change id — this is one of them, ${change.commitId.slice(
                    0,
                    12,
                  )}. Abandon the side you do not want to clear it."
                  >divergent<span class="which">${change.commitId.slice(0, 8)}</span></span
                >`
              : nothing
          }
          <span class="desc ${summary ? '' : 'empty-desc'}">
            ${summary || (change.workingCopy ? 'working copy' : '(no description)')}
          </span>
          <span class="when">${relativeTime(change.committer.timestamp)}</span>
        </span>
      </button>
    `;
  }
}

/** How many lane hues theme.css defines; lanes past it wrap around. */
const LANE_COLOURS = 6;

const laneStroke = (lane: number) => `stroke: var(--jj-lane-${lane % LANE_COLOURS})`;

/**
 * A bookmark dragged off one change and dropped on another.
 *
 * Carries `from` as well as `to` even though jj needs only the destination:
 * the confirmation names both ends, and this is the one moment both are known
 * — after the move the graph no longer says where the name used to be.
 */
export interface BookmarkDrop {
  name: string;
  from: Change;
  to: Change;
}

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
