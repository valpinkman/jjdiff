// Compose one line's content from up to two overlays:
//  - shiki tokens (color), when available
//  - word-diff spans (emphasis), UTF-16 [start, end) ranges from Rust
import { html, nothing, type TemplateResult } from 'lit';

import type { Token } from './highlight.js';

type Span = [number, number];

export function renderLineContent(
  text: string,
  tokens: Token[] | null,
  spans: Span[],
): TemplateResult | typeof nothing {
  if (text.length === 0) {
    return html`\n`;
  }
  if (!tokens || tokens.length === 0) {
    if (spans.length === 0) {
      return html`${text}\n`;
    }
    return html`${sliceBySpans(text, 0, spans, undefined)}\n`;
  }
  // Walk tokens with a UTF-16 cursor, splitting each against the span boundaries.
  const parts: (TemplateResult | string)[] = [];
  let cursor = 0;
  for (const token of tokens) {
    parts.push(...sliceBySpans(token.t, cursor, spans, token.c));
    cursor += token.t.length;
  }
  return html`${parts}\n`;
}

/** Split `text` (which starts at absolute offset `base`) at span boundaries. */
function sliceBySpans(
  text: string,
  base: number,
  spans: Span[],
  color: string | undefined,
): (TemplateResult | string)[] {
  const end = base + text.length;
  const cuts = new Set<number>([base, end]);
  for (const [s, e] of spans) {
    if (s < end && e > base) {
      cuts.add(Math.max(s, base));
      cuts.add(Math.min(e, end));
    }
  }
  const points = [...cuts].sort((a, b) => a - b);
  const parts: (TemplateResult | string)[] = [];
  for (let i = 0; i < points.length - 1; i++) {
    const from = points[i]!;
    const to = points[i + 1]!;
    const piece = text.slice(from - base, to - base);
    if (piece.length === 0) continue;
    const marked = spans.some(([s, e]) => from >= s && to <= e);
    if (!marked && color === undefined) {
      parts.push(piece);
    } else {
      const style = color ? `color:${color}` : nothing;
      parts.push(
        marked
          ? html`<span class="mark" style=${style}>${piece}</span>`
          : html`<span style=${style}>${piece}</span>`,
      );
    }
  }
  return parts;
}
