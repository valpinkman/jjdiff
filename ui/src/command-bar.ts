import { css, html, LitElement, nothing, type PropertyValues } from 'lit';
import { customElement, property, query, state } from 'lit/decorators.js';

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
export class CommandBar extends LitElement {
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
      width: min(580px, 90vw);
      padding-bottom: 6px;
      background: var(--jj-bg);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 14px);
      box-shadow: var(--jj-shadow-pop, 0 12px 40px rgb(0 0 0 / 0.3));
      overflow: hidden;
      animation: bar-in 0.16s ease-out;
    }
    @keyframes bar-in {
      from {
        opacity: 0;
        transform: translateY(-6px) scale(0.99);
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
      font-size: 14px;
      padding: 13px 17px;
      outline: none;
    }
    input::placeholder {
      color: var(--jj-fg-faint);
    }
    .item {
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 12px;
      padding: 7px 12px;
      margin: 0 5px;
      border-radius: var(--jj-r-sm, 7px);
      cursor: pointer;
      transition: background-color 0.12s ease;
    }
    .group {
      font-size: 10.5px;
      font-weight: 650;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      color: var(--jj-fg-faint);
      padding: 10px 17px 4px;
    }
    .list {
      max-height: 60vh;
      overflow-y: auto;
    }
    .item.active {
      background: var(--jj-accent-soft);
      color: var(--jj-accent);
    }
    .hint {
      color: var(--jj-fg-muted);
      font-size: 11px;
    }
    .none {
      padding: 12px 14px;
      color: var(--jj-fg-muted);
      font-style: italic;
    }
  `;

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

  private close() {
    this.dispatchEvent(new Event('close'));
  }

  private pick(command: Command) {
    this.close();
    command.run();
  }

  private onKeydown(event: KeyboardEvent) {
    const matches = this.filtered;
    if (event.key === 'Escape') {
      event.preventDefault();
      this.close();
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
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-command-bar': CommandBar;
  }
}
