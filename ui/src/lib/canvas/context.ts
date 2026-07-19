// Shared context for the canvas interaction controllers (#46, split in #135).
// Holds the identities every gesture needs (stage / doc / layoutId), the few
// genuinely CROSS-controller flags (zoom, placing, gesturing — each written by
// the controller that owns the concern, read by its peers), and the DOM +
// object-identity helpers. The CanvasInteraction coordinator wires the
// controller references after construction, so each controller reaches its
// peers through `ctx.<controller>` without circular constructor dependencies —
// controllers only touch peers inside event handlers, never at build time.

import type { EditorDoc } from '../doc.svelte';
import type { ObjectView } from '../model';
import type { ClipboardController } from './clipboard-controller';
import type { HoverController } from './hover';
import type { PlacementController } from './placement';
import type { TextEditController } from './text-edit';
import type { TransformController } from './transform';
import { EdgeAutoscroll } from './autoscroll';

export type IdentitySnapshot = {
  painted: HTMLElement[];
  ids: number[];
  elementToId: Map<Element, number>;
  idToElement: Map<number, HTMLElement>;
};
export type CanvasCoordinateFrame = {
  canvas: HTMLElement;
  rect: { left: number; top: number; right: number; bottom: number };
  zoom: number;
  clientLeft: number;
  clientTop: number;
  scrollLeft: number;
  scrollTop: number;
};

export class CanvasContext {
  readonly stage: HTMLElement;
  readonly doc: EditorDoc;
  readonly layoutId: string;
  readonly autoscroll: EdgeAutoscroll;

  /** Canvas zoom factor (#62) — the stage is CSS-scaled by this, so client→model
   * pointer coordinates divide by it when placing a new object. Written by the
   * transform controller's setZoom. */
  zoom = 1;
  /** True while a create/clone POST is in flight (draw placement, field drop,
   * paste/duplicate), so a second trigger can't double-place. Shared because both
   * the placement and clipboard controllers materialize new objects. */
  placing = false;
  /** True between a gesture's *Start and *End, so reactive re-syncs and hover
   * re-targeting don't fight the live transform moveable is driving. Written by
   * the transform controller. */
  gesturing = false;

  /** Identity snapshot pinned for the duration of a gesture (set by the
   * transform controller's begin/end), so mid-gesture DOM churn can't remap ids. */
  gestureIdentity: IdentitySnapshot | null = null;
  /** Cached paint-order identity snapshot for NON-gesture lookups (hover
   * pointermove, click/selection id resolution). Built lazily on first use and
   * invalidated by `invalidateIdentity()` — the coordinator's refresh() calls
   * that after every render-model change, i.e. whenever the canvas DOM / paint
   * order can differ — so a mouse move never rebuilds the full querySelectorAll
   * + paint order per event. Gestures still pin their own snapshot. */
  #identityCache: IdentitySnapshot | null = null;

  // Peer controllers, wired by the coordinator right after construction.
  hover!: HoverController;
  text!: TextEditController;
  placement!: PlacementController;
  clipboard!: ClipboardController;
  transform!: TransformController;

  constructor(stage: HTMLElement, doc: EditorDoc, layoutId: string) {
    this.stage = stage;
    this.doc = doc;
    this.layoutId = layoutId;
    this.autoscroll = new EdgeAutoscroll(stage);
  }

  /** The render model (and thus the canvas DOM) may have changed — drop the
   * cached paint-order snapshot so the next lookup re-reads the fresh DOM. */
  invalidateIdentity(): void {
    this.#identityCache = null;
  }

  // ── DOM lookups ────────────────────────────────────────────────────────────

  canvas(): HTMLElement | null {
    return this.stage.querySelector('.fm-canvas');
  }

  partOverlay(): HTMLElement | null {
    return this.stage.querySelector('.le-part-overlays');
  }

  /** Position an overlay element (draw/drop preview, hover outline, text editor)
   * over an object's part-relative box. The overlay layer is offset by the part's
   * top, so `y` is measured within the part. */
  placeOverlay(el: HTMLElement, box: { x: number; y: number; w: number; h: number }, partTop: number): void {
    el.style.left = `${box.x}px`;
    el.style.top = `${partTop + box.y}px`;
    el.style.width = `${box.w}px`;
    el.style.height = `${box.h}px`;
  }

