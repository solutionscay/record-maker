// Thin fetch wrappers over the #107 schema-management endpoints, mirroring the
// Layout editor's persist.ts. The store is the source of truth for what's on
// screen; these only talk to the server and return the views it assigns, so the
// store can reflect server truth after every op (#113 acceptance).
//
// The fetch/throw mechanics live in the shared HTTP helper (#132): every failed
// op throws its typed HttpError, which carries the server's status + message so
// the store can surface a real reason (the endpoints return CONFLICT/BAD_REQUEST
// with a human-readable string, e.g. a duplicate-name conflict).

import { getJson, postJson, postVoid as httpPostVoid } from '../shared/http';
import type { FieldKind, FieldOptions, FieldView, RelationshipInput, RelationshipView, TableView, ValueListView } from './types';

/** POST that returns no JSON body (the delete endpoints just 200/OK). */
const postVoid = (url: string, body: unknown = {}): Promise<void> => httpPostVoid(url, body);

// ── tables ──────────────────────────────────────────────────────────────────

export const listTables = (): Promise<TableView[]> => getJson('/schema/tables');

export const createTable = (name: string): Promise<TableView> =>
  postJson('/schema/tables', { name });

export const createTableWithNotes = (name: string, notes: string): Promise<TableView> =>
  postJson('/schema/tables', { name, notes });

export const updateTable = (id: number, name: string, notes: string): Promise<TableView> =>
  postJson(`/schema/tables/${id}`, { name, notes });

export const renameTable = (id: number, name: string): Promise<TableView> =>
  postJson(`/schema/tables/${id}/rename`, { name });

export const deleteTable = (id: number): Promise<void> => postVoid(`/schema/tables/${id}/delete`);

export const reorderTables = (tableIds: number[]): Promise<TableView[]> =>
  postJson('/schema/tables/order', { tableIds });

/** Persist a table box's Relationships-graph position (view-state, saved on drag-stop). */
export const setTableGraphPosition = (id: number, x: number, y: number): Promise<void> =>
  postVoid(`/schema/tables/${id}/graph`, { x, y });

// ── fields ──────────────────────────────────────────────────────────────────

export const listFields = (tableId: number): Promise<FieldView[]> =>
  getJson(`/schema/tables/${tableId}/fields`);

export const createField = (tableId: number, name: string, kind: FieldKind): Promise<FieldView> =>
  postJson(`/schema/tables/${tableId}/fields`, { name, kind });

export const createFieldWithNotes = (tableId: number, name: string, kind: FieldKind, notes: string): Promise<FieldView> =>
  postJson(`/schema/tables/${tableId}/fields`, { name, kind, notes });

export const createFieldWithDetails = (
  tableId: number,
  name: string,
  kind: FieldKind,
  notes: string,
  options: FieldOptions,
): Promise<FieldView> => postJson(`/schema/tables/${tableId}/fields`, { name, kind, notes, options });

export const updateField = (
  tableId: number,
  fieldId: number,
  name: string,
  kind: FieldKind,
  notes: string,
  options: FieldOptions,
): Promise<FieldView> => postJson(`/schema/tables/${tableId}/fields/${fieldId}`, { name, kind, notes, options });

export const reorderFields = (tableId: number, fieldIds: number[]): Promise<FieldView[]> =>
  postJson(`/schema/tables/${tableId}/fields/order`, { fieldIds });

export const deleteField = (tableId: number, fieldId: number): Promise<void> =>
  postVoid(`/schema/tables/${tableId}/fields/${fieldId}/delete`);

// ── relationships ───────────────────────────────────────────────────────────

export const listRelationships = (): Promise<RelationshipView[]> => getJson('/schema/relationships');

export const createRelationship = (rel: RelationshipInput): Promise<RelationshipView> =>
  postJson('/schema/relationships', rel);

export const updateRelationship = (id: number, rel: RelationshipInput): Promise<RelationshipView> =>
  postJson(`/schema/relationships/${id}`, rel);

export const deleteRelationship = (id: number): Promise<void> => postVoid(`/schema/relationships/${id}/delete`);

/** Set a relationship's portal create/delete permission flags (#110/#174).
 * Independent of the structural create/update route, so a referential edit never
 * touches the FK structure and vice versa. Returns the server's updated view. */
export const setRelationshipReferential = (
  id: number,
  allowCreate: boolean,
  allowDelete: boolean,
): Promise<RelationshipView> =>
  postJson(`/schema/relationships/${id}/referential`, { allowCreate, allowDelete });

// ── value lists ─────────────────────────────────────────────────────────────

export const listValueLists = (): Promise<ValueListView[]> => getJson('/value-lists');
