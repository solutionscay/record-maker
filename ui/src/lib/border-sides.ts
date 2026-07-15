// Persisted border-placement contract (#191). Missing metadata is the legacy
// state: all four outer edges and no portal row separators. Arrays are always
// written in this canonical order so mixed-selection comparisons stay stable.

export const OUTER_STROKE_SIDES = ['top', 'right', 'bottom', 'left'] as const;
export const STROKE_SIDES = [...OUTER_STROKE_SIDES, 'middle'] as const;

export type OuterStrokeSide = (typeof OUTER_STROKE_SIDES)[number];
export type StrokeSide = (typeof STROKE_SIDES)[number];

function isStrokeSide(value: unknown): value is StrokeSide {
  return typeof value === 'string' && (STROKE_SIDES as readonly string[]).includes(value);
}

/** Resolve a props value to the permanent contract. An absent/non-array value
 * is legacy all-outer; an explicit empty array means no border placements. */
export function strokeSides(props: Record<string, unknown>): StrokeSide[] {
  if (!Array.isArray(props.strokeSides)) return [...OUTER_STROKE_SIDES];
  const selected = new Set(props.strokeSides.filter(isStrokeSide));
  return STROKE_SIDES.filter((side) => selected.has(side));
}

export function hasAllOuterStrokeSides(sides: readonly StrokeSide[]): boolean {
  return OUTER_STROKE_SIDES.every((side) => sides.includes(side));
}

export function withStrokeSide(
  props: Record<string, unknown>,
  side: StrokeSide,
  enabled: boolean,
): StrokeSide[] {
  const selected = new Set(strokeSides(props));
  if (enabled) selected.add(side);
  else selected.delete(side);
  return STROKE_SIDES.filter((candidate) => selected.has(candidate));
}

/** Toggle the All control without ever changing the independent portal Middle. */
export function withAllOuterStrokeSides(
  props: Record<string, unknown>,
  enabled: boolean,
): StrokeSide[] {
  const selected = new Set(strokeSides(props));
  for (const side of OUTER_STROKE_SIDES) {
    if (enabled) selected.add(side);
    else selected.delete(side);
  }
  return STROKE_SIDES.filter((candidate) => selected.has(candidate));
}
