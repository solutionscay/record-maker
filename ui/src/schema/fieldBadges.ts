// Shared per-field badge computation (#140), used by both the relationship
// graph (SchemaTableNode via RelationshipsView) and the Fields list
// (FieldGrid/FieldRow), so the two views can't drift out of sync on what
// counts as a primary/required/unique/key/fk field.
import type { SchemaStore } from './store.svelte';
import type { FieldView } from './types';

export interface FieldBadgeInfo {
  primary: boolean;
  required: boolean;
  unique: boolean;
  /** Names of relationships where this field is the FK (references another field). */
  fkNames: string[];
  /** Names of relationships where this field is referenced by another field. */
  keyNames: string[];
}

function tableName(store: SchemaStore, id: number): string {
  return store.tableById(id)?.name ?? 'Missing table';
}

function fieldName(store: SchemaStore, tableId: number, fieldId: number): string {
  return store.fieldById(tableId, fieldId)?.name ?? 'Missing field';
}

export function fieldBadgeInfo(store: SchemaStore, tableId: number, field: FieldView): FieldBadgeInfo {
  const from = store.relationships.filter((r) => r.fromTable === tableId && r.fromField === field.id);
  const to = store.relationships.filter((r) => r.toTable === tableId && r.toField === field.id);
  return {
    primary: field.options?.validation?.primary ?? false,
    required: field.options?.validation?.required ?? false,
    unique: field.options?.validation?.unique ?? false,
    fkNames: from.map((r) => `${r.name} -> ${tableName(store, r.toTable)}.${fieldName(store, r.toTable, r.toField)}`),
    keyNames: to.map((r) => `${tableName(store, r.fromTable)}.${fieldName(store, r.fromTable, r.fromField)} -> ${r.name}`),
  };
}
