// Shiki runs here so tokenization never blocks the diff view (PLAN.md: highlighting off
// the main thread). Uses shiki/core + the JS regex engine (no wasm) and imports only the
// grammars named in highlight.ts's extension map — the full shiki bundle would ship every
// grammar in the installer.
import { createHighlighterCore, type HighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';

export interface HighlightRequest {
  id: number;
  lang: string;
  theme: 'github-light' | 'github-dark';
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

let highlighterPromise: Promise<HighlighterCore> | null = null;
const loadedLangs = new Set<string>();

const getHighlighter = () => {
  highlighterPromise ??= createHighlighterCore({
    themes: [import('@shikijs/themes/github-light'), import('@shikijs/themes/github-dark')],
    langs: [],
    engine: createJavaScriptRegexEngine({ forgiving: true }),
  });
  return highlighterPromise;
};

const tokenizeSide = (
  highlighter: HighlighterCore,
  text: string,
  lang: string,
  theme: HighlightRequest['theme'],
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
      tokenizeSide(highlighter, oldText, lang, theme),
      tokenizeSide(highlighter, newText, lang, theme),
    );
  } catch {
    respond(null, null);
  }
};
