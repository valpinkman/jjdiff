import { html, type TemplateResult } from 'lit';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';

import type { Tokens } from 'marked';

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
  return html`${unsafeHTML(await renderMarkdownToHtml(text))}`;
}

/**
 * The same thing as markup, for the caller that needs a string rather than a
 * template: the `.md` file preview hands its HTML to a diff row. It parsed with
 * `marked` directly until it went through here — a `.md` in a fetched proposal
 * is a stranger's markdown, and the scrubber is what makes it safe to inject.
 */
export async function renderMarkdownToHtml(text: string): Promise<string> {
  const { marked } = await import('marked');
  return toHtml(marked, text);
}

async function toHtml(marked: typeof import('marked').marked, text: string): Promise<string> {
  const raw = marked.parse(text, { async: false }) as string;
  // `template` parses without fetching images or running anything, unlike
  // assigning to a live element's innerHTML.
  const holder = document.createElement('template');
  holder.innerHTML = raw;
  for (const child of [...holder.content.children]) scrub(child);
  return holder.innerHTML;
}

/**
 * Markdown with ` ```mermaid ` fences drawn as diagrams.
 *
 * Split by *token*, not by a regex over the source: a fence nested inside a
 * wider fence — which the walkthrough guide's own examples use — is a code
 * block whose text merely happens to start with three backticks, and a regex
 * would cut the document in half there.
 *
 * A diagram that fails to parse falls back to its source as a code block rather
 * than taking the page down with it. Agent-authored mermaid is wrong often
 * enough that this is the normal path, not the exceptional one.
 */
export async function renderMarkdownWithDiagrams(
  text: string,
  seed: string,
): Promise<TemplateResult> {
  const { marked } = await import('marked');
  const tokens = marked.lexer(text);
  const parts: string[] = [];
  let prose = '';
  let index = 0;

  for (const token of tokens) {
    const fence = token.type === 'code' ? (token as Tokens.Code) : null;
    const lang = fence?.lang?.trim().split(/\s+/)[0];
    if (fence && (lang === 'mermaid' || lang === 'diff')) {
      if (prose) {
        parts.push(await toHtml(marked, prose));
        prose = '';
      }
      parts.push(
        lang === 'diff'
          ? diffFence(fence.text)
          : await renderDiagram(fence.text, `${seed}-${index++}`, marked),
      );
      continue;
    }
    prose += token.raw;
  }
  if (prose) parts.push(await toHtml(marked, prose));
  return html`${unsafeHTML(parts.join(''))}`;
}

/**
 * A ` ```diff ` fence, coloured per line.
 *
 * Handled here rather than left to the plain code-block path because the whole
 * point of the fence is which lines moved, and that is carried by one character
 * at the start of each. `marked` renders it as one grey block, and the sanitiser
 * would strip the `language-diff` class anyway — so the markup is built here,
 * with the text escaped rather than parsed. Nothing from the source reaches the
 * DOM as markup, which is why this does not go through the scrubber.
 */
function diffFence(source: string): string {
  const lines = source.split('\n').map((line) => {
    const kind = line.startsWith('+') ? 'add' : line.startsWith('-') ? 'del' : 'ctx';
    return `<span class="diff-line ${kind}">${escapeHtml(line) || '&nbsp;'}</span>`;
  });
  // Joined with nothing: the spans are blocks, and inside a `pre` a newline
  // between them is a *second* line break — every row came out double-spaced.
  return `<pre class="diff-fence"><code>${lines.join('')}</code></pre>`;
}

function escapeHtml(text: string): string {
  const holder = document.createElement('span');
  holder.textContent = text;
  return holder.innerHTML;
}

/**
 * Mermaid renders into an SVG string, which is then scrubbed like any other
 * untrusted markup.
 *
 * `securityLevel: 'strict'` plus `htmlLabels: false` is the real boundary —
 * labels become escaped `<text>` nodes and click directives are ignored — and
 * [`scrubSvg`] is the backstop, for the same reason the markdown scrubber
 * exists rather than leaning on the CSP.
 */
