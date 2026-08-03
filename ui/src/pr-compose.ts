import { css, html, nothing } from 'lit';
import { customElement, property, query, state } from 'lit/decorators.js';

import { OverlayElement, overlayChrome, panelButton, panelHeader } from './overlay.js';

/** What the dialog collected, once someone has pressed the button. */
export interface ComposeRequest {
  title: string;
  body: string;
  base: string;
  /** The branch to propose. May not exist yet — see `headExists`. */
  head: string;
  draft: boolean;
}

/**
 * Open a pull request without leaving the app.
 *
 * A dialog rather than `gh pr create --fill`, because the two things it collects
 * are the two a reviewer cannot take back: the **base**, which decides what the
 * proposal even contains, and the **title and body**, which are the first thing
 * anyone reads. `--fill` would infer both from the change description, and a
 * description written for a commit is frequently not the one you would open a
 * proposal with. Everything else the forge already knows.
 *
 * It also does not create anything. It collects, and hands the answer up: the
 * push is a jj mutation with its own narration and an operation to undo, and
 * opening a proposal is a public act that is neither — `App` keeps them apart
 * and in that order.
 *
 * Shadow DOM: leaf widget, no cross-boundary selection (DESIGN.md §6).
 */
@customElement('jj-pr-compose')
export class PrCompose extends OverlayElement {
  static override styles = [
    overlayChrome,
    panelHeader,
    panelButton,
    css`
      .panel {
        display: flex;
        flex-direction: column;
        width: min(620px, 92vw);
        max-height: 84vh;
        background: var(--jj-bg-panel);
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-lg, 0px);
        box-shadow: var(--jj-shadow-pop);
        overflow: hidden;
        font-family: var(--jj-sans);
        animation: pop var(--jj-t-3, 260ms) var(--jj-ease-pop, ease-out);
      }
      .fields {
        display: flex;
        flex-direction: column;
        gap: 12px;
        padding: 4px 20px 16px;
        overflow-y: auto;
      }
      label {
        display: flex;
        flex-direction: column;
        gap: 5px;
        font-size: var(--jj-text-sm, 11.5px);
        font-weight: 600;
        color: var(--jj-fg-muted);
      }
      /* The two refs read as one statement, so they sit on one line with the
         direction between them rather than as two unrelated fields. */
      .refs {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
        align-items: end;
        gap: 10px;
      }
      /* Anchors the chevron below, which has to hang over the select. */
      .refs label {
        position: relative;
      }
      /* Our chevron, not the platform's.
         The select is the same object as the input beside it — same box, same
         type, one statement read left to right — and macOS draws a menulist
         with its own metrics and a double stepper glyph, so the two sat at
         different heights with different furniture. Two rotated borders rather
         than an SVG: it is one glyph, and drawn this way it takes its colour
         from a theme token instead of freezing a stroke into a data URI that
         then has to be maintained per palette.
         On the label rather than the select, because a select's pseudo-elements
         are not reliably rendered. */
      .refs label:has(select)::after {
        content: '';
        position: absolute;
        right: 12px;
        bottom: 15px;
        width: 5px;
        height: 5px;
        border-right: 1.5px solid var(--jj-fg-faint);
        border-bottom: 1.5px solid var(--jj-fg-faint);
        transform: rotate(45deg);
        pointer-events: none;
      }
      .into {
        padding-bottom: 8px;
        font-size: var(--jj-text-sm, 11.5px);
        color: var(--jj-fg-faint);
      }
      input,
      select,
      textarea {
        width: 100%;
        box-sizing: border-box;
        padding: 7px 10px;
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-sm, 0px);
        background: var(--jj-surface);
        color: var(--jj-fg);
        font-family: var(--jj-sans);
        font-size: var(--jj-text-base, 13px);
        font-weight: 400;
        outline: none;
      }
      /* One height for every single-line field, stated rather than derived.
         Padding and border are already shared, but the content box is not: the
         two refs are 12px mono and the title 13px sans, and a select is a
         native widget that sizes itself. Three fields, three heights — and
         because the row bottom-aligns, an unequal Base and Head dragged their
         labels apart too. */
      input:not([type='checkbox']),
      select {
        height: 34px;
      }
      /* Off the platform widget, so the box above is the box that renders. */
      select {
        appearance: none;
        -webkit-appearance: none;
        padding-right: 26px;
      }
      /* Refs are identifiers, and an identifier you are about to publish is worth
         reading character by character. */
      input.ref,
      select {
        font-family: var(--jj-mono);
        font-size: var(--jj-text-sm, 11.5px);
      }
      /* Square, restating the bare textarea rule in theme.css. A multi-line
         field is a surface, not a control, and the small radius token is the
         pill — on a 150px box it draws a lozenge whose corners eat the first
         and last lines. The rule is written down in the light DOM and cannot
         reach in here: custom properties cross a shadow boundary, selectors do
         not, so a shadow root with a textarea has to say it again. */
      textarea {
        min-height: 150px;
        resize: vertical;
        line-height: 1.55;
        border-radius: 0;
      }
      input:focus,
      select:focus,
      textarea:focus {
        border-color: var(--jj-accent);
        box-shadow: var(--jj-focus-ring);
      }
      .note {
        font-weight: 400;
        font-size: var(--jj-text-xs, 11px);
        color: var(--jj-fg-faint);
      }
      .note .ahead {
        font-family: var(--jj-mono);
        color: var(--jj-ref);
      }
      .draft {
        flex-direction: row;
        align-items: center;
        gap: 8px;
        font-weight: 550;
        color: var(--jj-fg);
        cursor: pointer;
      }
      .draft input {
        width: auto;
        accent-color: var(--jj-accent);
      }
      footer {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 12px 20px;
        border-top: 1px solid var(--jj-border);
      }
      .hint {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .hint code {
        font-family: var(--jj-mono);
        color: var(--jj-fg);
      }
    `,
  ];

