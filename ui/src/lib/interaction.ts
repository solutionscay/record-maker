// Layout canvas interaction layer (#46) — binds moveable (drag / resize / snap +
// alignment guidelines / group transform) and selecto (marquee multi-select) to
// the editor document store (#45). The vanilla cores are consumed directly.
//
// Single source of truth is the store: pointer gestures never write element
// styles. moveable reports the new (already grid/edge-snapped) position/size and
// we push it through the store's command surface (setObjectGeometry); Svelte then
// re-renders the object from the store. moveable derives its control box from the
// gesture's cached start rect + pointer delta, so routing output back into the
// store does NOT create a feedback loop. A whole gesture commits as ONE undo step
// (mark on end) and persists to the engine via the bulk axum contract.
//
// Press-drag in one gesture: moveable only drags the element it already targets
// at mousedown, so we TARGET THE OBJECT UNDER THE CURSOR ON HOVER. By the time
// you press, moveable is already on it and its native drag starts immediately —
// no select-then-grab two-step, and no fragile selecto→moveable handoff. A
// multi-selection keeps the group as the target while you hover its members.
//
// Object identity without polluting the parity-checked canvas DOM: the Nth
// painted `.fm-obj` element maps to the Nth id in renderModel paint order (see
// canvas-edit.ts), so selections and hits resolve to ids by index — no data-*
// attributes added to the canvas.

import Moveable from 'moveable';
import Selecto from 'selecto';

import type { EditorDoc } from './doc.svelte';
import type { ObjectView } from './model';
import { GRID, SNAP_THRESHOLD, clampOrigin, elementsToObjectIds, objectIdsInPaintOrder } from './canvas-edit';
import { defaultBox, defaultProps, partAtY } from './create';
import { createObject } from './persist';
import { llog, lerror } from './log';

export class CanvasInteraction {
  readonly #stage: HTMLElement;
  readonly #doc: EditorDoc;
  readonly #layoutId: string;
  readonly #moveable: Moveable;
  readonly #selecto: Selecto;

  /** True between a gesture's *Start and *End, so reactive re-syncs and hover
   * re-targeting don't fight the live transform moveable is driving. */
  #gesturing = false;
  /** Whether the active gesture actually moved/resized something — gates mark +
   * persist so a plain click (select, no movement) doesn't POST or push undo. */
  #moved = false;
  /** Object id currently under the cursor (drives hover pre-targeting). */
  #hoverId: number | null = null;
  /** Object ids moveable currently targets, and a cheap key to dedupe setState. */
  #targetIds = new Set<number>();
  #targetKey = '';
  /** Canvas zoom factor (#62) — the stage is CSS-scaled by this, so client→model
   * pointer coordinates divide by it when placing a new object. */
  #zoom = 1;
  /** True while a placement POST is in flight, so a second click can't double-place. */
  #placing = false;

