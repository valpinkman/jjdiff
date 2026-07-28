/**
 * Named palettes, on top of light/dark.
 *
 * A theme is seeded from about a dozen colours and the rest of the token set is
 * *derived* — every alpha variant, every fill, every shadow. Hand-writing 19
 * complete palettes would guarantee that the twentieth token added to the app
 * is defined in three of them, and the drift would show up as one theme with a
 * white border where the others have a grey one.
 *
 * Derivation is `color-mix(… transparent)` rather than a computed rgba, so the
 * alpha lands on whatever the surface underneath actually is. That is the same
 * rule the hand-written base theme follows (DESIGN.md §2).
 *
 * Applied as **inline custom properties on `:root`**. `theme.css` defines the
 * light palette on `:root` and the dark one inside a `prefers-color-scheme`
 * block; an inline property beats both, which is what lets a named theme win
 * regardless of what the OS is doing. Clearing them (`system`) hands control
 * straight back to the media query.
 */

export type ThemeMode = 'light' | 'dark';

interface Seed {
  id: string;
  label: string;
  /** Groups the picker; also the family name people actually say out loud. */
  group: string;
  mode: ThemeMode;
  /** Shiki theme for the diff. Code in the chrome's palette or the whole thing looks fake. */
  shiki: string;
  /** Page → panel → card → recessed. */
  bg: string;
  panel: string;
  surface: string;
  surface2: string;
  /** Full-strength ink. Every muted level is derived from it. */
  fg: string;
  /** Selection and focus. */
  accent: string;
  /** Bookmarks — the one warm colour. */
  ref: string;
  added: string;
  removed: string;
  merged: string;
  /** Log graph lanes. Lane 0 should be the accent so the mainline is "the app's colour". */
  lanes: readonly [string, string, string, string, string, string];
}

/**
 * `system`, `light` and `dark` are not in here: they are the hand-written base
 * theme in theme.css. Everything below overrides it.
 */