  /** What this forge calls a proposal, for the heading and the button. */
  @property() noun = 'pull request';
  /** Branches offered as a base — remote bookmarks, default first. */
  @property({ attribute: false }) bases: readonly string[] = [];
  /** The bookmark already on the change, or `''` when it has none. */
  @property() headBookmark = '';
  /** Commits the head bookmark has that the remote does not. */
  @property({ type: Number }) ahead = 0;
  /** True while the push and the create are in flight. */
  @property({ type: Boolean }) busy = false;

  /**
   * What the four editable fields start as.
   *
   * Seeds rather than bindings, and the distinction is the whole reason they are
   * named this way: the values below are `@state`, owned by this element from
   * the moment it mounts. Bound directly from the parent they would be
   * reassigned on every one of *its* renders — and the parent re-renders on a
   * repo watcher tick, a window focus, a proposal refresh — so a description
   * being typed would be silently reverted by something happening elsewhere in
   * the app.
   */
  @property() seedTitle = '';
  @property() seedBody = '';
  @property() seedBase = '';
  @property() seedHead = '';

  /**
   * `subject`, not `title`: `title` is a property of every `HTMLElement`, so a
   * Lit property by that name makes the panel's tooltip and the proposal's title
   * one value — and the browser wins.
   */
  @state() private subject = '';
  @state() private bodyText = '';
  @state() private base = '';
  @state() private head = '';
  @state() private draft = false;

  @query('input.subject') private subjectField?: HTMLInputElement;

  override firstUpdated() {
    this.subject = this.seedTitle;
    this.bodyText = this.seedBody;
    this.base = this.seedBase;
    this.head = this.seedHead;
    this.subjectField?.focus();
    this.subjectField?.select();
  }

  protected override dismiss() {
    if (this.busy) return;
    this.dispatchEvent(new Event('close'));
  }

  /** Everything the forge needs; blank until both refs and a title are there. */
  private get ready(): boolean {
    return !this.busy && !!this.subject.trim() && !!this.base.trim() && !!this.head.trim();
  }

