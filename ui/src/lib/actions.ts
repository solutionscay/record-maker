// Editor command layer — delete / group / ungroup implemented ONCE over the
// document store + persist helpers, invoked by every surface (Inspector
// buttons, canvas keyboard, context menu). Post-conditions follow the canvas
// behaviour: after a delete the registered canvas cleanup clears the hover
// outline and forces moveable to re-derive its (now empty) target, so no stale
// selection chrome lingers whichever surface issued the command.

import type { EditorDoc } from './doc.svelte';
import { createObjectGroup, deleteObjectGroup, deleteObjects } from './persist';
import { llog, lerror } from './log';

/** Canvas chrome reset, registered by the interaction layer while it is alive.
 * Module-level (one editor per page): the Inspector island has no reference to
 * the interaction layer, so the command layer carries the callback across. */
let canvasCleanup: (() => void) | null = null;

/** Register the canvas's post-delete chrome reset. Returns an unregister fn for
 * the interaction layer's teardown. */
export function registerCanvasCleanup(fn: () => void): () => void {
  canvasCleanup = fn;
  return () => {
    if (canvasCleanup === fn) canvasCleanup = null;
  };
}

/** True while a delete is in flight, so repeat keys / double clicks don't fan
 * out. Shared across surfaces — the point of a single command layer. */
let deleting = false;
/** True while group/ungroup persistence is in flight, so menu repeats don't race. */
let grouping = false;

export function isDeleting(): boolean {
  return deleting;
}

/** Delete the current selection: one bulk POST (transactional server-side),
 * then remove from the store as ONE undo step and reset the canvas chrome. */
export async function deleteSelected(doc: EditorDoc, layoutId: string): Promise<void> {
  const ids = [...doc.selection];
  if (ids.length === 0 || deleting) return;
  deleting = true;
  llog('persist', 'delete selected object(s)', { ids });
  try {
    await deleteObjects(layoutId, ids);
    for (const id of ids) doc.removeObject(id);
    doc.mark();
    canvasCleanup?.();
  } catch (e) {
    lerror('persist', 'failed to delete selected object(s)', e);
    doc.setError(e instanceof Error ? e.message : String(e));
  } finally {
    deleting = false;
  }
}

/** ≥2 objects selected and they don't already form exactly one group. */
export function canGroupSelection(doc: EditorDoc): boolean {
  return doc.selection.size >= 2 && doc.groupIdForSelection() === null;
}

/** The selection is exactly one durable group. */
export function canUngroupSelection(doc: EditorDoc): boolean {
  return doc.groupIdForSelection() !== null;
}

/** Group the current selection into a durable object group (#75). */
export async function groupSelected(doc: EditorDoc, layoutId: string): Promise<void> {
  if (!canGroupSelection(doc) || grouping) return;
  const ids = [...doc.selection];
  grouping = true;
  llog('persist', 'group objects', { ids });
  try {
    const group = await createObjectGroup(layoutId, ids);
    doc.setGroup(group);
  } catch (e) {
    lerror('persist', 'failed to group selected object(s)', e);
    doc.setError(e instanceof Error ? e.message : String(e));
  } finally {
    grouping = false;
  }
}

/** Dissolve the group the current selection forms. */
export async function ungroupSelected(doc: EditorDoc, layoutId: string): Promise<void> {
  const groupId = doc.groupIdForSelection();
  if (groupId === null || grouping) return;
  grouping = true;
  llog('persist', 'ungroup objects', { groupId });
  try {
    await deleteObjectGroup(layoutId, groupId);
    doc.removeGroup(groupId);
  } catch (e) {
    lerror('persist', 'failed to ungroup selected object(s)', e);
    doc.setError(e instanceof Error ? e.message : String(e));
  } finally {
    grouping = false;
  }
}