const SEEDS: readonly Seed[] = [
  // ---- Catppuccin ----
  {
    id: 'catppuccin-latte',
    label: 'Latte',
    group: 'Catppuccin',
    mode: 'light',
    shiki: 'catppuccin-latte',
    bg: '#e6e9ef',
    panel: '#eff1f5',
    surface: '#ffffff',
    surface2: '#dce0e8',
    fg: '#4c4f69',
    accent: '#1e66f5',
    ref: '#df8e1d',
    added: '#40a02b',
    removed: '#d20f39',
    merged: '#8839ef',
    lanes: ['#1e66f5', '#8839ef', '#179299', '#df8e1d', '#ea76cb', '#04a5e5'],
  },
  {
    id: 'catppuccin-mocha',
    label: 'Mocha',
    group: 'Catppuccin',
    mode: 'dark',
    shiki: 'catppuccin-mocha',
    bg: '#11111b',
    panel: '#181825',
    surface: '#1e1e2e',
    surface2: '#313244',
    fg: '#cdd6f4',
    accent: '#89b4fa',
    ref: '#f9e2af',
    added: '#a6e3a1',
    removed: '#f38ba8',
    merged: '#cba6f7',
    lanes: ['#89b4fa', '#cba6f7', '#94e2d5', '#fab387', '#f5c2e7', '#89dceb'],
  },

  // ---- Rosé Pine ----
  {
    id: 'rose-pine-dawn',
    label: 'Dawn',
    group: 'Rosé Pine',
    mode: 'light',
    shiki: 'rose-pine-dawn',
    bg: '#f2e9e1',
    panel: '#faf4ed',
    surface: '#fffaf3',
    surface2: '#f2e9e1',
    fg: '#575279',
    accent: '#286983',
    ref: '#ea9d34',
    added: '#56949f',
    removed: '#b4637a',
    merged: '#907aa9',
    lanes: ['#286983', '#907aa9', '#56949f', '#ea9d34', '#b4637a', '#d7827e'],
  },
  {
    id: 'rose-pine',
    label: 'Main',
    group: 'Rosé Pine',
    mode: 'dark',
    shiki: 'rose-pine',
    bg: '#12101c',
    panel: '#191724',
    surface: '#1f1d2e',
    surface2: '#26233a',
    fg: '#e0def4',
    accent: '#9ccfd8',
    ref: '#f6c177',
    added: '#31748f',
    removed: '#eb6f92',
    merged: '#c4a7e7',
    lanes: ['#9ccfd8', '#c4a7e7', '#31748f', '#f6c177', '#eb6f92', '#ebbcba'],
  },
  {
    id: 'rose-pine-moon',
    label: 'Moon',
    group: 'Rosé Pine',
    mode: 'dark',
    shiki: 'rose-pine-moon',
    bg: '#1c1a2b',
    panel: '#232136',
    surface: '#2a273f',
    surface2: '#393552',
    fg: '#e0def4',
    accent: '#9ccfd8',
    ref: '#f6c177',
    added: '#3e8fb0',
    removed: '#eb6f92',
    merged: '#c4a7e7',
    lanes: ['#9ccfd8', '#c4a7e7', '#3e8fb0', '#f6c177', '#eb6f92', '#ea9a97'],
  },

  // ---- Ayu ----
  {
    id: 'ayu-light',
    label: 'Light',
    group: 'Ayu',
    mode: 'light',
    shiki: 'ayu-light',
    bg: '#f3f4f5',
    panel: '#fafafa',
    surface: '#ffffff',
    surface2: '#eceef0',
    fg: '#5c6166',
    accent: '#399ee6',
    ref: '#fa8d3e',
    added: '#86b300',
    removed: '#e65050',
    merged: '#a37acc',
    lanes: ['#399ee6', '#a37acc', '#4cbf99', '#fa8d3e', '#f07171', '#55b4d4'],
  },
  {
    id: 'ayu-mirage',
    label: 'Mirage',
    group: 'Ayu',
    mode: 'dark',
    shiki: 'ayu-mirage',
    bg: '#171b24',
    panel: '#1f2430',
    surface: '#242936',
    surface2: '#2d3441',
    fg: '#cccac2',
    accent: '#73d0ff',
    ref: '#ffcc66',
    added: '#d5ff80',
    removed: '#f28779',
    merged: '#dfbfff',
    lanes: ['#73d0ff', '#dfbfff', '#95e6cb', '#ffad66', '#f28779', '#5ccfe6'],
  },
  {
    id: 'ayu-dark',
    label: 'Dark',
    group: 'Ayu',
    mode: 'dark',
    shiki: 'ayu-dark',
    bg: '#080a0f',
    panel: '#0b0e14',
    surface: '#0f131a',
    surface2: '#1a1f29',
    fg: '#bfbdb6',
    accent: '#39bae6',
    ref: '#e6b450',
    added: '#aad94c',
    removed: '#d95757',
    merged: '#d2a6ff',
    lanes: ['#39bae6', '#d2a6ff', '#95e6cb', '#ffb454', '#f07178', '#59c2ff'],
  },

  // ---- Nord ----
  {
    id: 'nord',
    label: 'Nord',
    group: 'Nord',
    mode: 'dark',
    shiki: 'nord',
    bg: '#242933',
    panel: '#2e3440',
    surface: '#3b4252',
    surface2: '#434c5e',
    fg: '#eceff4',
    accent: '#88c0d0',
    ref: '#ebcb8b',
    added: '#a3be8c',
    removed: '#bf616a',
    merged: '#b48ead',
    lanes: ['#88c0d0', '#b48ead', '#8fbcbb', '#ebcb8b', '#bf616a', '#81a1c1'],
  },

  // ---- Tokyo Night ----
  {
    id: 'tokyo-night',
    label: 'Tokyo Night',
    group: 'Tokyo Night',
    mode: 'dark',
    shiki: 'tokyo-night',
    bg: '#16161e',
    panel: '#1a1b26',
    surface: '#1f2335',
    surface2: '#292e42',
    fg: '#c0caf5',
    accent: '#7aa2f7',
    ref: '#e0af68',
    added: '#9ece6a',
    removed: '#f7768e',
    merged: '#bb9af7',
    lanes: ['#7aa2f7', '#bb9af7', '#7dcfff', '#e0af68', '#f7768e', '#2ac3de'],
  },

  // ---- Gruvbox ----
  {
    id: 'gruvbox-dark',
    label: 'Dark',
    group: 'Gruvbox',
    mode: 'dark',
    shiki: 'gruvbox-dark-medium',
    bg: '#1d2021',
    panel: '#282828',
    surface: '#32302f',
    surface2: '#3c3836',
    fg: '#ebdbb2',
    accent: '#83a598',
    ref: '#fabd2f',
    added: '#b8bb26',
    removed: '#fb4934',
    merged: '#d3869b',
    lanes: ['#83a598', '#d3869b', '#8ec07c', '#fabd2f', '#fb4934', '#fe8019'],
  },
  {
    id: 'gruvbox-light',
    label: 'Light',
    group: 'Gruvbox',
    mode: 'light',
    shiki: 'gruvbox-light-medium',
    bg: '#f2e5bc',
    panel: '#fbf1c7',
    surface: '#fbf1c7',
    surface2: '#ebdbb2',
    fg: '#3c3836',
    accent: '#076678',
    ref: '#b57614',
    added: '#79740e',
    removed: '#9d0006',
    merged: '#8f3f71',
    lanes: ['#076678', '#8f3f71', '#427b58', '#b57614', '#9d0006', '#af3a03'],
  },

  // ---- Everforest ----
  {
    id: 'everforest-dark',
    label: 'Dark',
    group: 'Everforest',
    mode: 'dark',
    shiki: 'everforest-dark',
    bg: '#272e33',
    panel: '#2d353b',
    surface: '#343f44',
    surface2: '#3d484d',
    fg: '#d3c6aa',
    accent: '#7fbbb3',
    ref: '#dbbc7f',
    added: '#a7c080',
    removed: '#e67e80',
    merged: '#d699b6',
    lanes: ['#7fbbb3', '#d699b6', '#83c092', '#dbbc7f', '#e67e80', '#e69875'],
  },

  // ---- Solarized ----
  {
    id: 'solarized-light',
    label: 'Light',
    group: 'Solarized',
    mode: 'light',
    shiki: 'solarized-light',
    bg: '#eee8d5',
    panel: '#fdf6e3',
    surface: '#fdf6e3',
    surface2: '#eee8d5',
    fg: '#586e75',
    accent: '#268bd2',
    ref: '#b58900',
    added: '#859900',
    removed: '#dc322f',
    merged: '#6c71c4',
    lanes: ['#268bd2', '#6c71c4', '#2aa198', '#b58900', '#d33682', '#cb4b16'],
  },
  {
    id: 'solarized-dark',
    label: 'Dark',
    group: 'Solarized',
    mode: 'dark',
    shiki: 'solarized-dark',
    bg: '#002028',
    panel: '#002b36',
    surface: '#073642',
    surface2: '#0d4451',
    fg: '#93a1a1',
    accent: '#268bd2',
    ref: '#b58900',
    added: '#859900',
    removed: '#dc322f',
    merged: '#6c71c4',
    lanes: ['#268bd2', '#6c71c4', '#2aa198', '#b58900', '#d33682', '#cb4b16'],
  },

  // ---- Editor classics ----
  {
    id: 'dracula',
    label: 'Dracula',
    group: 'Dracula',
    mode: 'dark',
    shiki: 'dracula',
    bg: '#21222c',
    panel: '#282a36',
    surface: '#2f313f',
    surface2: '#44475a',
    fg: '#f8f8f2',
    accent: '#bd93f9',
    ref: '#f1fa8c',
    added: '#50fa7b',
    removed: '#ff5555',
    merged: '#ff79c6',
    lanes: ['#bd93f9', '#ff79c6', '#8be9fd', '#ffb86c', '#ff5555', '#50fa7b'],
  },
  {
    id: 'one-dark',
    label: 'One Dark',
    group: 'One',
    mode: 'dark',
    shiki: 'one-dark-pro',
    bg: '#21252b',
    panel: '#282c34',
    surface: '#2f343d',
    surface2: '#3a3f4b',
    fg: '#abb2bf',
    accent: '#61afef',
    ref: '#e5c07b',
    added: '#98c379',
    removed: '#e06c75',
    merged: '#c678dd',
    lanes: ['#61afef', '#c678dd', '#56b6c2', '#e5c07b', '#e06c75', '#d19a66'],
  },
  {
    id: 'kanagawa',
    label: 'Wave',
    group: 'Kanagawa',
    mode: 'dark',
    shiki: 'kanagawa-wave',
    bg: '#16161d',
    panel: '#1f1f28',
    surface: '#22222c',
    surface2: '#2a2a37',
    fg: '#dcd7ba',
    accent: '#7e9cd8',
    ref: '#e6c384',
    added: '#98bb6c',
    removed: '#c34043',
    merged: '#957fb8',
    lanes: ['#7e9cd8', '#957fb8', '#6a9589', '#e6c384', '#d27e99', '#ffa066'],
  },
];

