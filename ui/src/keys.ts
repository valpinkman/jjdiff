// Shortcut strings: "Mod+Shift+p" — Mod is Cmd on macOS, Ctrl elsewhere (jj-style keymap).

export interface Shortcut {
  meta: boolean;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
}

const IS_MAC = navigator.platform.toUpperCase().includes('MAC');

export function parseShortcut(binding: string): Shortcut {
  const shortcut: Shortcut = { meta: false, ctrl: false, shift: false, alt: false, key: '' };
  for (const part of binding.split('+')) {
    switch (part.trim().toLowerCase()) {
      case 'mod':
        if (IS_MAC) shortcut.meta = true;
        else shortcut.ctrl = true;
        break;
      case 'meta':
      case 'cmd':
        shortcut.meta = true;
        break;
      case 'ctrl':
        shortcut.ctrl = true;
        break;
      case 'shift':
        shortcut.shift = true;
        break;
      case 'alt':
        shortcut.alt = true;
        break;
      default:
        shortcut.key = part.trim().toLowerCase();
    }
  }
  return shortcut;
}

export const matchesShortcut = (event: KeyboardEvent, shortcut: Shortcut): boolean =>
  event.metaKey === shortcut.meta &&
  event.ctrlKey === shortcut.ctrl &&
  event.shiftKey === shortcut.shift &&
  event.altKey === shortcut.alt &&
  event.key.toLowerCase() === shortcut.key;

/** Render a binding for display: "Mod+k" → "⌘K" on macOS, "Ctrl+K" elsewhere. */
export function formatShortcut(binding: string): string {
  const symbols: Record<string, string> = IS_MAC
    ? { mod: '⌘', meta: '⌘', cmd: '⌘', ctrl: '⌃', shift: '⇧', alt: '⌥' }
    : { mod: 'Ctrl', meta: 'Meta', cmd: 'Meta', ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt' };
  const parts = binding.split('+').map((part) => {
    const key = part.trim().toLowerCase();
    return symbols[key] ?? (key.length === 1 ? key.toUpperCase() : part.trim());
  });
  // macOS writes modifiers as an unseparated glyph run (⌘⇧P); elsewhere they join with +.
  return IS_MAC ? parts.join('') : parts.join('+');
}

export interface KeyBinding {
  /** Display form, already formatted. Multiple alternatives render as "j / k". */
  keys: string;
  label: string;
}

export interface KeyGroup {
  title: string;
  bindings: KeyBinding[];
}

/**
 * The shortcut reference shown by the help sheet (`?`).
 *
 * This is documentation, not dispatch — the handlers live in `App.onGlobalKey`
 * and `PatchView`. Keep the two in step: a binding added there without an entry
 * here is a shortcut nobody can discover, which is the bug this sheet exists to
 * fix. `commandBar` is the one configurable binding (`[keymap] command-bar`).
 */
export const shortcutReference = (commandBar: string): KeyGroup[] => [
  {
    title: 'Review',
    bindings: [
      { keys: 'j / k', label: 'Next / previous file' },
      { keys: 'n / p', label: 'Next / previous hunk' },
      { keys: 'v', label: 'Mark file viewed' },
      { keys: 'o', label: 'Open file in editor' },
    ],
  },
  {
    title: 'Navigate',
    bindings: [
      { keys: formatShortcut(commandBar), label: 'Command palette' },
      { keys: formatShortcut('Mod+f'), label: 'Find in diffs' },
      { keys: formatShortcut('Mod+b'), label: 'Show / hide sidebar' },
      { keys: '?', label: 'This shortcut sheet' },
      { keys: 'Esc', label: 'Close overlay' },
    ],
  },
  {
    title: 'Walkthrough',
    bindings: [
      { keys: '→ / ←', label: 'Next / previous step' },
      { keys: 'Esc', label: 'Exit walkthrough' },
    ],
  },
];
