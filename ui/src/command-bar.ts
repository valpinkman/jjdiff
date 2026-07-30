import { css, html, nothing, type PropertyValues } from 'lit';
import { customElement, property, query, state } from 'lit/decorators.js';

import { OverlayElement } from './overlay.js';

export interface Command {
  id: string;
  label: string;
  hint?: string;
  /** Section header this command sits under. Commands keep their given order. */
  group?: string;
  run: () => void;
}

/** Cmd/Ctrl+Shift+P command palette: filter, arrows, enter, escape. */
@customElement('jj-command-bar')
export class CommandBar extends OverlayElement {
  static override styles = css`
    /* The palette is the app's one genuinely floating surface, so it is the one
       place the full elevation and blur budget is spent. */
    :host {
      position: fixed;
      inset: 0;
      display: flex;
      justify-content: center;
      align-items: flex-start;
      padding-top: 12vh;
      /* Blur rather than a flat scrim: the app stays legible as context behind
         the palette instead of being replaced by a grey sheet, which is what
         tells you this is a layer and not a new screen. */
      background: rgb(0 0 0 / 0.22);
      backdrop-filter: blur(3px) saturate(0.9);
      -webkit-backdrop-filter: blur(3px) saturate(0.9);
      z-index: 100;
      animation: scrim-in var(--jj-t-2, 180ms) ease-out;
    }
    @keyframes scrim-in {
      from {
        opacity: 0;
      }
    }
    .panel {
      width: min(600px, 92vw);
      padding-bottom: 6px;
      background: var(--jj-bg-panel);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 18px);
      box-shadow: var(--jj-shadow-pop, 0 12px 40px rgb(0 0 0 / 0.3));
      overflow: hidden;
      /* The one overshoot in the app. The palette does not slide in from
         somewhere — it arrives, and a hair of scale past 1 is what an arrival
         feels like. Everything else decelerates into place instead. */
      animation: bar-in var(--jj-t-3, 260ms) var(--jj-ease-pop, cubic-bezier(0.34, 1.32, 0.64, 1));
    }
    @keyframes bar-in {
      from {
        opacity: 0;
        transform: translateY(-10px) scale(0.965);
      }
    }
    input {
      width: 100%;
      box-sizing: border-box;
      border: 0;
      border-bottom: 1px solid var(--jj-border);
      background: transparent;
      color: var(--jj-fg);
      font-family: var(--jj-sans);
      font-size: 15px;
      letter-spacing: -0.01em;
      padding: 15px 18px;
      outline: none;
    }
    input::placeholder {
      color: var(--jj-fg-faint);
    }
    /* A row in a list: square and full-bleed, with its own horizontal padding
       so the highlight reaches both edges of the panel. A rounded fill inset by
       a margin reads as a card in a stack of cards. */
    .item {
      position: relative;
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 12px;
      padding: 8px 18px;
      border-radius: 0;
      cursor: pointer;
      transition:
        background-color var(--jj-t-1, 120ms) ease,
        color var(--jj-t-1, 120ms) ease;
    }
    .group {
      font-size: 10.5px;
      font-weight: 650;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      color: var(--jj-fg-faint);
      padding: 12px 18px 4px;
    }
    .list {
      max-height: 58vh;
      overflow-y: auto;
      padding-top: 4px;
      scroll-padding: 8px 0;
    }
    .item.active {
      background: var(--jj-accent-soft);
      color: var(--jj-accent);
    }
    /* A bar on the leading edge, grown from nothing rather than switched on, so
       arrowing down the list reads as one cursor moving. */
    .item.active::before {
      content: '';
      position: absolute;
      left: 0;
      top: 0;
      bottom: 0;
      width: 2px;
      background: var(--jj-accent);
      animation: cursor-in var(--jj-t-2, 180ms) var(--jj-ease-out, ease-out);
    }
    @keyframes cursor-in {
      from {
        transform: scaleY(0.2);
        opacity: 0;
      }
    }
    .hint {
      color: var(--jj-fg-muted);
      font-size: 11px;
      font-variant-numeric: tabular-nums;
    }
    .none {
      padding: 14px 18px;
      color: var(--jj-fg-muted);
    }
    /* theme.css has the app-wide switch, but a universal rule in a document
       stylesheet does not cross a shadow boundary — every shadow root that
       animates has to opt out for itself. */
    @media (prefers-reduced-motion: reduce) {
      :host,
      .panel,
      .item.active::before {
        animation: none;
      }
    }
  `;