/** What the picker and the config accept, in the order the picker shows them. */
export interface ThemeOption {
  id: string;
  label: string;
  group: string;
  mode: ThemeMode | 'system';
  /** Four colours for the picker's swatch: page, surface, accent, ref. */
  swatch: readonly [string, string, string, string];
}

/**
 * The base three come first and are handled by theme.css rather than by a seed.
 * Their swatches are the literal values from that file — duplicated on purpose,
 * because a swatch is a picture of a theme, not its source of truth.
 */
const BASE: readonly ThemeOption[] = [
  {
    id: 'system',
    label: 'System',
    group: 'Base',
    mode: 'system',
    swatch: ['#f4f4f5', '#18181b', '#2563eb', '#b26a10'],
  },
  {
    id: 'light',
    label: 'Light',
    group: 'Base',
    mode: 'light',
    swatch: ['#f4f4f5', '#ffffff', '#2563eb', '#b26a10'],
  },
  {
    id: 'dark',
    label: 'Dark',
    group: 'Base',
    mode: 'dark',
    swatch: ['#09090b', '#18181b', '#60a5fa', '#e0a13f'],
  },
];

export const THEMES: readonly ThemeOption[] = [
  ...BASE,
  ...SEEDS.map((seed) => ({
    id: seed.id,
    label: seed.label,
    group: seed.group,
    mode: seed.mode,
    swatch: [seed.bg, seed.surface, seed.accent, seed.ref] as const,
  })),
];

