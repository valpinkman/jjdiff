import { css, html, nothing } from 'lit';
import { customElement, property } from 'lit/decorators.js';

import { MODEL_KEY_FOR, type Config } from './ipc.js';
import { formatShortcut } from './keys.js';
import { OverlayElement, overlayChrome, panelHeader } from './overlay.js';
import { THEMES } from './themes.js';

/** What the page emits when a control is used: one config key, one value. */
export interface SettingChange {
  table: string;
  key: string;
  value: string | number | boolean;
}

const BACKENDS = [
  { id: 'claude', label: 'Claude Code', binary: 'claude' },
  { id: 'codex', label: 'Codex', binary: 'codex' },
  { id: 'opencode', label: 'OpenCode', binary: 'opencode' },
  { id: 'pi', label: 'Pi', binary: 'pi' },
];

/**
 * Every setting in `~/.config/jjdiff/config.toml`, in one place.
 *
 * The palette could already toggle four of these and the config file held
 * thirteen, so the rest were reachable only by editing TOML — and of the four,
 * only the theme was written back, which meant Split/Unified reset on every
 * restart and the file quietly disagreed with the app.
 *
 * The page does not own any state. Each control emits one `{table, key, value}`
 * and `App.applySetting` both applies it live and writes it, so the palette
 * toggles and this page go through the same path and cannot drift apart. Text
 * fields commit on blur or Enter rather than per keystroke — the alternative is
 * a file write per character typed into the editor command.
 *
 * Shadow DOM: leaf widget, no cross-boundary selection (DESIGN.md §6).
 */
