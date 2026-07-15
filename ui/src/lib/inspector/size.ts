// Shared dimension logic for the Inspector's Size section (#190). Kept outside
// the Svelte component so the single- and multi-selection behavior is one small,
// directly testable contract.

import type { EditorDoc, ObjectDoc } from '../doc.svelte';
import { applyLiveObjectGeometry } from './geometry-commit';

export type Dimension = 'w' | 'h';

export function dimensionPixels(raw: string): number | null {
  if (raw.trim() === '') return null;
  const value = Number(raw);
  return Number.isFinite(value) && value > 0 ? Math.max(1, Math.round(value)) : null;
}

/** Resolve one dimension across the current selection. A null value is reserved
 * for the defensive empty-selection case; SizeSection is only mounted when at
 * least one object is selected. */
export function sharedDimension(
  objects: readonly Readonly<ObjectDoc>[],
  dimension: Dimension,
): { mixed: boolean; value: number | null } {
  const value = objects[0]?.[dimension] ?? null;
  return {
    mixed: value !== null && objects.some((object) => object[dimension] !== value),
    value,
  };
}

/** Apply a valid dimension to every still-existing selected object. Lines flow
 * through the same live helper as single-object edits, so their derived stroke
 * geometry stays synchronized with the box. */
export function applyLiveDimension(
  doc: EditorDoc,
  ids: readonly number[],
  dimension: Dimension,
  value: number,
): void {
  for (const id of ids) applyLiveObjectGeometry(doc, id, { [dimension]: value });
}
