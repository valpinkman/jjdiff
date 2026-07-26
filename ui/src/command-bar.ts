import { css, html, LitElement } from 'lit';
import { customElement, property, query, state } from 'lit/decorators.js';

export interface Command {
  id: string;
  label: string;
  hint?: string;
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
      width: min(560px, 90vw);
      background: var(--jj-bg);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 12px);
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
      font: inherit;
      padding: 10px 14px;
      outline: none;
    }
    .item {
      display: flex;
      justify-content: space-between;
      gap: 12px;
      padding: 7px 14px;
      cursor: pointer;
      transition: background-color 0.12s ease;
    }
    .item.active {
      background: var(--jj-bg-panel);
      box-shadow: inset 2px 0 0 var(--jj-accent);
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

  override firstUpdated() {
    this.input.focus();
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
          : matches.map(
              (command, index) => html`
                <div
                  class="item ${index === active ? 'active' : ''}"
                  @click=${() => this.pick(command)}
                  @mousemove=${() => (this.active = index)}
                >
                  <span>${command.label}</span>
                  ${command.hint ? html`<span class="hint">${command.hint}</span>` : ''}
                </div>
              `,
            )}
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
