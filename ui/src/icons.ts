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

/**
 * The disclosure marker for every fold in the app. Always drawn pointing down
 * and rotated to the closed position by `.fold-chevron.closed`, so the two
 * states are one glyph turning rather than two glyphs swapping — the turn is
 * what shows which way the content went.
 */
export const iconChevron = icon(svg`<path d="m4 6 4 4 4-4" />`);

/** Search. Leads every filter field. */
export const iconSearch = icon(svg`
  <circle cx="7.2" cy="7.2" r="4.5" />
  <path d="m10.6 10.6 2.7 2.7" />`);

/** Something needs attention but nothing is broken — conflicts, drift. */
export const iconWarn = icon(svg`
  <path d="M8 2.7 14.2 13H1.8L8 2.7Z" />
  <path d="M8 6.7v3" />
  <path d="M8 11.6h.01" />`);

/** Neutral notice: state the user should know, no action implied. */
export const iconInfo = icon(svg`
  <circle cx="8" cy="8" r="6.2" />
  <path d="M8 7.4v3.4" />
  <path d="M8 5.3h.01" />`);

/** Agent-authored content: the walkthrough, and anything else generated. */
export const iconSparkle = icon(svg`
  <path d="M8 2.2 9.4 6 13.2 7.4 9.4 8.8 8 12.6 6.6 8.8 2.8 7.4 6.6 6 8 2.2Z" />
  <path d="M12.6 11.2l.5 1.3 1.3.5-1.3.5-.5 1.3-.5-1.3-1.3-.5 1.3-.5.5-1.3Z" />`);

/** A commit under review — the change detail card's mark. */
export const iconCommit = icon(svg`
  <circle cx="8" cy="8" r="2.9" />
  <path d="M8 1.8v3.3" />
  <path d="M8 10.9v3.3" />`);

/** Move a bookmark/ref from one commit to another. */
export const iconMoveRef = icon(svg`
  <circle cx="4.2" cy="4.2" r="1.6" />
  <circle cx="11.8" cy="11.8" r="1.6" />
  <path d="M5.5 5.5 10.5 10.5" />
  <path d="M10.5 7.5v3h-3" />`);

/* ---- Sidebar rail. One per pane; the pane's name is its tooltip and its title. ---- */

/** Log: the commit graph — a trunk with a branch off it. */
export const iconGraph = icon(svg`
  <circle cx="4.6" cy="3.6" r="1.7" />
  <circle cx="4.6" cy="12.4" r="1.7" />
  <circle cx="11.4" cy="8" r="1.7" />
  <path d="M4.6 5.3v5.4" />
  <path d="M9.7 8H6.3" />`);

/** Files: a stack of sheets. */
export const iconFiles = icon(svg`
  <path d="M9 2H4.6a1.3 1.3 0 0 0-1.3 1.3v9.4A1.3 1.3 0 0 0 4.6 14h6.8a1.3 1.3 0 0 0 1.3-1.3V5.4L9 2Z" />
  <path d="M9 2v3.4h3.7" />`);

/** Review: comments on a line. */
export const iconComment = icon(svg`
  <path d="M14 9.7a1.5 1.5 0 0 1-1.5 1.5H5.2L2 14V3.5A1.5 1.5 0 0 1 3.5 2h9a1.5 1.5 0 0 1 1.5 1.5v6.2Z" />`);

/** Workspaces: two checkouts of one repository, side by side. */
export const iconWorkspaces = icon(svg`
  <rect x="2" y="3.2" width="5.4" height="9.6" rx="1.2" />
  <rect x="8.6" y="3.2" width="5.4" height="9.6" rx="1.2" />`);
