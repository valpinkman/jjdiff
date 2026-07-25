import { css, html, LitElement, nothing, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

import type { FilePatch } from './ipc.js';

interface DirNode {
  name: string;
  path: string;
  dirs: Map<string, DirNode>;
  files: FilePatch[];
}

/** Collapsible changed-files tree. Selecting a file focuses the diff on it. */
@customElement('jj-file-tree')
export class FileTree extends LitElement {
  static override styles = css`
    :host {
      display: block;
      font-size: 12px;
    }
    .dir,
    .file {
      display: flex;
      align-items: center;
      gap: 6px;
      width: 100%;
      border: 0;
      background: none;
      color: var(--jj-fg);
      font: inherit;
      text-align: left;
      padding: 3px 6px;
      border-radius: 5px;
      cursor: pointer;
      white-space: nowrap;
    }
    .dir:hover,
    .file:hover {
      background: var(--jj-bg-panel);
    }
    .file.selected {
      background: var(--jj-bg-panel);
      outline: 1px solid var(--jj-accent);
    }
    .dir {
      color: var(--jj-fg-muted);
    }
    .twist {
      width: 10px;
      color: var(--jj-fg-muted);
    }
    .name {
      overflow: hidden;
      text-overflow: ellipsis;
      flex: 1;
    }
    .dot {
      width: 7px;
      height: 7px;
      border-radius: 50%;
      flex: none;
    }
    .dot.added { background: var(--jj-added-fg); }
    .dot.deleted { background: var(--jj-removed-fg); }
    .dot.modified { background: var(--jj-accent); }
    .dot.renamed { background: var(--jj-fg-muted); }
    .counts {
      font-family: var(--jj-mono);
      font-size: 10px;
    }
    .counts .plus { color: var(--jj-added-fg); }
    .counts .minus { color: var(--jj-removed-fg); }
  `;

  @property({ attribute: false }) files: FilePatch[] = [];
  @property() selected: string | null = null;

  @state() private collapsed = new Set<string>();

  protected override render() {
    const root = buildTree(this.files);
    return html`${this.renderDir(root, 0)}`;
  }

  private renderDir(node: DirNode, depth: number): TemplateResult {
    const pad = (n: number) => `padding-left:${6 + n * 12}px`;
    return html`
      ${[...node.dirs.values()].map((dir) => {
        const isCollapsed = this.collapsed.has(dir.path);
        return html`
          <button class="dir" style=${pad(depth)} @click=${() => this.toggle(dir.path)}>
            <span class="twist">${isCollapsed ? '▸' : '▾'}</span>
            <span class="name">${dir.name}</span>
          </button>
          ${isCollapsed ? nothing : this.renderDir(dir, depth + 1)}
        `;
      })}
      ${node.files.map(
        (file) => html`
          <button
            class="file ${file.path === this.selected ? 'selected' : ''}"
            style=${pad(depth)}
            title=${file.path}
            @click=${() => this.pick(file)}
          >
            <span class="dot ${file.status}"></span>
            <span class="name">${basename(file.path)}</span>
            <span class="counts">
              ${file.added ? html`<span class="plus">+${file.added}</span>` : nothing}
              ${file.removed ? html`<span class="minus">−${file.removed}</span>` : nothing}
            </span>
          </button>
        `,
      )}
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
