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
