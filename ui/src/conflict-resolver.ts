import { css, html, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

import type { ConflictedContent, ConflictPiece, ConflictRegion } from './ipc.js';
import { OverlayElement, overlayChrome, panelButton, panelHeader } from './overlay.js';

/** Lines of agreed text shown either side of a region before eliding the rest. */
const CONTEXT = 3;

/**
 * Resolve a conflicted file region by region.
 *
 * M4 deferred this with a reason that still holds — a merge editor spawned from
 * a GUI without a TTY hangs more often than it works — but the reason was about
 * *spawning* one, not about resolving. jj materializes a conflict as marker
 * text, `crates/diff/src/conflict.rs` reads that back into sides, and picking
 * one is then a button rather than a careful deletion of six fences. Nothing is
 * spawned and nothing needs a terminal; the result goes back through
 * `jj resolve` with jjdiff as the merge tool.
 *
 * Every region must be settled before the file can be written. A partial
 * resolution is not a thing jj can store: it takes the merge tool's output at
 * face value, so leaving one region alone would write its fences into the file
 * and call the conflict resolved.
 *
 * The sides keep jj's own labels. jj names each side after the commit and
 * description it came from — the one real advantage its markers have over
 * git's bare `<<<<<<< HEAD` — and relabelling them "ours" and "theirs" would
 * throw away the only thing on screen that says *whose* change is whose.
 *
 * Shadow DOM: leaf widget, no cross-boundary selection (DESIGN.md §6).
 */
@customElement('jj-conflict-resolver')
export class ConflictResolver extends OverlayElement {
  static override styles = [
    overlayChrome,
    panelHeader,
    panelButton,
    css`
    /* Taller than the shared chrome's 9vh: a conflicted file is a list of
       regions to read, and every centimetre of panel is one less scroll. */
    :host {
      padding-top: 6vh;
    }
    .panel {
      display: flex;
      flex-direction: column;
      width: min(860px, 94vw);
      max-height: 86vh;
      background: var(--jj-bg-panel);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-lg, 0px);
      box-shadow: var(--jj-shadow-pop);
      overflow: hidden;
      font-family: var(--jj-sans);
      animation: pop var(--jj-t-3, 260ms) var(--jj-ease-pop, ease-out);
    }
    .subject {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-family: var(--jj-mono);
      font-size: var(--jj-text-sm, 11.5px);
      color: var(--jj-fg-muted);
    }
    .body {
      flex: 1;
      min-height: 0;
      overflow-y: auto;
      border-top: 1px solid var(--jj-border);
      padding: 4px 0 12px;
    }
    pre {
      margin: 0;
      font-family: var(--jj-mono);
      font-size: var(--jj-text-sm, 11.5px);
      line-height: 1.55;
      white-space: pre-wrap;
      word-break: break-word;
    }
    .context {
      padding: 4px 20px;
      color: var(--jj-fg-faint);
    }
    .elided {
      padding: 2px 20px;
      font-size: var(--jj-text-xs, 11px);
      color: var(--jj-fg-faint);
      font-style: italic;
    }
    .region {
      margin: 10px 16px;
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-md, 0px);
      background: var(--jj-surface);
      overflow: hidden;
    }
    .region.settled {
      border-color: color-mix(in srgb, var(--jj-added-fg) 40%, var(--jj-border));
    }
    .region-head {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 8px 12px;
      background: var(--jj-surface-2);
      font-size: var(--jj-text-sm, 11.5px);
      color: var(--jj-fg-muted);
    }
    .region-head .state {
      margin-left: auto;
      font-weight: 600;
    }
    .region-head .state.open {
      color: var(--jj-removed-fg);
    }
    .side {
      border-top: 1px solid var(--jj-border);
    }
    .side-head {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 5px 12px;
      font-size: var(--jj-text-xs, 11px);
      color: var(--jj-fg-muted);
    }
    .side-head .label {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .side pre {
      padding: 0 12px 6px 12px;
      color: var(--jj-fg);
    }
    .side.empty pre {
      color: var(--jj-fg-faint);
      font-style: italic;
    }
    .actions {
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
      padding: 8px 12px;
      border-top: 1px solid var(--jj-border);
    }
    .result {
      border-top: 1px solid var(--jj-border);
      padding: 8px 12px 10px;
    }
    .result label {
      display: block;
      font-size: var(--jj-text-xs, 11px);
      color: var(--jj-fg-muted);
      margin-bottom: 4px;
    }
    /* Square for the reason theme.css gives: a multi-line field is a surface,
       not a control, and the pill radius turns one into a lozenge. Restated
       here because that rule is a light-DOM selector and this is a shadow
       root. */
    textarea {
      width: 100%;
      box-sizing: border-box;
      min-height: 60px;
      resize: vertical;
      padding: 7px 9px;
      border: 1px solid var(--jj-border);
      border-radius: 0;
      background: var(--jj-bg-panel);
      color: var(--jj-fg);
      font-family: var(--jj-mono);
      font-size: var(--jj-text-sm, 11.5px);
      line-height: 1.55;
      outline: none;
    }
    textarea:focus {
      border-color: var(--jj-accent);
      box-shadow: var(--jj-focus-ring);
    }
    button.pick {
      font-size: var(--jj-text-xs, 11px);
      border: 1px solid var(--jj-border);
      border-radius: var(--jj-r-pill, 999px);
      background: var(--jj-bg-panel);
      color: var(--jj-fg);
      padding: 4px 11px;
      cursor: pointer;
    }
    button.pick:hover {
      border-color: var(--jj-border-strong);
    }
    button.pick.chosen {
      background: var(--jj-accent-soft);
      border-color: var(--jj-accent-line);
      color: var(--jj-accent);
      font-weight: 600;
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
    }
    .spacer {
      flex: 1;
    }
  `,
  ];

  @property() path = '';
  /** jj's description of the conflict's shape ("2-sided conflict"). */
  @property() shape = '';
  @property({ attribute: false }) content: ConflictedContent | null = null;
  /** True while the resolution is being written. */
  @property({ type: Boolean }) busy = false;

  /**
   * The settled regions, by index: the text each one resolves to, and which
   * button produced it. A region absent from this map is still open, which is
   * what the Resolve button waits on.
   */
  @state() private settled = new Map<number, { choice: string; lines: string[] }>();

  /** The backdrop and window Escape both arrive here, from `OverlayElement`. */
  protected override dismiss() {
    this.dispatchEvent(new Event('close'));
  }

  private get regions(): ConflictRegion[] {
    return (this.content?.pieces ?? []).filter(
      (piece): piece is Extract<ConflictPiece, { kind: 'conflict' }> => piece.kind === 'conflict',
    );
  }

  private choose(index: number, choice: string, lines: string[]) {
    const next = new Map(this.settled);
    next.set(index, { choice, lines });
    this.settled = next;
  }

  /**
   * A hand-edited region.
   *
   * An emptied box means "nothing here", not "one blank line" — the two are
   * indistinguishable in a textarea and only the first is ever meant. A side
   * that genuinely is a single blank line survives because it arrives through
   * [`choose`] with its own lines rather than through this.
   */
  private edit(index: number, value: string) {
    const next = new Map(this.settled);
    next.set(index, { choice: 'custom', lines: value === '' ? [] : value.split('\n') });
    this.settled = next;
  }

  /**
   * The resolved file.
   *
   * Regions contribute their settled text, everything else is verbatim — so a
   * file whose conflicts are all resolved to one side comes back byte for byte
   * as that side, including the trailing newline the parser recorded rather
   * than assumed. A region resolved to nothing contributes no lines at all,
   * which is how "neither side, delete it" is expressed.
   */
  private assemble(): string {
    const lines: string[] = [];
    for (const piece of this.content?.pieces ?? []) {
      if (piece.kind === 'text') {
        lines.push(...piece.lines);
        continue;
      }
      lines.push(...(this.settled.get(piece.index)?.lines ?? []));
    }
    const body = lines.join('\n');
    return body && this.content?.trailingNewline ? `${body}\n` : body;
  }

  private confirm() {
    if (this.settled.size < this.regions.length) return;
    this.dispatchEvent(
      new CustomEvent('resolve-conflict', { detail: { content: this.assemble() } }),
    );
  }

  private renderContext(lines: string[]) {
    if (lines.length <= CONTEXT * 2) {
      return html`<pre class="context">${lines.join('\n')}</pre>`;
    }
    const hidden = lines.length - CONTEXT * 2;
    return html`
      <pre class="context">${lines.slice(0, CONTEXT).join('\n')}</pre>
      <div class="elided">… ${hidden} unconflicted line${hidden === 1 ? '' : 's'} …</div>
      <pre class="context">${lines.slice(-CONTEXT).join('\n')}</pre>
    `;
  }

  private renderRegion(region: ConflictRegion) {
    const settled = this.settled.get(region.index);
    const sides = region.sides.map((side, position) => ({
      ...side,
      choice: `side-${position}`,
      button: `Side #${position + 1}`,
    }));
    const both = sides.flatMap((side) => side.lines);
    return html`<div class="region ${settled ? 'settled' : ''}">
      <div class="region-head">
        <span>${region.label || `Conflict ${region.index + 1}`}</span>
        <span class="state ${settled ? '' : 'open'}">
          ${settled ? (settled.choice === 'custom' ? 'edited' : 'settled') : 'unresolved'}
        </span>
      </div>

      ${sides.map(
        (side) => html`<div class="side ${side.lines.length ? '' : 'empty'}">
          <div class="side-head">
            <span class="label">${side.button}${side.label ? ` — ${side.label}` : ''}</span>
          </div>
          <pre>${side.lines.length ? side.lines.join('\n') : '(nothing — this side removes it)'}</pre>
        </div>`,
      )}
      ${region.base
        ? html`<div class="side ${region.base.lines.length ? '' : 'empty'}">
            <div class="side-head">
              <span class="label"
                >Base${region.base.label && region.base.label !== 'base'
                  ? ` — ${region.base.label}`
                  : ''}</span
              >
            </div>
            <pre>
${region.base.lines.length ? region.base.lines.join('\n') : '(nothing was here)'}</pre
            >
          </div>`
        : nothing}

      <div class="actions">
        ${sides.map(
          (side) => html`<button
            class="pick ${settled?.choice === side.choice ? 'chosen' : ''}"
            @click=${() => this.choose(region.index, side.choice, side.lines)}
          >
            Take ${side.button}
          </button>`,
        )}
        ${region.base
          ? html`<button
              class="pick ${settled?.choice === 'base' ? 'chosen' : ''}"
              @click=${() => this.choose(region.index, 'base', region.base!.lines)}
            >
              Take base
            </button>`
          : nothing}
        <button
          class="pick ${settled?.choice === 'both' ? 'chosen' : ''}"
          title="Every side, one after another, in the order jj listed them"
          @click=${() => this.choose(region.index, 'both', both)}
        >
          Keep both
        </button>
        <button
          class="pick ${settled?.choice === 'none' ? 'chosen' : ''}"
          title="Neither side — the region goes away entirely"
          @click=${() => this.choose(region.index, 'none', [])}
        >
          Take neither
        </button>
      </div>

      ${settled
        ? html`<div class="result">
            <label>Resolves to — edit freely</label>
            <textarea
              .value=${settled.lines.join('\n')}
              spellcheck="false"
              @input=${(event: Event) =>
                this.edit(region.index, (event.target as HTMLTextAreaElement).value)}
            ></textarea>
          </div>`
        : nothing}
    </div>`;
  }

  protected override render() {
    const regions = this.regions;
    const open = regions.length - this.settled.size;
    return html`<div class="panel" @keydown=${this.onPanelKey}>
      <header>
        <h2>Resolve conflict</h2>
        <span class="subject">${this.path}${this.shape ? ` · ${this.shape}` : ''}</span>
      </header>

      <div class="body">
        ${this.content === null
          ? html`<div class="elided" style="padding: 22px 20px">Reading the conflict…</div>`
          : regions.length === 0
            ? html`<div class="elided" style="padding: 22px 20px">
                Nothing here holds conflict markers any more — the file may already be resolved.
              </div>`
            : this.content.pieces.map((piece) =>
                piece.kind === 'text' ? this.renderContext(piece.lines) : this.renderRegion(piece),
              )}
      </div>

      <footer>
        <span class="hint">
          ${regions.length === 0
            ? 'Nothing to resolve.'
            : open === 0
              ? `All ${regions.length} region${regions.length === 1 ? '' : 's'} settled. Undoable from the Ops tab.`
              : `${open} of ${regions.length} region${regions.length === 1 ? '' : 's'} still open — every one has to be settled.`}
        </span>
        <span class="spacer"></span>
        <button class="btn" @click=${() => this.dismiss()}>Cancel</button>
        <button
          class="btn primary"
          ?disabled=${this.busy || regions.length === 0 || open > 0}
          @click=${this.confirm}
        >
          ${this.busy ? 'Resolving…' : 'Resolve'}
        </button>
      </footer>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-conflict-resolver': ConflictResolver;
  }
  interface HTMLElementEventMap {
    'resolve-conflict': CustomEvent<{ content: string }>;
  }
}
