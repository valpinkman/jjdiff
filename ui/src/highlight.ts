// Main-thread client for the shiki worker: per-file token caches keyed by (side, line index).
import type { FilePatch } from './ipc.js';
import { sideTexts, type HlRef } from './rows.js';
import type { HighlightRequest, HighlightResponse, Token } from './highlight.worker.js';

export type { Token };

/** Files above these sizes render unhighlighted — keep the review responsive. */
const MAX_LINES = 5000;
const MAX_CHARS = 400_000;

const EXTENSION_LANGS: Record<string, string> = {
  c: 'c',
  cjs: 'javascript',
  cpp: 'cpp',
  cs: 'csharp',
  css: 'css',
  go: 'go',
  h: 'c',
  hpp: 'cpp',
  html: 'html',
  java: 'java',
  js: 'javascript',
  json: 'json',
  jsonc: 'jsonc',
  jsx: 'jsx',
  kt: 'kotlin',
  md: 'markdown',
  mjs: 'javascript',
  php: 'php',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  scss: 'scss',
  sh: 'shellscript',
  sql: 'sql',
  svelte: 'svelte',
  swift: 'swift',
  toml: 'toml',
  ts: 'typescript',
  tsx: 'tsx',
  vue: 'vue',
  yaml: 'yaml',
  yml: 'yaml',
  zsh: 'shellscript',
};

export const languageFor = (path: string): string | null => {
  const dot = path.lastIndexOf('.');
  if (dot === -1) return null;
  return EXTENSION_LANGS[path.slice(dot + 1).toLowerCase()] ?? null;
};

interface FileTokens {
  old: Token[][] | null;
  new: Token[][] | null;
}

/** Owns the worker and caches tokens per file fingerprint. */
export class HighlightStore extends EventTarget {
  private worker: Worker | null = null;
  private nextId = 1;
  private pending = new Map<number, string>(); // request id → cache key
  private cache = new Map<string, FileTokens>();
  private requested = new Set<string>();

  private get theme(): HighlightRequest['theme'] {
    // Config-forced theme (data attribute) wins over the system scheme.
    const forced = document.documentElement.dataset['jjTheme'];
    if (forced === 'dark') return 'github-dark';
    if (forced === 'light') return 'github-light';
    return matchMedia('(prefers-color-scheme: dark)').matches
      ? 'github-dark'
      : 'github-light';
  }

  private ensureWorker(): Worker | null {
    if (this.worker) return this.worker;
    try {
      this.worker = new Worker(new URL('./highlight.worker.ts', import.meta.url), {
        type: 'module',
      });
      this.worker.onmessage = (event: MessageEvent<HighlightResponse>) => {
        const key = this.pending.get(event.data.id);
        if (!key) return;
        this.pending.delete(event.data.id);
        this.cache.set(key, { old: event.data.old, new: event.data.new });
        this.dispatchEvent(new Event('tokens'));
      };
    } catch {
      this.worker = null;
    }
    return this.worker;
  }

  /** Cache key must change when content changes; path + counts is good enough per refresh. */
  private keyFor(file: FilePatch): string {
    return `${file.path}:${file.added}:${file.removed}:${file.hunks.length}`;
  }

  request(file: FilePatch): void {
    const lang = languageFor(file.path);
    if (!lang || file.binary || file.skipped) return;
    const key = this.keyFor(file);
    if (this.cache.has(key) || this.requested.has(key)) return;

    const sides = sideTexts(file);
    const total = sides.old.length + sides.new.length;
    const chars = sides.old.concat(sides.new).reduce((n, l) => n + l.length, 0);
    if (total > MAX_LINES || chars > MAX_CHARS) return;

    const worker = this.ensureWorker();
    if (!worker) return;
    this.requested.add(key);
    const id = this.nextId++;
    this.pending.set(id, key);
    worker.postMessage({
      id,
      lang,
      theme: this.theme,
      oldText: sides.old.join('\n'),
      newText: sides.new.join('\n'),
    } satisfies HighlightRequest);
  }

  tokensFor(file: FilePatch, ref: HlRef | null): Token[] | null {
    if (!ref) return null;
    const tokens = this.cache.get(this.keyFor(file));
    const side = ref.side === 'old' ? tokens?.old : tokens?.new;
    return side?.[ref.index] ?? null;
  }

  /** Drop everything (repo refreshed); pending worker replies are ignored via key miss. */
  clear(): void {
    this.cache.clear();
    this.requested.clear();
    this.pending.clear();
  }
}
