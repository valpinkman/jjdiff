# Animation Library Research

Context: Lit 3 + Tauri 2 desktop app, light DOM for the app shell and diff rows, `@lit-labs/virtualizer` for large lists, and `DESIGN.md`'s rule that motion is responsive, stateful, and never decorative.

## Recommendation

Use **native Web Animations API plus an app-local FLIP helper** for animated tree/list drag-and-drop, and keep most micro-interactions in CSS using the existing `--jj-t-*` duration and `--jj-ease-*` curve tokens.

Add **Motion (`motion`)** only if the same imperative animation patterns start repeating across components. Do not start with AutoAnimate for the virtualized tree.

## Ranking

1. **Native Web Animations API + FLIP**: best fit for virtualized tree and drag settle animations.
2. **Motion (`motion`)**: best optional dependency for shared imperative animation helpers.
3. **AutoAnimate**: good for small non-virtualized child lists, risky for virtualized rows.
4. **anime.js**: capable, but not more aligned than Motion or native WAAPI.
5. **GSAP**: excellent but too broad/heavy for this app's motion scale.
6. **Popmotion**: lower-level primitives, more assembly than Motion.
7. **View Transitions API**: useful for page/detail transitions, not for primary drag-and-drop reorder.

| Option | Lit/light DOM fit | Virtualized rows | Drag reorder | Subtle interactions | Verdict |
|---|---:|---:|---:|---:|---|
| Native WAAPI + FLIP | Excellent | Excellent | Excellent | Good | Choose first |
| Motion | Excellent | Good | Good | Excellent | Add later if helpers spread |
| AutoAnimate | Good | Weak | Medium | Good | Avoid for virtualized tree |
| anime.js | Good | Good | Medium | Good | Not the best fit |
| GSAP | Good | Good | Good | Excellent | Overpowered default |
| Popmotion | Medium | Good | Medium | Medium | Too low-level |
| View Transitions API | Medium | Weak | Weak | Medium | Use only for larger view transitions |

## Findings

### Native Web Animations API

The Web Animations API exposes browser-native animation control through `Element.animate()` and returns an `Animation` object that can be cancelled, finished, or inspected. That makes it a direct fit for Lit components, light DOM rows, and Tauri WebViews without framework glue.

For a virtualized tree, the app should own the layout animation explicitly:

1. Capture visible row rects keyed by stable row id before the state change.
2. Apply the reorder or tree mutation.
3. Wait for Lit to render.
4. Capture new rects for visible rows.
5. Animate shared rows from `translate(old - new)` back to `translate(0, 0)`.
6. Fade or scale inserted/removed visible rows sparingly.
7. Cancel existing row animations before starting new ones.
8. Respect `prefers-reduced-motion` inside every shadow root that animates.

This is the FLIP technique without a dependency. It also avoids animating layout properties, matching `DESIGN.md`'s motion rules.

Sources:

- MDN Web Animations API: https://developer.mozilla.org/en-US/docs/Web/API/Web_Animations_API
- MDN `Element.animate()`: https://developer.mozilla.org/en-US/docs/Web/API/Element/animate
- MDN `Animation`: https://developer.mozilla.org/en-US/docs/Web/API/Animation

### Motion (`motion`)

Motion's current docs expose vanilla JavaScript animation APIs, including `animate`, independent of React. It is the best dependency candidate if native WAAPI code starts spreading because it offers better ergonomics for imperative DOM animations while staying framework-agnostic.

Motion does not remove the need for explicit FLIP measurement in a virtualized tree; it mainly makes the animation call, sequencing, and spring/easing primitives nicer.

Sources:

- Motion docs: https://motion.dev/docs
- Motion `animate`: https://motion.dev/docs/animate
- Motion repository: https://github.com/motiondivision/motion

### AutoAnimate

AutoAnimate is designed to animate children when they are added, removed, or moved under a parent. That simplicity is attractive for normal lists, but it is a poor default for `@lit-labs/virtualizer` because virtualizers recycle and window DOM nodes. The animation engine sees DOM child movement, while the app cares about logical tree row movement.

Use it only for small, non-virtualized groups such as chips, forms, or static side panels.

Sources:

- AutoAnimate docs: https://auto-animate.formkit.com/
- AutoAnimate repository: https://github.com/formkit/auto-animate

### GSAP

GSAP is a mature imperative animation platform with timelines and broad DOM/SVG support. It would work, but it brings a large API surface and a custom license model relative to the app's current dependency style and motion rules.

Use it only if jjdiff grows complex timeline-heavy or visualization-heavy animation that native WAAPI and Motion cannot keep maintainable.

Sources:

- GSAP docs: https://gsap.com/docs/
- GSAP repository: https://github.com/greensock/GSAP
- GSAP standard license: https://gsap.com/standard-license/

### anime.js

anime.js is a general-purpose JavaScript animation engine for DOM, SVG, and JavaScript objects. It is capable and MIT licensed, but for this app it is less targeted than native WAAPI plus FLIP or Motion. It still requires custom layout measurement for virtualized drag reorder.

Sources:

- anime.js docs: https://animejs.com/documentation/
- anime.js repository: https://github.com/juliangarnier/anime

### Popmotion

Popmotion provides lower-level animation primitives. It can drive custom motion, but the app would need to assemble more DOM integration itself. Motion is the more practical choice for app-level DOM animation.

Sources:

- Popmotion docs: https://popmotion.io/
- Popmotion repository: https://github.com/Popmotion/popmotion

### View Transitions API

The View Transitions API captures old and new visual states and animates transitions between them. It can be useful for larger screen changes or detail-pane transitions after WebView support is verified, but it is not the right primitive for continuous drag-and-drop row movement in a virtualized tree.

Sources:

- MDN View Transitions API: https://developer.mozilla.org/en-US/docs/Web/API/View_Transitions_API
- Chrome View Transitions docs: https://developer.chrome.com/docs/web-platform/view-transitions/
- CSS View Transitions specification: https://www.w3.org/TR/css-view-transitions-1/

## Proposed architecture

Create a tiny `ui/src/motion.ts` module before adding a dependency. It should expose:

- `measureRows(container, selector, keyOf)`
- `animateFlip(previousRects, nextRects, options)`
- `prefersReducedMotion()`
- shared durations and curves sourced from CSS custom properties

For the log/tree drag-and-drop path, wire it around the state change that reorders or rebases rows. Existing drag state in `ui/src/log-graph.ts` already identifies source, destination, barred rows, and drop target classes, so the animation layer can stay visual and not own domain state.

Recommended default animation:

```ts
row.animate(
  [
    { transform: `translate(${dx}px, ${dy}px)` },
    { transform: 'translate(0, 0)' },
  ],
  {
    duration: 180,
    easing: 'cubic-bezier(0.22, 1, 0.36, 1)',
  },
)
```

Keep CSS for hover, press, focus, row highlighting, drop target glow, overlay entry, and card rise. Use WAAPI only when the start and end positions are computed in JavaScript or when an animation must be cancelled/coordinated with a drag gesture.

## Decision

Start with **native WAAPI + FLIP**. Reconsider **Motion** when there are at least three components with imperative animation code or when spring/sequencing primitives become a clear maintenance win.
