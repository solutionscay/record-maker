// Pure Create-zone placement helpers (#48): the default geometry/appearance a
// tool drops with, and the part-under-a-canvas-y lookup. No DOM and no library
// imports, so they are unit-tested headlessly alongside the store
// (scripts/doc-check.mjs). The interaction layer composes these with persist.ts
// and the store to turn a canvas click into a placed object.

import type { DesignModel } from './model';
import type { ToolKind } from './doc.svelte';

/** A default object box in px. */
export interface DefaultBox {
  w: number;
  h: number;
}

/** The size a freshly placed object takes (click-to-place; draw-to-size later).
 * A line is thin, text/field are a label row, shapes are a small box. */
export function defaultBox(tool: ToolKind): DefaultBox {
  switch (tool) {
    case 'line':
      return { w: 120, h: 2 };
    case 'text':
      return { w: 96, h: 24 };
    case 'field':
      return { w: 200, h: 24 };
    case 'portal':
      return { w: 280, h: 24 }; // #184: geometry is the reusable first row
    default:
      return { w: 80, h: 60 }; // rect / ellipse
  }
}

/** The appearance bag a shape is born with (#49 keys the server understands).
 * Text/field carry no appearance defaults (`undefined`). */
export function defaultProps(tool: ToolKind): Record<string, unknown> | undefined {
  switch (tool) {
    case 'rect':
      return { fill: '#f7f8fa', stroke: '#d3d8de', strokeWidth: 1, radius: 0 };
    case 'ellipse':
      return { fill: '#f7f8fa', stroke: '#d3d8de', strokeWidth: 1 };
    case 'line':
      return { stroke: '#888888', strokeWidth: 2 };
    case 'portal':
      return { rowCount: 5 }; // #184: preview/viewport capacity, independent of row height
    default:
      return undefined; // text / field
  }
}

/** Find the part whose stacked band contains canvas-y `y`, returning its id and
 * the y LOCAL to that band (geometry is part-relative). Parts stack top→bottom in
 * model order. A click below all bands falls to the last part; `null` only when
 * the layout has no parts. */
export function partAtY(model: DesignModel, y: number): { partId: number; localY: number } | null {
  let top = 0;
  for (const p of model.parts) {
    if (y >= top && y < top + p.height) return { partId: p.id, localY: y - top };
    top += p.height;
  }
  const last = model.parts.at(-1);
  return last ? { partId: last.id, localY: Math.max(0, last.height - 1) } : null;
}
