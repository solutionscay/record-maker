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
  import { flushSync, untrack } from 'svelte';
  import { registerEchoStage } from './lib/echo';
  import { CanvasInteraction } from './lib/interaction';
  import { setPartHeight as persistPartHeight } from './lib/persist';
  import { lerror, llog } from './lib/log';
  import LayoutPreview from './lib/LayoutPreview.svelte';
  import { snapToGrid } from './lib/canvas-edit';
  import { GestureLifecycle } from './lib/canvas/gesture-lifecycle';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  let stage = $state<HTMLElement>();
  let contextMenuEl = $state<HTMLElement>();
  let resizingPartId = $state<number | null>(null);
  const partResizeLifecycle = new GestureLifecycle('band-resize');
  const DESIGN_PAGE_WIDTH = 760;

  type ContextMenuItem = {
    label: string;
    hint?: string;
    disabled?: boolean;
    danger?: boolean;
    action: () => void;
  };
  type ContextMenuState = {
    x: number;
    y: number;
    title: string;
    items: ContextMenuItem[];
  };
  let contextMenu = $state<ContextMenuState | null>(null);

  const partBands = $derived.by(() => {
    let top = 0;
    return doc.renderModel.parts.map((part) => {
      const band = { part, top };
      top += part.height;
      return band;
    });
  });
  const layoutHeight = $derived(partBands.reduce((sum, band) => sum + band.part.height, 0));
  // A literal dot every 1–3px becomes a solid tint. Keep snapping exact while
  // drawing every tenth intersection for fine grids (#193).
  const visibleGridSize = $derived(doc.gridSize < 4 ? doc.gridSize * 10 : doc.gridSize);

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

  function partLabelChars(kind: string): string[] {
    return [...partLabel(kind)];
  }

  // Stand the interaction layer up once the canvas is in the DOM; tear it down on
  // unmount. moveable + selecto bind to the shared store, not this island.
  let interaction = $state<CanvasInteraction | null>(null);
  $effect(() => () => partResizeLifecycle.destroy());
  $effect(() => {
    if (!doc.hydrated || !stage) return;
    const ix = new CanvasInteraction(stage, doc, layoutId);
    const unregisterEcho = registerEchoStage(doc, stage);
    interaction = ix;
    return () => {
      unregisterEcho();
      ix.destroy();
      interaction = null;
    };
  });

  // Keep moveable's control box in sync with the store: re-run on any selection,
  // document-content, or active-tool change (arming a tool drops the target so a
  // press places instead of grabs). The controller ignores syncs during a live
  // gesture. `doc.version` is the narrow signal for "the render model changed" —
  // tracking the model itself would walk every part/object on each flush.
  $effect(() => {
    void [...doc.selection];
    void doc.version;
    void doc.activeTool;
    interaction?.refresh();
    // doc.version above bumps on each object's server-derived `textStyle` refresh,
    // so when the inspector changes the selected text's size/style this re-applies
    // it to an open inline editor LIVE — without committing/closing it (#5).
    interaction?.syncOpenTextEditor();
  });

  // Push the current zoom into the interaction layer so placement coordinates
  // compensate for the CSS scale.
  $effect(() => {
    interaction?.setZoom(doc.zoom);
  });

  $effect(() => {
    interaction?.setGrid(doc.gridSize, doc.snapToGrid);
  });

  $effect(() => {
    const command = doc.viewportCommand;
    if (command.sequence > 0) untrack(() => interaction?.runViewportCommand(command.kind));
  });

  $effect(() => {
    if (!contextMenu) return;
    const close = () => {
      contextMenu = null;
    };
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node | null;
      if (target && contextMenuEl?.contains(target)) return;
      close();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('scroll', close, true);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('scroll', close, true);
      window.removeEventListener('resize', close);
    };
  });

  function clampMenuPoint(clientX: number, clientY: number, rows: number): { x: number; y: number } {
    const width = 210;
    const height = 38 + rows * 31;
    return {
      x: Math.max(8, Math.min(clientX, window.innerWidth - width - 8)),
      y: Math.max(8, Math.min(clientY, window.innerHeight - height - 8)),
    };
  }

  function editableTarget(target: EventTarget | null): boolean {
    const el = target instanceof Element ? target : null;
    return !!el?.closest('input, textarea, select, [contenteditable="true"], .le-inline-text-editor');
  }

  // Both renderers stamp data-object-id / data-part-id (#134), so identity is
  // read straight off the element — no DOM-index-to-paint-order matching.
  function objectIdForElement(el: HTMLElement): number | null {
    const raw = el.dataset.objectId;
    if (raw === undefined) return null;
    const id = Number(raw);
    return Number.isFinite(id) ? id : null;
  }

  function objectIdFromPoint(event: MouseEvent): number | null {
    const target = event.target instanceof Element ? event.target : null;
    const direct = target?.closest('.fm-obj') as HTMLElement | null;
    if (direct) return objectIdForElement(direct);
    for (const el of document.elementsFromPoint(event.clientX, event.clientY)) {
      const obj = el.closest?.('.fm-obj') as HTMLElement | null;
      if (!obj) continue;
      const id = objectIdForElement(obj);
      if (id !== null) return id;
    }
    return null;
  }

  function partIdFromTarget(target: EventTarget | null): number | null {
    const el = target instanceof Element ? (target.closest('.fm-part') as HTMLElement | null) : null;
    const raw = el?.dataset.partId;
    if (raw === undefined) return null;
    const id = Number(raw);
    return Number.isFinite(id) ? id : null;
  }

  function openContextMenu(event: MouseEvent, title: string, items: ContextMenuItem[]): void {
    const point = clampMenuPoint(event.clientX, event.clientY, items.length);
    contextMenu = { ...point, title, items };
  }

  function objectMenuItems(): ContextMenuItem[] {
    const items: ContextMenuItem[] = [
      { label: 'Cut', hint: 'Ctrl+X', action: () => interaction?.cut() },
      { label: 'Copy', hint: 'Ctrl+C', action: () => interaction?.copy() },
      { label: 'Paste', hint: 'Ctrl+V', disabled: !interaction?.canPaste(), action: () => interaction?.paste() },
      { label: 'Duplicate', hint: 'Ctrl+D', action: () => interaction?.duplicate() },
    ];
    if (interaction?.canUngroup()) {
      items.push({ label: 'Ungroup', action: () => interaction?.ungroup() });
    } else if (interaction?.canGroup()) {
      items.push({ label: 'Group', action: () => interaction?.group() });
    }
    items.push({ label: 'Delete', danger: true, action: () => interaction?.deleteSelected() });
    return items;
  }

  function openObjectContextMenu(event: MouseEvent, objectId: number | null = null): void {
    event.preventDefault();
    event.stopPropagation();
    if (objectId !== null && !doc.isSelected(objectId)) doc.selectOnly([objectId]);
    const count = doc.selection.size || 1;
    openContextMenu(event, count === 1 ? 'Object' : `${count} Objects`, objectMenuItems());
  }

  function openBandContextMenu(event: MouseEvent, partId: number): void {
    event.preventDefault();
    event.stopPropagation();
    doc.selectPart(partId);
    const part = doc.getPart(partId);
    openContextMenu(event, part ? `${partLabel(part.kind)} Band` : 'Band', [
      { label: 'Paste Objects', hint: 'Ctrl+V', disabled: !interaction?.canPaste(), action: () => interaction?.paste() },
    ]);
  }

  function onContextMenu(event: MouseEvent): void {
    if (editableTarget(event.target)) return;
    const objectId = objectIdFromPoint(event);
    if (objectId !== null) {
      openObjectContextMenu(event, objectId);
      return;
    }
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('.moveable-control-box') && doc.selection.size > 0) {
      openObjectContextMenu(event);
      return;
    }

    const partId = partIdFromTarget(event.target);
    if (partId !== null) {
      openBandContextMenu(event, partId);
      return;
    }

    event.preventDefault();
    event.stopPropagation();
    doc.clearSelection();
    openContextMenu(event, 'Layout', [
      { label: 'Paste Objects', hint: 'Ctrl+V', disabled: !interaction?.canPaste(), action: () => interaction?.paste() },
    ]);
  }

  function runContextMenuItem(item: ContextMenuItem, event: MouseEvent): void {
    event.preventDefault();
    event.stopPropagation();
    if (item.disabled) return;
    contextMenu = null;
    item.action();
  }

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
    const part = doc.getPart(id);
    if (!part) return;
    const startHeight = part.height;
    let latestHeight = startHeight;
    const startClientY = event.clientY;
    let latestClientY = event.clientY;
    let scrollDeltaY = 0;
    const partElement = canvas.querySelector<HTMLElement>(`.fm-part[data-part-id="${id}"]`);
    const overlay = stage.querySelector<HTMLElement>('.le-part-overlays');
    const label = overlay?.querySelector<HTMLElement>(`.le-part-label[data-overlay-part-id="${id}"]`) ?? null;
    const handle = overlay?.querySelector<HTMLElement>(`.le-part-resize[data-overlay-part-id="${id}"]`) ?? null;
    const grid = stage.querySelector<HTMLElement>('.le-layout-grid');
    const laterIds = partBands.filter((band) => band.top > top).map((band) => band.part.id);
    const shifted = laterIds.flatMap((partId) => [
      overlay?.querySelector<HTMLElement>(`.le-part-label[data-overlay-part-id="${partId}"]`) ?? null,
      overlay?.querySelector<HTMLElement>(`.le-part-resize[data-overlay-part-id="${partId}"]`) ?? null,
    ]).filter((element): element is HTMLElement => !!element);
    const originals = {
      partHeight: partElement?.style.height ?? '',
      labelHeight: label?.style.height ?? '',
      handleTop: handle?.style.top ?? '',
      gridHeight: grid?.style.height ?? '',
      shifted: new Map(shifted.map((element) => [element, element.style.transform])),
    };
    const startLayoutHeight = doc.renderModel.parts.reduce((sum, candidate) => sum + candidate.height, 0);
    let previewFrame: number | null = null;
    let receivedMove = false;
    const selectionBefore = [...doc.selection];
    const selectedPartBefore = doc.selectedPartId;
    const checkpoint = doc.beginGestureTransaction();
    resizingPartId = id;
    interaction?.setExternalGesturing(true);
    doc.selectPart(id);
    llog('resize', 'part resizeStart', { id, minHeight });

    const paintPreview = () => {
      previewFrame = null;
      const delta = latestHeight - startHeight;
      if (partElement) partElement.style.height = `${latestHeight}px`;
      if (label) label.style.height = `${latestHeight}px`;
      if (handle) handle.style.top = `${top + latestHeight}px`;
      if (grid) grid.style.height = `${startLayoutHeight + delta}px`;
      for (const element of shifted) element.style.transform = `translateY(${delta}px)`;
    };
    const propose = (clientY: number) => {
      const zoom = doc.zoom || 1;
      const authored = startHeight + (clientY - startClientY + scrollDeltaY) / zoom;
      latestHeight = Math.max(minHeight, snapToGrid(authored, doc.snapToGrid ? doc.gridSize : 0));
      if (previewFrame === null) previewFrame = requestAnimationFrame(paintPreview);
    };
    const move = (e: PointerEvent) => {
      if (!partResizeLifecycle.owns(e)) return;
      receivedMove = true;
      latestClientY = e.clientY;
      interaction?.updateBandResizeAutoscroll(e.clientX, e.clientY, (deltaY) => {
        scrollDeltaY += deltaY;
        propose(latestClientY);
      });
      propose(e.clientY);
    };
    const clearPreview = (restore: boolean) => {
      if (previewFrame !== null) cancelAnimationFrame(previewFrame);
      previewFrame = null;
      if (restore) {
        if (partElement) partElement.style.height = originals.partHeight;
        if (label) label.style.height = originals.labelHeight;
        if (handle) handle.style.top = originals.handleTop;
        if (grid) grid.style.height = originals.gridHeight;
      }
      for (const [element, transform] of originals.shifted) element.style.transform = transform;
    };
    const cleanup = (restore: boolean) => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      clearPreview(restore);
      interaction?.stopBandResizeAutoscroll();
      interaction?.setExternalGesturing(false);
    };
    const restoreSelection = () => {
      if (selectedPartBefore !== null) doc.selectPart(selectedPartBefore);
      else doc.selectOnly(selectionBefore);
    };
    const up = (e: PointerEvent) => {
      if (!partResizeLifecycle.owns(e)) return;
      if (receivedMove) {
        latestClientY = e.clientY;
        propose(e.clientY);
      }
      if (receivedMove && previewFrame !== null) {
        cancelAnimationFrame(previewFrame);
        previewFrame = null;
        paintPreview();
      }
      partResizeLifecycle.commit();
      const changed = latestHeight !== startHeight;
      flushSync(() => {
        if (changed) doc.setPartHeight(id, latestHeight);
        resizingPartId = null;
      });
      cleanup(false);
      doc.commitGestureTransaction(checkpoint);
      llog('resize', 'part resizeEnd', { id, height: latestHeight, changed });
      if (!changed) return;
      void persistPartHeight(layoutId, id, latestHeight).catch((e) => {
        lerror('persist', 'failed to persist part height', e);
        doc.setError(e instanceof Error ? e.message : String(e));
      });
    };
    partResizeLifecycle.begin({
      inputEvent: event,
      pointerId: event.pointerId,
      captureTarget: event.currentTarget as Element | null,
      onCancel: (reason) => {
        flushSync(() => { resizingPartId = null; });
        cleanup(true);
        doc.cancelGestureTransaction(checkpoint);
        restoreSelection();
        llog('resize', 'part resize cancelled', { id, reason });
      },
    });
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }
</script>

