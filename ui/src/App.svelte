<script lang="ts">
  // Layout Mode editor CANVAS island. The editor document store (#45) is created
  // and hydrated by the coordinator (main.ts) and SHARED with the rail-tools
  // island, so both surfaces read/write ONE store (issue #62). This component owns
  // only the canvas: it renders from `doc.renderModel` (a reactive projection of
  // the store) and stands up the interaction layer (#46).
  //
  // The PURE <LayoutPreview> emits DOM byte-identical (after normalization) to
  // Browse's askama band macro (#44); its `fm-*` styling is inherited from the
  // server's shell.html. Zoom (#62) is a viewport concern: the `.le-workspace`
  // wrapper is CSS-scaled, while moveable hosts on the UNSCALED `.le-stage` so its
  // control box stays crisp; the interaction layer is told the zoom so pointer
  // placement maps back to model coordinates.
  import type { EditorDoc } from './lib/doc.svelte';
  import { CanvasInteraction } from './lib/interaction';
  import LayoutPreview from './lib/LayoutPreview.svelte';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  let stage = $state<HTMLElement>();

  // Stand the interaction layer up once the canvas is in the DOM; tear it down on
  // unmount. moveable + selecto bind to the shared store, not this island.
  let interaction: CanvasInteraction | null = null;
  $effect(() => {
    if (!doc.hydrated || !stage) return;
    const ix = new CanvasInteraction(stage, doc, layoutId);
    ix.setZoom(doc.zoom);
    interaction = ix;
    return () => {
      ix.destroy();
      interaction = null;
    };
  });

  // Keep moveable's control box in sync with the store: re-run on any selection,
  // geometry, or active-tool change (arming a tool drops the target so a press
  // places instead of grabs). The controller ignores syncs during a live gesture.
  $effect(() => {
    void [...doc.selection];
    void doc.renderModel;
    void doc.activeTool;
    interaction?.refresh();
  });

  // Push the current zoom into the interaction layer so placement coordinates
  // compensate for the CSS scale.
  $effect(() => {
    interaction?.setZoom(doc.zoom);
  });
</script>

{#if doc.error}
  <p class="layout-editor-msg layout-editor-error">Failed to load layout: {doc.error}</p>
{:else if doc.hydrated}
  <div
    class="le-stage"
    class:placing={doc.activeTool !== 'pointer'}
    bind:this={stage}
  >
    <div class="le-workspace" style={`transform: scale(${doc.zoom}); transform-origin: top left;`}>
      <LayoutPreview model={doc.renderModel} />
    </div>
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
    overflow: auto;
    min-height: calc(100vh - 8rem);
  }
  /* The zoom layer: transform scales the canvas without reflowing the chrome.
     `width: max-content` keeps it sized to the canvas. */
  .le-workspace {
    width: max-content;
  }
  .le-stage :global(.fm-obj) {
    cursor: move;
    user-select: none;
  }
  /* A tool is armed → the canvas is a placement surface: show a crosshair and
     stop objects advertising "move". */
  .le-stage.placing,
  .le-stage.placing :global(.fm-obj) {
    cursor: crosshair;
  }
</style>