const BY_ID = new Map(SEEDS.map((seed) => [seed.id, seed]));

export function isKnownTheme(id: string): boolean {
  return id === 'system' || id === 'light' || id === 'dark' || BY_ID.has(id);
}

/** The shiki theme to tokenize the diff with, for any theme id. */
export function shikiTheme(id: string, systemPrefersDark: boolean): string {
  const seed = BY_ID.get(id);
  if (seed) return seed.shiki;
  if (id === 'dark') return 'github-dark';
  if (id === 'light') return 'github-light';
  return systemPrefersDark ? 'github-dark' : 'github-light';
}

const mix = (colour: string, percent: number) =>
  `color-mix(in srgb, ${colour} ${percent}%, transparent)`;

/**
 * Every token a seed implies.
 *
 * The percentages are the same ones the hand-written base theme uses, so a
 * named theme is the base theme with different anchors rather than a different
 * design. Two exceptions are keyed off `mode`: shadows (a drop shadow is
 * invisible on a dark page and needs a lit top edge instead) and the sheen.
 */
function tokens(seed: Seed): Record<string, string> {
  const dark = seed.mode === 'dark';
  return {
    'color-scheme': seed.mode,

    '--jj-bg': seed.bg,
    '--jj-bg-panel': seed.panel,
    '--jj-surface': seed.surface,
    '--jj-surface-2': seed.surface2,

    '--jj-fg': seed.fg,
    '--jj-fg-soft': mix(seed.fg, 74),
    '--jj-fg-muted': mix(seed.fg, 54),
    '--jj-fg-faint': mix(seed.fg, 36),

    '--jj-border': mix(seed.fg, dark ? 13 : 16),
    '--jj-border-strong': mix(seed.fg, dark ? 20 : 24),
    '--jj-wash': mix(seed.fg, dark ? 7 : 6),

    // The primary action is the ink itself, inverted — the same rule as the base
    // theme, which is what keeps "the filled button" reading as one idea across
    // nineteen palettes instead of nineteen different brand colours.
    '--jj-primary': seed.fg,
    '--jj-primary-hi': `color-mix(in srgb, ${seed.fg} 88%, ${seed.accent})`,
    '--jj-primary-fg': seed.bg,

    '--jj-accent': seed.accent,
    '--jj-accent-soft': mix(seed.accent, dark ? 18 : 14),
    '--jj-accent-line': mix(seed.accent, 45),

    '--jj-ref': seed.ref,
    '--jj-ref-soft': mix(seed.ref, dark ? 18 : 15),

    '--jj-lane-0': seed.lanes[0],
    '--jj-lane-1': seed.lanes[1],
    '--jj-lane-2': seed.lanes[2],
    '--jj-lane-3': seed.lanes[3],
    '--jj-lane-4': seed.lanes[4],
    '--jj-lane-5': seed.lanes[5],

    '--jj-added-fg': seed.added,
    '--jj-added-bg': mix(seed.added, dark ? 14 : 13),
    '--jj-added-mark': mix(seed.added, dark ? 28 : 26),
    '--jj-removed-fg': seed.removed,
    '--jj-removed-bg': mix(seed.removed, dark ? 14 : 12),
    '--jj-removed-mark': mix(seed.removed, dark ? 28 : 24),
    '--jj-merged-fg': seed.merged,
    '--jj-merged-bg': mix(seed.merged, dark ? 18 : 14),

    '--jj-hunk-bg': mix(seed.fg, 5),
    '--jj-num-fg': mix(seed.fg, 34),

    '--jj-sheen': dark ? 'rgb(255 255 255 / 0.14)' : 'rgb(255 255 255 / 0.16)',
    '--jj-shadow-xs': dark ? '0 1px 1px rgb(0 0 0 / 0.24)' : '0 1px 1px rgb(0 0 0 / 0.04)',
    '--jj-shadow-card': dark
      ? '0 1px 2px rgb(0 0 0 / 0.3), inset 0 1px 0 rgb(255 255 255 / 0.03)'
      : '0 1px 2px rgb(0 0 0 / 0.05), 0 1px 1px rgb(0 0 0 / 0.03)',
    '--jj-shadow-raised': dark
      ? '0 6px 18px rgb(0 0 0 / 0.44), 0 1px 2px rgb(0 0 0 / 0.3), inset 0 1px 0 rgb(255 255 255 / 0.05)'
      : '0 4px 12px rgb(0 0 0 / 0.09), 0 1px 2px rgb(0 0 0 / 0.05)',
    '--jj-shadow-pop': dark
      ? '0 24px 64px rgb(0 0 0 / 0.6), 0 4px 12px rgb(0 0 0 / 0.44), inset 0 1px 0 rgb(255 255 255 / 0.05)'
      : '0 20px 56px rgb(0 0 0 / 0.18), 0 4px 12px rgb(0 0 0 / 0.08), 0 1px 2px rgb(0 0 0 / 0.05)',
  };
}

