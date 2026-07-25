import { css, html, LitElement, nothing } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { repeat } from 'lit/directives/repeat.js';

import {
  getDiff,
  getRepoState,
  onRepoChanged,
  type Change,
  type FilePatch,
  type RepoState,
} from './ipc.js';
import './patch-view.js';

/** App chrome (shadow DOM is fine here — no text the user selects across boundaries). */
@customElement('jj-app')
export class App extends LitElement {
  static override styles = css`
    :host {
      display: grid;
      grid-template-columns: 300px 1fr;
      grid-template-rows: auto 1fr;
      height: 100%;
    }
    header {
      grid-column: 1 / -1;
      display: flex;
      align-items: baseline;
      gap: 10px;
      padding: 8px 14px;
      border-bottom: 1px solid var(--jj-border);
      background: var(--jj-bg-panel);
    }
    header .root {
      font-weight: 600;
    }
    header .version {
      color: var(--jj-fg-muted);
      font-size: 11px;
    }
    aside {
      border-right: 1px solid var(--jj-border);
      overflow-y: auto;
      padding: 8px;
    }
    main {
      overflow-y: auto;
      padding: 12px 14px;
    }
    .change {
      display: block;
      width: 100%;
      text-align: left;
      border: 1px solid transparent;
      border-radius: 6px;
      background: none;
      color: var(--jj-fg);
      font: inherit;
      padding: 6px 8px;
      cursor: pointer;
    }
    .change:hover {
      background: var(--jj-bg-panel);
    }
    .change.selected {
      border-color: var(--jj-accent);
      background: var(--jj-bg-panel);
    }
    .change .id {
      font-family: var(--jj-mono);
      color: var(--jj-accent);
      font-size: 11px;
    }
    .change .desc {
      display: block;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .change .desc.empty-desc {
      color: var(--jj-fg-muted);
      font-style: italic;
    }
    .badge {
      font-size: 10px;
      border-radius: 4px;
      padding: 1px 5px;
      margin-left: 6px;
      background: var(--jj-hunk-bg);
      color: var(--jj-fg-muted);
    }
    .error {
      grid-column: 1 / -1;
      padding: 24px;
      color: var(--jj-removed-fg);
      font-family: var(--jj-mono);
      white-space: pre-wrap;
    }
  `;

  @state() private repo: RepoState | null = null;
  @state() private error: string | null = null;
  @state() private selected: string | null = null; // change id; null = working copy
  @state() private files: FilePatch[] = [];

  private unlisten: (() => void) | null = null;

  override connectedCallback() {
    super.connectedCallback();
    void this.refresh();
    void onRepoChanged(() => void this.refresh()).then((unlisten) => {
      this.unlisten = unlisten;
    });
  }

  override disconnectedCallback() {
    this.unlisten?.();
    super.disconnectedCallback();
  }

  private async refresh() {
    try {
      this.repo = await getRepoState();
      this.error = null;
      await this.loadDiff();
    } catch (error) {
      this.error = String(error);
    }
  }

  private async loadDiff() {
    const revset =
      this.selected && !this.isWorkingCopy(this.selected) ? this.selected : undefined;
    this.files = await getDiff(revset);
  }

  private isWorkingCopy(changeId: string) {
    return this.repo?.workingCopy.changeId === changeId;
  }

  private select(change: Change) {
    this.selected = change.changeId;
    void this.loadDiff();
  }

  protected override render() {
    if (this.error) {
      return html`<div class="error">${this.error}</div>`;
    }
    if (!this.repo) {
      return nothing;
    }
    const selected = this.selected ?? this.repo.workingCopy.changeId;
    return html`
      <header>
        <span class="root">${this.repo.root}</span>
        <span class="version">jj ${this.repo.jjVersion}</span>
      </header>
      <aside>
        ${repeat(
          this.repo.stack,
          (change) => change.changeId,
          (change) => html`
            <button
              class="change ${change.changeId === selected ? 'selected' : ''}"
              @click=${() => this.select(change)}
            >
              <span class="id">${change.changeId.slice(0, 8)}</span>
              ${change.workingCopy ? html`<span class="badge">@</span>` : nothing}
              ${change.conflict ? html`<span class="badge">conflict</span>` : nothing}
              ${change.bookmarks.map((b) => html`<span class="badge">${b}</span>`)}
              <span class="desc ${change.description ? '' : 'empty-desc'}">
                ${change.description.split('\n')[0] || '(no description)'}
              </span>
            </button>
          `,
        )}
      </aside>
      <main>
        <jj-patch-view .files=${this.files}></jj-patch-view>
      </main>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-app': App;
  }
}
