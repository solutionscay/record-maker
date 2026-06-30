import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Build the Layout Mode editor as a static, SSR-off bundle (ADR #42): a single
// mounted island, not an SPA page, so there is no index.html — the entry is
// src/main.ts. Output lands in ui/dist/ with predictable, NON-HASHED filenames
// (layout-editor.js / layout-editor.css) so the axum server and the askama
// templates can reference them by a fixed /ui/... path.
export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      input: 'src/main.ts',
      output: {
        entryFileNames: 'layout-editor.js',
        chunkFileNames: 'layout-editor-[name].js',
        assetFileNames: (asset) => {
          if (asset.name && asset.name.endsWith('.css')) {
            return 'layout-editor.css';
          }
          return '[name][extname]';
        },
      },
    },
  },
});
