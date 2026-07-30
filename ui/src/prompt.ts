import { css, html, nothing } from 'lit';
import { customElement, property, query, state } from 'lit/decorators.js';

import { OverlayElement } from './overlay.js';

/**
 * In-app replacements for `prompt()` and `confirm()`.
 *
 * These are not a style preference — the WebView has no dialogs. wry's
 * `WKUIDelegate` implements only the file-open panel and media permissions, so
 * on macOS `prompt()` returns null, `confirm()` returns false and `alert()`
 * does nothing at all. Every confirmation-gated action built on them is a
 * silent no-op in the shipped app. (DESIGN.md already banned `alert()`; this
 * closes the other two.)
 *
 * Promise-based so call sites read the way the built-ins did:
 *
 *     if (!(await askConfirm({ title: 'Abandon "wip"?' }))) return;
 *     const name = await askText({ title: 'Bookmark name' });
 */
@customElement('jj-prompt')
export class Prompt extends OverlayElement {
  static override styles = css`
    /* Above every other overlay, not on their layer: a confirmation is usually
       raised by a command run from one of them, and it has to land on top of
       whatever asked for it. */
    :host {
      position: fixed;
      inset: 0;
      display: flex;
      justify-content: center;
      align-items: flex-start;
      padding-top: 18vh;
      background: rgb(0 0 0 / 0.25);
      z-index: 200;
    }
    .panel {
      width: min(460px, 90vw);
      background: var(--jj-bg);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 14px);
      box-shadow: var(--jj-shadow-pop, 0 12px 40px rgb(0 0 0 / 0.3));
      padding: 16px 18px 14px;
      animation: prompt-in 0.16s ease-out;
    }
    @keyframes prompt-in {
      from {
        opacity: 0;
        transform: translateY(-6px) scale(0.99);
      }
    }
    h2 {
      margin: 0;
      font-family: var(--jj-sans);
      font-size: 14px;
      font-weight: 600;
      color: var(--jj-fg);
    }
    .detail {
      margin-top: 6px;
      font-size: 12.5px;
      line-height: 1.5;
      color: var(--jj-fg-muted);
      max-width: 70ch;
      white-space: pre-line;
    }
    input {
      width: 100%;
      box-sizing: border-box;
      margin-top: 12px;
      padding: 8px 11px;
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-sm, 7px);
      background: var(--jj-surface);
      color: var(--jj-fg);
      font-family: var(--jj-mono);
      font-size: 12.5px;
      outline: none;
    }
    input:focus {
      border-color: var(--jj-accent);
      box-shadow: var(--jj-focus-ring);
    }
    .actions {
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      margin-top: 14px;
    }
    button {
      font-family: var(--jj-sans);
      font-size: 12.5px;
      font-weight: 550;
      padding: 6px 13px;
      border-radius: var(--jj-r-sm, 7px);
      border: 1px solid var(--jj-border);
      background: var(--jj-surface);
      color: var(--jj-fg);
      cursor: pointer;
      transition: background-color 0.12s ease, border-color 0.12s ease;
    }
    button:hover {
      background: var(--jj-wash);
    }
    button:focus-visible {
      outline: 2px solid var(--jj-accent);
      outline-offset: 1px;
    }
    button.primary {
      background: var(--jj-accent);
      border-color: var(--jj-accent);
      color: #fff;
    }
    button.primary:hover {
      filter: brightness(1.06);
    }
    /* Destructive actions read as destructive before they are clicked. */
    button.danger {
      color: var(--jj-removed-fg);
      border-color: var(--jj-removed-fg);
      background: var(--jj-surface);
    }
    button.danger:hover {
      background: var(--jj-removed-bg);
    }
  `;

  @property() heading = '';
  @property() detail = '';
  @property() confirmLabel = 'OK';
  /** Text mode when true; plain confirmation otherwise. */
  @property({ type: Boolean }) input = false;
  @property({ type: Boolean }) danger = false;
  @property() placeholder = '';

  @state() value = '';

  @query('input') private field?: HTMLInputElement;

  /**
   * Escape is answered on the panel, which always holds focus — the field in
   * text mode, the confirm button otherwise — and stopped there, so no window
   * listener is needed and the overlay this was raised from keeps its own.
   */
  protected override escapeOnWindow = false;

  override firstUpdated() {
    // Focus the field in text mode, the confirm button otherwise, so Enter
    // does the obvious thing either way.
    if (this.input) {
      this.field?.focus();
      this.field?.select();
    } else {
      this.renderRoot.querySelector<HTMLButtonElement>('button.confirm')?.focus();
    }
  }

  /** Dismissing is a refusal: `askConfirm` resolves false, `askText` null. */
  protected override dismiss() {
    this.done(false);
  }

  private done(accepted: boolean) {
    this.dispatchEvent(
      new CustomEvent<string | null>('resolve', {
        detail: accepted ? (this.input ? this.value : '') : null,
      }),
    );
  }

  private onKeydown(event: KeyboardEvent) {
    // Contained here rather than on window: the app's global handler must not
    // see j/k/v/o while someone is typing a bookmark name.
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      this.done(false);
    } else if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      this.done(true);
    }
  }

  protected override render() {
    return html`
      <div class="panel" @keydown=${this.onKeydown}>
        <h2>${this.heading}</h2>
        ${this.detail ? html`<div class="detail">${this.detail}</div>` : nothing}
        ${this.input
          ? html`<input
              .value=${this.value}
              placeholder=${this.placeholder}
              @input=${(event: Event) => (this.value = (event.target as HTMLInputElement).value)}
            />`
          : nothing}
        <div class="actions">
          <button @click=${() => this.done(false)}>Cancel</button>
          <button
            class="confirm ${this.danger ? 'danger' : 'primary'}"
            @click=${() => this.done(true)}
          >
            ${this.confirmLabel}
          </button>
        </div>
      </div>
    `;
  }
}

/** Mount a prompt, await the answer, tear it down. */
function ask(configure: (element: Prompt) => void): Promise<string | null> {
  return new Promise((resolve) => {
    const element = document.createElement('jj-prompt');
    configure(element);
    const finish = (event: Event) => {
      const detail = (event as CustomEvent<string | null>).detail;
      element.remove();
      resolve(detail);
    };
    element.addEventListener('resolve', finish, { once: true });
    document.body.append(element);
  });
}

/** Replacement for `prompt()`. Resolves to null when cancelled. */
export function askText(options: {
  heading: string;
  detail?: string;
  value?: string;
  placeholder?: string;
  confirmLabel?: string;
}): Promise<string | null> {
  return ask((element) => {
    element.input = true;
    element.heading = options.heading;
    element.detail = options.detail ?? '';
    element.value = options.value ?? '';
    element.placeholder = options.placeholder ?? '';
    element.confirmLabel = options.confirmLabel ?? 'OK';
  });
}

/** Replacement for `confirm()`. */
export async function askConfirm(options: {
  heading: string;
  detail?: string;
  confirmLabel?: string;
  danger?: boolean;
}): Promise<boolean> {
  const answer = await ask((element) => {
    element.heading = options.heading;
    element.detail = options.detail ?? '';
    element.confirmLabel = options.confirmLabel ?? 'Confirm';
    element.danger = options.danger ?? false;
  });
  return answer !== null;
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-prompt': Prompt;
  }
}
