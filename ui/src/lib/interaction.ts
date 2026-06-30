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
// Object identity without polluting the parity-checked canvas DOM: the Nth
// painted `.fm-obj` element maps to the Nth id in renderModel paint order (see
// canvas-edit.ts), so selections and hits resolve to ids by index — no data-*
// attributes added to the canvas.

import Moveable from 'moveable';
import Selecto from 'selecto';

import type { EditorDoc } from './doc.svelte';
import { GRID, SNAP_THRESHOLD, clampOrigin, elementsToObjectIds, objectIdsInPaintOrder } from './canvas-edit';

export class CanvasInteraction {
  readonly #stage: HTMLElement;
  readonly #doc: EditorDoc;
  readonly #layoutId: string;
  readonly #moveable: Moveable;
  readonly #selecto: Selecto;
  /** True between a gesture's *Start and *End, so reactive re-syncs don't fight
   * the live transform moveable is driving. */
  #gesturing = false;

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
      bounds: { left: 0, top: 0, position: 'css' },
      origin: false,
    });

    // ── drag (single + group) ──
    this.#moveable.on('dragStart', () => this.#begin());
    this.#moveable.on('drag', (e) => this.#applyMove(e.target, e.left, e.top));
    this.#moveable.on('dragEnd', () => this.#end());
    this.#moveable.on('dragGroupStart', () => this.#begin());
    this.#moveable.on('dragGroup', (e) => e.events.forEach((ev) => this.#applyMove(ev.target, ev.left, ev.top)));
    this.#moveable.on('dragGroupEnd', () => this.#end());

    // ── resize (single + group) — e.drag carries the new left/top for top/left handles ──
    this.#moveable.on('resizeStart', () => this.#begin());
    this.#moveable.on('resize', (e) => this.#applyResize(e.target, e.width, e.height, e.drag.left, e.drag.top));
    this.#moveable.on('resizeEnd', () => this.#end());
    this.#moveable.on('resizeGroupStart', () => this.#begin());
    this.#moveable.on('resizeGroup', (e) =>
      e.events.forEach((ev) => this.#applyResize(ev.target, ev.width, ev.height, ev.drag.left, ev.drag.top)),
    );
    this.#moveable.on('resizeGroupEnd', () => this.#end());

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
    // Live-update the store selection as the marquee adds/removes objects.
    this.#selecto.on('select', (e) => this.#doc.selectOnly(this.#elementsToIds(e.selected)));
    // A press that lands on a moveable handle or an already-selected object is a
    // transform, not a marquee — let moveable take it.
    this.#selecto.on('dragStart', (e) => {
      const target = e.inputEvent.target as Element | null;
      const objEl = target?.closest('.fm-obj') ?? null;
      const onSelected = objEl ? this.#doc.isSelected(this.#idForElement(objEl) ?? -1) : false;
      if ((target && this.#moveable.isMoveableElement(target)) || onSelected) e.stop();
    });
    // If a marquee began on an empty spot but ended on a selection, hand the same
    // gesture to moveable so click-drag-in-one feels native.
    this.#selecto.on('selectEnd', (e) => {
      if (!e.isDragStart) return;
      const els = e.selected as HTMLElement[];
      e.inputEvent.preventDefault();
      this.#moveable.setState({ target: els }, () => this.#moveable.dragStart(e.inputEvent));
    });
  }

  /** Reflect the store's selection (and current geometry) into moveable: target
   * the selected elements and offer the rest as snap guidelines. Skipped during a
   * live gesture, which moveable owns. Call this reactively whenever the store's
   * selection or renderModel changes. */
  syncSelection(): void {
    if (this.#gesturing) return;
    const ids = [...this.#doc.selection];
    const targets = ids.map((id) => this.#elementForId(id)).filter((el): el is HTMLElement => !!el);
    const guidelines = this.#paintedElements().filter((el) => !targets.includes(el));
    this.#moveable.setState({ target: targets, elementGuidelines: guidelines });
  }

  destroy(): void {
    this.#moveable.destroy();
    this.#selecto.destroy();
  }

  // ── gesture lifecycle ──

  #begin(): void {
    this.#gesturing = true;
  }

  /** End a gesture: seal one undo step and persist the moved/resized group. */
  #end(): void {
    this.#gesturing = false;
    this.#doc.mark();
    void this.#persistSelection();
    this.syncSelection();
  }

  #applyMove(target: HTMLElement | SVGElement, left: number, top: number): void {
    const id = this.#idForElement(target);
    if (id === undefined) return;
    this.#doc.setObjectGeometry(id, { x: clampOrigin(left), y: clampOrigin(top) });
  }

  #applyResize(target: HTMLElement | SVGElement, width: number, height: number, left: number, top: number): void {
    const id = this.#idForElement(target);
    if (id === undefined) return;
    this.#doc.setObjectGeometry(id, {
      x: clampOrigin(left),
      y: clampOrigin(top),
      w: Math.max(1, Math.round(width)),
      h: Math.max(1, Math.round(height)),
    });
  }

  // ── persistence (#46 bulk axum contract) ──

  async #persistSelection(): Promise<void> {
    const items = [...this.#doc.selection]
      .map((id) => this.#doc.getObject(id))
      .filter((o): o is NonNullable<typeof o> => !!o)
      .map((o) => ({ id: o.id, x: o.x, y: o.y, w: o.w, h: o.h }));
    if (items.length === 0) return;
    try {
      const r = await fetch(`/design/${this.#layoutId}/geometry`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(items),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
    } catch (e) {
      // The store already reflects the edit; surface the persist failure rather
      // than tearing down the in-memory state (a reload would reveal divergence).
      console.error('failed to persist object geometry', e);
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
