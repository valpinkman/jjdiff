# Design System: jjdiff

Single source of truth for jjdiff's visual language. Any agent or human touching
`ui/` follows this file; deviations are bugs. Tokens live in
[ui/src/theme.css](ui/src/theme.css) — this document explains the *intent* behind them.

## 1. Visual Theme & Atmosphere

**Flat surfaces, pill controls, quietly alive.** A calm workspace where the code surface
sits on the page as a plain slab, everything else recedes, and the few things that move
do so because something is actually happening. Superseded the earlier brutalist direction
(mono-uppercase chrome, offset shadows) — it read as costume rather than craft.

- **Depth by surface, not by rule.** Three levels: page (`--jj-bg`) → panel
  (`--jj-bg-panel`) → raised card (`--jj-surface`). The code surface is the brightest
  thing on screen, so the diff reads as the content.
- **Alpha, not opaque.** Borders, muted text and washes are all `rgb(… / α)` so they tint
  with whatever they sit on and stay correct in both themes.
- **The page is one flat colour.** An "ambient" pair of radial tints lived here for a
  while, meant to give the page a direction. On a light page it was invisible; on a dark
  one it was a blue smear in the gap beside the first card, and the eye read it as a
  rendering artefact rather than as atmosphere. A page that is one colour is not a failure
  of imagination when everything on it is a card.
- **Sans for chrome, mono for code.** Uppercase is reserved for small section labels.
- **Everything comes from a scale.** Space, radius, elevation and duration each have a
  named ramp and nothing is allowed outside it. A value picked by eye is what makes an
  interface feel assembled rather than designed — two paddings a pixel apart read as a
  mistake even when nobody can name it.

The diff is still the content and the chrome is still furniture.

- **Density: 7/10.** 14px base, 12.5px code — one step up from where this started, and
  the paddings follow from the ramp rather than being tuned per element. All
  numerals tabular (`font-variant-numeric: tabular-nums`) — columns of counts must not shimmy.
- **Variance: 2/10 (predictable symmetric).** Fixed four-region layout (header / icon
  rail / sidebar / main). Asymmetry and broken grids are *banned here* — review tools
  reward spatial memory, not surprise.
- **Motion: 5/10 (responsive, never decorative).** Every interactive element moves under
  the cursor; state changes animate; two signature effects mark states that have
  duration. Nothing moves that the user did not cause, except the indicators in §7,
  which move *because a machine is still working*. No scroll choreography, no parallax,
  ever.
- Both light and dark are first-class; system-following by default, forceable via
  `ui.theme` config.

## 2. Color Palette & Roles

**The neutrals are a pure grey ramp — zero chroma**, rebased on shadcn/ui's default
theme. The previous set was warm, and that was the single biggest thing making a careful
layout look untended: three off-whites within 3% of each other, so the page → panel →
card hierarchy the layout depends on was invisible, and the warmth read as a screen that
had yellowed rather than as a decision. Grey also gets out of the way of the only colours
that carry meaning here.

| Role | Light | Dark | Rule |
|---|---|---|---|
| Page | `#f4f4f5` | `#09090b` | Never pure `#fff`/`#000` on the page |
| Panel | `#fafafa` | `#0e0e11` | Header, sidebar |
| Surface | `#ffffff` | `#18181b` | Cards: code, detail, banners |
| Text | `#18181b` | `#fafafa` | Full-strength ink only for primary content |
| Soft / muted / faint | α 0.72 / 0.55 / 0.36 | α 0.74 / 0.52 / 0.33 | Three levels, all alpha |
| Border | α 0.12 (strong 0.19) | α 0.10 (strong 0.17) | 1px hairlines, visible enough to draw an edge |
| **Primary** | `#18181b` → `#27272a` | `#fafafa` → `#ffffff` | The main action. Neutral, flat |
| **Accent** | `#2563eb` | `#60a5fa` | Selection, focus, links, active tab — *nothing else* |
| **Ref** | `#b26a10` | `#e0a13f` | Bookmarks only — the one warm colour |
| Added | `#1a7f44` on α 0.09 | `#6fd094` on α 0.1 | Semantic, untouchable |
| Removed | `#c0392f` on α 0.08 | `#ee7e7e` on α 0.09 | Semantic, untouchable |