  /**
   * Escape arrives on the panel — the filter box has focus the whole time the
   * palette is up — and the palette must not stop the keys it does not use, or
   * the binding that opened it would no longer close it.
   */
  protected override escapeOnWindow = false;

  @property({ attribute: false }) commands: Command[] = [];

  @state() private filter = '';
  @state() private active = 0;

  @query('input') private input!: HTMLInputElement;

  /**
   * Last real pointer position. Scrolling the list under a stationary cursor
   * can emit a `mousemove` whose coordinates never changed, which would drag
   * the cursor back to whatever the pointer happens to be over and fight the
   * arrow keys. Hover only wins when the mouse actually moved.
   */
  private pointer: { x: number; y: number } | null = null;

  private onHover(index: number, event: MouseEvent) {
    if (this.pointer?.x === event.clientX && this.pointer?.y === event.clientY) return;
    this.pointer = { x: event.clientX, y: event.clientY };
    this.active = index;
  }

  override firstUpdated() {
    this.input.focus();
  }

  protected override updated(changed: PropertyValues) {
    // The list scrolls, so moving the cursor has to bring it along — arrowing
    // past the fold otherwise selects something you cannot see.
    if (changed.has('active')) {
      const item = this.renderRoot.querySelector('.item.active');
      // When the cursor lands on the first entry of a group, scroll its header
      // in too; the label is what tells you where you now are.
      const header =
        item?.previousElementSibling?.classList.contains('group') === true
          ? item.previousElementSibling
          : item;
      header?.scrollIntoView({ block: 'nearest' });
    }
    // A new filter means a new list — start it at the top rather than wherever
    // the previous one happened to be scrolled to.
    if (changed.has('filter')) {
      this.renderRoot.querySelector('.list')?.scrollTo({ top: 0 });
    }
  }

  private get filtered(): Command[] {
    const needle = this.filter.trim().toLowerCase();
    if (!needle) return this.commands;
    return this.commands.filter((command) => command.label.toLowerCase().includes(needle));
  }

  protected override dismiss() {
    this.dispatchEvent(new Event('close'));
  }

  private pick(command: Command) {
    this.dismiss();
    command.run();
  }

  private onKeydown(event: KeyboardEvent) {
    const matches = this.filtered;
    if (event.key === 'Escape') {
      event.preventDefault();
      // Stopped, unlike the keys below: dismissing here clears the flag
      // `App.onGlobalKey` guards on, so the same Escape would go on to end the
      // review behind the palette it just closed.
      event.stopPropagation();
      this.dismiss();
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      this.active = Math.min(this.active + 1, matches.length - 1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      this.active = Math.max(this.active - 1, 0);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const command = matches[this.active];
      if (command) this.pick(command);
    }
  }

  protected override render() {
    const matches = this.filtered;
    const active = Math.min(this.active, Math.max(matches.length - 1, 0));
    return html`
      <div class="panel" @keydown=${this.onKeydown}>
        <input
          placeholder="Type a command…"
          .value=${this.filter}
          @input=${(event: Event) => {
            this.filter = (event.target as HTMLInputElement).value;
            this.active = 0;
          }}
        />
        ${matches.length === 0
          ? html`<div class="none">No matching commands</div>`
          : html`<div class="list">
              ${matches.map((command, index) => {
                // A header appears whenever the group changes, so filtering keeps its
                // sections without any regrouping pass.
                const header =
                  command.group && command.group !== matches[index - 1]?.group
                    ? html`<div class="group">${command.group}</div>`
                    : nothing;
                return html`${header}
                  <div
                    class="item ${index === active ? 'active' : ''}"
                    @click=${() => this.pick(command)}
                    @mousemove=${(event: MouseEvent) => this.onHover(index, event)}
                  >
                    <span>${command.label}</span>
                    ${command.hint ? html`<span class="hint">${command.hint}</span>` : ''}
                  </div>`;
              })}
            </div>`}
      </div>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-command-bar': CommandBar;
  }
}
