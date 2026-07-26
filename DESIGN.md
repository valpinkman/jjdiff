# Design System: jjdiff

Single source of truth for jjdiff's visual language. Any agent or human touching
`ui/` follows this file; deviations are bugs. Tokens live in
[ui/src/theme.css](ui/src/theme.css) — this document explains the *intent* behind them.

## 1. Visual Theme & Atmosphere

**Tempered industrial brutalism** — a precision instrument, not a consumer app. Two
substrates, one per theme, never mixed within a theme:

- **Light = Swiss Industrial Print**: unbleached paper, carbon ink, hairline rules.
- **Dark = Tactical Telemetry**: deactivated-CRT charcoal, phosphor text.

The diff is the content, the chrome is machinery. Every visual decision defers to code
legibility — brutalism supplies structure and typography, never noise (no scanlines,
dithering, or grain: banned below).

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

| Role | Light (Print) | Dark (Telemetry) | Rule |
|---|---|---|---|
| Background | `#f3f2ed` paper | `#0e0f10` CRT | Never pure `#000`/`#fff` |
| Panel | `#eae8e1` | `#17181a` | The only chrome surface |
| Text | `#131311` ink | `#e6e6e2` phosphor | One gray family per substrate |
| Muted | `#6d6a61` | `#8b8b84` | Labels, hints, gutters |
| Border | `#c9c6ba` | `#2d2f30` | 1px hairlines, structural |
| **Signal** | `#1d5a94` | `#52aec4` | Blueprint blue / cyan phosphor — see below |
| Added | `#1a7f37` on `#e9ffef` | `#57ab5a` on `#10231a` | Semantic, untouchable |
| Removed | `#cf222e` on `#ffedeb` | `#e5534b` on `#291416` | Semantic, untouchable |

### Signal-color rules
- Exactly **one** signal per substrate, same duty: *selection and live state*
  (selection bars, active tab underline + indices, change ids, focus rings) — thin
  bars and text, never large fills. Print mode signals in **blueprint blue** (the
  drafting-ink of machinery blueprints); telemetry mode signals in **cyan phosphor**
  (authentic CRT emission).
- The brutalist canon's aviation red is **deliberately rejected**: red is reserved for
  removed lines and conflicts in a diff tool. Blue/cyan stays clear of both semantic
  colors.
- **Banned:** gradients, translucency, second accents, mixing substrates within one
  theme. Green/red are *diff semantics*, not decoration.

## 3. Typography Rules

- **Telemetry layer (dominant):** `Geist Mono Variable` — ALL controls, labels, tags,
  tabs, ids, counts, and code. 9–12.5px, uppercase for labels, tracking
  `0.06–0.08em` (mechanical spacing).
- **Prose layer:** `Geist Variable` for sentences only — descriptions, narratives,
  hints. Readability text is never uppercase.
- Tabs and steps carry **index numerals** (`01`, `02`, …) in signal amber — CSS
  counters, decimal-leading-zero.
- Narrative text (walkthrough) caps at `68ch`, line-height 1.55.
- **Banned:** serifs anywhere, Inter, decorative display sizes. Hierarchy comes from
  weight + color, never from size inflation.

## 4. Component Stylings

- **Geometry:** every corner is 90 degrees (`--jj-r-*: 0`). No pills, no rounding.
- **Buttons (`.tool`):** mono uppercase, 1px ink border, **inverted on hover**
  (fg/bg swap), `translateY(1px)` mechanical press. Primary = solid ink, amber on
  hover. Disabled = 50% opacity.
- **Selection:** 2–3px signal bar inset on the left edge (`box-shadow: inset 3px 0`),
  never rounded outlines.
- **Tags** (status, badges): 1px-bordered square boxes, mono uppercase 9px
  (`MODIFIED`, `CONFLICT`, bookmark names).
- **Focus:** keyboard focus is always visible — 2px accent outline (controls) or
  3px soft accent ring (`--jj-focus-ring`) on text fields. `outline: none` without a
  replacement is a defect.
- **Shadows:** only the command bar floats — and it gets a **hard offset shadow**
  (`6px 6px 0`), not a soft blur. Nothing else casts shadows.
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
- The sidebar is tabbed (Stack / Files / Steps) — panes swap, the frame never moves.
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
11. Brutalism's noise layer: scanlines, halftone/dither filters, grain overlays,
    phosphor glow. Structure yes, degradation no — code legibility is the product.
12. Aviation-red accents (collides with removed-line semantics).
