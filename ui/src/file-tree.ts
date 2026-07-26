import { css, html, LitElement, nothing, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

import { fileIcon, folderIcon } from './file-icons.js';
import type { FilePatch } from './ipc.js';

interface DirNode {
  name: string;
  path: string;
  dirs: Map<string, DirNode>;
  files: FilePatch[];
}

/**
 * VS Code-style changed-files tree: chevrons + folder icons, per-filetype icons,
 * filenames colored by status, single-child directory chains compacted (`src/sync`).
 * Selecting a file focuses the diff on it.
 */
@customElement('jj-file-tree')
export class FileTree extends LitElement {
  static override styles = css`
    :host {
      display: block;
      font-size: 12.5px;
      user-select: none;
    }
    .row {
      transition: background-color 0.13s ease;
      display: flex;
      align-items: center;
      gap: 5px;
      width: 100%;
      border: 0;
      background: none;
      color: var(--jj-fg);
      font: inherit;
      text-align: left;
      padding: 3px 7px;
      border-radius: var(--jj-r-sm, 7px);
      cursor: pointer;
      white-space: nowrap;
      line-height: 20px;
    }
    .row:hover {
      background: var(--jj-wash);
    }
    .row:focus-visible {
      outline: 2px solid var(--jj-accent);
      outline-offset: -2px;
    }
    .row.selected {
      background: var(--jj-accent-soft);
      color: var(--jj-accent);
    }
    .chevron {
      flex: none;
      width: 12px;
      font-size: 9px;
      color: var(--jj-fg-muted);
      text-align: center;
    }
    .icon {
      flex: none;
      display: inline-flex;
      color: var(--jj-fg-muted);
    }
    .name {
      overflow: hidden;
      text-overflow: ellipsis;
      flex: 1;
      min-width: 0;
    }
    .dir-name {
      color: var(--jj-fg);
    }
    .name.added { color: var(--jj-added-fg); }
    .name.deleted { color: var(--jj-removed-fg); text-decoration: line-through; }
    .name.modified { color: var(--jj-fg); }
    .name.renamed { color: var(--jj-ref); }
    .row.viewed .name {
      color: var(--jj-fg-muted);
      text-decoration: line-through;
      text-decoration-color: var(--jj-border);
    }
    .meta {
      flex: none;
      display: inline-flex;
      align-items: center;
      gap: 5px;
      font-family: var(--jj-mono);
      font-size: 10px;
    }
    .meta .plus { color: var(--jj-added-fg); }
    .meta .minus { color: var(--jj-removed-fg); }
    .meta .check { color: var(--jj-added-fg); font-size: 11px; }
    .empty {
      color: var(--jj-fg-muted);
      font-style: italic;
      padding: 6px 8px;
    }
  `;

  @property({ attribute: false }) files: FilePatch[] = [];
  @property() selected: string | null = null;
  @property({ attribute: false }) viewed: ReadonlySet<string> = new Set();

  @state() private collapsed = new Set<string>();

  protected override render() {
    if (this.files.length === 0) {
      return html`<div class="empty">No changed files</div>`;
    }
    const root = buildTree(this.files);
    return html`${this.renderDir(root, 0)}`;
  }

  private renderDir(node: DirNode, depth: number): TemplateResult {
    const pad = (n: number) => `padding-left:${6 + n * 14}px`;
    return html`
      ${[...node.dirs.values()].map((dir) => {
        // Compact single-child chains: src/ → sync/ with nothing else becomes "src/sync".
        let display = dir.name;
        let target = dir;
        while (target.files.length === 0 && target.dirs.size === 1) {
          const only = [...target.dirs.values()][0]!;
          display = `${display}/${only.name}`;
          target = only;
        }
        const isCollapsed = this.collapsed.has(target.path);
        return html`
          <button class="row" style=${pad(depth)} @click=${() => this.toggle(target.path)}>
            <span class="chevron">${isCollapsed ? '▶' : '▼'}</span>
            <span class="icon">${folderIcon(!isCollapsed)}</span>
            <span class="name dir-name">${display}</span>
          </button>
          ${isCollapsed ? nothing : this.renderDir(target, depth + 1)}
        `;
      })}
      ${node.files.map((file) => {
        const isViewed = this.viewed.has(file.path);
        return html`
          <button
            class="row ${file.path === this.selected ? 'selected' : ''} ${isViewed
              ? 'viewed'
              : ''}"
            style=${pad(depth)}
            title=${file.path}
            @click=${() => this.pick(file)}
          >
            <span class="chevron"></span>
            <span class="icon">${fileIcon(file.path)}</span>
            <span class="name ${file.status}">${basename(file.path)}</span>
            <span class="meta">
              ${isViewed ? html`<span class="check">✓</span>` : nothing}
              ${file.added ? html`<span class="plus">+${file.added}</span>` : nothing}
              ${file.removed ? html`<span class="minus">−${file.removed}</span>` : nothing}
            </span>
          </button>
        `;
      })}
    `;
  }

  private toggle(path: string) {
    const next = new Set(this.collapsed);
    if (!next.delete(path)) {
      next.add(path);
    }
    this.collapsed = next;
  }

  private pick(file: FilePatch) {
    this.dispatchEvent(
      new CustomEvent<string | null>('file-selected', {
        detail: file.path === this.selected ? null : file.path,
      }),
    );
  }
}

const basename = (path: string) => path.slice(path.lastIndexOf('/') + 1);

function buildTree(files: FilePatch[]): DirNode {
  const root: DirNode = { name: '', path: '', dirs: new Map(), files: [] };
  for (const file of files) {
    const parts = file.path.split('/');
    let node = root;
    for (const part of parts.slice(0, -1)) {
      let dir = node.dirs.get(part);
      if (!dir) {
        dir = {
          name: part,
          path: node.path ? `${node.path}/${part}` : part,
          dirs: new Map(),
          files: [],
        };
        node.dirs.set(part, dir);
      }
      node = dir;
    }
    node.files.push(file);
  }
  return root;
}

declare global {
  interface HTMLElementTagNameMap {
    'jj-file-tree': FileTree;
  }
}
