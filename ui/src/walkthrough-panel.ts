import { css, html, LitElement, nothing } from 'lit';
import { customElement, property } from 'lit/decorators.js';

import type { FilePatch, Walkthrough } from './ipc.js';

/**
 * Sidebar panel during guided review: the step list, with each step's files grouped under
 * it (Linear-style). A step whose files are all marked viewed gets a completion check.
 */
@customElement('jj-walkthrough-panel')
export class WalkthroughPanel extends LitElement {
  static override styles = css`
    :host {
      display: block;
      font-size: 12px;
    }
    button {
      transition: background-color 0.13s ease, box-shadow 0.13s ease;
      display: block;
      width: 100%;
      text-align: left;
      border: 0;
      border-bottom: 1px solid var(--jj-border);
      border-radius: 0;
      background: none;
      color: var(--jj-fg);
      font: inherit;
      padding: 7px 8px;
      cursor: pointer;
    }
    button:hover {
      background: var(--jj-bg-panel);
    }
    button:focus-visible {
      outline: 2px solid var(--jj-accent);
      outline-offset: -2px;
    }
    button.current {
      background: var(--jj-bg-panel);
      box-shadow: inset 3px 0 0 var(--jj-accent);
    }
    .step-head {
      display: flex;
      align-items: baseline;
      gap: 7px;
    }
    .index {
      font-family: var(--jj-mono);
      flex: none;
      width: 17px;
      height: 17px;
      border-radius: 0;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      font-size: 10px;
      font-weight: 700;
      background: var(--jj-hunk-bg);
      color: var(--jj-fg-muted);
    }
    .current .index {
      background: var(--jj-accent);
      color: var(--jj-bg);
    }
    .index.done {
      background: var(--jj-added-bg);
      color: var(--jj-added-fg);
    }
    .title {
      font-weight: 600;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    ul {
      list-style: none;
      margin: 3px 0 0;
      padding: 0 0 0 24px;
    }
    li {
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 1px 0;
      color: var(--jj-fg-muted);
      font-size: 11px;
      overflow: hidden;
      white-space: nowrap;
    }
    li .name {
      overflow: hidden;
      text-overflow: ellipsis;
    }
    li.viewed .name {
      text-decoration: line-through;
      text-decoration-color: var(--jj-border);
    }
    .dot {
      flex: none;
      width: 6px;
      height: 6px;
      border-radius: 50%;
      background: var(--jj-fg-muted);
    }
    .dot.added { background: var(--jj-added-fg); }
    .dot.deleted { background: var(--jj-removed-fg); }
    .dot.modified { background: var(--jj-accent); }
    .dot.renamed { background: var(--jj-fg-muted); }
    .check {
      color: var(--jj-added-fg);
      font-size: 10px;
      margin-left: auto;
    }
  `;

  @property({ attribute: false }) walkthrough: Walkthrough | null = null;
  @property({ attribute: false }) files: FilePatch[] = [];
  @property({ attribute: false }) viewed: ReadonlySet<string> = new Set();
  /** -1 = overview. */
  @property({ type: Number }) current = -1;

  private pick(index: number) {
    this.dispatchEvent(new CustomEvent<number>('step-selected', { detail: index }));
  }

  /** Unique file paths referenced by a step's hunk ids, in first-mention order. */
  private stepPaths(hunkIds: string[]): string[] {
    const paths: string[] = [];
    for (const id of hunkIds) {
      const path = id.slice(0, id.lastIndexOf('#'));
      if (!paths.includes(path)) {
        paths.push(path);
      }
    }
    return paths;
  }

  protected override render() {
    const walkthrough = this.walkthrough;
    if (!walkthrough) {
      return nothing;
    }
    const byPath = new Map(this.files.map((file) => [file.path, file]));
    return html`
      <button class=${this.current === -1 ? 'current' : ''} @click=${() => this.pick(-1)}>
        <span class="step-head">
          <span class="index">◎</span>
          <span class="title">Overview</span>
        </span>
      </button>
      ${walkthrough.steps.map((step, index) => {
        const paths = this.stepPaths(step.hunkIds);
        const done = paths.length > 0 && paths.every((path) => this.viewed.has(path));
        return html`
          <button
            class=${this.current === index ? 'current' : ''}
            @click=${() => this.pick(index)}
          >
            <span class="step-head">
              <span class="index ${done ? 'done' : ''}">${done ? '✓' : index + 1}</span>
              <span class="title">${step.title}</span>
            </span>
            <ul>
              ${paths.map((path) => {
                const file = byPath.get(path);
                return html`<li class=${this.viewed.has(path) ? 'viewed' : ''}>
                  <span class="dot ${file?.status ?? ''}"></span>
                  <span class="name">${path.slice(path.lastIndexOf('/') + 1)}</span>
                  ${this.viewed.has(path) ? html`<span class="check">✓</span>` : nothing}
                </li>`;
              })}
            </ul>
          </button>
        `;
      })}
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-walkthrough-panel': WalkthroughPanel;
  }
}
