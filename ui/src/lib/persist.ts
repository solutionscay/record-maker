// Persistence wrappers for the Layout editor's Create/Style zones (#62/#48/#49).
// Thin fetch helpers over the design endpoints (ADR #42). The store is the source
// of truth; these only SYNC server state and return what the server assigns —
// new object ids (so the store can add the object undoably) and the server-derived
// shape style (the single source of that derivation, [[layout-object-types]]).
//
// The fetch/throw mechanics live in the shared HTTP helper (#132); this module
// wires in the editor's `llog` channel and keeps the endpoint vocabulary.

import type { ObjectGroupView, ObjectView, PartView } from './model';
import { postJson as httpPostJson, postVoid as httpPostVoid, type HttpLog } from '../shared/http';
import { llog } from './log';

export interface StyleResult {
  objectStyle: string;
  textStyle: string;
  shapeStyle: string;
}

/** The object the Create zone places (#48). For a `field`, `fieldId` names which
 * field to bind (the server builds the binding + spawns the caption label). `rec`
 * is the record the canvas shows, so the returned object resolves its live value. */
export interface NewObjectRequest {
  partId: number;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  rec?: number;
  fieldId?: number | null;
  createLabel?: boolean;
  content?: string | null;
  props?: Record<string, unknown> | null;
  /** The source object's binding, sent by a value-only field copy (duplicate/
   * paste) so the server can recreate it even when `fieldId` is null — a field
   * whose binding doesn't resolve (empty table, unresolved relationship path)
   * renders with `fieldId: null`, and the binding is what actually defines it. */
  binding?: string | null;
  /** Owning portal for a placed column (#168/#169, Model B). When set, the server
   * creates the object as a CHILD of that portal (self-FK) and — for a `field` —
   * binds it ROUTE-RELATIVE to the portal's related table (`<route>.<field>`)
   * instead of the primary table. Absent/null for ordinary top-level placement. */
  parentObjectId?: number | null;
}

// The editor's logging hook for the shared helper: JSON posts log their body,
// success payload, and failures on the persist/error channels (as before).
const log: HttpLog = {
  start: (url, body) => llog('persist', `POST ${url}`, { body }),
  ok: (url, response) => llog('persist', `POST ${url} ✓`, { response }),
  fail: (url, status) => llog('error', `POST ${url} → HTTP ${status}`),
};

function postJson<T>(url: string, body: unknown): Promise<T> {
  return httpPostJson(url, body, log);
}

/** Create an object; returns the created view(s) — a field returns its value
 * object AND its spawned caption label (#60), other kinds return one. */
export function createObject(layoutId: string, req: NewObjectRequest): Promise<ObjectView[]> {
  return postJson(`/design/${layoutId}/object`, req);
}

/** Append a band; returns its (object-less) part view plus the layout's
 * post-insert `[{id, position}]` ordering. The server places summary bands
 * between the body and footer and shifts trailing parts down, so the store must
 * resync positions from `positions` — otherwise the new band renders below the
 * footer. */
export function createPart(
  layoutId: string,
  kind: string,
  height: number,
): Promise<{ part: PartView; positions: { id: number; position: number }[] }> {
  return postJson(`/design/${layoutId}/part`, { kind, height });
}

/** Persist a band's height and return the updated band view. */
export function setPartHeight(layoutId: string, id: number, height: number): Promise<PartView> {
  return postJson(`/design/${layoutId}/part/${id}/height`, { height });
}

/** Persist a band's kind and return the updated band view. */
export function setPartKind(layoutId: string, id: number, kind: string): Promise<PartView> {
  return postJson(`/design/${layoutId}/part/${id}/kind`, { kind });
}

/** Commit a band's appearance bag (#49/Issue 7) and return the updated band view
 * — its server-derived `partStyle` refreshes the canvas without a client-side
 * re-derivation (mirrors `setObjectProps`). */
export function setPartProps(
  layoutId: string,
  id: number,
  props: Record<string, unknown>,
): Promise<PartView> {
  return postJson<PartView>(`/design/${layoutId}/part/${id}/props`, { props });
}

/** Move a summary band up/down (Issue 4); returns the layout's parts as
 * `[{id, position}]` after the move so the store can resync positions. */
export function movePart(
  layoutId: string,
  id: number,
  up: boolean,
): Promise<{ id: number; position: number }[]> {
  return postJson(`/design/${layoutId}/part/${id}/move`, { up });
}

/** Delete a band. Objects in the band are deleted with it. */
export function deletePart(layoutId: string, id: number): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/part/${id}/delete`);
}

/** Reparent an object to another band (cross-band drag, #46): persist its new
 * owning part + part-relative origin. No body on success (200). */
export function setObjectPart(
  layoutId: string,
  id: number,
  partId: number,
  x: number,
  y: number,
): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/object/${id}/part`, { partId, x, y });
}

