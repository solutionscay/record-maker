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
  import { setPartHeight as persistPartHeight } from './lib/persist';
  import { lerror, llog } from './lib/log';
  import LayoutPreview from './lib/LayoutPreview.svelte';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  let stage = $state<HTMLElement>();
  let resizingPartId = $state<number | null>(null);
  const DESIGN_PAGE_WIDTH = 760;

  const partBands = $derived.by(() => {
    let top = 0;
    return doc.renderModel.parts.map((part) => {
      const band = { part, top };
      top += part.height;
      return band;
    });
  });

  function partLabel(kind: string): string {
    switch (kind) {
      case 'header':
        return 'Header';
      case 'body':
        return 'Body';
      case 'footer':
        return 'Footer';
      case 'subsummary':
        return 'Sub-summary';
      case 'grandsummary':
        return 'Grand summary';
      default:
        return kind;
    }
  }

  // Stand the interaction layer up once the canvas is in the DOM; tear it down on
  // unmount. moveable + selecto bind to the shared store, not this island.
  let interaction = $state<CanvasInteraction | null>(null);
  $effect(() => {
    if (!doc.hydrated || !stage) return;
    const ix = new CanvasInteraction(stage, doc, layoutId);
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

  function selectPart(id: number, event: MouseEvent): void {
    if (doc.activeTool !== 'pointer') return;
    event.stopPropagation();
    doc.selectPart(id);
  }

  function startPartResize(id: number, top: number, event: PointerEvent): void {
    if (doc.activeTool !== 'pointer' || !stage) return;
    event.preventDefault();
    event.stopPropagation();
    const canvas = stage.querySelector<HTMLElement>('.fm-canvas');
    if (!canvas) return;
    const minHeight = doc.minPartHeight(id);
    const canvasRect = canvas.getBoundingClientRect();
    resizingPartId = id;
    doc.selectPart(id);
    llog('resize', 'part resizeStart', { id, minHeight });

    const move = (e: PointerEvent) => {
      const zoom = doc.zoom || 1;
      const modelY = (e.clientY - canvasRect.top) / zoom;
      const height = Math.max(minHeight, Math.round(modelY - top));
      doc.setPartHeight(id, height);
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      resizingPartId = null;
      const part = doc.getPart(id);
      if (!part) return;
      doc.mark();
      llog('resize', 'part resizeEnd', { id, height: part.height });
      void persistPartHeight(layoutId, id, part.height).catch((e) => {
        lerror('persist', 'failed to persist part height', e);
        doc.setError(e instanceof Error ? e.message : String(e));
      });
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up, { once: true });
  }
</script>

{#if doc.error}
  <p class="layout-editor-msg layout-editor-error">Failed to load layout: {doc.error}</p>
{:else if doc.hydrated}
  <div
    class="le-stage"
    class:placing={doc.activeTool !== 'pointer'}
    class:no-object-selection={doc.selection.size === 0}
    bind:this={stage}
  >
    <div class="le-workspace" style={`transform: scale(${doc.zoom}); transform-origin: top left;`}>
      <div class="le-canvas-wrap">
        <LayoutPreview model={doc.renderModel} />
        <div class="le-part-overlays" style={`width: ${doc.renderModel.width}px; min-width: ${DESIGN_PAGE_WIDTH}px;`}>
          {#each partBands as band (band.part.id)}
            <button
              type="button"
              class="le-part-label"
              class:selected={doc.selectedPartId === band.part.id}
              style={`top: ${band.top}px; height: ${band.part.height}px;`}
              title={`Select ${partLabel(band.part.kind)} band`}
              onclick={(e) => selectPart(band.part.id, e)}
            >
              {partLabel(band.part.kind)}
            </button>
            <button
              type="button"
              class="le-part-resize"
              class:selected={doc.selectedPartId === band.part.id}
              class:resizing={resizingPartId === band.part.id}
              style={`top: ${band.top + band.part.height - 4}px;`}
              title={`Resize ${partLabel(band.part.kind)} band`}
              onpointerdown={(e) => startPartResize(band.part.id, band.top, e)}
            ></button>
          {/each}
        </div>
      </div>
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
    position: relative;
    width: max-content;
    --le-part-gutter: 1.45rem;
    padding-left: var(--le-part-gutter);
  }
  .le-canvas-wrap {
    position: relative;
    width: max-content;
    margin: 1rem 0;
  }
  .le-part-overlays {
    position: absolute;
    top: 1px;
    left: 0;
    pointer-events: none;
  }
  .le-part-label {
    position: absolute;
    left: calc(-1 * var(--le-part-gutter));
    width: var(--le-part-gutter);
    padding: 0.35rem 0;
    border: 1px solid #9d9780;
    border-right: 0;
    border-radius: 0;
    background: #f1eed8;
    color: #2f3742;
    font: 600 0.68rem/1 system-ui, sans-serif;
    writing-mode: vertical-rl;
    text-orientation: mixed;
    transform: rotate(180deg);
    overflow: hidden;
    white-space: nowrap;
    cursor: pointer;
    pointer-events: auto;
    box-sizing: border-box;
  }
  .le-part-label.selected {
    border-color: #1f6feb;
    background: #e8f1ff;
    color: #174ea6;
  }
  .le-part-resize {
    position: absolute;
    left: 0;
    right: 0;
    height: 8px;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: row-resize;
    pointer-events: auto;
  }
  .le-part-resize::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    top: 4px;
    border-top: 1px solid rgba(31, 111, 235, 0.45);
  }
  .le-part-resize.selected::after,
  .le-part-resize.resizing::after {
    border-top: 2px solid #1f6feb;
  }
  .le-stage :global(.le-draw-preview) {
    position: absolute;
    box-sizing: border-box;
    z-index: 1000;
    pointer-events: none;
    border: 1px dashed #1f6feb;
    background: rgba(31, 111, 235, 0.08);
  }
  .le-stage :global(.le-draw-ellipse) {
    border-radius: 50%;
  }
  .le-stage :global(.le-draw-line) {
    border-style: solid;
    border-color: #777;
    background: #777;
  }
  .le-stage :global(.le-draw-text),
  .le-stage :global(.le-draw-field) {
    background: rgba(255, 255, 255, 0.75);
  }
  .le-stage :global(.le-hover-outline) {
    position: absolute;
    box-sizing: border-box;
    z-index: 999;
    pointer-events: none;
    border: 1px dashed #6b7280;
    background: rgba(107, 114, 128, 0.05);
  }
  .le-stage :global(.le-inline-text-editor) {
    position: absolute;
    box-sizing: border-box;
    z-index: 1002;
    resize: none;
    pointer-events: auto;
    padding: 0.1rem 0.2rem;
    border: 1px solid #1f6feb;
    outline: 2px solid rgba(31, 111, 235, 0.18);
    background: #fff;
    color: #1b1b1f;
    font: 0.8rem system-ui, sans-serif;
  }
  /* Design mode: make each part band's bounds visible. Browse keeps the bands
     subtle (the faint shell.html divider), but on the canvas the designer needs
     to see where parts begin and end. CSS-only + design-mode-scoped, so the
     parity-checked Browse markup is untouched. */
  .le-stage :global(.fm-part) {
    outline: 1px dashed #aeb6bf;
    outline-offset: -1px;
  }
  .le-stage :global(.fm-canvas) {
    min-width: 760px;
    margin: 0;
  }
  .le-stage :global(.fm-part.selected-part) {
    outline-color: #1f6feb;
    outline-style: solid;
  }
  .le-stage :global(.fm-obj) {
    cursor: move;
    user-select: none;
  }
  .le-stage.no-object-selection :global(.moveable-control-box) {
    display: none !important;
  }
  /* A tool is armed → the canvas is a placement surface: show a crosshair and
     stop objects advertising "move". */
  .le-stage.placing :global(.fm-canvas),
  .le-stage.placing :global(.fm-obj) {
    cursor: crosshair;
  }
</style>
