import { defineConfig } from 'vite';

export default defineConfig({
  root: 'ui',
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
  },
  worker: {
    // Shiki lazy-loads grammars via dynamic import; iife workers can't code-split.
    format: 'es',
  },
});
