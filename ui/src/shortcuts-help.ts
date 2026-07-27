import { css, html, LitElement } from 'lit';
import { customElement, property } from 'lit/decorators.js';

import { shortcutReference } from './keys.js';

/**
 * The `?` shortcut sheet. A leaf overlay with no cross-boundary text selection,
 * so shadow DOM is allowed here (DESIGN.md §5) — it inherits the theme through
 * custom properties, which do pierce the boundary.
 */
@customElement('jj-shortcuts-help')
export class ShortcutsHelp extends LitElement {
  static override styles = css`
    :host {
      position: fixed;
      inset: 0;
      display: flex;
      justify-content: center;
      align-items: flex-start;
      padding-top: 12vh;
      background: rgb(0 0 0 / 0.25);
      z-index: 100;
    }
    .panel {
      width: min(560px, 90vw);
      max-height: 70vh;
      overflow-y: auto;
      background: var(--jj-bg);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 14px);
      box-shadow: var(--jj-shadow-pop, 0 12px 40px rgb(0 0 0 / 0.3));
      animation: sheet-in 0.16s ease-out;
    }
    @keyframes sheet-in {
      from {
        opacity: 0;
        transform: translateY(-6px) scale(0.99);
      }
    }
    header {
      display: flex;
      justify-content: space-between;
      align-items: baseline;
      padding: 14px 18px 10px;
      border-bottom: 1px solid var(--jj-border);
    }
    h2 {
      margin: 0;
      font-size: 14.5px;
      font-weight: 600;
      font-family: var(--jj-sans);
    }
    .dismiss {
      font-size: 11px;
      color: var(--jj-fg-muted);
    }
    .group {
      font-size: 10.5px;
      font-weight: 650;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      color: var(--jj-fg-faint);
      padding: 13px 18px 5px;
    }
    .row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 16px;
      padding: 5px 18px;
      font-size: 12.5px;
    }
    kbd {
      flex: none;
      font-family: var(--jj-mono);
      font-size: 11.5px;
      font-variant-numeric: tabular-nums;
      color: var(--jj-fg);
      background: var(--jj-surface);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-sm, 7px);
      box-shadow: var(--jj-shadow-card);
      padding: 2px 7px;
    }
    ul {
      margin: 0 0 12px;
      padding: 0;
      list-style: none;
    }
  `;

  /** The configured command-palette binding, e.g. "Mod+k". */
  @property() commandBar = 'Mod+k';

  private close() {
    this.dispatchEvent(new Event('close'));
  }

  override connectedCallback() {
    super.connectedCallback();
    this.addEventListener('click', this.onBackdrop);
  }

  override disconnectedCallback() {
    this.removeEventListener('click', this.onBackdrop);
    super.disconnectedCallback();
  }

  private onBackdrop = (event: MouseEvent) => {
    if (event.target === this) this.close();
  };

  protected override render() {
    return html`
      <div class="panel">
        <header>
          <h2>Keyboard shortcuts</h2>
          <span class="dismiss">Esc to close</span>
        </header>
        ${shortcutReference(this.commandBar).map(
          (group) => html`
            <div class="group">${group.title}</div>
            <ul>
              ${group.bindings.map(
                (binding) => html`
                  <li class="row">
                    <span>${binding.label}</span>
                    <kbd>${binding.keys}</kbd>
                  </li>
                `,
              )}
            </ul>
          `,
        )}
      </div>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-shortcuts-help': ShortcutsHelp;
  }
}
