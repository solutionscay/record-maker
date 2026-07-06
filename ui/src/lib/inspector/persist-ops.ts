// Shared Inspector write paths: optimistic doc-store edit + undo mark + persist,
// with the server-derived style refresh and the store error banner on failure.
// Every section component routes its writes through here so the optimistic/
// undoable/persist choreography is defined once.

import type { EditorDoc } from '../doc.svelte';
import {
  setObjectGeometry as persistGeometry,
  setObjectProps as persistProps,
  setObjectsZ as persistObjectsZ,
} from '../persist';
import { llog, lerror } from '../log';
import { parseProps } from '../object-props';
import type { Geom } from '../arrange';

export function reportPersistError(doc: EditorDoc, label: string, e: unknown): void {
  lerror('persist', `${label} failed`, e);
  doc.setError(e instanceof Error ? e.message : String(e));
}

/** Persist one object's props and refresh its server-derived styles. Does NOT
 * touch the doc's props or undo history — callers apply the optimistic edit and
 * mark() first. */
export async function persistObjectPropsAndRefresh(
  doc: EditorDoc,
  layoutId: string,
  id: number,
  props: Record<string, unknown>,
  label: string,
): Promise<void> {
  try {
    const styles = await persistProps(layoutId, id, props);
    doc.setObjectStyles(id, styles);
  } catch (e) {
    reportPersistError(doc, label, e);
  }
}

/** Merge one full props bag into an object as one undo step, then persist (the
 * single-object style/format commit). */
export async function writeObjectProps(
  doc: EditorDoc,
  layoutId: string,
  id: number,
  props: Record<string, unknown>,
  label: string,
): Promise<void> {
  doc.setObjectProps(id, JSON.stringify(props));
  doc.mark();
  await persistObjectPropsAndRefresh(doc, layoutId, id, props, label);
}

/** Write one style/text key to EVERY object in `ids` as a SINGLE undo step (#82).
 * Each object's own props bag is merged optimistically (unchanged objects produce
 * no diff), the whole batch is sealed with one `doc.mark()`, then persisted in
 * parallel so each object's server-derived style refreshes. Callers gate the
 * control by shared capability, so the key already applies to all objects. */
export async function writeStyleMany(
  doc: EditorDoc,
  layoutId: string,
  ids: readonly number[],
  key: string,
  value: string | number | boolean,
): Promise<void> {
  if (ids.length === 0) return;
  llog('persist', 'inspector: set style (many)', { ids, key, value });
  // Optimistic + undoable: apply to each object, accumulating into the open group.
  const nexts = new Map<number, Record<string, unknown>>();
  for (const id of ids) {
    const o = doc.getObject(id);
    if (!o) continue;
    const next = { ...parseProps(o.props), [key]: value };
    nexts.set(id, next);
    doc.setObjectProps(id, JSON.stringify(next));
  }
  doc.mark(); // one atomic undo step for the whole batch
  try {
    await Promise.all(
      [...nexts].map(async ([id, next]) => {
        const styles = await persistProps(layoutId, id, next);
        doc.setObjectStyles(id, styles);
      }),
    );
  } catch (e) {
    reportPersistError(doc, 'set style (many)', e);
  }
}

/** Apply a batch of absolute geometries as one undo step, then persist each in
 * parallel (per-object endpoint, as #83 specifies). No-op writes are skipped by
 * the store's diff, and only changed objects are passed in by the callers. */
export async function applyGeometryMany(doc: EditorDoc, layoutId: string, geoms: Map<number, Geom>): Promise<void> {
  if (geoms.size === 0) return;
  llog('persist', 'inspector: arrange geometry', { ids: [...geoms.keys()] });
  for (const [id, g] of geoms) doc.setObjectGeometry(id, g);
  doc.mark(); // one atomic undo step for the whole align/distribute action
  try {
    await Promise.all([...geoms].map(([id, g]) => persistGeometry(layoutId, id, g)));
  } catch (e) {
    reportPersistError(doc, 'arrange geometry', e);
  }
}

/** Apply changed `(id, z)` pairs as ONE undo step, then persist the batch. */
export async function applyZChanges(doc: EditorDoc, layoutId: string, changed: [number, number][]): Promise<void> {
  if (changed.length === 0) return;
  for (const [id, z] of changed) doc.setProp(id, 'z', z);
  doc.mark(); // one atomic undo step for the whole restack
  try {
    await persistObjectsZ(layoutId, changed.map(([id, z]) => ({ id, z })));
  } catch (e) {
    reportPersistError(doc, 'z-order', e);
  }
}
