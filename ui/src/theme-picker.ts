import { css, html } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

import { OverlayElement, overlayChrome, panelHeader } from './overlay.js';
import { THEMES, type ThemeOption } from './themes.js';

/**
 * The theme chooser.
 *
 * Twenty entries in the command palette would be twenty lines of text for a
 * decision that is entirely visual — nobody picks Kanagawa over Everforest by
 * reading the words. Each theme is a swatch of its own colours instead, and
 * hovering one **applies it live**, so choosing is looking rather than
 * guess-and-undo. Escaping restores whatever was active on open.
 *
 * Shadow DOM: leaf widget, no cross-boundary selection (DESIGN.md §6).
 */
@customElement('jj-theme-picker')
export class ThemePicker extends OverlayElement {
  static override styles = [
    overlayChrome,
    panelHeader,
    css`
      .panel {
        display: flex;
        flex-direction: column;
        width: min(680px, 92vw);
        max-height: 78vh;
        background: var(--jj-bg-panel);
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-lg, 22px);
        box-shadow: var(--jj-shadow-pop);
        overflow: hidden;
        animation: pop var(--jj-t-3, 260ms) var(--jj-ease-pop, ease-out);
      }
      input {
        width: 100%;
        box-sizing: border-box;
        border: 0;
        border-top: 1px solid var(--jj-border);
        border-bottom: 1px solid var(--jj-border);
        background: transparent;
        color: var(--jj-fg);
        font-family: var(--jj-sans);
        font-size: 14px;
        padding: 11px 20px;
        outline: none;
      }
      input::placeholder {
        color: var(--jj-fg-faint);
      }
      .list {
        overflow-y: auto;
        padding: 12px 14px 16px;
      }
      .group {
        font-family: var(--jj-sans);
        font-size: 10.5px;
        font-weight: 650;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--jj-fg-faint);
        padding: 10px 6px 6px;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
        gap: 8px;
      }
      .theme {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 8px;
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-md, 16px);
        background: var(--jj-surface);
        color: var(--jj-fg);
        font: inherit;
        font-family: var(--jj-sans);
        text-align: left;
        cursor: pointer;
        transition:
          border-color var(--jj-t-2, 180ms) ease,
          box-shadow var(--jj-t-2, 180ms) ease,
          transform var(--jj-t-2, 180ms) ease;
      }
      .theme:hover {
        border-color: var(--jj-border-strong);
        box-shadow: var(--jj-shadow-raised);
        transform: translateY(-1px);
      }
      .theme.active {
        border-color: var(--jj-accent-line);
        box-shadow: 0 0 0 1px var(--jj-accent-line);
      }
      .theme:focus-visible {
        outline: 2px solid var(--jj-accent);
        outline-offset: 2px;
      }
      /* The swatch is the label. Four bars — page, card, accent, bookmark — which
         is the smallest set that distinguishes any two themes in the list. */
      .swatch {
        display: grid;
        grid-template-columns: 1fr 1fr;
        grid-template-rows: 1fr 1fr;
        width: 30px;
        height: 30px;
        flex: none;
        border-radius: 8px;
        overflow: hidden;
        box-shadow: inset 0 0 0 1px rgb(128 128 128 / 0.25);
      }
      .swatch span {
        display: block;
      }
      .name {
        display: flex;
        flex-direction: column;
        gap: 1px;
        min-width: 0;
      }
      .name strong {
        font-size: 12.5px;
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .name small {
        font-size: 10.5px;
        color: var(--jj-fg-muted);
      }
      .none {
        padding: 20px;
        color: var(--jj-fg-muted);
        font-family: var(--jj-sans);
      }
      @media (prefers-reduced-motion: reduce) {
        .theme:hover {
          transform: none;
        }
      }
    `,
  ];

  /** The theme in effect when the picker opened; restored on cancel. */
  @property() current = 'system';

  @state() private filter = '';
  /** What the pointer is previewing, or null when nothing is hovered. */
  @state() private preview: string | null = null;

  override firstUpdated() {
    this.renderRoot.querySelector('input')?.focus();
  }

  /**
   * Set once a theme is chosen, after which no more previews are emitted.
   *
   * Choosing removes this element, and removing it makes the pointer leave the
   * list — which fires `mouseleave`, which used to emit a preview of `current`
   * and quietly undo the choice a frame after it was made.
   */
  private committed = false;

  /**
   * Leaving without choosing has to put back what was on screen when the
   * picker opened, or the last thing hovered stays applied.
   */
  protected override dismiss() {
    if (!this.committed) {
      this.dispatchEvent(new CustomEvent('preview-theme', { detail: this.current }));
    }
    this.dispatchEvent(new Event('close'));
  }

  private choose(theme: ThemeOption) {
    this.committed = true;
    this.dispatchEvent(new CustomEvent('pick-theme', { detail: theme.id }));
    this.dispatchEvent(new Event('close'));
  }

  private hover(id: string | null) {
    if (this.committed) return;
    this.preview = id;
    this.dispatchEvent(new CustomEvent('preview-theme', { detail: id ?? this.current }));
  }

  private get filtered(): ThemeOption[] {
    const needle = this.filter.trim().toLowerCase();
    if (!needle) return [...THEMES];
    return THEMES.filter(
      (theme) =>
        theme.label.toLowerCase().includes(needle) || theme.group.toLowerCase().includes(needle),
    );
  }

  protected override render() {
    const filtered = this.filtered;
    const groups: { name: string; themes: ThemeOption[] }[] = [];
    for (const theme of filtered) {
      const last = groups[groups.length - 1];
      if (last?.name === theme.group) last.themes.push(theme);
      else groups.push({ name: theme.group, themes: [theme] });
    }
    return html`<div class="panel" @keydown=${this.onPanelKey}>
      <header>
        <h2>Theme</h2>
        <span class="hint">Hover to preview · Esc to cancel</span>
      </header>
      <input
        placeholder="Filter themes…"
        .value=${this.filter}
        @input=${(event: Event) => (this.filter = (event.target as HTMLInputElement).value)}
      />
      <div class="list" @mouseleave=${() => this.hover(null)}>
        ${filtered.length === 0
          ? html`<div class="none">No theme matches that.</div>`
          : groups.map(
              (group) => html`
                <div class="group">${group.name}</div>
                <div class="grid">
                  ${group.themes.map(
                    (theme) => html`<button
                      class="theme ${theme.id === (this.preview ?? this.current) ? 'active' : ''}"
                      @click=${() => this.choose(theme)}
                      @mouseenter=${() => this.hover(theme.id)}
                      @focus=${() => this.hover(theme.id)}
                    >
                      <span class="swatch">
                        ${theme.swatch.map(
                          (colour) => html`<span style="background: ${colour}"></span>`,
                        )}
                      </span>
                      <span class="name">
                        <strong>${theme.label}</strong>
                        <small
                          >${theme.group === theme.label ? theme.mode : `${theme.group}`}</small
                        >
                      </span>
                    </button>`,
                  )}
                </div>
              `,
            )}
      </div>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-theme-picker': ThemePicker;
  }
}