@customElement('jj-settings-view')
export class SettingsView extends OverlayElement {
  static override styles = [
    overlayChrome,
    panelHeader,
    css`
      /* Taller than the other panels, so it starts a little higher. */
      :host {
        padding-top: 7vh;
      }
      .panel {
        display: flex;
        flex-direction: column;
        width: min(680px, 92vw);
        max-height: 82vh;
        background: var(--jj-bg-panel);
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-md, 6px);
        box-shadow: var(--jj-shadow-pop);
        overflow: hidden;
        animation: pop var(--jj-t-3, 260ms) var(--jj-ease-pop, ease-out);
      }
      .panel:focus {
        outline: none;
      }
      /* The body scrolls under the title, so the rule is what keeps the heading
         from touching the first group as it goes past. */
      header {
        border-bottom: 1px solid var(--jj-border);
      }
      /* The file is the real store, and saying where it is keeps hand-editing an
         obvious option rather than a secret the page has taken over. */
      .path {
        font-family: var(--jj-mono);
        font-size: 11px;
        color: var(--jj-fg-faint);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        max-width: 46%;
      }
      .body {
        overflow-y: auto;
        padding: 4px 20px 20px;
      }
      .group {
        font-family: var(--jj-sans);
        font-size: 10.5px;
        font-weight: 650;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--jj-fg-faint);
        padding: 18px 0 2px;
      }
      .row {
        display: flex;
        align-items: center;
        gap: 16px;
        padding: 9px 0;
        border-top: 1px solid var(--jj-border);
      }
      .group + .row {
        border-top: 0;
      }
      .row.stacked {
        display: block;
      }
      .label {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
        flex: 1;
      }
      .label strong {
        font-family: var(--jj-sans);
        font-size: 12.5px;
        font-weight: 600;
        color: var(--jj-fg);
      }
      .label small {
        font-size: 11.5px;
        line-height: 1.45;
        color: var(--jj-fg-muted);
      }
      .control {
        flex: none;
        display: flex;
        align-items: center;
        gap: 8px;
      }
      .row.stacked .control {
        margin-top: 8px;
        display: block;
      }
      input[type='text'],
      textarea,
      select {
        box-sizing: border-box;
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-md, 6px);
        background: var(--jj-surface);
        color: var(--jj-fg);
        font-family: var(--jj-sans);
        font-size: 12.5px;
        padding: 6px 9px;
        outline: none;
      }
      input[type='text'],
      textarea {
        font-family: var(--jj-mono);
        font-size: 12px;
        width: 100%;
      }
      textarea {
        min-height: 68px;
        resize: vertical;
        line-height: 1.5;
      }
      input:focus-visible,
      textarea:focus-visible,
      select:focus-visible {
        border-color: var(--jj-accent);
        box-shadow: 0 0 0 1px var(--jj-accent);
      }
      /* A segmented control, not a dropdown: two mutually exclusive options that
         both fit are a choice you should be able to see without opening it. */
      .segmented {
        display: inline-flex;
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-md, 6px);
        overflow: hidden;
      }
      .segmented button {
        border: 0;
        background: var(--jj-surface);
        color: var(--jj-fg-muted);
        font-family: var(--jj-sans);
        font-size: 12px;
        font-weight: 550;
        padding: 5px 13px;
        cursor: pointer;
      }
      .segmented button + button {
        border-left: 1px solid var(--jj-border);
      }
      .segmented button.on {
        background: var(--jj-accent);
        color: #fff;
      }
      .switch {
        position: relative;
        width: 38px;
        height: 22px;
        flex: none;
        border: 1px solid var(--jj-border);
        border-radius: 999px;
        background: var(--jj-surface);
        cursor: pointer;
        transition:
          background var(--jj-t-2, 180ms) ease,
          border-color var(--jj-t-2, 180ms) ease;
      }
      .switch::after {
        content: '';
        position: absolute;
        top: 2px;
        left: 2px;
        width: 16px;
        height: 16px;
        border-radius: 50%;
        background: var(--jj-fg-muted);
        transition:
          transform var(--jj-t-2, 180ms) var(--jj-ease-out, ease),
          background var(--jj-t-2, 180ms) ease;
      }
      .switch[aria-checked='true'] {
        background: var(--jj-accent);
        border-color: var(--jj-accent);
      }
      .switch[aria-checked='true']::after {
        background: #fff;
        transform: translateX(16px);
      }
      .switch:focus-visible {
        outline: 2px solid var(--jj-accent);
        outline-offset: 2px;
      }
      button.link {
        border: 1px solid var(--jj-border);
        border-radius: var(--jj-r-md, 6px);
        background: var(--jj-surface);
        color: var(--jj-fg-soft);
        font-family: var(--jj-sans);
        font-size: 12px;
        font-weight: 550;
        padding: 5px 11px;
        cursor: pointer;
      }
      button.link:hover {
        border-color: var(--jj-border-strong);
        color: var(--jj-fg);
      }
      .size {
        display: flex;
        align-items: center;
        gap: 10px;
      }
      .size input {
        width: 140px;
      }
      .size output {
        font-family: var(--jj-mono);
        font-size: 11.5px;
        font-variant-numeric: tabular-nums;
        color: var(--jj-fg-muted);
        width: 6ch;
        text-align: right;
      }
      @media (prefers-reduced-motion: reduce) {
        .switch,
        .switch::after {
          transition: none;
        }
      }
    `,
  ];

  @property({ attribute: false }) config: Config | null = null;
  @property() configPath = '';

  override firstUpdated() {
    this.renderRoot.querySelector<HTMLElement>('.panel')?.focus();
  }

  protected override dismiss() {
    this.dispatchEvent(new Event('close'));
  }

  private set(table: string, key: string, value: string | number | boolean) {
    this.dispatchEvent(
      new CustomEvent<SettingChange>('setting-changed', { detail: { table, key, value } }),
    );
  }

  /** Commit a text field on blur or Enter, and only when it actually changed. */
  private commit(table: string, key: string, current: string) {
    return (event: Event) => {
      const value = (event.target as HTMLInputElement | HTMLTextAreaElement).value;
      if (value !== current) this.set(table, key, value);
    };
  }

  private onTextKey = (event: KeyboardEvent) => {
    if (event.key === 'Enter' && !(event.target as HTMLElement).matches('textarea')) {
      (event.target as HTMLInputElement).blur();
    }
  };