async function renderDiagram(
  source: string,
  id: string,
  marked: typeof import('marked').marked,
): Promise<string> {
  try {
    const mermaid = (await import('mermaid')).default;
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      // The palette is jjdiff's, read off the document so a diagram matches the
      // theme in force. `theme: 'base'` is the one that honours themeVariables.
      theme: 'base',
      themeVariables: diagramPalette(),
      // Both, not one: the flowchart renderer reads the nested key, but the
      // shared label helper reads the top-level one, and with only the nested
      // one set the node labels came out as `foreignObject` after all — which
      // [`scrubSvg`] then dropped, leaving three empty boxes joined by arrows.
      htmlLabels: false,
      flowchart: { htmlLabels: false, curve: 'basis' },
      fontFamily: cssVar('--jj-sans') || 'sans-serif',
    });
    const { svg } = await mermaid.render(`jj-mermaid-${id}`, source);
    const holder = document.createElement('template');
    holder.innerHTML = svg;
    for (const child of [...holder.content.children]) scrubSvg(child);
    // A named button rather than a hover affordance (DESIGN.md §6): the figure
    // is clickable too, but nothing about a picture says so on its own.
    return (
      `<div class="mermaid-figure">` +
      `<button type="button" class="mermaid-expand" title="Open this diagram larger">Expand</button>` +
      `${holder.innerHTML}</div>`
    );
  } catch (error) {
    // Show the source, and say why. A broken diagram is still the author's
    // description of the system, so dropping it would lose the section — but a
    // silent fallback is indistinguishable from an agent that wrote a fence of
    // plain text, which sent one real failure (a missing chunk in the WebView)
    // several hours in the wrong direction.
    const note = `<p class="diagram-error">Diagram not rendered: ${escapeHtml(String(error))}</p>`;
    return (await toHtml(marked, `\`\`\`\n${source}\n\`\`\``)) + note;
  }
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/**
 * A `--jj-*` token as a colour mermaid's parser will accept.
 *
 * Not `getPropertyValue`: a custom property's computed value is its *token
 * text*, unsubstituted — so a theme whose `--jj-border` is
 * `color-mix(in srgb, #d4d4d8 64%, transparent)` handed mermaid that string
 * verbatim and khroma threw `Unsupported color format`, taking the whole
 * diagram down to its source. Which themes broke depended on which happened to
 * be written as plain hex, so it looked like an intermittent WebView bug.
 *
 * Two steps, and both are load-bearing. Reading `color` off a probe makes the
 * engine resolve `var()` and `color-mix()`. Then the value is *painted* and the
 * pixel read back, rather than round-tripped through `canvas.fillStyle` — an
 * engine is free to serialise a resolved colour in whatever space it computed
 * it in, and WebKit hands back `color(srgb 0.878 0.871 0.957 / 0.54)`, which
 * khroma rejects exactly as it rejected the `color-mix()` we started with.
 * A rasterised pixel is eight bits per channel whatever the colour space, so
 * this cannot be defeated by a spelling.
 */
function cssColor(name: string, fallback: string): string {
  const probe = document.createElement('span');
  probe.style.cssText = `display:none;color:var(${name})`;
  document.body.appendChild(probe);
  const resolved = getComputedStyle(probe).color;
  probe.remove();
  if (!resolved) return fallback;
  const context = document.createElement('canvas').getContext('2d', {
    willReadFrequently: true,
  });
  if (!context) return fallback;
  context.clearRect(0, 0, 1, 1);
  context.fillStyle = resolved;
  context.fillRect(0, 0, 1, 1);
  const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
  return `rgba(${red}, ${green}, ${blue}, ${((alpha ?? 255) / 255).toFixed(3)})`;
}

/**
 * jjdiff's tokens, mapped onto the names mermaid's `base` theme reads.
 *
 * No `transparent` anywhere, even where it would read better: it is a keyword
 * rather than a colour and the same parser rejects it. The figure's container
 * already paints the panel background, so the diagram's own is the same colour.
 */
function diagramPalette(): Record<string, string> {
  const fg = cssColor('--jj-fg', '#1c1c1c');
  const border = cssColor('--jj-border', '#d4d4d4');
  const surface = cssColor('--jj-surface', '#ffffff');
  const panel = cssColor('--jj-bg-panel', surface);
  const wash = cssColor('--jj-wash', '#f4f4f5');
  return {
    background: panel,
    primaryColor: wash,
    primaryTextColor: fg,
    primaryBorderColor: border,
    secondaryColor: surface,
    tertiaryColor: surface,
    lineColor: cssColor('--jj-fg-muted', '#6b7280'),
    textColor: fg,
    mainBkg: wash,
    nodeBorder: border,
    clusterBkg: panel,
    clusterBorder: border,
    edgeLabelBackground: panel,
    titleColor: cssColor('--jj-accent', '#3b82f6'),
  };
}

/**
 * The SVG allow-list, and it is shaped the other way round from [`scrub`].
 *
 * Markdown names a handful of elements worth keeping; an SVG is a few hundred
 * generated ones with generated attributes, so naming them all would be a list
 * that goes stale on every mermaid release and quietly unwraps the diagram.
 * What actually carries risk is small and stable: script, embedded HTML, event
 * handlers and non-http URLs. `<style>` stays — mermaid puts the diagram's
 * entire appearance in it, and dropping it leaves black-on-black boxes.
 */
const SVG_DROP = new Set(['SCRIPT', 'IFRAME', 'IMAGE', 'USE', 'ANIMATE']);

function scrubSvg(node: Element) {
  const tag = node.tagName.toUpperCase();
  if (SVG_DROP.has(tag)) {
    node.remove();
    return;
  }
  // `foreignObject` is embedded HTML inside the SVG, which is the one part of a
  // diagram that is not SVG at all — so it goes through the HTML allow-list
  // rather than this one. Dropping it outright was the first attempt and it
  // deleted the node labels of any diagram type that still uses it.
  if (tag === 'FOREIGNOBJECT') {
    for (const child of [...node.children]) scrub(child);
    return;
  }
  for (const child of [...node.children]) scrubSvg(child);
  for (const attr of [...node.attributes]) {
    const name = attr.name.toLowerCase();
    if (name.startsWith('on')) {
      node.removeAttribute(attr.name);
    } else if ((name === 'href' || name === 'xlink:href') && !safeUrl(attr.value)) {
      node.removeAttribute(attr.name);
    }
  }
}
