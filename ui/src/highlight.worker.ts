// Shiki runs here so tokenization never blocks the diff view (PLAN.md: highlighting off
// the main thread). Uses shiki/core + the JS regex engine (no wasm) and imports only the
// grammars named in highlight.ts's extension map — the full shiki bundle would ship every
// grammar in the installer.
import { createHighlighterCore, type HighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';

export interface HighlightRequest {
  id: number;
  lang: string;
  /** A key of `THEME_LOADERS`. Unknown values fall back to `github-dark`. */
  theme: string;
  oldText: string;
  newText: string;
}

export interface Token {
  t: string;
  c: string | undefined;
}

export interface HighlightResponse {
  id: number;
  old: Token[][] | null;
  new: Token[][] | null;
}

/** Keys must cover every value of EXTENSION_LANGS in highlight.ts. */
const LANG_LOADERS: Record<string, () => Promise<unknown>> = {
  c: () => import('@shikijs/langs/c'),
  cpp: () => import('@shikijs/langs/cpp'),
  csharp: () => import('@shikijs/langs/csharp'),
  css: () => import('@shikijs/langs/css'),
  go: () => import('@shikijs/langs/go'),
  html: () => import('@shikijs/langs/html'),
  java: () => import('@shikijs/langs/java'),
  javascript: () => import('@shikijs/langs/javascript'),
  json: () => import('@shikijs/langs/json'),
  jsonc: () => import('@shikijs/langs/jsonc'),
  jsx: () => import('@shikijs/langs/jsx'),
  kotlin: () => import('@shikijs/langs/kotlin'),
  markdown: () => import('@shikijs/langs/markdown'),
  php: () => import('@shikijs/langs/php'),
  python: () => import('@shikijs/langs/python'),
  ruby: () => import('@shikijs/langs/ruby'),
  rust: () => import('@shikijs/langs/rust'),
  scss: () => import('@shikijs/langs/scss'),
  shellscript: () => import('@shikijs/langs/shellscript'),
  sql: () => import('@shikijs/langs/sql'),
  svelte: () => import('@shikijs/langs/svelte'),
  swift: () => import('@shikijs/langs/swift'),
  toml: () => import('@shikijs/langs/toml'),
  tsx: () => import('@shikijs/langs/tsx'),
  typescript: () => import('@shikijs/langs/typescript'),
  vue: () => import('@shikijs/langs/vue'),
  yaml: () => import('@shikijs/langs/yaml'),
};

/**
 * One entry per named theme in themes.ts, plus the GitHub pair the base themes
 * use. Loaded on demand and one at a time — eagerly registering twenty themes
 * would put every one of them in the worker's first chunk to colour a diff with
 * exactly one.
 *
 * Keys must stay in step with `Seed.shiki`; an id with no loader here silently
 * falls back to `github-dark`, which is a diff in the wrong palette rather than
 * a crash.
 */
const THEME_LOADERS: Record<string, () => Promise<unknown>> = {
  'github-light': () => import('@shikijs/themes/github-light'),
  'github-dark': () => import('@shikijs/themes/github-dark'),
  'catppuccin-latte': () => import('@shikijs/themes/catppuccin-latte'),
  'catppuccin-mocha': () => import('@shikijs/themes/catppuccin-mocha'),
  'rose-pine': () => import('@shikijs/themes/rose-pine'),
  'rose-pine-dawn': () => import('@shikijs/themes/rose-pine-dawn'),
  'rose-pine-moon': () => import('@shikijs/themes/rose-pine-moon'),
  'ayu-light': () => import('@shikijs/themes/ayu-light'),
  'ayu-mirage': () => import('@shikijs/themes/ayu-mirage'),
  'ayu-dark': () => import('@shikijs/themes/ayu-dark'),
  nord: () => import('@shikijs/themes/nord'),
  'tokyo-night': () => import('@shikijs/themes/tokyo-night'),
  'gruvbox-dark-medium': () => import('@shikijs/themes/gruvbox-dark-medium'),
  'gruvbox-light-medium': () => import('@shikijs/themes/gruvbox-light-medium'),
  'everforest-dark': () => import('@shikijs/themes/everforest-dark'),
  'solarized-light': () => import('@shikijs/themes/solarized-light'),
  'solarized-dark': () => import('@shikijs/themes/solarized-dark'),
  dracula: () => import('@shikijs/themes/dracula'),
  'one-dark-pro': () => import('@shikijs/themes/one-dark-pro'),
  'kanagawa-wave': () => import('@shikijs/themes/kanagawa-wave'),
};

let highlighterPromise: Promise<HighlighterCore> | null = null;
const loadedLangs = new Set<string>();
const loadedThemes = new Set<string>();

const getHighlighter = () => {
  highlighterPromise ??= createHighlighterCore({
    themes: [],
    langs: [],
    engine: createJavaScriptRegexEngine({ forgiving: true }),
  });
  return highlighterPromise;
};

/** Resolves to the theme actually registered, which may be the fallback. */
const ensureTheme = async (highlighter: HighlighterCore, theme: string): Promise<string> => {
  const name = THEME_LOADERS[theme] ? theme : 'github-dark';
  if (!loadedThemes.has(name)) {
    await highlighter.loadTheme((await THEME_LOADERS[name]!()) as never);
    loadedThemes.add(name);
  }
  return name;
};

const tokenizeSide = (
  highlighter: HighlighterCore,
  text: string,
  lang: string,
  theme: string,
): Token[][] | null => {
  try {
    const lines = highlighter.codeToTokensBase(text, { lang: lang as never, theme });
    return lines.map((line) => line.map((token) => ({ t: token.content, c: token.color })));
  } catch {
    return null;
  }
};

self.onmessage = async (event: MessageEvent<HighlightRequest>) => {
  const { id, lang, theme, oldText, newText } = event.data;
  const respond = (old: Token[][] | null, newTokens: Token[][] | null) =>
    self.postMessage({ id, old, new: newTokens } satisfies HighlightResponse);

  try {
    const highlighter = await getHighlighter();
    const resolvedTheme = await ensureTheme(highlighter, theme);
    if (!loadedLangs.has(lang)) {
      const loader = LANG_LOADERS[lang];
      if (!loader) {
        respond(null, null);
        return;
      }
      await highlighter.loadLanguage((await loader()) as never);
      loadedLangs.add(lang);
    }
    respond(
      tokenizeSide(highlighter, oldText, lang, resolvedTheme),
      tokenizeSide(highlighter, newText, lang, resolvedTheme),
    );
  } catch {
    respond(null, null);
  }
};
