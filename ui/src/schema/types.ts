// Shared types for the schema-builder island (#113). These mirror the #107
// `/schema/*` JSON contract exactly (server structs `TableSchemaView` /
// `FieldSchemaView`, camelCase serde). Keep field names in sync with the server.

/** The six logical field kinds the engine understands (`FieldKind`, model.rs).
 * The schema builder never exposes physical/SQL types — only these. */
export type FieldKind = 'text' | 'number' | 'date' | 'time' | 'timestamp' | 'bool';

/** Ordered kind choices for the type picker: `kind` is what we POST, `label` is
 * the human name, `icon` names the shared sprite symbol (`#icon-type-<kind>`). */
export const FIELD_KINDS: { kind: FieldKind; label: string; icon: string }[] = [
  { kind: 'text', label: 'Text', icon: 'type-text' },
  { kind: 'number', label: 'Number', icon: 'type-number' },
  { kind: 'date', label: 'Date', icon: 'type-date' },
  { kind: 'time', label: 'Time', icon: 'type-time' },
  { kind: 'timestamp', label: 'Timestamp', icon: 'type-timestamp' },
  { kind: 'bool', label: 'Boolean', icon: 'type-bool' },
];

/** Human label for a kind string (falls back to the raw string if unknown). */
export function kindLabel(kind: string): string {
  return FIELD_KINDS.find((k) => k.kind === kind)?.label ?? kind;
}

/** Sprite symbol name for a kind string (falls back to the generic field icon). */
export function kindIcon(kind: string): string {
  return FIELD_KINDS.find((k) => k.kind === kind)?.icon ?? 'field';
}

/** A user table — mirrors the server's `TableSchemaView`. `phys` is the physical
 * table name in data.db (derived from `name`; read-only in the UI for now). */
export interface TableView {
  id: number;
  name: string;
  notes: string;
  phys: string;
}

/** A field on a table — mirrors the server's `FieldSchemaView`. `position` is the
 * server-authoritative order; `phys` is the derived physical column name. */
export interface FieldView {
  id: number;
  name: string;
  notes: string;
  phys: string;
  kind: FieldKind;
  position: number;
}

/** A named relationship between two table fields. Mirrors `RelationshipSchemaView`. */
export interface RelationshipView {
  id: number;
  name: string;
  fromTable: number;
  toTable: number;
  fromField: number;
  toField: number;
}