### Colour rules
- **The primary action is neutral, not accent.** A blue fill puts the app's loudest colour
  next to a diff whose green and red are the only colours that mean anything, and it made
  the accent do two jobs at once — "press this" and "this is selected". Flat, too: a
  gradient on the highest-contrast object in the view reads as a theme rather than as a
  control.
- **Two hues carry meaning, and only two.** Accent (blue) = *selection and focus*: active
  tab, selected row, focus ring, links, expanders. Ref (amber) = *bookmarks*, nothing
  else. A row that is both selected and bookmarked shows both, and they don't collide.
- **Diff fills are alpha-tinted, not flat.** Added/removed backgrounds sit at 8–10% over
  the card, so a long diff doesn't read as stripes of solid colour.
- **Added/removed also carry *outcome*.** The green/red pair means one thing — this
  succeeded, this did not — and forge review is the second place that axis exists: a check
  that passed or failed, a reviewer who approved or requested changes, a proposal open or
  closed. Anything that is *not* an outcome — queued, in progress, draft — stays neutral.
- **Proposal state is the one deliberate exception, and it borrows the forge's own
  vocabulary:** grey draft, green open, **purple merged**, red closed. Purple is a third
  hue and would otherwise be banned; it earns its place because *merged* is genuinely not
  on the pass/fail axis — a landed proposal is neither a success nor a failure, it is
  gone — and colouring it green made merged work read as a passing check. Recognition
  beats purity for a vocabulary people already read fluently on GitHub and GitLab.
  `--jj-merged-fg` is tuned deeper and less saturated than GitHub's `#8250df` so it sits
  at the same visual weight as our green and red.
- **Purple means merged and nothing else.** It is not a second accent: no purple buttons,
  links, selection or fills anywhere outside the state pill.
- **The lane palette is the one multi-hue set**, and it exists for the log graph, where
  the job is telling six branches apart. `jj-orbs` borrows it rather than inventing a
  seventh and eighth colour. Nothing else may.
- **Banned:** a *fourth* hue, gradient fills on controls, coloured shadows, using
  green/red for decoration or for anything that is neither diff nor outcome.

### Named themes

Light and dark are the *base* theme and live in `theme.css`. Everything else — Nord,
Catppuccin, Ayu, Rosé Pine, Gruvbox, Tokyo Night, Everforest, Solarized, Dracula, One
Dark, Kanagawa — lives in [ui/src/themes.ts](ui/src/themes.ts) and is **derived**, not
written out. Each is seeded with about a dozen colours (four surfaces, ink, accent, ref,
the diff pair, merged, six lanes) and the rest of the token set is computed from them
using the same percentages the base theme uses by hand.

Three rules keep that honest:

- **Never hand-write a full palette.** Nineteen copies of the token set guarantees that
  the twentieth token added to the app is defined in three of them, and the drift shows
  up as one theme with a white border where the others have grey.
- **Named palettes are inline custom properties on `:root`.** That is what lets them beat
  both the `:root` block and the `prefers-color-scheme` block in theme.css. `system`
  clears them and hands control back to the media query.
- **The diff follows the chrome.** Every theme names a shiki theme, loaded on demand in
  the worker. Nord chrome around GitHub-coloured code is the single thing that would make
  the whole feature look fake.

Two attributes carry the state and they answer different questions: `data-jj-theme` is
the *mode* (`light`/`dark`), which is all theme.css needs; `data-jj-palette` is the
identity, which is what the highlighter needs.

The chooser is a **picker, not a list** — twenty entries in the command palette would be
twenty lines of text for a decision that is entirely visual. Swatches, grouped by family,
and hovering one applies it live.

### On shadcn/ui

