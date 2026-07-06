// Value coercers shared by the Inspector sections: each control reads its props
// key through one of these, and `sharedValue` resolves one attribute across a
// whole selection into a `{ mixed, value }` pair (#82). Kept as a plain module so
// the Style/Text/Format sections share exactly one definition.

import type { ObjectDoc } from '../doc.svelte';
import { parseProps } from '../object-props';

export function colorValue(v: unknown, fallback: string): string {
  return typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v) ? v : fallback;
}

export function numberValue(v: unknown, fallback: number): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
}

export function boolValue(v: unknown): boolean {
  return v === true;
}

export function alignValue(v: unknown): string {
  return typeof v === 'string' && ['left', 'center', 'right'].includes(v) ? v : 'left';
}

/** Resolve one attribute across the whole selection into `{ mixed, value }`:
 * `mixed` is true when the selected objects disagree, and `value` is the first
 * object's resolved value (the control's shown value when not mixed). Reads each
 * object's props bag through the same coercers as the single-object controls. */
export function sharedValue<T>(
  objects: readonly Readonly<ObjectDoc>[],
  resolve: (props: Record<string, unknown>) => T,
): { mixed: boolean; value: T } {
  const vals = objects.map((o) => resolve(parseProps(o.props)));
  const mixed = vals.some((v) => v !== vals[0]);
  return { mixed, value: vals[0] };
}