/** Every property `apply` may have set, so switching themes never leaves a stale one behind. */
const ALL_KEYS = Object.keys(tokens(SEEDS[0]!));

/**
 * Put a theme on the document.
 *
 * `system`, `light` and `dark` clear every inline property and fall back to the
 * `data-jj-theme` attribute that theme.css already keys off — the base theme
 * stays declarative, and only the named palettes are computed.
 */
export function applyThemeTokens(id: string) {
  const root = document.documentElement;
  for (const key of ALL_KEYS) root.style.removeProperty(key);

  // Two attributes, because two different questions get asked. `jjTheme` is the
  // *mode* — theme.css keys its dark block off it and does not care which
  // palette produced it. `jjPalette` is the identity, which is what the
  // highlighter needs to pick a matching shiki theme.
  root.dataset['jjPalette'] = id;

  const seed = BY_ID.get(id);
  if (!seed) {
    if (id === 'light' || id === 'dark') {
      root.dataset['jjTheme'] = id;
      root.style.colorScheme = id;
    } else {
      delete root.dataset['jjTheme'];
      root.style.removeProperty('color-scheme');
    }
    return;
  }

  root.dataset['jjTheme'] = seed.mode;
  const values = tokens(seed);
  for (const [key, value] of Object.entries(values)) root.style.setProperty(key, value);
}