  coordinateFrame(): CanvasCoordinateFrame | null {
    const canvas = this.canvas();
    if (!canvas) return null;
    const r = canvas.getBoundingClientRect();
    return {
      canvas,
      rect: { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
      zoom: this.zoom || 1,
      clientLeft: canvas.clientLeft,
      clientTop: canvas.clientTop,
      scrollLeft: this.stage.scrollLeft,
      scrollTop: this.stage.scrollTop,
    };
  }

  canvasPoint(
    clientX: number,
    clientY: number,
    frame: CanvasCoordinateFrame | null = this.coordinateFrame(),
  ): { x: number; y: number } | null {
    if (!frame) return null;
    return {
      x: Math.max(0, (clientX - frame.rect.left + this.stage.scrollLeft - frame.scrollLeft) / frame.zoom - frame.clientLeft),
      y: Math.max(0, (clientY - frame.rect.top + this.stage.scrollTop - frame.scrollTop) / frame.zoom - frame.clientTop),
    };
  }

  pointInCanvas(
    clientX: number,
    clientY: number,
    frame: CanvasCoordinateFrame | null = this.coordinateFrame(),
  ): boolean {
    if (!frame) return false;
    const { rect } = frame;
    return clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom;
  }

  // ── id ↔ element mapping ───────────────────────────────────────────────────

  paintedElements(): HTMLElement[] {
    const canvas = this.canvas();
    // History echo ghosts deliberately reuse data-object-id for animation, but
    // they are not authored targets and must never enter the identity maps.
    return canvas ? Array.from(canvas.querySelectorAll<HTMLElement>('.fm-obj:not(.le-echo-ghost)')) : [];
  }

  identitySnapshot(): IdentitySnapshot {
    // Identity comes from the data-object-id both renderers stamp (#134), so
    // element↔id pairing can never drift from paint-order assumptions. An
    // element without the attribute maps to NaN, which matches no id.
    const painted = this.paintedElements();
    const ids = painted.map((el) => Number(el.dataset.objectId));
    return {
      painted,
      ids,
      elementToId: new Map(painted.map((element, index) => [element, ids[index]])),
      idToElement: new Map(painted.map((element, index) => [ids[index], element])),
    };
  }

  currentIdentity(): IdentitySnapshot {
    return this.gestureIdentity ?? (this.#identityCache ??= this.identitySnapshot());
  }

  /** Hit-test a client point through the FULL element stack (not just the
   * topmost element), so a `.fm-obj` underneath one of moveable's own overlay
   * proxies (e.g. the group `moveable-area` drag-proxy) can still be found. */
  objectElementAt(clientX: number, clientY: number): HTMLElement | null {
    for (const el of document.elementsFromPoint(clientX, clientY)) {
      const objEl = el.closest('.fm-obj');
      if (objEl) return objEl as HTMLElement;
    }
    return null;
  }

  elementsToIds(elements: Array<HTMLElement | SVGElement>): number[] {
    const identity = this.currentIdentity();
    return elements
      .map((element) => identity.elementToId.get(element))
      .filter((id): id is number => id !== undefined && Number.isFinite(id));
  }

  elementForId(id: number, identity: IdentitySnapshot = this.currentIdentity()): HTMLElement | undefined {
    return identity.idToElement.get(id);
  }

  idForElement(el: Element, identity: IdentitySnapshot = this.currentIdentity()): number | undefined {
    return identity.elementToId.get(el);
  }

  // ── model lookups ──────────────────────────────────────────────────────────

  partTop(partId: number): number | null {
    let top = 0;
    for (const part of this.doc.renderModel.parts) {
      if (part.id === partId) return top;
      top += part.height;
    }
    return null;
  }

  /** The current render-model view of one object (carries the server-derived
   * `textStyle`/styles the document `ObjectDoc` doesn't). */
  objectView(id: number): ObjectView | undefined {
    for (const p of this.doc.renderModel.parts) {
      const v = p.objects.find((obj) => obj.id === id);
      if (v) return v;
    }
    return undefined;
  }

  /** Surface a caught error to the store's error banner (the caller has already
   * `lerror`'d it with scope-specific context). */
  reportError(e: unknown): void {
    this.doc.setError(e instanceof Error ? e.message : String(e));
  }
}
