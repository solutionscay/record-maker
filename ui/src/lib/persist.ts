// Persistence wrappers for the Layout editor's Create/Style zones (#62/#48/#49).
// Thin fetch helpers over the design endpoints (ADR #42). The store is the source
// of truth; these only SYNC server state and return what the server assigns —
// new object ids (so the store can add the object undoably) and the server-derived
// shape style (the single source of that derivation, [[layout-object-types]]).

import type { ObjectView, PartView } from './model';

export interface StyleResult {
  objectStyle: string;
  textStyle: string;
  shapeStyle: string;
}
import { llog } from './log';

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
}

async function postJson<T>(url: string, body: unknown): Promise<T> {
  llog('persist', `POST ${url}`, { body });
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!r.ok) {
    llog('error', `POST ${url} → HTTP ${r.status}`);
    throw new Error(`HTTP ${r.status}`);
  }
  const json = (await r.json()) as T;
  llog('persist', `POST ${url} ✓`, { response: json });
  return json;
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
export async function deletePart(layoutId: string, id: number): Promise<void> {
  const r = await fetch(`/design/${layoutId}/part/${id}/delete`, { method: 'POST' });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
}

/** Reparent an object to another band (cross-band drag, #46): persist its new
 * owning part + part-relative origin. No body on success (200). */
export async function setObjectPart(
  layoutId: string,
  id: number,
  partId: number,
  x: number,
  y: number,
): Promise<void> {
  const r = await fetch(`/design/${layoutId}/object/${id}/part`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ partId, x, y }),
  });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
}

/** Persist one object's geometry. */
export async function setObjectGeometry(
  layoutId: string,
  id: number,
  geom: { x: number; y: number; w: number; h: number },
): Promise<void> {
  const r = await fetch(`/design/${layoutId}/object/${id}/geometry`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(geom),
  });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
}

/** Batch-persist objects' stacking order (#83 Arrange z-order). The panel
 * re-densifies a part's `z` after a Bring-to-Front / Send-to-Back / step command
 * and POSTs `[{id,z}, …]`; the server writes them in one transaction, scoped to
 * the layout. No body needed on success (the server returns a count, which the
 * store doesn't consume — z reaches the DOM straight from the document `z`). */
export async function setObjectsZ(
  layoutId: string,
  items: { id: number; z: number }[],
): Promise<void> {
  const r = await fetch(`/design/${layoutId}/z`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(items),
  });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
}

/** Delete an object (the undo of a create / the Create-zone delete). */
export async function deleteObject(layoutId: string, id: number): Promise<void> {
  const r = await fetch(`/design/${layoutId}/object/${id}/delete`, { method: 'POST' });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
}

/** Commit a props bag (#49); returns the server-derived shape style for the
 * canvas (empty for a non-shape object). */
export async function setObjectProps(
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
): Promise<ObjectView> {
  return postJson(`/design/${layoutId}/object/${id}/binding-path`, { binding, rec });
}
