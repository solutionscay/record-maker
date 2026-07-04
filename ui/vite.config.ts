import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Build the design-mode islands as static, SSR-off bundles (ADR #42): mounted
// islands, not SPA pages, so there is no index.html. Two entries today —
// `src/main.ts` (Layout Mode editor) and `src/schema/main.ts` (the schema
// builder, #113). Output lands in ui/dist/ with predictable, NON-HASHED
// per-entry filenames (`<entry>.js` / `<entry>.css`) so the axum server and the
// askama templates can reference them by a fixed /ui/... path.
export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        'layout-editor': 'src/main.ts',
        'schema-builder': 'src/schema/main.ts',
      },
      output: {
        // Entry key → `layout-editor.js` / `schema-builder.js`; each entry's CSS
        // is emitted next to it as `<entry>.css`.
        entryFileNames: '[name].js',
        chunkFileNames: '[name].js',
        assetFileNames: '[name][extname]',
      },
    },
  },
});
