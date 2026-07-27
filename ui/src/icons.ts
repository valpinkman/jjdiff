// Toolbar icon set. One geometry for all of them — 16px grid, 1.5 stroke, round caps
// and joins, `currentColor` — so they inherit button colour and theme, and sit together
// as a family instead of the assorted Unicode glyphs they replace.
import { svg, type TemplateResult } from 'lit';

const icon = (paths: TemplateResult): TemplateResult => svg`
  <svg
    viewBox="0 0 16 16"
    width="16"
    height="16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >${paths}</svg>`;

/** jj git fetch — pull remote-tracking state down from the remote. */
export const iconFetch = icon(svg`
  <path d="M2.93 10.18A4.67 4.67 0 1 1 10.47 5.33h1.19a3 3 0 0 1 1.62 5.52" />
  <path d="M8 8.67V14" />
  <path d="m5.33 11.33 2.67 2.67 2.67-2.67" />`);

/** jj absorb — push working-copy changes down into the ancestors they belong to. */
export const iconAbsorb = icon(svg`
  <path d="M8 2v9.33" />
  <path d="m4 7.33 4 4 4-4" />
  <path d="M3.33 14h9.34" />`);

/** jj undo — wind the last operation back. */
export const iconUndo = icon(svg`
  <path d="M2 8a6 6 0 1 0 6-6 6.5 6.5 0 0 0-4.49 1.83L2 5.33" />
  <path d="M2 2v3.33h3.33" />`);

/** Side-by-side diff: one pane split down the middle. */
export const iconSplit = icon(svg`
  <rect x="2.2" y="3" width="11.6" height="10" rx="1.8" />
  <path d="M8 3v10" />`);

/** Unified diff: one column of lines. */
export const iconUnified = icon(svg`
  <rect x="2.2" y="3" width="11.6" height="10" rx="1.8" />
  <path d="M4.9 6.5h6.2" />
  <path d="M4.9 9.5h6.2" />`);
