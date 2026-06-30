<script lang="ts">
  // Layout Mode editor island. On mount it fetches the read model from the
  // engine (ADR #42 HTTP endpoint) and hands it to the PURE <LayoutPreview>,
  // which renders the canvas client-side — DOM byte-identical (after
  // normalization) to Browse's askama band macro (issue #44). The canvas
  // `fm-*` styling is inherited from the server's shell.html; this component
  // only owns its own editor-chrome classes below.
  import type { DesignModel } from './lib/model';
  import LayoutPreview from './lib/LayoutPreview.svelte';

  let { layoutId = '' }: { layoutId?: string } = $props();

  let model = $state<DesignModel | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    let cancelled = false;
    fetch(`/design/${layoutId}/model`)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((data: DesignModel) => {
        if (!cancelled) model = data;
      })
      .catch((e: unknown) => {
        if (!cancelled) error = e instanceof Error ? e.message : String(e);
      });
    return () => {
      cancelled = true;
    };
  });
</script>

{#if error}
  <p class="layout-editor-msg layout-editor-error">Failed to load layout: {error}</p>
{:else if model}
  <LayoutPreview {model} />
{:else}
  <p class="layout-editor-msg">Loading…</p>
{/if}

<style>
  /* Editor chrome only — must NOT define any fm-* class (those live in the
     server's shell.html and are inherited by the design page). */
  .layout-editor-msg {
    margin: 0;
    color: #555;
    font: 0.9rem system-ui, sans-serif;
  }
  .layout-editor-error {
    color: #b00020;
  }
</style>
