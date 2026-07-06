// Thin fetch wrappers over the #107 schema-management endpoints, mirroring the
// Layout editor's persist.ts. The store is the source of truth for what's on
// screen; these only talk to the server and return the views it assigns, so the
// store can reflect server truth after every op (#113 acceptance).

import type { FieldKind, FieldOptions, FieldView, RelationshipView, TableView } from './types';

/** A failed schema op — carries the server's status + message body so the store
 * can surface a real reason (the endpoints return CONFLICT/BAD_REQUEST with a
 * human-readable string, e.g. a duplicate-name conflict). */
export class SchemaError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message || `HTTP ${status}`);
    this.name = 'SchemaError';
    this.status = status;
  }
}

async function getJson<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new SchemaError(r.status, await r.text().catch(() => ''));
  return (await r.json()) as T;
}

async function postJson<T>(url: string, body?: unknown): Promise<T> {
  const r = await fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body ?? {}),
  });
  if (!r.ok) throw new SchemaError(r.status, await r.text().catch(() => ''));
  return (await r.json()) as T;
}

/** POST that returns no JSON body (the delete endpoints just 200/OK). */
async function postVoid(url: string): Promise<void> {
  const r = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' });
  if (!r.ok) throw new SchemaError(r.status, await r.text().catch(() => ''));
}

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

export const renameField = (tableId: number, fieldId: number, name: string): Promise<FieldView> =>
  postJson(`/schema/tables/${tableId}/fields/${fieldId}/rename`, { name });

export const retypeField = (tableId: number, fieldId: number, kind: FieldKind): Promise<FieldView> =>
  postJson(`/schema/tables/${tableId}/fields/${fieldId}/retype`, { kind });

export const reorderFields = (tableId: number, fieldIds: number[]): Promise<FieldView[]> =>
  postJson(`/schema/tables/${tableId}/fields/order`, { fieldIds });

export const deleteField = (tableId: number, fieldId: number): Promise<void> =>
  postVoid(`/schema/tables/${tableId}/fields/${fieldId}/delete`);

// ── relationships ───────────────────────────────────────────────────────────

export const listRelationships = (): Promise<RelationshipView[]> => getJson('/schema/relationships');

export const createRelationship = (rel: Omit<RelationshipView, 'id'>): Promise<RelationshipView> =>
  postJson('/schema/relationships', rel);

export const updateRelationship = (id: number, rel: Omit<RelationshipView, 'id'>): Promise<RelationshipView> =>
  postJson(`/schema/relationships/${id}`, rel);

export const deleteRelationship = (id: number): Promise<void> => postVoid(`/schema/relationships/${id}/delete`);
