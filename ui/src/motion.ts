export type RectSnapshot = Map<string, DOMRect>;

const running = new WeakMap<Element, Animation>();

export function prefersReducedMotion(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

export function cssTime(element: Element, name: string, fallback: number): number {
  const value = getComputedStyle(element).getPropertyValue(name).trim();
  if (!value) return fallback;
  if (value.endsWith('ms')) return Number(value.slice(0, -2)) || fallback;
  if (value.endsWith('s')) return (Number(value.slice(0, -1)) || fallback / 1000) * 1000;
  return Number(value) || fallback;
}

export function cssValue(element: Element, name: string, fallback: string): string {
  return getComputedStyle(element).getPropertyValue(name).trim() || fallback;
}

export function measureByKey<T extends Element>(
  root: ParentNode,
  selector: string,
  keyOf: (element: T) => string | null,
): RectSnapshot {
  const rects: RectSnapshot = new Map();
  for (const element of root.querySelectorAll<T>(selector)) {
    const key = keyOf(element);
    if (!key) continue;
    rects.set(key, element.getBoundingClientRect());
  }
  return rects;
}

export function animateFlip<T extends Element>(
  root: ParentNode,
  selector: string,
  previous: RectSnapshot,
  keyOf: (element: T) => string | null,
  options: KeyframeAnimationOptions,
) {
  if (prefersReducedMotion()) return;
  for (const element of root.querySelectorAll<T>(selector)) {
    const key = keyOf(element);
    const before = key ? previous.get(key) : null;
    if (!before) continue;
    const after = element.getBoundingClientRect();
    const dx = before.left - after.left;
    const dy = before.top - after.top;
    if (Math.abs(dx) < 0.5 && Math.abs(dy) < 0.5) continue;
    running.get(element)?.cancel();
    const animation = element.animate(
      [
        { transform: `translate(${dx}px, ${dy}px)` },
        { transform: 'translate(0, 0)' },
      ],
      options,
    );
    running.set(element, animation);
    animation.finished
      .catch(() => undefined)
      .finally(() => {
        if (running.get(element) === animation) running.delete(element);
      });
  }
}
