# Design System: jjdiff

Single source of truth for jjdiff's visual language. Any agent or human touching
`ui/` follows this file; deviations are bugs. Tokens live in
[ui/src/theme.css](ui/src/theme.css) — this document explains the *intent* behind them.

## 1. Visual Theme & Atmosphere

**Modern, soft, layered.** A calm workspace where the code surface floats above the page
on rounded cards, and everything else recedes. Superseded the earlier brutalist direction
(hard corners, mono-uppercase chrome, offset shadows) — it read as costume rather than
craft, and the uppercase-everything hurt scanning.

- **Depth by surface, not by rule.** Three levels: page (`--jj-bg`) → panel
  (`--jj-bg-panel`) → raised card (`--jj-surface`). The code surface is the brightest
  thing on screen, so the diff reads as the content.
- **Alpha, not opaque.** Borders, muted text and washes are all `rgb(… / α)` so they tint
  with whatever they sit on and stay correct in both themes.
- **Sans for chrome, mono for code.** Uppercase is reserved for small section labels.

The diff is still the content and the chrome is still furniture; what changed is that the
furniture stopped shouting.

- **Density: 8/10 (cockpit dense).** Compact paddings, 13px base, 12.5px code. All
  numerals tabular (`font-variant-numeric: tabular-nums`) — columns of counts must not shimmy.
- **Variance: 2/10 (predictable symmetric).** Fixed three-region layout (header /
  sidebar / main). Asymmetry and broken grids are *banned here* — review tools reward
  spatial memory, not surprise.
- **Motion: 3/10 (restrained fluid).** Micro-transitions only (100–180ms ease).
  Nothing moves unless the user caused it. No scroll choreography, no parallax, ever.
- Both light and dark are first-class; system-following by default, forceable via
  `ui.theme` config.

## 2. Color Palette & Roles

| Role | Light | Dark | Rule |
|---|---|---|---|
| Page | `#f7f7f5` | `#131316` | Never pure `#fff`/`#000` |
| Panel | `#fbfbfa` | `#17171b` | Header, sidebar, banners |
| Surface | `#ffffff` | `#1c1c21` | Cards: code, description, files |
| Text | `#17171a` | `#e8e8ea` | Full-strength ink only for primary content |
| Soft / muted / faint | α 0.72 / 0.5 / 0.32 | α 0.74 / 0.5 / 0.3 | Three levels, all alpha |
| Border | α 0.1 (strong 0.16) | α 0.1 (strong 0.17) | 1px hairlines |
| **Accent** | `#3d7ff5` | `#6ba5ff` | Selection, focus, links, active tab |
| **Ref** | `#b26a10` | `#e0a13f` | Bookmarks only — the one warm colour |
| Added | `#1a7f44` on α 0.09 | `#6fd094` on α 0.1 | Semantic, untouchable |
| Removed | `#c0392f` on α 0.08 | `#ee7e7e` on α 0.09 | Semantic, untouchable |

### Colour rules
- **Two hues carry meaning, and only two.** Accent (blue) = *selection and focus*: active
  tab, selected row, focus ring, links, expanders. Ref (amber) = *bookmarks*, nothing
  else. A row that is both selected and bookmarked shows both, and they don't collide.
- **Diff fills are alpha-tinted, not flat.** Added/removed backgrounds sit at 8–10% over
  the card, so a long diff doesn't read as stripes of solid colour.
- **Added/removed also carry *outcome*, and only outcome.** The green/red pair means one
  thing — this succeeded, this did not — and forge review is the second place that axis
  exists: a check that passed or failed, a proposal that merged or was closed, a reviewer
  who approved or requested changes. Reusing the pair keeps the palette at two hues where
  GitHub would reach for a third (its purple "merged" is exactly the banned case). Anything
  that is *not* a binary outcome — open, draft, queued, in progress — stays neutral, which
  is what makes the coloured states worth noticing.
- **Banned:** a third accent, gradients, coloured shadows, using green/red for decoration
  or for anything that is neither diff nor outcome.

## 3. Typography Rules

- **`Geist Variable` (sans) is the default** — chrome, buttons, tabs, labels, file
  status, descriptions. Weights 500/550/600; 600 for titles and subjects.
- **`Geist Mono Variable` for code and identity** — diff content, change ids, paths,
  counts, revsets, operation argv. Anything the user might copy or compare character by
  character.
- **Uppercase only for small section labels** (`4 FILES`, `PRESENTATION`) at 10.5px /
  650 weight / `0.04em`. Never for buttons or content.