It cannot be used here and should not be attempted: it is React source built on Radix
React primitives, Tailwind and `class-variance-authority`, and Radix has no
web-component equivalent. Tailwind would also only reach half of this app — the shell and
diff pane are light DOM, but every leaf widget is a shadow root that document CSS
cannot cross. The Lit ports that exist are single-maintainer packages, and adopting one
would mean replacing widgets jjdiff already has.

What *was* taken is the part that carries the look: the neutral ramp above, the
discipline that every surface is a card on a background with a visible hairline, and the
neutral primary. Those are plain CSS. If a headless primitive is ever genuinely needed,
the framework-agnostic options are Zag.js or Shoelace — not shadcn.

## 3. Typography Rules

- **Every view has exactly one title**, at `--jj-text-title` (20px / 650 / `-0.02em`), and
  it is the name of the thing on screen: the change's subject on the detail card,
  "Working copy", "Operation Log", the proposal's title. The app used to top out at
  14.5px, which is what made a dense tool read as a database client — no size said
  *this is what you are looking at*. Two titles in one view means one of them is not a
  title.
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

## 4. The scales

Four ramps. Every padding, corner, shadow and duration in the app is one of these
values; there are no others.

| Ramp | Tokens | Use |
|---|---|---|
| Space | `--jj-s-1…8` = 4, 6, 8, 12, 16, 20, 28, 40 | 4pt, doubling at the top |
| Radius | surfaces `--jj-r-md/lg` = 6, 8; controls `--jj-r-pill` | see below — two families, not a ramp |
| Type | `--jj-text-xs/sm/base/md/lg/title` = 11, 12, 13, 14, 16, 22 | one `title` per view |
| Elevation | `--jj-shadow-xs/card/raised/pop` | Each is a contact shadow plus a diffuse one, tinted to the page |
| Duration | `--jj-t-1…4` = 120, 180, 260, 420ms | Paired with a curve, below |

**Two families, not a ramp: controls are pills, surfaces are softened.** Anything you
*press or type into* is fully round (`--jj-r-xs/sm/chip` all alias `--jj-r-pill`).
Anything you put things *into* — a card, a panel, a pane — gets a small radius: `8px`
for a card on the page, `6px` for the file cards inside the diff, so nesting reads as
nesting rather than as two unrelated shapes.

Surfaces still carry **no border**; they are told apart from the page by background and
shadow. That is the part that matters and it has not changed. The earlier rule here was
that surfaces are square, and its diagnosis — a 16px card holding 9px buttons, each with
its own hairline, made the app read as boxes inside boxes — was about *nesting and
hairlines*, not about radius as such. Three nested frames at every point on screen is the
failure; rounding the one edge each surface already has is not.

**Elevation is a ladder, not a set of options.** `raised` is the hover state of `card`;
nothing skips a step, and `pop` belongs only to things that genuinely float. In dark, a
drop shadow is nearly invisible, so each step also carries a hairline of light along its
top edge — that is what reads as height on a dark page.

## 5. Component Stylings

- **The main pane is a page and everything on it is a card** — the change detail, the
  describe box, every banner. Each gets `--jj-surface`, `--jj-shadow-card` and
  `--jj-r-lg`, no border, with the page showing between them. They used to run edge to
  edge with a rule under each, and the pane read as a stack of dividers: a form, not a
  workspace. The shadow is the only thing left saying "in front of", which is one signal
  instead of three.
- **A completed action reports itself in a card, not a floating toast.** jj's own
  narration of the last mutation sits in the main pane beside the error and info lines,
  as `.outcome` — same surface, radius and shadow as every other banner, because the
  thing it describes is the pane it sits in. It carries an Undo (`jj op revert` on the
  operation the mutation created, so it stays right even once it is no longer the tip)
  and a Dismiss, and the next action replaces it. It is **not** timed: a card that
  removes itself is motion the user did not cause (§7), and jj's narration is often the
  only place a command says what it touched. Actions that are not mutations — a review
  posted, the terminal helper installed, a review copied — report here too with no
  operation, and then there is no Undo button rather than one that cannot work. It is a
  sibling of the pane's other cards, so a document that owns the pane (§6) hides it with
  them; submitting a review from the proposal view and reverting from the operation log
  therefore narrate into a card nobody can see, which is a gap rather than a rule.
