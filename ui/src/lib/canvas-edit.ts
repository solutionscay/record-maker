// Pure geometry/identity helpers for the Layout interaction layer (#46). No DOM
// and no library imports, so they are unit-tested headlessly alongside the store
// (scripts/doc-check.mjs). The moveable/selecto glue (interaction.svelte.ts)
// builds on these, keeping the verifiable logic separate from the browser-only
// integration.

import type { DesignModel } from './model';

/** Editor snap grid in px. Feeds both moveable's `snapGridWidth/Height` options
 * and the geometry we persist, so on-screen snapping and stored values agree. */
export const GRID = 8;

/** Pixel distance within which moveable snaps to a grid line or sibling edge. */
export const SNAP_THRESHOLD = 5;

/** Snap a px value to the nearest grid line. `grid <= 0` disables snapping (the
 * value is still rounded to a whole px, since geometry is integer). */
export function snapToGrid(v: number, grid: number = GRID): number {
  if (grid <= 0) return Math.round(v);
  return Math.round(v / grid) * grid;
}

/** Clamp a part-relative coordinate to the canvas origin (never negative). */
export function clampOrigin(v: number): number {
  return Math.max(0, Math.round(v));
}

/** Object ids in the order LayoutPreview paints `.fm-obj` elements (parts
 * top→bottom, objects back→front). The Nth painted element maps to the Nth id,
 * so a pointer hit or selecto's selected elements resolve to object ids by index
 * — WITHOUT stamping ids onto the parity-checked canvas DOM. */
export function objectIdsInPaintOrder(model: DesignModel): number[] {
  return model.parts.flatMap((p) => p.objects.map((o) => o.id));
}

/** Map elements (selecto's selection, or a hit `.fm-obj`) to object ids by their
 * index among all painted `.fm-obj` elements. Elements not present are dropped.
 * Pure given the two element lists (identity-based), so the index mapping — the
 * only DOM-coupled assumption — is testable and mirrors objectIdsInPaintOrder. */
export function elementsToObjectIds(
  elements: readonly Element[],
  paintedInOrder: readonly Element[],
  ids: readonly number[],
): number[] {
  const out: number[] = [];
  for (const el of elements) {
    const idx = paintedInOrder.indexOf(el);
    if (idx >= 0 && idx < ids.length) out.push(ids[idx]);
  }
  return out;
}
