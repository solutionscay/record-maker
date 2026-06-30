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
  // The drag stub (#15) is wired here as editor chrome: pointer handlers on the
  // stage wrapper (NOT inside the pure canvas, so parity is untouched) move the
  // object in the store on drag and POST the committed geometry on drop. This is
  // the minimal single-object round-trip; #46 swaps it for the full moveable /
  // selecto interaction layer, reusing the same store command surface.
  import type { DesignModel } from './lib/model';
  import { EditorDoc, type ObjectDoc } from './lib/doc.svelte';
  import LayoutPreview from './lib/LayoutPreview.svelte';

  let { layoutId = '' }: { layoutId?: string } = $props();

  const doc = new EditorDoc();
  let error = $state<string | null>(null);

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

  // ── drag stub (#15) ────────────────────────────────────────────────────────

  /** The in-flight drag, or null when idle. Origin is the object's pre-drag
   * geometry so we can detect a real move and clamp from a fixed reference. */
  type Drag = { id: number; pointerId: number; startX: number; startY: number; originX: number; originY: number };
  let drag: Drag | null = null;

  /** Object ids in DOM order — identical to the order `LayoutPreview` paints
   * `.fm-obj` elements (parts top→bottom, objects back→front), so the Nth painted
   * element maps to the Nth id here. Lets us resolve a clicked element to its
   * object id WITHOUT stamping ids onto the pure canvas DOM (which would break
   * the #44 parity golden). */
  function flatObjectIds(): number[] {
    return doc.renderModel.parts.flatMap((p) => p.objects.map((o) => o.id));
  }

  /** Resolve the object id under a pointer event's target, or null if the press
   * landed on empty canvas (a deselect). */
  function objectIdAt(stage: HTMLElement, target: EventTarget | null): number | null {
    if (!(target instanceof Element)) return null;
    const el = target.closest('.fm-obj');
    if (!el) return null;
    const painted = [...stage.querySelectorAll('.fm-obj')];
    const idx = painted.indexOf(el);
    if (idx < 0) return null;
    return flatObjectIds()[idx] ?? null;
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const stage = e.currentTarget as HTMLElement;
    const id = objectIdAt(stage, e.target);
    if (id === null) {
      doc.clearSelection();
      return;
    }
    const o = doc.getObject(id);
    if (!o) return;
    doc.selectOnly([id]);
    drag = { id, pointerId: e.pointerId, startX: e.clientX, startY: e.clientY, originX: o.x, originY: o.y };
    stage.setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    // Absolute set from the fixed origin (not relative deltas) so the object
    // tracks the pointer exactly and never drifts; clamp to the canvas origin.
    const x = Math.max(0, drag.originX + Math.round(e.clientX - drag.startX));
    const y = Math.max(0, drag.originY + Math.round(e.clientY - drag.startY));
    doc.setObjectGeometry(drag.id, { x, y });
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const { id, originX, originY } = drag;
    drag = null;
    const stage = e.currentTarget as HTMLElement;
    if (stage.hasPointerCapture(e.pointerId)) stage.releasePointerCapture(e.pointerId);
    // Seal the drag into one undo step; persist only a geometry that actually
    // changed (a plain click shouldn't POST).
    doc.mark();
    const o = doc.getObject(id);
    if (o && (o.x !== originX || o.y !== originY)) void persistGeometry(id, o);
  }

  async function persistGeometry(id: number, o: Readonly<ObjectDoc>) {
    try {
      const r = await fetch(`/design/${layoutId}/object/${id}/geometry`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ x: o.x, y: o.y, w: o.w, h: o.h }),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
    } catch (e) {
      // The store already reflects the move; surface the persist failure without
      // tearing down the in-memory edit (a reload would reveal the divergence).
      console.error('failed to persist object geometry', e);
    }
  }
</script>

{#if error}
  <p class="layout-editor-msg layout-editor-error">Failed to load layout: {error}</p>
{:else if doc.hydrated}
  <!-- The stage is a pointer-driven design surface, not a discrete widget; the
       full keyboard/ARIA interaction model lands with #46's interaction layer. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="le-stage"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
  >
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