- **The diff pane is the exception: a container of cards, not a card.** It has no
  background, no shadow and no radius; the *file* cards are the objects, and the space
  between them is the page. It was a panel-coloured slab for a while, with the file cards
  sitting on it — which made the gap between two files a third colour that meant nothing,
  neither page nor card. The sidebar is the same call for the same reason: it is
  navigation, not content, so it is a transparent column and the rows carry their own
  hover and selected fills.
- **A card is header then content.** The header is `.chip` + a heading stack (title over
  the identity line) + right-aligned meta + the fold chevron. Both the change detail and
  the describe box are built this way, so a card the user has not met before is already
  legible.
- **`.chip`:** a round 28px well (22px at `.sm`) holding one icon
  on a soft fill. It is the smallest unit of the language and its job is to give an icon
  a *body*, so a row starting with one has a consistent left edge and optical weight
  whatever glyph is in it. Variants `accent`/`warn`/`good`/`ref` reuse existing palette
  roles; a chip never introduces a colour.
- **Text fields are pills with the icon inside them** (`.field` + `.field-icon`), not
  rectangles with a label above. One box, one target, one focus ring — and the icon
  takes the accent while the field is focused.
- **A card inside a card loses its border and takes `--jj-wash` instead** — a well in the
  surface rather than a second frame on it. Two frames around one list is the most
  common way a dense UI starts to look accidental.
- **Buttons (`.tool`):** sans, sentence case, 1px hairline, pill-shaped. They
  lift toward the cursor on hover (`translateY(-1px)` + one step up the elevation ramp)
  and are pushed back down on press (`scale(0.975)`, at the faster duration, so the
  control feels like it resists rather than sags). Primary = the accent sweep, with a
  sheen that crosses on hover and is gone by the time the pointer settles. Danger = red
  text and border, red wash on hover.
- **Runs of icon buttons become a `.tool-group`:** the group owns the border and shadow,
  the keys own only their hover, and the hairlines between them come from the group so
  there is no doubled seam. Four loose keys read as clutter; one instrument does not.
- **Selection is one shape everywhere:** a soft accent fill (`--jj-accent-soft`) with a
  2px accent bar on the leading edge, and the bar grows from nothing rather than
  switching on. Rows are square and full-bleed so the bar sits on the pane's own edge —
  a rounded row inside a padded list reads as a card in a stack of cards. Log rows,
  palette entries, and any list added later. Two exceptions: the keyboard cursor in the
  diff keeps its 3px inset bar (it is a *cursor*, not a selection), and the icon rail
  has no bar at all, because its leading edge is the window's.
- **Tabs are a segmented control** on a recessed track, and the active pane is marked by a
  raised slab that *slides* between segments. The movement is what says the panes are
  neighbours on one strip; a fill that blinks on somewhere else does not.
- **Tags:** pill-shaped, soft-filled, sentence case. Bookmarks amber, states neutral,
  conflicts red. **The conflict chip explains itself:** `⚠ conflicts` names the state but
  not what it conflicts *with*, so it carries a tooltip — "This pull request has conflicts
  with its base branch" — in the banner and in the proposal view alike. The view used to
  have the bare chip; that was the copy that had drifted, not a decision, and the view is
  the screen where the reader has the least other context.
- **A file is a card inside the diff pane** — `--jj-r-md` on the header's top corners
  and the footer's bottom ones, full-bleed horizontally (no side gutter: the pane is not
  a frame, so an inset one would draw a second edge around every file). The header
  carries a hairline **under** it as well as over: the header is the file's identity and
  its actions, the code is the content, and with no rule between them the diff's first
  line reads as part of the title bar.
- **The gap between files is a row of its own** (`.file-gap`), never a margin and never a
  transparent border. A margin is height the virtualizer cannot see — it measures
  `offsetHeight`, so rows drift out of position as you scroll. A transparent border can
  be measured but cannot be *rounded*: `background-clip: padding-box` clips the surface
  to a box whose corner radius is the border radius minus the border width, so an 18px
  strip flattens any radius under it. All the card's hairlines are *inset shadows* for a
  related reason — they follow the radius instead of squaring off the corner.
