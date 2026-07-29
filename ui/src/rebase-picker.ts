import { css, html, LitElement, nothing } from 'lit';
import { customElement, property, query, state } from 'lit/decorators.js';

import type { Change } from './ipc.js';
import { relativeTime } from './time.js';

/** How much of the graph a rebase moves. jj's `-r` / `-s` / `-b`. */
export type RebaseMode = 'revision' | 'source' | 'branch';

const MODES: { mode: RebaseMode; flag: string; label: string; detail: string }[] = [
  {
    mode: 'source',
    flag: '-s',
    label: 'This change and its descendants',
    detail: 'Everything built on top comes along. The usual answer.',
  },
  {
    mode: 'revision',
    flag: '-r',
    label: 'This change alone',
    detail: 'Its children are rebased onto its old parent, closing the gap it leaves.',
  },
  {
    mode: 'branch',
    flag: '-b',
    label: 'Its whole branch',
    detail: 'Everything back to where it diverged from the destination.',
  },
];

/**
 * Where to rebase, chosen from the graph rather than typed.
 *
 * A revset prompt was the first cut and asked the wrong thing: the destination
 * is almost always a commit already on screen, and naming it means reading an
 * id off the graph and retyping it — a transcription step whose only possible
 * contribution is an error. The free-form field is still here, below the list,
 * because `trunk()` and `main@origin` are real answers that no list of commits
 * contains.
 *
 * Destinations that would form a cycle — the change itself, and anything
 * descended from it — are not offered. jj would refuse them anyway; refusing
 * here means the refusal arrives before the confirmation rather than after it.
 *
 * Shadow DOM: leaf widget, no cross-boundary selection (DESIGN.md §6).
 */
@customElement('jj-rebase-picker')
export class RebasePicker extends LitElement {
  static override styles = css`
    :host {
      position: fixed;
      inset: 0;
      display: flex;
      justify-content: center;
      align-items: flex-start;
      padding-top: 9vh;
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
    .panel {
      display: flex;
      flex-direction: column;
      width: min(680px, 92vw);
      max-height: 78vh;
      background: var(--jj-bg-panel);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 0px);
      box-shadow: var(--jj-shadow-pop);
      overflow: hidden;
      font-family: var(--jj-sans);
      animation: pop var(--jj-t-3, 260ms) var(--jj-ease-pop, ease-out);
    }
    @keyframes pop {
      from {
        opacity: 0;
        transform: translateY(-10px) scale(0.965);
      }
    }
    header {
      display: flex;
      align-items: baseline;
      gap: 10px;
      padding: 16px 20px 12px;
    }
    h2 {
      margin: 0;
      font-size: var(--jj-text-title, 20px);
      font-weight: 650;
      letter-spacing: -0.02em;
      color: var(--jj-fg);
    }
    .subject {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-size: var(--jj-text-sm, 11.5px);
      color: var(--jj-fg-muted);
    }
    .modes {
      display: flex;
      flex-direction: column;
      gap: 2px;
      padding: 0 20px 12px;
    }
    .mode {
      display: grid;
      grid-template-columns: 18px auto minmax(0, 1fr);
      align-items: baseline;
      gap: 8px;
      padding: 5px 8px;
      border-radius: var(--jj-r-sm, 0px);
      cursor: pointer;
      color: var(--jj-fg);
      font-size: var(--jj-text-base, 13px);
    }
    .mode:hover {
      background: var(--jj-surface);
    }
    .mode code {
      font-family: var(--jj-mono);
      font-size: var(--jj-text-xs, 11px);
      color: var(--jj-fg-muted);
    }
    .mode .detail {
      font-size: var(--jj-text-xs, 11px);
      color: var(--jj-fg-muted);
    }
    input[type='radio'] {
      accent-color: var(--jj-accent);
      margin: 0;
      justify-self: center;
    }
    .filter {
      padding: 0 20px 10px;
    }
    input[type='text'] {
      width: 100%;
      box-sizing: border-box;
      padding: 7px 11px;
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-sm, 0px);
      background: var(--jj-surface);
      color: var(--jj-fg);
      font-family: var(--jj-sans);
      font-size: var(--jj-text-base, 13px);
      outline: none;
    }
    input.revset {
      font-family: var(--jj-mono);
      font-size: var(--jj-text-sm, 11.5px);
    }
    input:focus {
      border-color: var(--jj-accent);
      box-shadow: var(--jj-focus-ring);
    }
    .list {
      flex: 1;
      min-height: 0;
      overflow-y: auto;
      border-top: 1px solid var(--jj-border);
    }
    .target {
      display: grid;
      grid-template-columns: auto minmax(0, 1fr) auto;
      align-items: center;
      gap: 10px;
      width: 100%;
      box-sizing: border-box;
      padding: 8px 20px;
      border: 0;
      background: transparent;
      color: var(--jj-fg);
      text-align: left;
      cursor: pointer;
      box-shadow: inset 0 -1px 0 var(--jj-border);
      font: inherit;
    }
    .target:hover {
      background: var(--jj-surface);
    }
    .target.active {
      background: var(--jj-surface-2);
      box-shadow: inset 3px 0 0 var(--jj-accent), inset 0 -1px 0 var(--jj-border);
    }
    .target code {
      font-family: var(--jj-mono);
      font-size: var(--jj-text-xs, 11px);
      color: var(--jj-fg-muted);
    }
    .target .what {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-size: var(--jj-text-base, 13px);
    }
    .target .what.none {
      color: var(--jj-fg-faint);
      font-style: italic;
    }
    .tags {
      display: flex;
      align-items: center;
      gap: 6px;
      white-space: nowrap;
    }
    .tag {
      font-size: 10.5px;
      font-weight: 600;
      border-radius: var(--jj-r-pill, 999px);
      padding: 1.5px 8px;
      background: var(--jj-surface-2);
      color: var(--jj-fg-muted);
    }
    .tag.bookmark {
      background: var(--jj-accent-soft);
      color: var(--jj-accent);
    }
    .age {
      font-size: var(--jj-text-xs, 11px);
      color: var(--jj-fg-faint);
    }
    .none-yet {
      padding: 22px 20px;
      font-size: var(--jj-text-base, 13px);
      color: var(--jj-fg-muted);
    }
    footer {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 12px 20px;
      border-top: 1px solid var(--jj-border);
    }
    .hint {
      font-size: var(--jj-text-sm, 11.5px);
      color: var(--jj-fg-muted);
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .hint code {
      font-family: var(--jj-mono);
      color: var(--jj-fg);
    }
    .spacer {
      flex: 1;
    }
    button.action {
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
    button.action:hover:not(:disabled) {
      border-color: var(--jj-border-strong);
    }
    button.action.primary {
      background: var(--jj-primary);
      color: var(--jj-primary-fg);
      border-color: transparent;
    }
    button.action:disabled {
      opacity: 0.45;
      cursor: default;
    }
    @media (prefers-reduced-motion: reduce) {
      :host,
      .panel {
        animation: none;
      }
    }
  `;

