import { html, LitElement, nothing } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import { repeat } from 'lit/directives/repeat.js';

import type { FilePatch, Hunk } from './ipc.js';

/**
 * Renders a list of file patches.
 *
 * Light DOM on purpose (PLAN.md): native text selection, copy, and find must work across
 * lines and files without crossing shadow boundaries. Styles live in theme.css.
 */
@customElement('jj-patch-view')
export class PatchView extends LitElement {
  @property({ attribute: false }) files: FilePatch[] = [];

  protected override createRenderRoot() {
    return this; // light DOM
  }

  protected override render() {
    if (this.files.length === 0) {
      return html`<p class="jj-empty">No changes.</p>`;
    }
    return html`<div class="jj-patch">
      ${repeat(this.files, (file) => file.path, renderFile)}
    </div>`;
  }
}

const renderFile = (file: FilePatch) => html`
  <section class="file">
    <header class="file-header">
      <span class="file-status ${file.status}">${file.status}</span>
      <span class="file-path">
        ${file.oldPath ? html`${file.oldPath} → ` : nothing}${file.path}
      </span>
    </header>
    ${file.binary
      ? html`<div class="hunk-header">binary file</div>`
      : repeat(file.hunks, renderHunk)}
  </section>
`;

const renderHunk = (hunk: Hunk) => html`
  <div class="hunk-header">
    @@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@ ${hunk.context}
  </div>
  <pre>${hunk.lines.map(
    (line) =>
      html`<span class="line ${line.kind}">${prefix(line.kind)}${line.text}\n</span>`,
  )}</pre>
`;

const prefix = (kind: string) => (kind === 'added' ? '+' : kind === 'removed' ? '-' : ' ');

declare global {
  interface HTMLElementTagNameMap {
    'jj-patch-view': PatchView;
  }
}