- **Menus** — repo switcher, file context menu, the change's More overflow — are one
  component in three places: `--jj-r-md`, `--jj-shadow-pop`, and `jj-pop`, which unfolds
  the panel downward out of the control that opened it.
- **Overlay chrome is one file, not eight.** `ui/src/overlay.ts` holds the scrim
  (`overlayChrome`: fixed inset, `rgb(0 0 0 / 0.22)` under a 3px blur, `z-index: 110`,
  the `scrim-in`/`pop` keyframes and the reduced-motion opt-out every shadow root has to
  make for itself), the panel header (`panelHeader`: `16px 20px 12px`, `--jj-text-title`
  at 650 with `-0.02em`, the muted `.hint` beside it, the `.spacer` after it) and the
  footer button (`panelButton`: `.btn`, sans, hairline, `--jj-r-pill`, `.primary` on
  `--jj-primary`, `:disabled` at 0.45). An overlay composes
  `[overlayChrome, panelHeader, panelButton, …]` with its own block last, so
  a local rule wins on equal specificity — that is where the settings header's
  `border-bottom`, the rebase picker's truncated command preview and the diagram view's
  centred, darker scrim live. The reduced-motion opt-out is the one rule that must *not*
  lose that race, and a media query carries no specificity of its own, so it is written
  `:host .panel` rather than `.panel`: written plainly it sat before every panel's
  `animation: pop` and the panels animated anyway. Three overlays sit off that layer on purpose: the palette
  and the shortcuts sheet at 100, the prompt at 200, so a confirmation raised by a
  command lands on top of whatever asked for it. The button is always `.btn`, never the
  bare element: these shadow roots also hold list rows, toggles and swatches that are
  buttons and are not this.
- **Focus:** keyboard focus is always visible — 2px accent outline (controls) or
  3px soft accent ring (`--jj-focus-ring`) on text fields. `outline: none` without a
  replacement is a defect.
- **Scrollbars are ours.** Thin, rounded, inset from their track, and drawn only when the
  pane they belong to is hovered or focused. This app is a stack of independently
  scrolling panes and system bars would put four grey rails on screen at once.