- A change description renders as **subject + body**: first line at 14.5px/600, the rest
  as prose at 13px/1.55. Same idea as a mail client.
- Narrative and prose cap at `70–78ch`. Tabular numerals everywhere.
- **Banned:** serifs, uppercase buttons, decorative display sizes. Hierarchy comes from
  weight, size and colour together — never from size alone.

## 4. Component Stylings

- **Geometry:** rounded — 7px controls, 10px cards, 14px floating panels, 999px pills for
  tags and filter chips. No hard corners.
- **Buttons (`.tool`):** sans, sentence case, 1px hairline, soft card shadow, surface
  lift on hover, `translateY(1px)` press. Primary = filled accent. Danger = red text and
  border, red wash on hover.
- **Selection:** soft accent fill (`--jj-accent-soft`), not a hard bar. The keyboard
  cursor in the diff keeps its 3px inset bar — it is a *cursor*, not a selection.
- **Tags:** pill-shaped, soft-filled, sentence case. Bookmarks amber, states neutral,
  conflicts red.
- **File cards:** each file is a rounded card — header, hunks, code, footer — on the
  raised surface. The gap between cards lives in the header's **transparent top border**,
  never a margin (virtualizers measure `offsetHeight`).
- **Focus:** keyboard focus is always visible — 2px accent outline (controls) or
  3px soft accent ring (`--jj-focus-ring`) on text fields. `outline: none` without a
  replacement is a defect.
- **Shadows:** two only. `--jj-shadow-card` (1px, barely there) on cards and buttons;
  `--jj-shadow-pop` on things that genuinely float (command bar, repo menu). Both tinted
  to the page, never pure black.
- **Cards:** chrome uses flat panels + 1px borders; elevation is reserved for things
  that actually float. Diff rows are flat, full-width, separated by borders — never
  boxed cards (virtualizers measure offsetHeight; margins produce phantom gaps).
- **Empty states:** composed (glyph + title + hint), never a bare sentence.
- **Loading:** pulse the initiating control (`.generating`); skeletons if a whole
  region loads. No spinners.
- **Errors:** inline, plain language, active voice ("jjdiff can't open this
  repository"), never `alert()`, never "Oops".

## 5. Layout Principles

- Fixed grid: `280px` sidebar + fluid main, header spanning both. CSS Grid, no
  percentage flexbox math.
- The sidebar is tabbed (Log / Files / Steps / Ops) — panes swap, the frame never moves.
  It sits on the panel surface with pill filter chips and rounded rows.
- Diff rows are full-bleed rows in one virtualized flat list. Spacing between file
  sections comes from in-row borders/padding, **never margins** (virtualizer
  measurement).
- Code pane and everything above it in the DOM tree is **light DOM** — document CSS
  must reach diff rows, and text selection must cross row boundaries. Shadow DOM is
  allowed only for leaf widgets with no cross-boundary selection (file tree, command
  bar, walkthrough panel).

## 6. Motion & Interaction

- Durations: 100–180ms, `ease`/`ease-out` only. No springs, no bounces — this is an
  instrument, not a toy.
- Transitions animate `background-color`, `border-color`, `color`, `transform`,
  `opacity` — never layout properties.
- Step changes in guided review get one 180ms fade-slide (`.walk-content`, re-keyed
  per step). The command bar enters with a 160ms drop. That is the entire animation
  budget.
- Every interactive element has all four states: rest, hover, active (pressed), focus.

## 7. Anti-Patterns (Banned)

1. Gradients, glassmorphism, noise overlays — legibility is the product.
2. Neon accents, second accents, accent-colored large fills.
3. `outline: none` without a focus replacement.
4. Margins on virtualized rows.
5. Shadow DOM anywhere above the diff pane (severs theme.css + selection — this
   shipped as a real bug once; see the styling-fix change).
6. Spinners, skeleton-less blank loading, bare-text empty states.
7. Marketing-page patterns: heroes, three-card feature rows, testimonial carousels,
   scroll-triggered reveals.
8. Proportional-figure numerals in any column of numbers.
9. `height: 100vh` (the app shell uses its own fixed grid; panes scroll internally).
10. Exclamation marks and "Oops!" in any user-facing string.
11. Noise layers: scanlines, halftone/dither, grain, glow. Legibility is the product.
12. Uppercase button labels, hard-cornered controls, offset "brutalist" shadows — the
    superseded direction; do not reintroduce piecemeal.
13. Flat saturated diff fills. Added/removed backgrounds stay alpha-tinted.
