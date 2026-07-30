import { css, html, LitElement } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';

/**
 * A walkthrough diagram, large enough to read.
 *
 * The overview's figure is capped at the width of the document column, which is
 * fine for the four-box flowchart most changes produce and useless for anything
 * wider — the labels are still drawn at full size, so a busy graph shrinks to a
 * band of grey rectangles. Rather than let the figure grow and push the prose
 * around it, the diagram opens here: the pane, a zoom, and a drag.
 *
 * It takes the **rendered SVG**, not the mermaid source. The markup was already
 * produced and scrubbed once when the document rendered; re-running mermaid to
 * show the same picture bigger would be a second parse of the same text, and a
 * second chance for it to fail differently. Duplicate element ids across the two
 * copies are harmless: the stylesheet mermaid ships inside the SVG selects by
 * id, and CSS matches every element carrying one, while the arrow markers each
 * copy references live inside that copy.
 *
 * Shadow DOM: leaf widget, no cross-boundary selection (DESIGN.md §6).
 */
@customElement('jj-diagram-view')
export class DiagramView extends LitElement {
  static override styles = css`
    :host {
      position: fixed;
      inset: 0;
      display: flex;
      justify-content: center;
      align-items: center;
      background: rgb(0 0 0 / 0.32);
      backdrop-filter: blur(3px) saturate(0.9);
      -webkit-backdrop-filter: blur(3px) saturate(0.9);
      z-index: 110;
      animation: scrim-in var(--jj-t-2, 180ms) ease-out;
    }
    @keyframes scrim-in {
      from {
        opacity: 0;
      }
    }
    .panel {
      display: flex;
      flex-direction: column;
      width: min(1400px, 94vw);
      height: min(900px, 88vh);
      background: var(--jj-bg-panel);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 22px);
      box-shadow: var(--jj-shadow-pop);
      overflow: hidden;
      animation: pop var(--jj-t-3, 260ms) var(--jj-ease-pop, ease-out);
    }
    @keyframes pop {
      from {
        opacity: 0;
        transform: scale(0.975);
      }
    }
    .panel:focus {
      outline: none;
    }
    header {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 12px 14px 12px 20px;
      border-bottom: 1px solid var(--jj-border);
    }
    h2 {
      margin: 0;
      font-family: var(--jj-sans);
      font-size: var(--jj-text-md, 13px);
      font-weight: 650;
      color: var(--jj-fg);
    }
    .hint {
      font-family: var(--jj-sans);
      font-size: var(--jj-text-sm, 11.5px);
      color: var(--jj-fg-muted);
    }
    .spacer {
      flex: 1;
    }
    button {
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-sm, 8px);
      background: var(--jj-surface);
      color: var(--jj-fg-soft);
      font-family: var(--jj-sans);
      font-size: var(--jj-text-sm, 11.5px);
      font-weight: 550;
      padding: 4px 10px;
      cursor: pointer;
      transition:
        border-color var(--jj-t-2, 180ms) ease,
        color var(--jj-t-2, 180ms) ease;
    }
    button:hover {
      border-color: var(--jj-border-strong);
      color: var(--jj-fg);
    }
    button:focus-visible {
      outline: 2px solid var(--jj-accent);
      outline-offset: 1px;
    }
    /* Fixed width, tabular figures: the readout sits between the two buttons it
       describes, and letting it size to its content shifted them on every step. */
    .level {
      width: 5ch;
      text-align: center;
      font-family: var(--jj-mono);
      font-size: var(--jj-text-sm, 11.5px);
      font-variant-numeric: tabular-nums;
      color: var(--jj-fg-muted);
    }
    .viewport {
      position: relative;
      flex: 1;
      min-height: 0;
      overflow: hidden;
      cursor: grab;
      touch-action: none;
    }
    .viewport.dragging {
      cursor: grabbing;
    }
    .stage {
      position: absolute;
      inset: 0;
      transform-origin: 0 0;
    }
    /* mermaid writes its own width and a max-width in a style attribute, sized
       for the column the figure sat in. The stage is the frame now, so the SVG
       fills it and its viewBox does the letterboxing. */
    .stage svg {
      width: 100% !important;
      height: 100% !important;
      max-width: none !important;
    }
    @media (prefers-reduced-motion: reduce) {
      :host,
      .panel {
        animation: none;
      }
    }
  `;

  /** Rendered, already-scrubbed SVG markup. */
  @property() svg = '';
  /** The section heading the diagram sat under, if the document had one. */
  @property() caption = 'Diagram';

  @state() private scale = 1;
  @state() private x = 0;
  @state() private y = 0;
  @state() private dragging = false;

  private static readonly MIN = 0.4;
  private static readonly MAX = 8;

  override connectedCallback() {
    super.connectedCallback();
    this.addEventListener('click', this.onBackdrop);
    window.addEventListener('keydown', this.onWindowKey);
  }