  private submit() {
    if (!this.ready) return;
    this.dispatchEvent(
      new CustomEvent<ComposeRequest>('create', {
        detail: {
          title: this.subject.trim(),
          body: this.bodyText,
          base: this.base.trim(),
          head: this.head.trim(),
          draft: this.draft,
        },
      }),
    );
  }

  /**
   * The panel's keys, stopped here for the reason every overlay stops them:
   * `App.onGlobalKey` reads `event.target`, which by the time this leaves the
   * shadow root is the host, so a `j` typed into the title would scroll the diff
   * behind the dialog.
   *
   * Enter does *not* submit. There is a textarea in this panel and Enter belongs
   * to it, and this is a dialog whose button publishes something — Cmd+Enter is
   * the deliberate version of the same keystroke, and it is written in the
   * footer rather than left to be discovered.
   */
  private onKey = (event: KeyboardEvent) => {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      this.dismiss();
      return;
    }
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      this.submit();
    }
  };

  protected override render() {
    // A head that is not the bookmark already on the change is one jjdiff will
    // create, and it says so before rather than after — a bookmark appearing in
    // the graph unannounced is the kind of thing you go looking for a cause of.
    const creating = this.head.trim() !== this.headBookmark;
    return html`<div class="panel" @keydown=${this.onKey}>
      <header>
        <h2>Open a ${this.noun}</h2>
        <span class="spacer"></span>
        <span class="hint">${this.ahead ? html`${this.ahead} to push` : nothing}</span>
      </header>
      <div class="fields">
        <div class="refs">
          <label>
            Base
            <!-- Selected state on the options, not a value binding on the
                 select: lit commits a property binding before it commits the
                 children, so setting the value against options that do not
                 exist yet leaves the field on whatever rendered first. -->
            <select
              ?disabled=${this.busy}
              @change=${(event: Event) => (this.base = (event.target as HTMLSelectElement).value)}
            >
              ${this.bases.map(
                (branch) =>
                  html`<option value=${branch} ?selected=${branch === this.base}>${branch}</option>`,
              )}
            </select>
          </label>
          <span class="into">←</span>
          <label>
            Head
            <input
              class="ref"
              .value=${this.head}
              ?disabled=${this.busy}
              placeholder="branch to propose"
              @input=${(event: Event) => (this.head = (event.target as HTMLInputElement).value)}
            />
          </label>
        </div>
        <span class="note">
          ${
            creating
              ? html`jjdiff will set the bookmark <code>${this.head.trim() || '…'}</code> on this
                change and push it.`
              : this.ahead
                ? html`Pushing <span class="ahead">${this.ahead}</span> commit${
                    this.ahead === 1 ? '' : 's'
                  } to this branch first.`
                : html`This branch is already on the remote.`
          }
        </span>
        <label>
          Title
          <input
            class="subject"
            .value=${this.subject}
            ?disabled=${this.busy}
            @input=${(event: Event) => (this.subject = (event.target as HTMLInputElement).value)}
          />
        </label>
        <label>
          Description
          <textarea
            .value=${this.bodyText}
            ?disabled=${this.busy}
            @input=${(event: Event) =>
              (this.bodyText = (event.target as HTMLTextAreaElement).value)}
          ></textarea>
        </label>
        <label class="draft">
          <input
            type="checkbox"
            .checked=${this.draft}
            ?disabled=${this.busy}
            @change=${(event: Event) => (this.draft = (event.target as HTMLInputElement).checked)}
          />
          Draft — open it, but do not ask anyone to look yet
        </label>
      </div>
      <footer>
        <span class="hint"><code>⌘↵</code> to open</span>
        <span class="spacer"></span>
        <button class="btn" ?disabled=${this.busy} @click=${this.dismiss}>Cancel</button>
        <button class="btn primary" ?disabled=${!this.ready} @click=${this.submit}>
          ${this.busy ? 'Opening…' : this.draft ? 'Create draft' : 'Create'}
        </button>
      </footer>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-pr-compose': PrCompose;
  }
}