  /** Candidates, in graph order (newest first). */
  @property({ attribute: false }) changes: Change[] = [];
  /** The change being moved. */
  @property({ attribute: false }) subject: Change | null = null;

  @state() private mode: RebaseMode = 'source';
  @state() private filter = '';
  /** Free-form revset; when non-empty it wins over the highlighted row. */
  @state() private revset = '';
  /** Index into `candidates`. */
  @state() private active = 0;

  @query('input.filter-input') private filterField?: HTMLInputElement;

  override connectedCallback() {
    super.connectedCallback();
    this.addEventListener('click', this.onBackdrop);
    window.addEventListener('keydown', this.onEscape);
  }

  override disconnectedCallback() {
    this.removeEventListener('click', this.onBackdrop);
    window.removeEventListener('keydown', this.onEscape);
    super.disconnectedCallback();
  }

  override firstUpdated() {
    this.filterField?.focus();
  }

  private onBackdrop = (event: MouseEvent) => {
    // Retargeting: an event from inside the shadow root reports the host as its
    // target, so only the composed path can tell the scrim from the panel.
    if (event.composedPath()[0] === this) this.dispatchEvent(new Event('close'));
  };

  /**
   * Escape when focus has left the panel — a click on the scrim moves it to the
   * body, past the panel's own handler. Everything else is handled inside,
   * where propagation stops.
   */
  private onEscape = (event: KeyboardEvent) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    this.dispatchEvent(new Event('close'));
  };

  /**
   * Bound to the panel, not the window, and it stops propagation: the app's
   * global handler treats bare letters as review shortcuts, and it cannot see
   * that a filter box two shadow roots down has focus (the event is retargeted
   * to this host on the way out). Typing `j` in the filter would otherwise
   * scroll the diff behind the dialog.
   */
  private onKey = (event: KeyboardEvent) => {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      this.dispatchEvent(new Event('close'));
      return;
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const count = this.candidates.length;
      if (count === 0) return;
      const step = event.key === 'ArrowDown' ? 1 : -1;
      this.active = (this.active + step + count) % count;
      void this.updateComplete.then(() =>
        this.renderRoot.querySelector('.target.active')?.scrollIntoView({ block: 'nearest' }),
      );
      return;
    }
    if (event.key === 'Enter' && this.destination) {
      event.preventDefault();
      this.confirm();
    }
  };

  /**
   * Everything the change could legally move onto.
   *
   * Its own descendants are excluded because a commit cannot be its own
   * ancestor; walking forward from the subject is how they are found, since
   * `Change` records parents and nothing else.
   */
  private get candidates(): Change[] {
    const subject = this.subject;
    if (!subject) return [];
    const barred = new Set<string>([subject.commitId]);
    // jj log order is reverse-topological — children before parents — so
    // walking it backwards visits every parent before its children, and one
    // pass is enough to reach the whole descendant set.
    for (let index = this.changes.length - 1; index >= 0; index--) {
      const change = this.changes[index]!;
      if (change.parents.some((parent) => barred.has(parent))) barred.add(change.commitId);
    }
    const needle = this.filter.trim().toLowerCase();
    return this.changes.filter((change) => {
      if (barred.has(change.commitId)) return false;
      if (!needle) return true;
      return (
        change.changeId.toLowerCase().includes(needle) ||
        change.commitId.toLowerCase().includes(needle) ||
        change.description.toLowerCase().includes(needle) ||
        change.bookmarks.some((bookmark) => bookmark.toLowerCase().includes(needle))
      );
    });
  }

  /** What would be passed to `-d`: the typed revset, else the highlighted row. */
  private get destination(): string {
    const typed = this.revset.trim();
    if (typed) return typed;
    return this.candidates[this.active]?.changeId ?? '';
  }

  /**
   * The destination as the command preview should show it. A change id from
   * the list is abbreviated the way jj itself prints one — the full 32
   * characters are what gets passed, but they are unreadable in a line meant
   * to be read, and a truncation with an ellipsis looks like a mistake.
   */
  private get destinationLabel(): string {
    if (this.revset.trim()) return this.revset.trim();
    return this.candidates[this.active]?.changeId.slice(0, 8) ?? '';
  }

  private confirm() {
    const destination = this.destination;
    if (!destination) return;
    this.dispatchEvent(
      new CustomEvent('rebase', { detail: { mode: this.mode, destination } }),
    );
  }

  protected override render() {
    const subject = this.subject;
    const candidates = this.candidates;
    const typed = this.revset.trim();
    const flag = MODES.find((entry) => entry.mode === this.mode)?.flag ?? '-s';
    return html`<div class="panel" @keydown=${this.onKey}>
      <header>
        <h2>Rebase</h2>
        ${subject
          ? html`<span class="subject"
              >${subject.description.split('\n')[0] || subject.changeId.slice(0, 8)}</span
            >`
          : nothing}
      </header>

      <div class="modes">
        ${MODES.map(
          (entry) => html`<label class="mode">
            <input
              type="radio"
              name="mode"
              .checked=${this.mode === entry.mode}
              @change=${() => (this.mode = entry.mode)}
            />
            <code>${entry.flag}</code>
            <span>
              ${entry.label} <span class="detail">— ${entry.detail}</span>
            </span>
          </label>`,
        )}
      </div>

      <div class="filter">
        <input
          class="filter-input"
          type="text"
          placeholder="Filter destinations by description, bookmark or id…"
          .value=${this.filter}
          @input=${(event: Event) => {
            this.filter = (event.target as HTMLInputElement).value;
            this.active = 0;
          }}
        />
      </div>

      ${candidates.length === 0
        ? html`<div class="none-yet">
            ${this.filter.trim()
              ? 'No change here matches that. A revset below still works.'
              : 'Nothing in the graph can hold this change — widen the log scope, or name a revset below.'}
          </div>`
        : html`<div class="list">
            ${candidates.map((change, index) => {
              const subjectLine = change.description.split('\n')[0]?.trim() ?? '';
              return html`<button
                class="target ${!typed && index === this.active ? 'active' : ''}"
                @click=${() => {
                  this.active = index;
                  // Clicking a row is a choice; a half-typed revset would
                  // otherwise silently override it.
                  this.revset = '';
                }}
                @dblclick=${() => {
                  this.active = index;
                  this.revset = '';
                  this.confirm();
                }}
              >
                <code>${change.changeId.slice(0, 8)}</code>
                <span class="what ${subjectLine ? '' : 'none'}"
                  >${subjectLine || '(no description set)'}</span
                >
                <span class="tags">
                  ${change.bookmarks.map(
                    (bookmark) => html`<span class="tag bookmark">${bookmark}</span>`,
                  )}
                  ${change.immutable ? html`<span class="tag">immutable</span>` : nothing}
                  ${change.conflict ? html`<span class="tag">conflict</span>` : nothing}
                  <span class="age">${relativeTime(change.committer.timestamp)}</span>
                </span>
              </button>`;
            })}
          </div>`}

      <div class="filter" style="padding-top: 10px">
        <input
          class="revset"
          type="text"
          placeholder="…or a revset: trunk(), main@origin, @--"
          .value=${this.revset}
          @input=${(event: Event) => (this.revset = (event.target as HTMLInputElement).value)}
        />
      </div>

      <footer>
        <span class="hint">
          ${this.destination
            ? html`<code
                >jj rebase ${flag} ${subject?.changeId.slice(0, 8) ?? ''} -d
                ${this.destinationLabel}</code
              >`
            : 'Pick a destination.'}
        </span>
        <span class="spacer"></span>
        <button class="action" @click=${() => this.dispatchEvent(new Event('close'))}>Cancel</button>
        <button class="action primary" ?disabled=${!this.destination} @click=${this.confirm}>
          Rebase
        </button>
      </footer>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-rebase-picker': RebasePicker;
  }
}
