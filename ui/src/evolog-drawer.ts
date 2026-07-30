import { css, html, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

import type { ChangeVersion } from './ipc.js';
import { OverlayElement, overlayChrome, panelButton, panelHeader } from './overlay.js';
import { relativeTime } from './time.js';

/**
 * Every version a change has been, and an interdiff between any two of them.
 *
 * jj keeps the whole evolution of a change — each `describe`, each amend, each
 * rebase leaves the previous commit addressable by id. Git has no equivalent
 * (a rewritten commit is garbage the moment nothing points at it), and jjdiff
 * already fetched this list for "what changed since I reviewed"; this exposes
 * the rest of it.
 *
 * The two-column A/B selection is the wiki page-history idiom rather than an
 * invention: A is the older side, B the newer, and the radios that would invert
 * that are disabled rather than silently reordered — a comparison whose
 * direction flips under you is worse than one you cannot express.
 *
 * Shadow DOM: leaf widget, no cross-boundary selection (DESIGN.md §6).
 */
@customElement('jj-evolog-drawer')
export class EvologDrawer extends OverlayElement {
  static override styles = [
    overlayChrome,
    panelHeader,
    panelButton,
    css`
      .panel {
        display: flex;
        flex-direction: column;
        width: min(720px, 92vw);
        max-height: 78vh;
        background: var(--jj-bg-panel);
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-lg, 0px);
        box-shadow: var(--jj-shadow-pop);
        overflow: hidden;
        animation: pop var(--jj-t-3, 260ms) var(--jj-ease-pop, ease-out);
      }
      .list {
        overflow-y: auto;
        border-top: 1px solid var(--jj-border);
        font-family: var(--jj-sans);
      }
      .version {
        display: grid;
        grid-template-columns: 26px 26px minmax(0, 1fr) auto;
        align-items: center;
        gap: 10px;
        padding: 9px 20px;
        color: var(--jj-fg);
        box-shadow: inset 0 -1px 0 var(--jj-border);
        transition: background var(--jj-t-1, 120ms) ease-out;
      }
      .version:hover {
        background: var(--jj-surface);
      }
      .version.picked {
        background: var(--jj-surface-2);
      }
      input[type='radio'] {
        accent-color: var(--jj-accent);
        margin: 0;
        cursor: pointer;
      }
      input[type='radio']:disabled {
        opacity: 0.25;
        cursor: default;
      }
      .subject {
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
      }
      .subject strong {
        font-weight: 550;
        font-size: var(--jj-text-base, 13px);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .subject strong.none {
        color: var(--jj-fg-faint);
        font-weight: 450;
        font-style: italic;
      }
      .meta {
        display: flex;
        align-items: center;
        gap: 8px;
        font-size: var(--jj-text-xs, 11px);
        color: var(--jj-fg-muted);
      }
      code {
        font-family: var(--jj-mono);
        font-size: var(--jj-text-xs, 11px);
        color: var(--jj-fg-muted);
      }
      .current {
        font-size: var(--jj-text-xs, 11px);
        color: var(--jj-accent);
        border: 1px solid var(--jj-accent);
        border-radius: var(--jj-r-pill, 999px);
        padding: 1px 8px;
      }
      .cols {
        display: grid;
        grid-template-columns: 26px 26px minmax(0, 1fr);
        gap: 10px;
        padding: 0 20px 8px;
        font-family: var(--jj-sans);
        font-size: 10.5px;
        font-weight: 650;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--jj-fg-faint);
      }
      footer {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 12px 20px;
        border-top: 1px solid var(--jj-border);
      }
      .none-yet {
        padding: 22px 20px;
        font-family: var(--jj-sans);
        font-size: var(--jj-text-base, 13px);
        color: var(--jj-fg-muted);
      }
    `,
  ];

  /** Newest first; entry 0 is the change as it stands now. */
  @property({ attribute: false }) versions: ChangeVersion[] = [];
  /** Shown in the heading so the drawer names what it is listing. */
  @property() changeId = '';
  @property({ type: Boolean }) loading = false;

  /** Indices into `versions`. A is the older side, so `from` > `to`. */
  @state() private from = 1;
  @state() private to = 0;

  protected override dismiss() {
    this.dispatchEvent(new Event('close'));
  }

  private compare() {
    const from = this.versions[this.from];
    const to = this.versions[this.to];
    if (!from || !to) return;
    this.dispatchEvent(
      new CustomEvent('compare-versions', {
        detail: { from: from.commitId, to: to.commitId },
      }),
    );
  }

  protected override render() {
    const versions = this.versions;
    const ready = versions.length > 1 && this.from > this.to;
    return html`<div class="panel">
      <header>
        <h2>Versions</h2>
        ${this.changeId ? html`<code>${this.changeId.slice(0, 8)}</code>` : nothing}
        <span class="hint">
          ${this.loading
            ? 'Reading the evolution log…'
            : versions.length === 1
              ? 'One version — this change has not been rewritten.'
              : `${versions.length} versions · pick two, then compare`}
        </span>
      </header>
      ${versions.length > 1
        ? html`<div class="cols"><span>Old</span><span>New</span><span></span></div>`
        : nothing}
      ${versions.length === 0
        ? html`<div class="none-yet">
            ${this.loading ? 'Loading…' : 'No recorded versions for this change.'}
          </div>`
        : html`<div class="list">
            ${versions.map((version, index) => {
              const subject = version.description.split('\n')[0]?.trim() ?? '';
              // A div, not a label: a label wrapping both radios targets its
              // first control, so clicking anywhere in the row would silently
              // move the older end no matter which column was aimed at.
              return html`<div
                class="version ${index === this.from || index === this.to ? 'picked' : ''}"
              >
                <input
                  type="radio"
                  name="from"
                  title="Compare from this version"
                  aria-label=${`Compare from ${subject || 'this version'}`}
                  .checked=${index === this.from}
                  ?disabled=${index <= this.to}
                  @change=${() => (this.from = index)}
                />
                <input
                  type="radio"
                  name="to"
                  title="Compare to this version"
                  aria-label=${`Compare to ${subject || 'this version'}`}
                  .checked=${index === this.to}
                  ?disabled=${index >= this.from}
                  @change=${() => (this.to = index)}
                />
                <span class="subject">
                  ${subject
                    ? html`<strong>${subject}</strong>`
                    : html`<strong class="none">(no description set)</strong>`}
                  <span class="meta">
                    <code>${version.commitId.slice(0, 12)}</code>
                    <span>${relativeTime(version.timestamp, true)}</span>
                  </span>
                </span>
                ${index === 0 ? html`<span class="current">current</span>` : nothing}
              </div>`;
            })}
          </div>`}
      <footer>
        <span class="hint">
          ${ready
            ? 'Rebase noise is excluded — jj diffs the changes, not the commits.'
            : 'Nothing to compare.'}
        </span>
        <span class="spacer"></span>
        <button class="btn" @click=${this.dismiss}>Close</button>
        <button class="btn primary" ?disabled=${!ready} @click=${this.compare}>Compare</button>
      </footer>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-evolog-drawer': EvologDrawer;
  }
}