  constructor(stage: HTMLElement, doc: EditorDoc, layoutId: string) {
    this.#stage = stage;
    this.#doc = doc;
    this.#layoutId = layoutId;

    this.#moveable = new Moveable(stage, {
      target: [],
      draggable: true,
      resizable: true,
      snappable: true,
      snapGridWidth: GRID,
      snapGridHeight: GRID,
      snapThreshold: SNAP_THRESHOLD,
      isDisplaySnapDigit: false,
      elementGuidelines: [],
      origin: false,
    });

    // ── drag (single + group). Single-target start also makes it the selection. ──
    this.#moveable.on('dragStart', (e) => {
      llog('drag', 'dragStart', { id: this.#idForElement(e.target) });
      this.#begin();
      this.#selectFromTarget(e.target);
    });
    this.#moveable.on('drag', (e) => this.#applyMove(e.target, e.left, e.top));
    this.#moveable.on('dragEnd', () => this.#end('drag'));
    this.#moveable.on('dragGroupStart', () => this.#begin());
    this.#moveable.on('dragGroup', (e) => e.events.forEach((ev) => this.#applyMove(ev.target, ev.left, ev.top)));
    this.#moveable.on('dragGroupEnd', () => this.#end('drag'));

    // ── resize (single + group) — e.drag carries the new left/top for top/left handles ──
    this.#moveable.on('resizeStart', (e) => {
      llog('resize', 'resizeStart', { id: this.#idForElement(e.target) });
      this.#begin();
      this.#selectFromTarget(e.target);
    });
    this.#moveable.on('resize', (e) => this.#applyResize(e.target, e.width, e.height, e.drag.left, e.drag.top));
    this.#moveable.on('resizeEnd', () => this.#end('resize'));
    this.#moveable.on('resizeGroupStart', () => this.#begin());
    this.#moveable.on('resizeGroup', (e) =>
      e.events.forEach((ev) => this.#applyResize(ev.target, ev.width, ev.height, ev.drag.left, ev.drag.top)),
    );
    this.#moveable.on('resizeGroupEnd', () => this.#end('resize'));

    // ── marquee multi-select ──
    this.#selecto = new Selecto({
      container: stage,
      rootContainer: stage,
      selectableTargets: ['.fm-obj'],
      selectByClick: true,
      selectFromInside: false,
      hitRate: 0,
      toggleContinueSelect: 'shift',
    });
    // A marquee over empty canvas live-updates the store selection.
    this.#selecto.on('select', (e) => this.#doc.selectOnly(this.#elementsToIds(e.selected)));
    // Decide, at press time, who owns the gesture:
    this.#selecto.on('dragStart', (e) => {
      const input = e.inputEvent;
      // A non-pointer tool is armed → this press PLACES an object, not selects.
      if (this.#doc.activeTool !== 'pointer') {
        llog('place', 'press while tool armed', {
          tool: this.#doc.activeTool,
          clientX: input.clientX,
          clientY: input.clientY,
        });
        e.stop();
        void this.#placeAt(input.clientX, input.clientY);
        return;
      }
      const target = input.target as Element | null;
      // moveable's own control box (a resize handle / the drag area) → its gesture.
      if (target && this.#moveable.isMoveableElement(target)) {
        llog('drag', 'press on moveable control box → moveable owns gesture');
        e.stop();
        return;
      }
      const objEl = (target?.closest('.fm-obj') ?? null) as HTMLElement | null;
      if (!objEl) {
        llog('select', 'press on empty canvas → marquee');
        return; // empty canvas → selecto runs its marquee
      }
      const id = this.#idForElement(objEl);
      if (id === undefined) {
        llog('target', 'press on object but id UNRESOLVED', { painted: this.#paintedElements().length });
        return;
      }
      // Shift toggles selection membership without starting a drag.
      if (input.shiftKey) {
        llog('select', 'shift-toggle membership', { id });
        this.#doc.toggle(id);
        e.stop();
        this.#updateTarget();
        return;
      }
      // Hover already made this object (or its group) moveable's target → let
      // moveable drag it in THIS gesture. This is the press-drag-in-one path.
      if (this.#targetIds.has(id)) {
        llog('drag', 'press on pre-targeted object → moveable drags it', { id });
        e.stop();
        return;
      }
      // Not pre-targeted (e.g. a touch with no hover): select it so the next press
      // drags. Retarget immediately so it's grabbable.
      llog('select', 'press on un-targeted object → select + retarget', { id });
      this.#doc.selectOnly([id]);
      this.#updateTarget();
      e.stop();
    });

    this.#stage.addEventListener('pointermove', this.#onPointerMove);
    this.#stage.addEventListener('pointerleave', this.#onPointerLeave);
    llog('init', 'CanvasInteraction ready', { layoutId, painted: this.#paintedElements().length });
  }

  /** Reconcile moveable's target with the store selection (called reactively when
   * selection or geometry changes — e.g. after an undo). No-op during a gesture. */
  refresh(): void {
    this.#updateTarget();
  }

  /** Tell the interaction layer the current canvas zoom (#62), so client→model
   * pointer conversion during placement divides by it. */
  setZoom(zoom: number): void {
    const z = zoom > 0 ? zoom : 1;
    if (z !== this.#zoom) llog('zoom', 'setZoom', { zoom: z });
    this.#zoom = z;
  }

  /** Place a new object where the user clicked while a tool is armed (#48). Maps
   * the client point into model coordinates (undoing the zoom scale), finds the
   * part under it, POSTs the create, and adds the returned object(s) to the store
   * as ONE undo step — then disarms back to the pointer tool. A `field` adds both
   * its value object and its spawned caption label (#60). */
  async #placeAt(clientX: number, clientY: number): Promise<void> {
    const tool = this.#doc.activeTool;
    if (tool === 'pointer' || this.#placing) {
      llog('place', 'placeAt ignored', { tool, placing: this.#placing });
      return;
    }
    const canvas = this.#canvas();
    if (!canvas) {
      llog('error', 'placeAt: no .fm-canvas in stage');
      this.#doc.setTool('pointer');
      return;
    }
    const rect = canvas.getBoundingClientRect();
    const z = this.#zoom || 1;
    const cx = Math.max(0, Math.round((clientX - rect.left) / z));
    const cy = Math.max(0, (clientY - rect.top) / z);
    // The single most useful line for "object landed in the wrong place": the
    // client point, the canvas rect it's measured against, the zoom, and the
    // resulting MODEL coordinates (the object's top-left, not its centre).
    llog('place', 'click → model coords', {
      clientX,
      clientY,
      canvasLeft: Math.round(rect.left),
      canvasTop: Math.round(rect.top),
      canvasW: Math.round(rect.width),
      canvasH: Math.round(rect.height),
      zoom: z,
      modelX: cx,
      modelY: Math.round(cy),
    });
    const where = partAtY(this.#doc.renderModel, cy);
    if (!where) {
      llog('place', 'no part under the click', { modelY: Math.round(cy) });
      this.#doc.setTool('pointer');
      return;
    }
    const box = defaultBox(tool);
    // Centre the object on the cursor (click-to-place), clamped to the band
    // origin — placing the top-left at the cursor made objects appear down-right
    // of where you clicked.
    const x = Math.max(0, cx - Math.round(box.w / 2));
    const y = Math.max(0, Math.round(where.localY) - Math.round(box.h / 2));
    llog('place', 'resolved drop (centred on cursor)', { tool, partId: where.partId, x, y, w: box.w, h: box.h });

    this.#placing = true;
    try {
      let views: ObjectView[];
      if (tool === 'field') {
        const fieldId = this.#doc.toolFieldId;
        if (fieldId == null) {
          llog('place', 'field tool armed but no field chosen — nothing to place');
          return;
        }
        views = await createObject(this.#layoutId, {
          partId: where.partId,
          kind: 'field',
          x,
          y,
          w: box.w,
          h: box.h,
          fieldId,
          rec: this.#doc.rec,
        });
      } else {
        views = await createObject(this.#layoutId, {
          partId: where.partId,
          kind: tool,
          x,
          y,
          w: box.w,
          h: box.h,
          content: tool === 'text' ? 'Text' : null,
          props: defaultProps(tool) ?? null,
          rec: this.#doc.rec,
        });
      }
      llog('create', 'server created object(s)', {
        objects: views.map((v) => ({ id: v.id, kind: v.kind, x: v.x, y: v.y, w: v.w, h: v.h })),
      });
      for (const v of views) this.#doc.addObject(v, where.partId);
      this.#doc.mark();
      const placed = views.at(-1); // the field VALUE (its label sorts before it)
      if (placed) {
        this.#doc.selectOnly([placed.id]);
        // The cursor now sits over the freshly-placed object, so make it the hover
        // too: otherwise `#updateTarget` prefers a STALE hover (whatever was under
        // the cursor before the click) and points moveable at the wrong object —
        // the reported "resize does nothing" (it resized the hovered object).
        this.#hoverId = placed.id;
        llog('place', 'added to store + selected + hover pinned to placed', { selectedId: placed.id });
      }
    } catch (e) {
      lerror('place', 'create failed', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      this.#placing = false;
      this.#doc.setTool('pointer');
    }
  }

  destroy(): void {
    this.#stage.removeEventListener('pointermove', this.#onPointerMove);
    this.#stage.removeEventListener('pointerleave', this.#onPointerLeave);
    this.#moveable.destroy();
    this.#selecto.destroy();
  }

  // ── hover pre-targeting ──

  #onPointerMove = (e: PointerEvent): void => {
    if (this.#gesturing) return;
    const t = e.target as Element | null;
    // Over moveable's own control box → keep the current target (don't flicker).
    if (t && this.#moveable.isMoveableElement(t)) return;
    const objEl = (t?.closest('.fm-obj') ?? null) as HTMLElement | null;
    const id = objEl ? this.#idForElement(objEl) ?? null : null;
    if (id === this.#hoverId) return;
    this.#hoverId = id;
    this.#updateTarget();
  };

  #onPointerLeave = (): void => {
    if (this.#gesturing || this.#hoverId === null) return;
    this.#hoverId = null;
    this.#updateTarget();
  };

  /** Choose moveable's target: the hovered object (so a press grabs it), unless
   * you're hovering a member of a 2+ selection (then keep the whole group so the
   * press drags the group). Falls back to the selection when not hovering, and to
   * nothing when idle. Dedupes redundant setState by a target-id key. */
  #updateTarget(): void {
    if (this.#gesturing) return;
    // A placement tool is armed → the canvas is a drawing surface, not a select/
    // drag surface: drop moveable's target so a press places instead of grabs.
    if (this.#doc.activeTool !== 'pointer') {
      if (this.#targetKey === '') return;
      this.#targetKey = '';
      this.#targetIds = new Set();
      this.#moveable.setState({ target: [], elementGuidelines: [] });
      llog('target', 'tool armed → moveable target cleared');
      return;
    }
    const sel = [...this.#doc.selection];
    const selSet = new Set(sel);
    let ids: number[];
    if (this.#hoverId !== null && selSet.has(this.#hoverId) && sel.length >= 2) {
      ids = sel; // hovering a group member → drag the whole group
    } else if (this.#hoverId !== null) {
      ids = [this.#hoverId]; // hovering any object → grab just it
    } else if (sel.length > 0) {
      ids = sel; // not hovering → keep the selection box up
    } else {
      ids = [];
    }
    const key = ids.slice().sort((a, b) => a - b).join(',');
    if (key === this.#targetKey) return;
    this.#targetKey = key;
    this.#targetIds = new Set(ids);
    const targets = ids.map((id) => this.#elementForId(id)).filter((el): el is HTMLElement => !!el);
    const guidelines = this.#paintedElements().filter((el) => !targets.includes(el));
    this.#moveable.setState({ target: targets, elementGuidelines: guidelines });
    // THE key line for "resize does nothing": if `chosenIds` has an id but
    // `resolvedEls` is fewer, moveable has no element to attach handles to — the
    // store id didn't map to a painted `.fm-obj` (stale paint order / DOM not yet
    // committed after a create).
    llog('target', 'moveable target set', {
      hoverId: this.#hoverId,
      selection: sel,
      chosenIds: ids,
      resolvedEls: targets.length,
      paintedCount: this.#paintedElements().length,
      paintOrderIds: objectIdsInPaintOrder(this.#doc.renderModel),
    });
  }

  // ── gesture lifecycle ──

  #begin(): void {
    this.#gesturing = true;
    this.#moved = false;
  }

  /** End a gesture: if it actually changed geometry, seal one undo step and
   * persist the moved/resized group; a no-move click does neither. Then re-target. */
  #end(kind: 'drag' | 'resize' = 'drag'): void {
    this.#gesturing = false;
    llog(kind, `${kind}End`, { moved: this.#moved, selection: [...this.#doc.selection] });
    if (this.#moved) {
      this.#doc.mark();
      void this.#persistSelection();
    }
    this.#targetKey = ''; // force a re-sync after the gesture
    this.#updateTarget();
  }

  /** Make the dragged/resized single target the selection (if it wasn't already). */
  #selectFromTarget(el: HTMLElement | SVGElement): void {
    const id = this.#idForElement(el);
    if (id !== undefined && !this.#doc.isSelected(id)) this.#doc.selectOnly([id]);
  }

  #applyMove(target: HTMLElement | SVGElement, left: number, top: number): void {
    const id = this.#idForElement(target);
    if (id === undefined) {
      llog('target', 'drag: target element has NO mapped id — move is a no-op', {
        painted: this.#paintedElements().length,
      });
      return;
    }
    this.#moved = true;
    this.#doc.setObjectGeometry(id, { x: clampOrigin(left), y: clampOrigin(top) });
    llog('drag', 'apply move', { id, x: clampOrigin(left), y: clampOrigin(top) });
  }

  #applyResize(target: HTMLElement | SVGElement, width: number, height: number, left: number, top: number): void {
    const id = this.#idForElement(target);
    if (id === undefined) {
      llog('target', 'resize: target element has NO mapped id — resize is a no-op', {
        painted: this.#paintedElements().length,
        paintOrderIds: objectIdsInPaintOrder(this.#doc.renderModel),
      });
      return;
    }
    this.#moved = true;
    this.#doc.setObjectGeometry(id, {
      x: clampOrigin(left),
      y: clampOrigin(top),
      w: Math.max(1, Math.round(width)),
      h: Math.max(1, Math.round(height)),
    });
    llog('resize', 'apply resize', {
      id,
      w: Math.max(1, Math.round(width)),
      h: Math.max(1, Math.round(height)),
      x: clampOrigin(left),
      y: clampOrigin(top),
    });
  }

  // ── persistence (#46 bulk axum contract) ──

  async #persistSelection(): Promise<void> {
    const items = [...this.#doc.selection]
      .map((id) => this.#doc.getObject(id))
      .filter((o): o is NonNullable<typeof o> => !!o)
      .map((o) => ({ id: o.id, x: o.x, y: o.y, w: o.w, h: o.h }));
    if (items.length === 0) return;
    llog('persist', 'POST geometry', { items });
    try {
      const r = await fetch(`/design/${this.#layoutId}/geometry`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(items),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      llog('persist', 'geometry saved', { count: items.length });
    } catch (e) {
      // The store already reflects the edit; surface the persist failure rather
      // than tearing down the in-memory state (a reload would reveal divergence).
      lerror('persist', 'failed to persist object geometry', e);
    }
  }

  // ── id ↔ element mapping (paint-order index; see canvas-edit.ts) ──

  #canvas(): HTMLElement | null {
    return this.#stage.querySelector('.fm-canvas');
  }

  #paintedElements(): HTMLElement[] {
    const canvas = this.#canvas();
    return canvas ? Array.from(canvas.querySelectorAll<HTMLElement>('.fm-obj')) : [];
  }

  #elementsToIds(elements: Array<HTMLElement | SVGElement>): number[] {
    return elementsToObjectIds(elements, this.#paintedElements(), objectIdsInPaintOrder(this.#doc.renderModel));
  }

  #elementForId(id: number): HTMLElement | undefined {
    const ids = objectIdsInPaintOrder(this.#doc.renderModel);
    const i = ids.indexOf(id);
    return i >= 0 ? this.#paintedElements()[i] : undefined;
  }

  #idForElement(el: Element): number | undefined {
    const i = this.#paintedElements().indexOf(el as HTMLElement);
    if (i < 0) return undefined;
    return objectIdsInPaintOrder(this.#doc.renderModel)[i];
  }
}
