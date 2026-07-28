import { css, html, LitElement } from 'lit';
import { customElement, property } from 'lit/decorators.js';

/**
 * The thinking indicator, used while an agent is working.
 *
 * jjdiff shells a walkthrough out to an agent CLI, and that call has no
 * progress to report — it finishes when the model is done. A determinate bar
 * would be a lie and a spinner says only "blocked". Orbs say *thinking*: three
 * lights drifting on their own orbits, never quite repeating, which is what the
 * thing behind them is actually doing.
 *
 * Shadow DOM is fine here — this is a leaf widget with no text to select and it
 * never sits above the diff pane (see DESIGN.md).
 *
 * The hues are the log graph's lanes, not new colours. The design system has
 * exactly one multi-hue palette and this borrows it rather than inventing a
 * second one.
 */
@customElement('jj-orbs')
export class Orbs extends LitElement {
  static override styles = css`
    :host {
      display: inline-block;
      /* Overridden per use; everything below is relative to this. */
      --orb-size: 44px;
      width: var(--orb-size);
      height: var(--orb-size);
      /* No paint containment: the bloom is wider than the box it is laid out
         in, and clipping it to that box would put a straight edge through the
         one thing here that is supposed to have none. */
    }

    .field {
      position: relative;
      width: 100%;
      height: 100%;
      /* Blur alone, deliberately. The usual gooey recipe pairs it with a big
         contrast pass to snap the blurred alpha back to a hard edge — but that
         needs an opaque backdrop inside the filtered box to push against, and
         over a transparent one it crushes three hues into a single blown-out
         colour. Lights that overlap and bleed is the effect that was wanted
         anyway; the hard-edged blob was never the point. */
      filter: blur(calc(var(--orb-size) * 0.095));
    }

    .orb {
      position: absolute;
      top: 50%;
      left: 50%;
      width: calc(var(--orb-size) * 0.46);
      height: calc(var(--orb-size) * 0.46);
      margin: calc(var(--orb-size) * -0.23) 0 0 calc(var(--orb-size) * -0.23);
      border-radius: 50%;
      background: radial-gradient(circle at 50% 50%, var(--orb-hue) 30%, transparent 70%);
      /* Where two orbs cross, the light adds rather than the nearer one winning
         — which is what stops the group reading as three stacked stickers. */
      mix-blend-mode: plus-lighter;
      animation: orbit var(--orb-period) linear infinite;
    }

    /* Three orbits, deliberately incommensurate periods: 3.1 / 3.9 / 4.7s take
       nearly two minutes to line up again, so the loop never announces itself
       the way three equal periods would. */
    .orb.a {
      --orb-hue: var(--jj-lane-0, #3d7ff5);
      --orb-period: 3.1s;
      --orb-radius: calc(var(--orb-size) * 0.23);
    }

    .orb.b {
      --orb-hue: var(--jj-lane-1, #7d5cd6);
      --orb-period: 3.9s;
      --orb-radius: calc(var(--orb-size) * 0.29);
      animation-delay: -1.3s;
    }

    .orb.c {
      --orb-hue: var(--jj-lane-2, #10877a);
      --orb-period: 4.7s;
      --orb-radius: calc(var(--orb-size) * 0.18);
      animation-delay: -2.6s;
    }

    /* An ellipse rather than a circle, and each orb scales through its lap, so
       the blob reads as turning in depth instead of spinning flat. */
    @keyframes orbit {
      0% {
        transform: translate(var(--orb-radius), 0) scale(1);
      }
      25% {
        transform: translate(0, calc(var(--orb-radius) * 0.62)) scale(0.78);
      }
      50% {
        transform: translate(calc(var(--orb-radius) * -1), 0) scale(1);
      }
      75% {
        transform: translate(0, calc(var(--orb-radius) * -0.62)) scale(1.22);
      }
      100% {
        transform: translate(var(--orb-radius), 0) scale(1);
      }
    }

    /* Still an object, still three colours, just not moving. The blur stays:
       the composed bloom is the identity of the thing, and dropping to three
       plain dots would read as a different indicator entirely. */
    @media (prefers-reduced-motion: reduce) {
      .orb {
        animation: none;
      }
      .orb.a {
        transform: translate(var(--orb-radius), 0);
      }
      .orb.b {
        transform: translate(calc(var(--orb-radius) * -0.5), calc(var(--orb-radius) * 0.7));
      }
      .orb.c {
        transform: translate(calc(var(--orb-radius) * -0.5), calc(var(--orb-radius) * -0.7));
      }
    }
  `;

  /** Edge length in pixels. The orbits and the blur are all derived from it. */
  @property({ type: Number }) size = 44;

  /** Announced to assistive tech in place of the animation. */
  @property({ type: String }) label = 'Working';

  protected override render() {
    this.style.setProperty('--orb-size', `${this.size}px`);
    return html`<div class="field" role="img" aria-label=${this.label}>
      <span class="orb a"></span>
      <span class="orb b"></span>
      <span class="orb c"></span>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-orbs': Orbs;
  }
}
