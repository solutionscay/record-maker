// Pure Arrange math for the Inspector's Arrange panel (#83): align / distribute /
// resize-to-match geometry and z-order reordering. No DOM and no store imports —
// like canvas-edit.ts, the verifiable logic lives apart from the UI glue, so the
// Inspector section components stay thin.

export type Geom = { x: number; y: number; w: number; h: number };

/** The structural slice of an object the arrange math needs (a Readonly
 * ObjectDoc satisfies it). */
export type ArrangeObject = Readonly<{ id: number; kind: string; x: number; y: number; w: number; h: number }>;

export type AlignEdge = 'left' | 'hcenter' | 'right' | 'top' | 'vmiddle' | 'bottom';
export type ZCmd = 'front' | 'back' | 'forward' | 'backward';

/** The union bounding box of the current selection (the v1 reference frame). */
export function selectionBounds(os: readonly ArrangeObject[]): {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
  cx: number;
  cy: number;
} {
  const minX = Math.min(...os.map((o) => o.x));
  const minY = Math.min(...os.map((o) => o.y));
  const maxX = Math.max(...os.map((o) => o.x + o.w));
  const maxY = Math.max(...os.map((o) => o.y + o.h));
  return { minX, minY, maxX, maxY, cx: (minX + maxX) / 2, cy: (minY + maxY) / 2 };
}

/** Align every object to the selection bounding box. Only x/y move — w/h (and a
 * line's angle/length) are untouched, so lines never distort. Returns only the
 * objects that actually move. */
export function alignGeometries(objects: readonly ArrangeObject[], edge: AlignEdge): Map<number, Geom> {
  const geoms = new Map<number, Geom>();
  if (objects.length < 2) return geoms;
  const b = selectionBounds(objects);
  for (const o of objects) {
    let x = o.x;
    let y = o.y;
    switch (edge) {
      case 'left': x = b.minX; break;
      case 'right': x = b.maxX - o.w; break;
      case 'hcenter': x = Math.round(b.cx - o.w / 2); break;
      case 'top': y = b.minY; break;
      case 'bottom': y = b.maxY - o.h; break;
      case 'vmiddle': y = Math.round(b.cy - o.h / 2); break;
    }
    x = Math.max(0, x);
    y = Math.max(0, y);
    if (x !== o.x || y !== o.y) geoms.set(o.id, { x, y, w: o.w, h: o.h });
  }
  return geoms;
}

/** Distribute with equal gaps between adjacent edges along one axis (#83, locked
 * decision). Outermost objects stay put; interior objects move so the empty space
 * between neighbours is equal. Needs ≥3 objects. */
export function distributeGeometries(objects: readonly ArrangeObject[], axis: 'h' | 'v'): Map<number, Geom> {
  const os = objects.slice();
  const geoms = new Map<number, Geom>();
  if (os.length < 3) return geoms;
  if (axis === 'h') {
    const sorted = os.sort((a, b) => a.x - b.x || a.id - b.id);
    const left = sorted[0].x;
    const right = sorted[sorted.length - 1].x + sorted[sorted.length - 1].w;
    const sumW = sorted.reduce((s, o) => s + o.w, 0);
    const gap = (right - left - sumW) / (sorted.length - 1);
    let cursor = left;
    for (const o of sorted) {
      const nx = Math.max(0, Math.round(cursor));
      if (nx !== o.x) geoms.set(o.id, { x: nx, y: o.y, w: o.w, h: o.h });
      cursor += o.w + gap;
    }
  } else {
    const sorted = os.sort((a, b) => a.y - b.y || a.id - b.id);
    const top = sorted[0].y;
    const bottom = sorted[sorted.length - 1].y + sorted[sorted.length - 1].h;
    const sumH = sorted.reduce((s, o) => s + o.h, 0);
    const gap = (bottom - top - sumH) / (sorted.length - 1);
    let cursor = top;
    for (const o of sorted) {
      const ny = Math.max(0, Math.round(cursor));
      if (ny !== o.y) geoms.set(o.id, { x: o.x, y: ny, w: o.w, h: o.h });
      cursor += o.h + gap;
    }
  }
  return geoms;
}

/** Resize objects to the largest width/height/both among them. Lines are
 * excluded (their w/h encode direction); needs ≥2 non-line objects. */
export function resizeMatchGeometries(objects: readonly ArrangeObject[], dim: 'w' | 'h' | 'both'): Map<number, Geom> {
  const targets = objects.filter((o) => o.kind !== 'line');
  const geoms = new Map<number, Geom>();
  if (targets.length < 2) return geoms;
  const w = Math.max(...targets.map((o) => o.w));
  const h = Math.max(...targets.map((o) => o.h));
  for (const o of targets) {
    const nw = dim === 'w' || dim === 'both' ? w : o.w;
    const nh = dim === 'h' || dim === 'both' ? h : o.h;
    if (nw !== o.w || nh !== o.h) geoms.set(o.id, { x: o.x, y: o.y, w: nw, h: nh });
  }
  return geoms;
}

/** Reorder a part's object ids for one z-command, preserving the selection's own
 * relative order when it moves as a block. `ids` is back→front; the result is the
 * new back→front order (index becomes the densified `z`). */
export function reorderZ(ids: readonly number[], sel: ReadonlySet<number>, cmd: ZCmd): number[] {
  const isSel = (id: number): boolean => sel.has(id);
  if (cmd === 'front') return [...ids.filter((id) => !isSel(id)), ...ids.filter((id) => isSel(id))];
  if (cmd === 'back') return [...ids.filter((id) => isSel(id)), ...ids.filter((id) => !isSel(id))];
  const a = ids.slice();
  if (cmd === 'forward') {
    // Shift each selected one step toward the front (higher index), as a block.
    for (let i = a.length - 2; i >= 0; i--) if (isSel(a[i]) && !isSel(a[i + 1])) [a[i], a[i + 1]] = [a[i + 1], a[i]];
  } else {
    for (let i = 1; i < a.length; i++) if (isSel(a[i]) && !isSel(a[i - 1])) [a[i], a[i - 1]] = [a[i - 1], a[i]];
  }
  return a;
}

/** Compute the changed `(id, z)` pairs for one z-command over every part that
 * holds a selected object. z-order is per-part (paint order is `(z, id)` within a
 * band), so a selection spanning bands reorders each independently. Densifying z
 * touches non-selected objects too; only real changes (per `zOf`) are returned. */
export function zOrderChanges(
  partObjectIds: readonly (readonly number[])[],
  sel: ReadonlySet<number>,
  cmd: ZCmd,
  zOf: (id: number) => number | undefined,
): [number, number][] {
  const zmap = new Map<number, number>();
  for (const ids of partObjectIds) {
    // ids are already back→front by (z, id)
    if (!ids.some((id) => sel.has(id))) continue;
    reorderZ(ids, sel, cmd).forEach((id, i) => zmap.set(id, i));
  }
  return [...zmap].filter(([id, z]) => {
    const current = zOf(id);
    return current !== undefined && current !== z;
  });
}
