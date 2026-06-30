import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

// Enable Vite/TypeScript preprocessing for <script lang="ts"> in components.
export default {
  preprocess: vitePreprocess(),
};