  private toggle(table: string, key: string, value: boolean, label: string) {
    return html`<button
      class="switch"
      role="switch"
      aria-checked=${value ? 'true' : 'false'}
      aria-label=${label}
      @click=${() => this.set(table, key, !value)}
    ></button>`;
  }

  protected override render() {
    const config = this.config;
    if (!config) return nothing;
    const theme = THEMES.find((entry) => entry.id === config.ui.theme);
    const backend = config.walkthrough.backend || 'claude';
    const modelKey = MODEL_KEY_FOR[backend] ?? 'claudeModel';
    const model = config.walkthrough[modelKey];
    const binary = BACKENDS.find((entry) => entry.id === backend)?.binary ?? backend;

    return html`<div class="panel" tabindex="-1" @keydown=${this.onPanelKey}>
      <header>
        <h2>Settings</h2>
        <span class="spacer"></span>
        <span class="path" title=${this.configPath}>${this.configPath}</span>
      </header>
      <div class="body">
        <div class="group">Appearance</div>

        <div class="row">
          <span class="label">
            <strong>Theme</strong>
            <small>${theme?.label ?? config.ui.theme}${theme ? ` · ${theme.group}` : ''}</small>
          </span>
          <span class="control">
            <!-- Opens the picker rather than listing names here. Choosing a
                 palette is a visual decision and the picker previews it live;
                 twenty names in a dropdown is the same decision made blind. -->
            <button class="link" @click=${() => this.dispatchEvent(new Event('pick-theme'))}>
              Change…
            </button>
          </span>
        </div>

        <div class="row">
          <span class="label">
            <strong>Diff layout</strong>
            <small>Side-by-side, or one column with additions under removals.</small>
          </span>
          <span class="control">
            <span class="segmented">
              ${['split', 'unified'].map(
                (style) => html`<button
                  class=${config.ui.diffStyle === style ? 'on' : ''}
                  @click=${() => this.set('ui', 'diff-style', style)}
                >
                  ${style === 'split' ? 'Split' : 'Unified'}
                </button>`,
              )}
            </span>
          </span>
        </div>

        <div class="row">
          <span class="label">
            <strong>Code size</strong>
            <small>Font size of the diff itself, in pixels.</small>
          </span>
          <span class="control size">
            <input
              type="range"
              min="9"
              max="20"
              step="0.5"
              .value=${String(config.ui.codeFontSize)}
              @input=${(event: Event) =>
                this.set('ui', 'code-font-size', Number((event.target as HTMLInputElement).value))}
            />
            <output>${config.ui.codeFontSize}px</output>
          </span>
        </div>

        <div class="row">
          <span class="label">
            <strong>Word wrap</strong>
            <small>Wrap long lines instead of scrolling sideways.</small>
          </span>
          <span class="control">
            ${this.toggle('ui', 'word-wrap', config.ui.wordWrap, 'Word wrap')}
          </span>
        </div>

        <div class="group">Review</div>

        <div class="row">
          <span class="label">
            <strong>Ignore whitespace</strong>
            <small>Hide changes that only alter spacing or indentation.</small>
          </span>
          <span class="control">
            ${this.toggle(
              'ui',
              'ignore-whitespace',
              config.ui.ignoreWhitespace,
              'Ignore whitespace',
            )}
          </span>
        </div>

        <div class="group">Walkthroughs</div>

        <div class="row">
          <span class="label">
            <strong>Agent</strong>
            <small>Runs headlessly with the prompt on stdin. <code>${binary}</code> must be on
              your PATH.</small>
          </span>
          <span class="control">
            <select
              @change=${(event: Event) =>
                this.set('walkthrough', 'backend', (event.target as HTMLSelectElement).value)}
            >
              ${BACKENDS.map(
                (entry) => html`<option value=${entry.id} ?selected=${entry.id === backend}>
                  ${entry.label}
                </option>`,
              )}
            </select>
          </span>
        </div>

        <div class="row stacked">
          <span class="label">
            <strong>Model for ${BACKENDS.find((e) => e.id === backend)?.label ?? backend}</strong>
            <small>Leave empty to use whatever that CLI is already configured for. Stored per
              agent, so switching back keeps the model you set.</small>
          </span>
          <span class="control">
            <input
              type="text"
              placeholder="(the CLI's default)"
              .value=${model}
              @keydown=${this.onTextKey}
              @change=${this.commit('walkthrough', modelKey.replace('Model', '-model'), model)}
              @blur=${this.commit('walkthrough', modelKey.replace('Model', '-model'), model)}
            />
          </span>
        </div>

        <div class="row stacked">
          <span class="label">
            <strong>Extra instructions</strong>
            <small>Appended to every generation prompt — house style, what to emphasise, what
              to leave out.</small>
          </span>
          <span class="control">
            <textarea
              placeholder="e.g. Call out anything that changes a public API."
              .value=${config.walkthrough.prompt}
              @keydown=${this.onTextKey}
              @change=${this.commit('walkthrough', 'prompt', config.walkthrough.prompt)}
              @blur=${this.commit('walkthrough', 'prompt', config.walkthrough.prompt)}
            ></textarea>
          </span>
        </div>

        <div class="group">Commit messages</div>

        <div class="row stacked">
          <span class="label">
            <strong>House rules</strong>
            <small>Appended to the prompt behind Generate on the working copy. The agent is
              already shown the diff and your last few commit messages, so it copies the
              repository's existing style on its own — this is for what the history cannot
              show: a ticket reference to include, a subject-line limit, a body to always
              (or never) write.</small>
          </span>
          <span class="control">
            <textarea
              placeholder="e.g. Start the subject with the Jira key when the branch has one."
              .value=${config.describe.prompt}
              @keydown=${this.onTextKey}
              @change=${this.commit('describe', 'prompt', config.describe.prompt)}
              @blur=${this.commit('describe', 'prompt', config.describe.prompt)}
            ></textarea>
          </span>
        </div>

        <div class="row">
          <span class="label">
            <strong>Model</strong>
            <small>Overrides the walkthrough model above, because the two jobs differ: a
              message summarises a diff the prompt already carries, while a walkthrough is a
              careful reading of it. Left empty jjdiff asks Claude for a fast model rather
              than letting the CLI choose, which is its largest — on a 39KB diff that was the
              difference between 3 seconds and 14.</small>
          </span>
          <span class="control">
            <input
              type="text"
              placeholder="claude-sonnet-5 (default)"
              .value=${config.describe.model}
              @keydown=${this.onTextKey}
              @change=${this.commit('describe', 'model', config.describe.model)}
              @blur=${this.commit('describe', 'model', config.describe.model)}
            />
          </span>
        </div>

        <div class="group">External editor</div>

        <div class="row stacked">
          <span class="label">
            <strong>Editor command</strong>
            <small>Opens the file under the cursor with <code>o</code>. Placeholders:
              <code>{file}</code>, <code>{line}</code>, <code>{repo}</code>. Split on whitespace
              and run directly — there is no shell, so pipes and <code>&amp;&amp;</code> do not
              work.</small>
          </span>
          <span class="control">
            <input
              type="text"
              placeholder="zed {file}:{line}"
              .value=${config.editor.command}
              @keydown=${this.onTextKey}
              @change=${this.commit('editor', 'command', config.editor.command)}
              @blur=${this.commit('editor', 'command', config.editor.command)}
            />
          </span>
        </div>

        <div class="group">Keyboard</div>

        <div class="row stacked">
          <span class="label">
            <strong>Command palette</strong>
            <small>Currently ${formatShortcut(config.keymap.commandBar)}. Written jj-style:
              <code>Mod</code> is Cmd on macOS and Ctrl elsewhere, joined with
              <code>+</code>.</small>
          </span>
          <span class="control">
            <input
              type="text"
              placeholder="Mod+k"
              .value=${config.keymap.commandBar}
              @keydown=${this.onTextKey}
              @change=${this.commit('keymap', 'command-bar', config.keymap.commandBar)}
              @blur=${this.commit('keymap', 'command-bar', config.keymap.commandBar)}
            />
          </span>
        </div>
      </div>
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-settings-view': SettingsView;
  }
}