{#if doc.error}
  <p class="layout-editor-msg layout-editor-error">Failed to load layout: {doc.error}</p>
{:else if doc.hydrated}
  <div
    class="le-stage"
    class:placing={doc.activeTool !== 'pointer'}
    class:no-object-selection={doc.selection.size === 0}
    style={`--le-canvas-inverse-zoom: ${1 / doc.zoom};`}
    bind:this={stage}
    role="application"
    aria-label="Layout canvas"
    oncontextmenu={onContextMenu}
  >
    <div
      class="le-workspace"
      style={`transform: scale(${doc.zoom}); transform-origin: top left; --le-zoom: ${doc.zoom}; --le-inverse-zoom: ${1 / doc.zoom};`}
    >
      <div class="le-canvas-wrap">
        <LayoutPreview model={doc.renderModel} />
        {#if doc.showGrid}
          <div
            class="le-layout-grid"
            style={`width:${doc.renderModel.width}px;min-width:${DESIGN_PAGE_WIDTH}px;height:${layoutHeight}px;--le-grid-size:${visibleGridSize}px;`}
            aria-hidden="true"
          ></div>
        {/if}
        <div class="le-part-overlays" style={`width: ${doc.renderModel.width}px; min-width: ${DESIGN_PAGE_WIDTH}px;`}>
          {#each partBands as band (band.part.id)}
            <button
              type="button"
              class="le-part-label"
              class:selected={doc.selectedPartId === band.part.id}
              data-overlay-part-id={band.part.id}
              style={`top: ${band.top}px; height: ${band.part.height}px;`}
              title={`Select ${partLabel(band.part.kind)} band`}
              onclick={(e) => selectPart(band.part.id, e)}
              oncontextmenu={(e) => openBandContextMenu(e, band.part.id)}
            >
              {#each partLabelChars(band.part.kind) as char}
                <span>{char}</span>
              {/each}
            </button>
            <button
              type="button"
              class="le-part-resize"
              class:selected={doc.selectedPartId === band.part.id}
              class:resizing={resizingPartId === band.part.id}
              data-overlay-part-id={band.part.id}
              style={`top: ${band.top + band.part.height}px;`}
              title={`Resize ${partLabel(band.part.kind)} band`}
              onpointerdown={(e) => startPartResize(band.part.id, band.top, e)}
              oncontextmenu={(e) => openBandContextMenu(e, band.part.id)}
            ></button>
          {/each}
        </div>
      </div>
    </div>
  </div>
  {#if contextMenu}
    <div
      class="le-context-menu"
      bind:this={contextMenuEl}
      style={`left: ${contextMenu.x}px; top: ${contextMenu.y}px;`}
      role="menu"
      aria-label={contextMenu.title}
    >
      <div class="le-context-title">{contextMenu.title}</div>
      {#each contextMenu.items as item}
        <button
          type="button"
          class="le-context-item"
          class:danger={item.danger}
          disabled={item.disabled}
          role="menuitem"
          onclick={(e) => runContextMenuItem(item, e)}
        >
          <span>{item.label}</span>
          {#if item.hint}<kbd>{item.hint}</kbd>{/if}
        </button>
      {/each}
    </div>
  {/if}
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
    --le-transform-accent: var(--rm-accent, #0a84ff);
    --le-transform-contrast: #fff;
    position: relative;
    touch-action: none;
    overflow: auto;
    height: 100%;
    padding: 30px;
  }
  :global(.le-stage.is-pan-ready),
  :global(.le-stage.is-pan-ready *) {
    cursor: grab !important;
  }
  :global(.le-stage.is-panning),
  :global(.le-stage.is-panning *) {
    cursor: grabbing !important;
    user-select: none !important;
  }
  /* The zoom layer: transform scales the canvas without reflowing the chrome.
     The workspace FILLS the pane so the canvas card stretches to a symmetric 30px
     gutter (matching the top) instead of sitting fixed-width and centred. The
     content-derived model width just becomes a floor (min-width below). */
  .le-workspace {
    position: relative;
    width: 100%;
    --le-part-gutter: 28px;
    padding-left: var(--le-part-gutter);
  }
  .le-canvas-wrap {
    position: relative;
    width: 100%;
    margin: 0;
  }
  /* One paint-only grid spans the complete stacked layout, so its origin never
     resets at a band boundary. It deliberately sits above band fills (otherwise
     a coloured Body would hide the layout grid) but is pointer-transparent and
     faint enough not to obscure authored objects. */
  .le-layout-grid {
    position: absolute;
    inset: 0 auto auto 0;
    pointer-events: none;
    background-image: radial-gradient(circle, rgba(75, 91, 112, 0.34) 0.65px, transparent 0.8px);
    background-position: 0 0;
    background-size: var(--le-grid-size) var(--le-grid-size);
  }
  .le-part-overlays {
    position: absolute;
    top: 1px;
    left: 0;
    width: 100% !important;
    pointer-events: none;
  }

  :global(.le-smart-guide) {
    position: absolute;
    z-index: 20;
    pointer-events: none;
    background: color-mix(in srgb, var(--le-transform-accent) 82%, #5b21b6);
    box-shadow: 0 0 0 calc(1px * var(--le-inverse-zoom)) var(--le-transform-contrast);
    opacity: 1;
  }

  /* Moveable's own nearby/grid hints are candidates. The thicker, fully opaque
     application guide above exists only after the numeric resolver snaps. */
  .le-stage :global(.moveable-guideline) {
    opacity: 0.32 !important;
  }

  :global(.le-smart-guide-x) {
    width: calc(1px * var(--le-inverse-zoom));
  }

  :global(.le-smart-guide-y) {
    height: calc(1px * var(--le-inverse-zoom));
  }
  .le-part-label {
    position: absolute;
    left: calc(-1 * var(--le-part-gutter));
    width: var(--le-part-gutter);
    padding: 0.35rem 0;
    border: 0;
    border-right: 1px dashed rgba(0, 0, 0, 0.13);
    border-radius: 0;
    background: rgba(0, 0, 0, 0.025);
    color: #9a9aa0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    font: 600 9px/1 -apple-system, BlinkMacSystemFont, 'Helvetica Neue', sans-serif;
    overflow: hidden;
    cursor: pointer;
    pointer-events: auto;
    box-sizing: border-box;
  }
  .le-part-label.selected {
    border-right-color: var(--rm-accent, #0a84ff);
    background: var(--rm-accent-soft, rgba(10, 132, 255, 0.12));
    color: #174ea6;
  }
  .le-part-resize {
    --le-band-hit-before: calc(4px * var(--le-inverse-zoom));
    --le-band-gutter-hit: calc(14px * var(--le-inverse-zoom));
    --le-band-gutter-before: calc(7px * var(--le-inverse-zoom));
    position: absolute;
    left: 0;
    right: 0;
    height: calc(8px * var(--le-inverse-zoom));
    padding: 0;
    border: 0;
    background: transparent;
    cursor: row-resize;
    pointer-events: auto;
    margin-top: calc(-1 * var(--le-band-hit-before));
    touch-action: none;
  }
  .le-part-resize::before {
    content: '';
    position: absolute;
    left: calc(-28px * var(--le-inverse-zoom));
    top: calc(var(--le-band-hit-before) - var(--le-band-gutter-before));
    width: calc(28px * var(--le-inverse-zoom));
    height: var(--le-band-gutter-hit);
    pointer-events: auto;
  }
  .le-part-resize::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    top: var(--le-band-hit-before);
    border-top: calc(1px * var(--le-inverse-zoom)) solid color-mix(in srgb, var(--le-transform-accent) 55%, transparent);
  }
  .le-part-resize.selected::after,
  .le-part-resize.resizing::after {
    border-top: calc(2px * var(--le-inverse-zoom)) solid var(--le-transform-accent);
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
    border: 0;
    background: #777;
    transform-origin: center center;
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
    border: calc(1px * var(--le-inverse-zoom)) dashed var(--le-transform-accent);
    outline: calc(1px * var(--le-inverse-zoom)) solid var(--le-transform-contrast);
    outline-offset: calc(1px * var(--le-inverse-zoom));
    background: color-mix(in srgb, var(--le-transform-accent) 7%, transparent);
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
  .le-stage :global(.le-echo-ghost) {
    pointer-events: none;
    user-select: none;
    opacity: 0;
    filter: saturate(0.75) brightness(1.03);
    mix-blend-mode: multiply;
  }
  .le-stage :global(.le-echo-undo) {
    box-shadow: 0 0 0 3px rgba(31, 111, 235, 0.5), 0 10px 28px rgba(31, 111, 235, 0.26);
  }
  .le-stage :global(.le-echo-redo) {
    box-shadow: 0 0 0 3px rgba(217, 119, 6, 0.5), 0 10px 28px rgba(217, 119, 6, 0.26);
  }
  .le-stage :global(.le-echo-active) {
    will-change: transform;
  }
  .le-stage :global(.le-echo-active-undo) {
    outline: 2px solid rgba(31, 111, 235, 0.38);
    outline-offset: 2px;
  }
  .le-stage :global(.le-echo-active-redo) {
    outline: 2px solid rgba(217, 119, 6, 0.38);
    outline-offset: 2px;
  }
  .le-context-menu {
    position: fixed;
    z-index: 10000;
    min-width: 190px;
    padding: 5px;
    border: 0.5px solid var(--rm-border-strong, rgba(0, 0, 0, 0.16));
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.98);
    color: var(--rm-text, #1c1c1e);
    box-shadow: 0 10px 32px rgba(0, 0, 0, 0.18), 0 2px 8px rgba(0, 0, 0, 0.1);
    font: 13px/1.2 -apple-system, BlinkMacSystemFont, 'Helvetica Neue', sans-serif;
  }
  .le-context-title {
    padding: 5px 8px 6px;
    color: var(--rm-text-dim, #8a8a8e);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .le-context-item {
    width: 100%;
    min-height: 28px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    padding: 5px 8px;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: inherit;
    box-shadow: none;
    font: inherit;
    text-align: left;
  }
  .le-context-item:hover:not(:disabled),
  .le-context-item:focus-visible:not(:disabled) {
    outline: none;
    background: var(--rm-accent, #0a84ff);
    color: #fff;
  }
  .le-context-item.danger {
    color: var(--rm-danger, #ff453a);
  }
  .le-context-item.danger:hover:not(:disabled),
  .le-context-item.danger:focus-visible:not(:disabled) {
    color: #fff;
    background: var(--rm-danger, #ff453a);
  }
  .le-context-item:disabled {
    color: #b8b8bd;
    cursor: default;
  }
  .le-context-item kbd {
    color: currentColor;
    opacity: 0.62;
    font: inherit;
    font-size: 11px;
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
    width: 100% !important;
    min-width: 760px;
    margin: 0;
    border: 0.5px solid var(--rm-border-strong, rgba(0, 0, 0, 0.16));
    border-radius: 8px;
    box-shadow: var(--rm-shadow-card, 0 1px 3px rgba(0, 0, 0, 0.08), 0 8px 26px rgba(0, 0, 0, 0.07));
  }
  .le-stage :global(.fm-part.selected-part) {
    outline-color: var(--rm-accent, #0a84ff);
    outline-style: solid;
  }
  .le-stage :global(.fm-obj) {
    cursor: grab;
    user-select: none;
  }
  /* #220 transform target contract: a 24px invisible acquisition square with a
     crisp 7px visual centre. Moveable lives outside the scaled workspace, so the
     target stays 24 screen px at every canvas zoom. */
  .le-stage :global(.moveable-control) {
    --le-control-clip: calc(8px * var(--le-canvas-inverse-zoom));
    width: calc(24px * var(--le-canvas-inverse-zoom)) !important;
    height: calc(24px * var(--le-canvas-inverse-zoom)) !important;
    margin-top: calc(-12px * var(--le-canvas-inverse-zoom)) !important;
    margin-left: calc(-12px * var(--le-canvas-inverse-zoom)) !important;
    border: 0 !important;
    border-radius: 0 !important;
    background: transparent !important;
    box-sizing: border-box !important;
  }
  .le-stage :global(.moveable-control::after) {
    content: '';
    position: absolute;
    left: 50%;
    top: 50%;
    width: calc(7px * var(--le-canvas-inverse-zoom));
    height: calc(7px * var(--le-canvas-inverse-zoom));
    box-sizing: border-box;
    transform: translate(-50%, -50%);
    border: calc(1px * var(--le-canvas-inverse-zoom)) solid var(--le-transform-contrast);
    border-radius: 50%;
    background: var(--le-transform-accent);
    outline: calc(1px * var(--le-canvas-inverse-zoom)) solid color-mix(in srgb, #001b38 58%, transparent);
  }
  .le-stage :global(.moveable-line) {
    height: calc(1px * var(--le-canvas-inverse-zoom)) !important;
    background: var(--le-transform-accent) !important;
    box-shadow: 0 0 0 calc(1px * var(--le-canvas-inverse-zoom)) var(--le-transform-contrast);
  }
  .le-stage :global(.moveable-area) {
    cursor: grab !important;
  }
  /* Bias invisible acquisition geometry outward. Only four screen pixels cross
     into the object, so even a small target retains a central grab surface. */
  .le-stage :global(.moveable-control[data-direction='n']) {
    clip-path: inset(0 0 var(--le-control-clip) 0);
  }
  .le-stage :global(.moveable-control[data-direction='s']) {
    clip-path: inset(var(--le-control-clip) 0 0 0);
  }
  .le-stage :global(.moveable-control[data-direction='e']) {
    clip-path: inset(0 0 0 var(--le-control-clip));
  }
  .le-stage :global(.moveable-control[data-direction='w']) {
    clip-path: inset(0 var(--le-control-clip) 0 0);
  }
  .le-stage :global(.moveable-control[data-direction='nw']) {
    clip-path: inset(0 var(--le-control-clip) var(--le-control-clip) 0);
  }
  .le-stage :global(.moveable-control[data-direction='ne']) {
    clip-path: inset(0 0 var(--le-control-clip) var(--le-control-clip));
  }
  .le-stage :global(.moveable-control[data-direction='sw']) {
    clip-path: inset(var(--le-control-clip) var(--le-control-clip) 0 0);
  }
  .le-stage :global(.moveable-control[data-direction='se']) {
    clip-path: inset(var(--le-control-clip) 0 0 var(--le-control-clip));
  }
  .le-stage :global(.moveable-control[data-direction='n']),
  .le-stage :global(.moveable-control[data-direction='s']) {
    cursor: ns-resize !important;
  }
  .le-stage :global(.moveable-control[data-direction='e']),
  .le-stage :global(.moveable-control[data-direction='w']) {
    cursor: ew-resize !important;
  }
  .le-stage :global(.moveable-control[data-direction='nw']),
  .le-stage :global(.moveable-control[data-direction='se']) {
    cursor: nwse-resize !important;
  }
  .le-stage :global(.moveable-control[data-direction='ne']),
  .le-stage :global(.moveable-control[data-direction='sw']) {
    cursor: nesw-resize !important;
  }
  .le-stage :global(.moveable-rotation-control) {
    cursor: grab !important;
  }
  :global(.le-stage.is-transforming),
  :global(.le-stage.is-transforming *) {
    cursor: var(--le-active-transform-cursor, grabbing) !important;
  }
  :global(.le-stage.is-transforming .moveable-line) {
    background: color-mix(in srgb, var(--le-transform-accent) 78%, #5b21b6) !important;
  }
  .le-stage :global(.le-geometry-badge) {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 10020;
    min-width: 54px;
    padding: 4px 7px;
    border: 1px solid var(--le-transform-contrast);
    border-radius: 5px;
    background: color-mix(in srgb, #071a2e 92%, transparent);
    color: #fff;
    pointer-events: none;
    font: 600 11px/1.2 ui-monospace, SFMono-Regular, Menlo, monospace;
    font-variant-numeric: tabular-nums;
    text-align: center;
    white-space: nowrap;
  }
  /* #194: pin Moveable's frame-coalesced bounds to the same authored transform
     as the object. The variables live on `html`, outside both Svelte and
     Moveable's independently rewritten style attributes. */
  :global(html.syncing-drag-bounds .le-stage .moveable-control-box[data-rm-drag-bounds]) {
    transform: translate3d(var(--le-drag-bounds-x), var(--le-drag-bounds-y), 0) !important;
  }
  /* #215: raw group resize projects the captured control frame instead of
     remeasuring every selected target. */
  :global(html.syncing-resize-bounds .le-stage .moveable-control-box[data-rm-resize-bounds]) {
    transform: translate3d(var(--le-resize-bounds-x), var(--le-resize-bounds-y), 0)
      scale(var(--le-resize-bounds-scale-x), var(--le-resize-bounds-scale-y)) !important;
    transform-origin: 0 0;
  }
  .le-stage :global(.fm-obj:has(.fm-line)) {
    overflow: visible;
    pointer-events: auto;
  }
  .le-stage :global(.fm-obj:has(.fm-line)::before) {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 12px;
    transform: translateY(-50%);
    pointer-events: auto;
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

  @media (pointer: coarse) {
    .le-stage :global(.moveable-control) {
      --le-control-clip: calc(12px * var(--le-canvas-inverse-zoom));
      width: calc(32px * var(--le-canvas-inverse-zoom)) !important;
      height: calc(32px * var(--le-canvas-inverse-zoom)) !important;
      margin-top: calc(-16px * var(--le-canvas-inverse-zoom)) !important;
      margin-left: calc(-16px * var(--le-canvas-inverse-zoom)) !important;
    }
    .le-part-resize {
      --le-band-gutter-hit: calc(28px * var(--le-inverse-zoom));
      --le-band-gutter-before: calc(14px * var(--le-inverse-zoom));
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .le-stage :global(.moveable-control),
    .le-stage :global(.moveable-line),
    .le-stage :global(.le-hover-outline),
    .le-stage :global(.le-geometry-badge) {
      animation: none !important;
      transition: none !important;
    }
  }

  @media (forced-colors: active) {
    .le-stage :global(.moveable-line),
    :global(.le-smart-guide),
    .le-part-resize::after {
      forced-color-adjust: none;
      background: Highlight !important;
      border-color: Highlight !important;
      box-shadow: none;
    }
    .le-stage :global(.moveable-control::after) {
      forced-color-adjust: none;
      border: calc(2px * var(--le-canvas-inverse-zoom)) solid Highlight;
      background: Canvas;
      outline: calc(1px * var(--le-canvas-inverse-zoom)) solid CanvasText;
    }
    .le-stage :global(.le-hover-outline) {
      forced-color-adjust: none;
      border-color: Highlight;
      outline-color: Canvas;
      background: transparent;
    }
    .le-stage :global(.le-geometry-badge) {
      forced-color-adjust: none;
      border-color: CanvasText;
      background: Canvas;
      color: CanvasText;
    }
  }
</style>
