import { css, html, LitElement, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

import { getFileBytes, type FileBytes } from './ipc.js';

/**
 * Side-by-side old/new image view for binary image files. Fetches base64
 * bytes via IPC and renders them as data URLs. Renames show old → new.
 */
@customElement('jj-image-view')
export class ImageView extends LitElement {
  static override styles = css`
    :host {
      display: block;
      padding: 10px 12px;
    }
    .images {
      display: flex;
      gap: 16px;
      flex-wrap: wrap;
    }
    .side {
      flex: 1;
      min-width: 200px;
    }
    .label {
      font-size: 11px;
      color: var(--jj-fg-faint);
      margin-bottom: 5px;
    }
    .label .arrow {
      color: var(--jj-fg-muted);
      margin: 0 4px;
    }
    img {
      max-width: 100%;
      border-radius: 6px;
      border: 1px solid var(--jj-border);
      background: var(--jj-panel);
    }
    .error {
      color: var(--jj-fg-muted);
      font-size: 12px;
      font-style: italic;
    }
    .loading {
      color: var(--jj-fg-faint);
      font-size: 12px;
    }
  `;

  @property() path = '';
  @property() oldPath: string | null = null;
  @property() revset: string | null = null;

  @state() private oldImg: FileBytes | null = null;
  @state() private newImg: FileBytes | null = null;
  @state() private error: string | null = null;
  @state() private loading = false;

  override connectedCallback() {
    super.connectedCallback();
    void this.load();
  }

  override updated(changed: Map<string, unknown>) {
    if (changed.has('path') || changed.has('oldPath') || changed.has('revset')) {
      void this.load();
    }
  }

  private async load() {
    this.loading = true;
    this.error = null;
    this.oldImg = null;
    this.newImg = null;
    try {
      const tasks: Promise<void>[] = [];
      // For added images there is no old side; for deleted images no new side; for
      // modified/renamed both may exist. We fetch the new side (path) at the revset,
      // and the old side (oldPath or path) at the parent revset — but the IPC takes
      // a revset, and the working copy's parent is what we're diffing against, so
      // for working-copy diffs (revset=null), old bytes come from `@-`.
      const oldRevset = this.revset ? `${this.revset}-` : null;
      const oldPath = this.oldPath ?? this.path;

      if (oldRevset) {
        tasks.push(
          getFileBytes(oldRevset, oldPath)
            .then((b) => { this.oldImg = b; })
            .catch(() => { /* old side may not exist for added files */ }),
        );
      }
      // New side: for working-copy diffs (revset=null), read the file from disk.
      tasks.push(
        getFileBytes(this.revset, this.path)
          .then((b) => { this.newImg = b; })
          .catch((e) => { this.error = String(e); }),
      );
      await Promise.all(tasks);
    } finally {
      this.loading = false;
    }
  }

  private dataUrl(img: FileBytes | null): string | null {
    if (!img || !img.data || img.mime === 'application/octet-stream') return null;
    return `data:${img.mime};base64,${img.data}`;
  }

  protected override render() {
    if (this.loading) return html`<div class="loading">Loading image…</div>`;
    if (this.error) return html`<div class="error">${this.error}</div>`;

    const oldUrl = this.dataUrl(this.oldImg);
    const newUrl = this.dataUrl(this.newImg);
    const isRename = this.oldPath && this.oldPath !== this.path;

    return html`<div class="images">
      ${oldUrl
        ? html`<div class="side">
            <div class="label">
              ${isRename ? html`<span>${this.oldPath}</span><span class="arrow">→</span><span>${this.path}</span>` : html`<span>old</span>`}
            </div>
            <img src=${oldUrl} alt="old version" />
          </div>`
        : nothing}
      ${newUrl
        ? html`<div class="side">
            <div class="label">${oldUrl ? 'new' : this.path}</div>
            <img src=${newUrl} alt="new version" />
          </div>`
        : nothing}
      ${!oldUrl && !newUrl
        ? html`<div class="error">Could not render this image.</div>`
        : nothing}
    </div>`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-image-view': ImageView;
  }
}
