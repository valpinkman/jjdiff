// VS Code-style file-type icons: colored letter badges for known types, generic glyphs
// otherwise. Colors are fixed brand-ish hues (recognizable in both themes).
import { svg, type TemplateResult } from 'lit';

const badge = (label: string, bg: string, fg = '#ffffff'): TemplateResult => svg`
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <rect x="1" y="1" width="14" height="14" rx="3.5" fill="${bg}" />
    <text
      x="8"
      y="11.2"
      text-anchor="middle"
      font-size="${label.length > 2 ? 5.5 : 6.8}"
      font-weight="700"
      fill="${fg}"
      font-family="-apple-system, system-ui, sans-serif"
    >${label}</text>
  </svg>`;

const fileGlyph = svg`
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path
      d="M4 1.5h5.2L12.5 5v9a1 1 0 0 1-1 1h-7.5a1 1 0 0 1-1-1v-11.5a1 1 0 0 1 1-1z"
      fill="none" stroke="currentColor" stroke-width="1.1" stroke-linejoin="round" />
    <path d="M9.2 1.5V5h3.3" fill="none" stroke="currentColor" stroke-width="1.1" />
  </svg>`;

const imageGlyph = svg`
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <rect x="1.5" y="2.5" width="13" height="11" rx="1.5"
      fill="none" stroke="#9d86ff" stroke-width="1.1" />
    <circle cx="5.4" cy="6.2" r="1.3" fill="#e8b339" />
    <path d="M3 12l3.4-3.6 2.3 2.3 2.6-3 3.2 4.3" fill="none" stroke="#57ab5a" stroke-width="1.1" stroke-linejoin="round" />
  </svg>`;

export const folderIcon = (open: boolean): TemplateResult => svg`
  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
    <path
      d="M1.5 3.5a1 1 0 0 1 1-1h3.2l1.6 1.8h6.2a1 1 0 0 1 1 1v7.2a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"
      fill="${open ? '#8fa8c7' : '#7d95b5'}" opacity="0.9" />
    ${open ? svg`<path d="M1.5 6.5h13l-1.3 6a1 1 0 0 1-1 .8H3.4a1 1 0 0 1-1-.8z" fill="#a5bbd6" />` : ''}
  </svg>`;

const ICONS: Record<string, () => TemplateResult> = {
  cjs: () => badge('JS', '#e8c33d', '#332b00'),
  css: () => badge('#', '#663399'),
  html: () => badge('<>', '#dd4b25'),
  jpeg: () => imageGlyph,
  jpg: () => imageGlyph,
  js: () => badge('JS', '#e8c33d', '#332b00'),
  json: () => badge('{}', '#8a929c'),
  jsonc: () => badge('{}', '#8a929c'),
  jsx: () => badge('JS', '#00b4d8', '#00303a'),
  md: () => badge('M', '#4a90d9'),
  mjs: () => badge('JS', '#e8c33d', '#332b00'),
  png: () => imageGlyph,
  py: () => badge('PY', '#3572A5'),
  rs: () => badge('RS', '#ce5c26'),
  scss: () => badge('#', '#c6538c'),
  sh: () => badge('$', '#4d5a66'),
  svg: () => imageGlyph,
  swift: () => badge('SW', '#f05138'),
  toml: () => badge('T', '#9c4221'),
  ts: () => badge('TS', '#3178c6'),
  tsx: () => badge('TS', '#00b4d8', '#00303a'),
  webp: () => imageGlyph,
  yaml: () => badge('Y', '#a07d3a'),
  yml: () => badge('Y', '#a07d3a'),
};

export const fileIcon = (path: string): TemplateResult => {
  const dot = path.lastIndexOf('.');
  const ext = dot === -1 ? '' : path.slice(dot + 1).toLowerCase();
  return ICONS[ext]?.() ?? fileGlyph;
};
