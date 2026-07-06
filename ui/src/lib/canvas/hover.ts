// Hover controller (#46, split in #135): tracks the object under the cursor and
// paints the lightweight hover outline. Hover never pre-targets moveable — the
// outline is a separate overlay so resize handles never appear on mere hover.

import type { CanvasContext } from './context';

export class HoverController {
  readonly #ctx: CanvasContext;
  /** Object id currently under the cursor (drives hover pre-targeting). */
  #hoverId: number | null = null;
  #outline: HTMLElement | null = null;

  constructor(ctx: CanvasContext) {
    this.#ctx = ctx;
  }

  get hoverId(): number | null {
    return this.#hoverId;
  }

  onPointerMove = (e: PointerEvent): void => {
    if (this.#ctx.gesturing) return;
    const t = e.target as Element | null;
    // Over moveable's own control box → keep the current target (don't flicker).
    if (t && this.#ctx.transform.isMoveableElement(t)) return;
    const objEl = (t?.closest('.fm-obj') ?? null) as HTMLElement | null;
    const id = objEl ? this.#ctx.idForElement(objEl) ?? null : null;
    if (id === this.#hoverId) return;
    this.set(id);
  };

  onPointerLeave = (): void => {
    if (this.#ctx.gesturing || this.#hoverId === null) return;
    this.set(null);
  };

  /** Set the hover id, mirror it into the store, and repaint the outline. */
  set(id: number | null): void {
    this.#hoverId = id;
    this.#ctx.doc.hover(id);
    this.paint();
  }

  /** Pin the hover id WITHOUT a store write or repaint — used right after a
   * placement/paste selects the new object, so the following target sync
   * resolves against the placed object. */
  pin(id: number | null): void {
    this.#hoverId = id;
  }

  /** Clear the hover id and outline WITHOUT a `doc.hover(null)` write — the
   * empty-canvas click deselect path. */
  clearVisual(): void {
    this.#hoverId = null;
    this.paint();
  }

  paint(): void {
    const id = this.#hoverId;
    const o = id === null ? undefined : this.#ctx.doc.getObject(id);
    if (!o || this.#ctx.doc.isSelected(o.id) || this.#ctx.text.isEditing) {
      this.#outline?.remove();
      this.#outline = null;
      return;
    }
    const top = this.#ctx.partTop(o.partId);
    const overlay = this.#ctx.partOverlay();
    if (top === null || !overlay) return;
    if (!this.#outline) {
      this.#outline = document.createElement('div');
      this.#outline.className = 'le-hover-outline';
      overlay.append(this.#outline);
    }
    this.#ctx.placeOverlay(this.#outline, o, top);
  }

  destroy(): void {
    this.#outline?.remove();
  }
}
