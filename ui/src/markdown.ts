import { html, type TemplateResult } from 'lit';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';

/**
 * Markdown → sanitised HTML, for text jjdiff did not write.
 *
 * A pull request body and its comments are arbitrary text from anyone who can
 * open a proposal, and they render inside a WebView that can call every Tauri
 * command jjdiff exposes. `marked` does not sanitise; the app's CSP
 * (`default-src 'self'`) blocks inline handlers and remote scripts, but CSP is
 * a backstop, not a sanitiser — and it is absent entirely under `pnpm dev`, so
 * relying on it would mean the browser build is the exploitable one.
 *
 * Allow-list, not deny-list: anything not named here is unwrapped (its text
 * survives, its behaviour does not), which fails closed as markdown grows new
 * output than a blocklist that fails open on whatever we forgot.
 */
const ALLOWED = new Set([
  'P', 'BR', 'HR', 'EM', 'STRONG', 'DEL', 'CODE', 'PRE', 'BLOCKQUOTE',
  'UL', 'OL', 'LI', 'A', 'IMG', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6',
  'TABLE', 'THEAD', 'TBODY', 'TR', 'TH', 'TD', 'SPAN', 'DIV',
]);

/** Attributes worth keeping. Everything else — every `on*` among them — goes. */
const ALLOWED_ATTRS: Record<string, Set<string>> = {
  A: new Set(['href', 'title']),
  IMG: new Set(['src', 'alt', 'title']),
};

/** Only http(s). Blocks `javascript:`, and `data:` payloads with it. */
function safeUrl(value: string): boolean {
  try {
    const url = new URL(value, 'https://example.invalid');
    return url.protocol === 'http:' || url.protocol === 'https:';
  } catch {
    return false;
  }
}

/**
 * Elements whose *text* is code rather than prose. Unwrapping these would be
 * safe — nothing executes — but it would paste the source of a `<script>` into
 * the middle of someone's comment as though they had written it.
 */
const DROP_ENTIRELY = new Set(['SCRIPT', 'STYLE', 'TEMPLATE', 'NOSCRIPT']);

function scrub(node: Element) {
  if (DROP_ENTIRELY.has(node.tagName)) {
    node.remove();
    return;
  }
  for (const child of [...node.children]) scrub(child);
  if (!ALLOWED.has(node.tagName)) {
    // Unwrap rather than delete: a stripped <details> should still show its
    // text, and dropping it silently would misrepresent what someone wrote.
    node.replaceWith(...node.childNodes);
    return;
  }
  const keep = ALLOWED_ATTRS[node.tagName];
  for (const attr of [...node.attributes]) {
    if (!keep?.has(attr.name)) {
      node.removeAttribute(attr.name);
      continue;
    }
    if ((attr.name === 'href' || attr.name === 'src') && !safeUrl(attr.value)) {
      node.removeAttribute(attr.name);
    }
  }
}

/**
 * Render `text` as markdown. Returns a template ready to place in light DOM.
 *
 * Links are left as plain `<a href>` and handled by a delegated click listener
 * at the app level — the WebView has no tabs, so `target="_blank"` silently
 * does nothing and every outbound link has to go through `open_url`.
 */
export async function renderMarkdown(text: string): Promise<TemplateResult> {
  const { marked } = await import('marked');
  const raw = marked.parse(text, { async: false }) as string;
  // `template` parses without fetching images or running anything, unlike
  // assigning to a live element's innerHTML.
  const holder = document.createElement('template');
  holder.innerHTML = raw;
  for (const child of [...holder.content.children]) scrub(child);
  return html`${unsafeHTML(holder.innerHTML)}`;
}
