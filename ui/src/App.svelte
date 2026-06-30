<script lang="ts">
  // Layout Mode editor island. On mount it fetches the read model from the
  // engine (ADR #42 HTTP endpoint) and HYDRATES the editor document store (#45)
  // — the reactive core that owns document/session/presence state and the undo
  // history. The canvas renders from `doc.renderModel` (a reactive projection of
  // the store), NOT the raw fetch, so edits re-render reactively. The PURE
  // <LayoutPreview> emits DOM byte-identical (after normalization) to Browse's
  // askama band macro (issue #44); its `fm-*` styling is inherited from the
  // server's shell.html.
  //
  // The interaction layer (#46) is wired as editor chrome: a CanvasInteraction
  // binds moveable (drag/resize/snap/group) + selecto (marquee multi-select) to
  // the store and persists committed geometry via the bulk axum contract. It
  // attaches its own listeners to the stage wrapper and never touches the pure
  // <LayoutPreview> DOM, so the #44 parity golden is untouched.
  import type { DesignModel } from './lib/model';
  import { EditorDoc } from './lib/doc.svelte';
  import { CanvasInteraction } from './lib/interaction';
  import LayoutPreview from './lib/LayoutPreview.svelte';

  let { layoutId = '' }: { layoutId?: string } = $props();

  const doc = new EditorDoc();
  let error = $state<string | null>(null);
  let stage = $state<HTMLElement>();

  $effect(() => {
    let cancelled = false;
    fetch(`/design/${layoutId}/model`)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json();
      })
      .then((data: DesignModel) => {
        if (!cancelled) doc.hydrate(data);
      })
      .catch((e: unknown) => {
        if (!cancelled) error = e instanceof Error ? e.message : String(e);
      });
    return () => {
      cancelled = true;
    };
  });

  // Stand the interaction layer up once the canvas is in the DOM; tear it down on
  // unmount / layout change. moveable + selecto bind to the store, not this island.
  let interaction: CanvasInteraction | null = null;
  $effect(() => {
    if (!doc.hydrated || !stage) return;
    const ix = new CanvasInteraction(stage, doc, layoutId);
    interaction = ix;
    return () => {
      ix.destroy();
      interaction = null;
    };
  });

  // Keep moveable's control box in sync with the store: re-run on any selection
  // or geometry change (e.g. an undo while the selection holds). The controller
  // ignores syncs during a live gesture, which it owns.
  $effect(() => {
    void [...doc.selection];
    void doc.renderModel;
    interaction?.refresh();
  });
</script>

{#if error}
  <p class="layout-editor-msg layout-editor-error">Failed to load layout: {error}</p>
{:else if doc.hydrated}
  <div class="le-stage" bind:this={stage}>
    <LayoutPreview model={doc.renderModel} />
  </div>
{:else}
  <p class="layout-editor-msg">Loading…</p>
{/if}

<style>
  /* Editor chrome only — must NOT define any fm-* class (those live in the
     server's shell.html and are inherited by the design page). The drag affords
     come from :global rules scoped under the stage, so they never touch the
     parity-checked canvas markup. */
  .layout-editor-msg {
    margin: 0;
    color: #555;
    font: 0.9rem system-ui, sans-serif;
  }
  .layout-editor-error {
    color: #b00020;
  }
  .le-stage {
    position: relative;
    touch-action: none;
  }
  .le-stage :global(.fm-obj) {
    cursor: move;
    user-select: none;
  }
</style>