/** Persist one object's geometry. */
export function setObjectGeometry(
  layoutId: string,
  id: number,
  geom: { x: number; y: number; w: number; h: number },
): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/object/${id}/geometry`, geom);
}

/** Batch-persist objects' stacking order (#83 Arrange z-order). The panel
 * re-densifies a part's `z` after a Bring-to-Front / Send-to-Back / step command
 * and POSTs `[{id,z}, …]`; the server writes them in one transaction, scoped to
 * the layout. No body needed on success (the server returns a count, which the
 * store doesn't consume — z reaches the DOM straight from the document `z`). */
export function setObjectsZ(
  layoutId: string,
  items: { id: number; z: number }[],
): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/z`, items);
}

/** Batch-persist objects' geometry after a drag/resize/align settles. The store
 * already holds the new rects; this only SYNCs them to the server (siblings that
 * crossed a band boundary go through `setObjectPart` instead). */
export function setObjectsGeometry(
  layoutId: string,
  items: { id: number; x: number; y: number; w: number; h: number }[],
): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/geometry`, items);
}

/** Create a durable group over existing objects (#75). History replay supplies
 * the original id so redo can restore the same group identity. */
export function createObjectGroup(layoutId: string, objectIds: number[], id?: number): Promise<ObjectGroupView> {
  return postJson(`/design/${layoutId}/group`, id === undefined ? { objectIds } : { id, objectIds });
}

/** Remove a durable group without changing child geometry/styles (#75). */
export function deleteObjectGroup(layoutId: string, id: number): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/group/${id}/delete`);
}

/** Delete an object (the undo of a create / the Create-zone delete). */
export function deleteObject(layoutId: string, id: number): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/object/${id}/delete`);
}

/** Batch-delete objects in one transaction (multi-delete / cut) — the bulk
 * sibling of `deleteObject`, mirroring `setObjectsGeometry`: the server scopes
 * each id to the layout and skips unknown ids, returning a count the store
 * doesn't consume. */
export function deleteObjects(layoutId: string, ids: number[]): Promise<void> {
  return httpPostVoid(`/design/${layoutId}/objects/delete`, ids);
}

/** Commit a props bag (#49); returns the server-derived shape style for the
 * canvas (empty for a non-shape object). */
export function setObjectProps(
  layoutId: string,
  id: number,
  props: Record<string, unknown>,
): Promise<StyleResult> {
  return postJson<StyleResult>(`/design/${layoutId}/object/${id}/props`, { props });
}

/** Rebind a field object and return the updated server projection. */
export function setObjectBinding(
  layoutId: string,
  id: number,
  fieldId: number,
  rec: number,
): Promise<ObjectView> {
  return postJson(`/design/${layoutId}/object/${id}/binding`, { fieldId, rec });
}

/** Update a text object's static content and return the updated view. */
export function setObjectContent(layoutId: string, id: number, content: string): Promise<ObjectView> {
  return postJson(`/design/${layoutId}/object/${id}/content`, { content });
}

/** Toggle whether the object is editable in Browse mode and return the updated view. */
export function setObjectReadOnly(
  layoutId: string,
  id: number,
  readOnly: boolean,
  rec: number,
): Promise<ObjectView> {
  return postJson(`/design/${layoutId}/object/${id}/read-only`, { readOnly, rec });
}

/** An object row being restored at its ORIGINAL id (#84). Field-for-field the
 *  store's ObjectDoc plus the record index to project the returned view against. */
export interface RestoreObjectRequest {
  id: number;
  partId: number;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
  readOnly: boolean;
  binding: string;
  content: string;
  props: string;
  /** Owning portal (#168/#169), so restoring a deleted portal column at its
   * original id re-links it to its portal. Absent/null for top-level objects. */
  parentObjectId?: number | null;
}

/** Restore one or more deleted objects at their EXACT original ids, atomically
 *  (undo-of-delete / redo-of-create). Returns each restored object's server view. */
export function restoreObjects(
  layoutId: string,
  objects: RestoreObjectRequest[],
  rec: number,
): Promise<ObjectView[]> {
  return postJson(`/design/${layoutId}/object/restore`, { objects, rec });
}

/** Write an object's binding dot-path VERBATIM (history replay of a binding diff).
 *  Distinct from setObjectBinding, which is keyed by fieldId for live field-picking. */
export function setObjectBindingPath(
  layoutId: string,
  id: number,
  binding: string,
  rec: number,
  validatePortal = false,
): Promise<ObjectView> {
  return postJson(`/design/${layoutId}/object/${id}/binding-path`, {
    binding,
    rec,
    validatePortal,
  });
}
