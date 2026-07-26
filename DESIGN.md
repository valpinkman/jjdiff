# Design System: jjdiff

Single source of truth for jjdiff's visual language. Any agent or human touching
`ui/` follows this file; deviations are bugs. Tokens live in
[ui/src/theme.css](ui/src/theme.css) — this document explains the *intent* behind them.

## 1. Visual Theme & Atmosphere

A **quiet instrument**. jjdiff is a code-review cockpit: the diff is the content, the
chrome is furniture. Every visual decision defers to code legibility.

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
| Background | `#ffffff` | `#14161a` | Never pure `#000` |
| Panel | `#f6f7f9` | `#1c1f24` | The only chrome surface |
| Text | `#1c2128` | `#dde1e6` | Cool gray family only |
| Muted | `#6a737d` | `#8b939e` | Labels, hints, gutters |
| Border | `#d7dbe0` | `#2c313a` | 1px, always |
| **Accent** | `#5e6ad2` | `#7b87e0` | THE one accent — see below |
| Added | `#1a7f37` on `#e9ffef` | `#57ab5a` on `#10231a` | Semantic, untouchable |
| Removed | `#cf222e` on `#ffedeb` | `#e5534b` on `#291416` | Semantic, untouchable |

### Accent rules
- Exactly **one** accent: desaturated indigo (S≈55%). It marks *selection and
  identity* (selected change, active tab, focus rings, change ids) — never large fills.
- Subtle accent surfaces are derived, not new colors:
  `color-mix(in srgb, var(--jj-accent) 6–12%, var(--jj-bg))`.
- **Banned:** neon/saturated "AI purple" (the old `#7c5cff`), gradients of any kind,
  second accents, warm grays mixed into the cool family. Green/red are *diff
  semantics*, not decoration — never reuse them for chrome.

## 3. Typography Rules

- **UI chrome:** `Geist Variable` (bundled via Fontsource — CSP forbids remote fonts).
  Weights: 400 body, 500 controls, 600 emphasis, 700 reserved for tiny labels.
- **Code & all numerals:** `Geist Mono Variable`, falling back to `ui-monospace`.
  Line-height 1.5 in diffs. Change ids, counts, and line numbers are always mono.
- Micro-labels (section titles, step progress) are 10px, 700, uppercase,
  `letter-spacing: 0.06–0.08em` — used sparingly, one per panel.
- Narrative text (walkthrough) caps at `68ch`, line-height 1.55.
- **Banned:** serifs anywhere, Inter, decorative display sizes. Hierarchy comes from
  weight + color, never from size inflation.

## 4. Component Stylings

- **Radius scale:** `--jj-r-sm: 5px` (buttons, badges, rows) · `--jj-r-md: 8px`
  (inputs, cards) · `--jj-r-lg: 12px` (floating panels). Never one radius everywhere;
  never pill buttons.
- **Buttons (`.tool`):** 1px border, transparent-to-tinted hover
  (`accent 6%` mix), `scale(0.97)` on press, 150ms transitions. Primary = solid accent,
  darkened 12% on hover. Disabled = 50% opacity, no cursor.
- **Focus:** keyboard focus is always visible — 2px accent outline (controls) or
  3px soft accent ring (`--jj-focus-ring`) on text fields. `outline: none` without a
  replacement is a defect.
- **Shadows:** only on floating elements (command bar), tinted to the background hue
  (`--jj-shadow-pop`), never pure black.
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
