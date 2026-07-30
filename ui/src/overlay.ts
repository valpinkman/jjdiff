import { css, LitElement } from 'lit';

/**
 * What every modal overlay shares, as stylesheets and a base class.
 *
 * Parts rather than a wrapper element on purpose: these mount above the diff
 * pane, and a shadow root anywhere above a diff row severs theme.css from it
 * and breaks cross-row selection (CLAUDE.md). A base class contributes no
 * element of its own, so it is safe where a component would not be.
 *
 * Each overlay composes what it needs and keeps its own block **after** the
 * shared ones — same specificity, so the local rule is the one that wins.
 */

/**
 * The scrim, for the overlays that share the z-index-110 layer.
 *
 * Not all of them: the command palette and the shortcuts sheet sit at 100 and
 * the prompt at 200, so a confirmation raised from a palette command lands on
 * top of the palette rather than behind it. That stacking is deliberate and
 * this sheet would flatten it.
 */
export const overlayChrome = css`
  :host {
    position: fixed;
    inset: 0;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 9vh;
    /* Blur rather than a flat sheet: the app stays legible as context behind
       the panel, which is what tells you this is a layer and not a new screen. */
    background: rgb(0 0 0 / 0.22);
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
  /* Named here, applied by each panel — the panels differ in size and radius,
     but they all arrive the same way. Redefining pop in a local block replaces
     this one, since keyframes are resolved per tree scope and the local sheet
     is adopted last. */
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-10px) scale(0.965);
    }
  }
  /* theme.css has the app-wide switch, but a universal rule in a document
     stylesheet does not cross a shadow boundary — every shadow root that
     animates has to opt out for itself.

     ":host .panel", not ".panel": a media query adds no specificity, and this
     sheet is adopted *before* each overlay's own, which is where
     ".panel { animation: pop }" is written. At equal specificity the later
     declaration wins, so the plain selector lost to the very rule it exists to
     cancel — the panels went on animating under reduced motion. The extra
     ":host" outranks it whatever the adoption order. */
  @media (prefers-reduced-motion: reduce) {
    :host,
    :host .panel {
      animation: none;
    }
  }
`;

/** A panel's title row: the heading, the aside that qualifies it, the gap. */
export const panelHeader = css`
  header {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 16px 20px 12px;
  }
  h2 {
    margin: 0;
    font-family: var(--jj-sans);
    font-size: var(--jj-text-title, 20px);
    font-weight: 650;
    letter-spacing: -0.02em;
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
`;

/**
 * The pill button a panel's footer is made of.
 *
 * Class-scoped, never bare `button`: a panel's shadow root holds buttons that
 * are not this — a list row, a toggle, a swatch — and styling the element
 * reaches all of them.
 */
export const panelButton = css`
  .btn {
    font-family: var(--jj-sans);
    font-size: var(--jj-text-base, 13px);
    border: 1px solid var(--jj-border);
    border-radius: var(--jj-r-pill, 999px);
    background: var(--jj-surface);
    color: var(--jj-fg);
    padding: 7px 16px;
    cursor: pointer;
    transition:
      background var(--jj-t-1, 120ms) ease-out,
      border-color var(--jj-t-1, 120ms) ease-out;
  }
  .btn:hover:not(:disabled) {
    border-color: var(--jj-border-strong);
  }
  .btn.primary {
    background: var(--jj-primary);
    color: var(--jj-primary-fg);
    border-color: transparent;
  }
  .btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
`;

/**
 * The wiring an overlay needs to be dismissable and to keep its keys.
 *
 * Dismissal is left abstract because the ways out are not interchangeable:
 * most dispatch `close`, the theme picker has to re-emit the theme in effect
 * before it goes or the live preview stays applied, and the prompt resolves the
 * promise `askText` is waiting on. A base that dispatched one event for all of
 * them would drop the restore and leave the promise unsettled.
 */
export abstract class OverlayElement extends LitElement {
  /** How this overlay ends. Called by the backdrop, and by Escape. */
  protected abstract dismiss(): void;

  /**
   * Whether Escape at window level dismisses this one.
   *
   * On by default, because a click on the scrim moves focus to the body, past
   * the panel's own handler, leaving the window as the only place left to catch
   * it. Off where Escape already belongs to someone else: the palette and the
   * prompt answer it on the panel and stop it there, and the shortcuts sheet is
   * opened and closed by `App.onGlobalKey`, which a listener here would race to
   * close on the same keystroke.
   */
  protected escapeOnWindow = true;

  override connectedCallback() {
    super.connectedCallback();
    this.addEventListener('click', this.onBackdrop);
    if (this.escapeOnWindow) window.addEventListener('keydown', this.onWindowEscape);
  }

  override disconnectedCallback() {
    this.removeEventListener('click', this.onBackdrop);
    window.removeEventListener('keydown', this.onWindowEscape);
    super.disconnectedCallback();
  }

  private onBackdrop = (event: MouseEvent) => {
    // `event.target` is retargeted to the host for a listener bound to the host,
    // so an inside click reads as an outside one. The composed path's first
    // entry is the real origin — the host itself only when the scrim was hit.
    if (event.composedPath()[0] === this) this.dismiss();
  };

  private onWindowEscape = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    this.dismiss();
  };

  /**
   * Bind on the panel, not the window: `App.onGlobalKey` decides someone is
   * typing by reading `event.target`, which by the time an event from inside a
   * shadow root reaches window is this host and not the input two roots down.
   * Without this, `j`, `k`, `v` and `c` typed into a filter box would drive the
   * diff behind the dialog. Escape is let through — the window listener above
   * owns it.
   */
  protected onPanelKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape') return;
    event.stopPropagation();
  };
}
