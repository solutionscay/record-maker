import type { ObjectDoc } from '../doc.svelte';

export type ObjectEdge = 'left' | 'right' | 'top' | 'bottom';

type Box = Readonly<Pick<ObjectDoc, 'x' | 'y' | 'w' | 'h'>>;
type FullGeometry = Pick<ObjectDoc, 'x' | 'y' | 'w' | 'h'>;

/** Part-relative edge coordinates shown by the Position inspector. */
export function edgeValue(box: Box, edge: ObjectEdge): number {
  switch (edge) {
    case 'left': return box.x;
    case 'right': return box.x + box.w;
    case 'top': return box.y;
    case 'bottom': return box.y + box.h;
  }
}

/** Move one edge while preserving the opposite edge. Returns null when the
 * requested coordinate would cross the opposite edge or leave the grid origin. */
export function geometryForEdge(box: Box, edge: ObjectEdge, value: number): FullGeometry | null {
  if (!Number.isFinite(value) || value < 0 || !Number.isInteger(value)) return null;
  const right = box.x + box.w;
  const bottom = box.y + box.h;

  switch (edge) {
    case 'left':
      return value < right ? { x: value, y: box.y, w: right - value, h: box.h } : null;
    case 'right':
      return value > box.x ? { x: box.x, y: box.y, w: value - box.x, h: box.h } : null;
    case 'top':
      return value < bottom ? { x: box.x, y: value, w: box.w, h: bottom - value } : null;
    case 'bottom':
      return value > box.y ? { x: box.x, y: box.y, w: box.w, h: value - box.y } : null;
  }
}
