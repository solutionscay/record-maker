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

/** Position the object by one of its edges while preserving width and height.
 * Returns null when the requested coordinate would move it before the origin. */
export function geometryForEdge(box: Box, edge: ObjectEdge, value: number): FullGeometry | null {
  if (!Number.isFinite(value) || value < 0 || !Number.isInteger(value)) return null;

  switch (edge) {
    case 'left':
      return { x: value, y: box.y, w: box.w, h: box.h };
    case 'right':
      return value >= box.w ? { x: value - box.w, y: box.y, w: box.w, h: box.h } : null;
    case 'top':
      return { x: box.x, y: value, w: box.w, h: box.h };
    case 'bottom':
      return value >= box.h ? { x: box.x, y: value - box.h, w: box.w, h: box.h } : null;
  }
}