- **Empty states:** composed (glyph + title + hint), never a bare sentence.
- **Errors:** inline, plain language, active voice ("jjdiff can't open this
  repository"), never `alert()`, never "Oops".

## 6. Layout Principles

- Fixed grid: `52px` icon rail + `292px` sidebar + 4px resize column + fluid main, header
  spanning all of them. CSS Grid, no percentage flexbox math.
- **The sidebar's panes are switched from the icon rail**, not from tabs. Four labels plus
  two count badges never fit the sidebar at a readable size, and a four-segment strip made
  the indicator look wrong on the short label because equal quarters gave "Log" the same
  width as "Files 17". The rail scales to any number of panes and hands the sidebar's
  whole width back to content; the pane's name is not hidden, it is the sidebar's title.
- **The header is the window's drag region.** `titleBarStyle: Overlay` puts the WebView
  over the title bar, so the OS stops receiving the mousedown that moves the window —
  `data-tauri-drag-region` gives it back. It must be on the spacer as well as the header:
  Tauri only starts a drag when the *event target itself* carries the attribute.
- Diff rows are full-bleed rows in one virtualized flat list. Spacing between file
  sections comes from in-row borders/padding, **never margins** (virtualizer
  measurement).
- Code pane and everything above it in the DOM tree is **light DOM** — document CSS
  must reach diff rows, and text selection must cross row boundaries. Shadow DOM is
  allowed only for leaf widgets with no cross-boundary selection (file tree, command
  bar, walkthrough panel, orbs, and the overlays).
- **An overlay dismisses on a click that landed on its scrim, not on any click it
  receives.** The scrim is the host element, so a listener bound to the host sees an
  inside click *retargeted to the host* and reads it as an outside one — every overlay
  closed the moment you touched anything in it, which presented as a filter box that
  would not focus and a radio that would not take. Test `composedPath()[0] === this`.
- **An overlay with a text field owns the keyboard while it is open.** Handle keys on
  the panel and stop propagation; a window-level listener is for Escape alone. Retargeting
  again: the app's global handler decides "is someone typing" from the event's target, and
  by the time the event reaches window that target is the overlay host, not the input two
  shadow roots down — so `j`, `k`, `c` and `v` would scroll the diff behind the dialog
  while you filter it.
- **A mode announces itself in a bar and can be left from there.** Picking hunks to split
  or squash puts checkboxes on rows that are otherwise read-only, which is exactly the kind
  of change that needs a visible reason: an accented bar states what is selected, offers
  All/None, and carries both Cancel and the action. Escape leaves it. Where the mode's
  action cannot succeed — nothing picked, or (for a split alone) everything — the button is
  disabled with the reason in its title, rather than deferring to an error from jj that
  names neither. One bar serves both verbs, and says which one it is in every label it
  carries, down to the checkbox tooltips: the same tick means two different things.
- **A gesture that rewrites history proposes, it does not act.** Dragging a change onto
  another in the log is the one route in that can start a rebase by accident — everything
  else is a menu item or a filled-in picker — so the drop opens a confirmation naming both
  ends. While the drag is in flight the graph shows what it will not accept: rows that
  would form a cycle dim and refuse it, so the answer arrives under the pointer rather
  than as an error afterwards.
- **Hierarchy beats completeness in a row of controls.** Four verbs stay out; the rest go
  behind one overflow, with anything destructive below a rule at the bottom. Nine
  buttons in a row is nine decisions of equal weight, and the one that erases a commit
  was the same size as the one that opens it.
- **Toolbars are ordered by what the verbs do**, not by how often they were added.
  The header reads left to right as *bring work in → rearrange what you have → step
  back*: fetch, absorb, undo. Controls that change nothing about the repository (the
  diff layout toggle) get their own group.
- **A document owns the pane; it does not hang above the diff.** The proposal view, the
  operation log and the walkthrough overview are all prose of unbounded length, and each
  one started life as a block above the code. In every case the thing being reviewed began
  halfway down the window, and the banners still on screen were context for a diff nobody
  was looking at. A view claims `flex: 1; min-height: 0` and its siblings are hidden by one
  `main.showing-* > *:not(.the-view)` rule. The exception is anything that is the only way
  *out* of the view, that describes the document rather than the code, or that reports what
  the view just did — guided review's nav bar and its stale warning stay, and so do the
  outcome card and the status line. That last one is not symmetry: submitting a review is
  reachable only from the proposal view and undo, restore and revert only from the operation
  log, so hiding them left a *failed* submission saying nothing while the composer emptied,
  which reads as success.
- **Never hide options behind a hover.** The log scope was a deck of pills that
  collapsed to single initials and named themselves one at a time; five of six choices
  were invisible and the only way to learn them was to sweep the pointer across the row.
  A named button that opens a labelled menu shows everything in one glance for one
  click. Menus also show the underlying revset — this app is for people who write them,
  and a preset is a shortcut, not a replacement.

## 7. Motion & Interaction

**Curves.** Duration says how far a thing travelled; the curve says what kind of thing
it is.

| Token | Curve | Meaning |
|---|---|---|
| `--jj-ease-out` | `cubic-bezier(0.22, 1, 0.36, 1)` | The default. Decelerates into place — the element is arriving, not being thrown. |
| `--jj-ease-in-out` | `cubic-bezier(0.65, 0, 0.35, 1)` | Something travelling between two known positions. |
| `--jj-ease-pop` | `cubic-bezier(0.34, 1.32, 0.64, 1)` | A hair of overshoot, for things that appear from nothing: menus, the palette. |

- Transitions animate `background-color`, `border-color`, `color`, `transform`,
  `opacity`, `box-shadow` — never layout properties. The one exception is
  `grid-template-rows` in the fold, below, which is the only way to animate to a
  content-derived height.
- **One fold.** Everything that opens and closes uses `.fold` (`grid-template-rows:
  1fr → 0fr`, inner element `overflow: hidden; min-height: 0`) with a `.fold-chevron`
  that rotates rather than swapping glyphs. Folded content stays **mounted** — an
  element that does not exist cannot animate out.
- **Views rise.** A screen entering the main pane, an empty state's copy, a card
  appearing: all use `jj-rise` (8px, fade). One arrival gesture, so a screen the user has
  not seen before behaves the way the rest of the app taught them.
- Every interactive element has all four states: rest, hover, active (pressed), focus.
- **`prefers-reduced-motion` is honoured, and it is not optional.** theme.css carries the
  app-wide switch, but a universal rule in a document stylesheet does not cross a shadow
  boundary — **every shadow root that animates must opt out for itself**. Motion here is
  affordance, so it degrades to the same states arriving instantly, never to a different
  interface.

### The two signature effects

Each marks something that has **duration**, and each is allowed in exactly one class of
place. Used anywhere else they are decoration, which is the failure mode this section
exists to prevent.

- **Beam** (`.beam`, theme.css) — a light running the border of something still working.
  A registered `--jj-beam-angle` drives a conic gradient; `mask-composite` hollows out
  the middle so it is a border rather than a fill; `drop-shadow` after the mask is what
  makes it glow along the arc and nothing else. **Allowed on: an element whose work is
  in flight** and cannot report progress. A beam on an idle control is a progress bar
  that never moves.
- **Orbs** (`jj-orbs`) — the agent thinking indicator. Three blurred lights on
  incommensurate orbits (3.1 / 3.9 / 4.7s, so the loop takes two minutes to repeat),
  blended `plus-lighter`, in the log lane hues. **Allowed on: waiting for an agent.**
  Walkthrough generation is the only current caller.
**An empty state gets no animated surface.** A WebGL sheet of liquid metal lived behind
"Nothing to review" for a while, on the argument that it was the one place with no diff
whose legibility it could cost. That argument was right and still not enough: the pane a
reviewer stares at all day should not be the pane that performs when it is idle, and a
background that moves when nothing is happening reads as the app doing something. Empty
states are composed text (§5), and that is all.

## 8. Anti-Patterns (Banned)

1. Glassmorphism and noise overlays — legibility is the product. Surfaces are **opaque**:
   a card's background is a colour, never the page showing through it. The ambient wash in
   §1 is painted *on* the page, under opaque cards; that is the whole allowance. (One blur
   exists: the command palette's backdrop, so the app stays readable as context behind
   it.)
2. Neon accents, second accents, accent-colored large fills, two-colour gradients.
3. `outline: none` without a focus replacement.
4. Margins on virtualized rows.
5. Shadow DOM anywhere above the diff pane (severs theme.css + selection — this
   shipped as a real bug once; see the styling-fix change).
6. Spinners. Waiting is shown by the three effects in §7 or not at all.
7. Marketing-page patterns: heroes, three-card feature rows, testimonial carousels,
   scroll-triggered reveals.
8. Proportional-figure numerals in any column of numbers.
9. `height: 100vh` (the app shell uses its own fixed grid; panes scroll internally).
10. Exclamation marks and "Oops!" in any user-facing string.
11. Noise layers: scanlines, halftone/dither, grain.
12. Uppercase button labels, hard-cornered controls, offset "brutalist" shadows — the
    superseded direction; do not reintroduce piecemeal.
13. Flat saturated diff fills. Added/removed backgrounds stay alpha-tinted.
14. A hard-coded padding, radius, shadow or duration. Use the ramps in §4; if none fits,
    the ramp is wrong and changes — one value, everywhere.
15. Motion the user did not cause, unless it is reporting that something is **live** —
    the two effects in §7, the ring a running CI check sends out, the halo breathing
    under the working-copy dot. That list is the whole allowance. Anything else that
    loops is decoration.