  override disconnectedCallback() {
    this.removeEventListener('click', this.onBackdrop);
    window.removeEventListener('keydown', this.onWindowKey);
    super.disconnectedCallback();
  }

  override firstUpdated() {
    // The panel owns the keyboard while it is open, so it has to be able to
    // receive keys in the first place.
    this.renderRoot.querySelector<HTMLElement>('.panel')?.focus();
  }

  private onBackdrop = (event: MouseEvent) => {
    // Retargeting: a listener on the host sees inside clicks as the host too, so
    // the composed path's origin is the only thing that distinguishes them.
    if (event.composedPath()[0] === this) this.close();
  };

  /**
   * Escape only. Every other key is handled on the panel and stopped there —
   * `App.onGlobalKey` reads `event.target`, which by the time an event from
   * inside a shadow root reaches window is this host, so `+` and `0` would fall
   * through to the diff behind the dialog.
   */
  private onWindowKey = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    this.close();
  };

  private onPanelKey = (event: KeyboardEvent) => {
    const step = event.shiftKey ? 2 : 1.25;
    if (event.key === '+' || event.key === '=') {
      this.zoomAtCentre(step);
    } else if (event.key === '-' || event.key === '_') {
      this.zoomAtCentre(1 / step);
    } else if (event.key === '0') {
      this.reset();
    } else if (event.key === 'Escape') {
      return; // window listener owns it
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
  };

  private close() {
    this.dispatchEvent(new Event('close'));
  }

  private reset() {
    this.scale = 1;
    this.x = 0;
    this.y = 0;
  }

  /**
   * Scale about a fixed point, in viewport coordinates.
   *
   * The stage is `translate(x, y) scale(s)` from its own top-left, so the point
   * of the diagram under `(px, py)` is `(p - t) / s`. Holding that constant
   * across the change is what makes the wheel zoom into the thing beneath the
   * pointer rather than into the middle of the frame.
   */
  private zoomAbout(factor: number, px: number, py: number) {
    const next = Math.min(DiagramView.MAX, Math.max(DiagramView.MIN, this.scale * factor));
    if (next === this.scale) return;
    this.x = px - ((px - this.x) / this.scale) * next;
    this.y = py - ((py - this.y) / this.scale) * next;
    this.scale = next;
  }

  private zoomAtCentre(factor: number) {
    const box = this.renderRoot.querySelector('.viewport')?.getBoundingClientRect();
    this.zoomAbout(factor, (box?.width ?? 0) / 2, (box?.height ?? 0) / 2);
  }

  private onWheel = (event: WheelEvent) => {
    event.preventDefault();
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    // Exponential so a fast scroll and several slow ones land in the same place,
    // and so the step is proportional at every level rather than coarse when
    // zoomed out and imperceptible when zoomed in.
    this.zoomAbout(
      Math.exp(-event.deltaY * 0.0015),
      event.clientX - box.left,
      event.clientY - box.top,
    );
  };

  private onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) return;
    const viewport = event.currentTarget as HTMLElement;
    viewport.setPointerCapture(event.pointerId);
    this.dragging = true;
    const startX = event.clientX - this.x;
    const startY = event.clientY - this.y;

    const move = (moved: PointerEvent) => {
      this.x = moved.clientX - startX;
      this.y = moved.clientY - startY;
    };
    const up = () => {
      this.dragging = false;
      viewport.removeEventListener('pointermove', move);
      viewport.removeEventListener('pointerup', up);
      viewport.removeEventListener('pointercancel', up);
    };
    viewport.addEventListener('pointermove', move);
    viewport.addEventListener('pointerup', up);
    viewport.addEventListener('pointercancel', up);
  };

  protected override render() {
    return html`<div class="panel" tabindex="-1" @keydown=${this.onPanelKey}>
      <header>
        <h2>${this.caption}</h2>
        <span class="hint">Scroll to zoom · drag to pan</span>
        <span class="spacer"></span>
        <button title="Zoom out (−)" @click=${() => this.zoomAtCentre(1 / 1.25)}>−</button>
        <span class="level">${Math.round(this.scale * 100)}%</span>
        <button title="Zoom in (+)" @click=${() => this.zoomAtCentre(1.25)}>+</button>
        <button title="Fit the diagram to the pane (0)" @click=${this.reset}>Fit</button>
        <button title="Close (Esc)" @click=${this.close}>Close</button>
      </header>
      <div
        class="viewport ${this.dragging ? 'dragging' : ''}"
        @wheel=${this.onWheel}
        @pointerdown=${this.onPointerDown}
      >
        <div
          class="stage"
          style="transform: translate(${this.x}px, ${this.y}px) scale(${this.scale})"
        >
          ${unsafeHTML(this.svg)}
        </div>
      </div>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-diagram-view': DiagramView;
  }
}
