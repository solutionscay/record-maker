// Shared types for the schema-builder island (#113). These mirror the #107
// `/schema/*` JSON contract exactly (server structs `TableSchemaView` /
// `FieldSchemaView`, camelCase serde). Keep field names in sync with the server.

// The field-kind vocabulary (FieldKind, FIELD_KINDS, kindLabel, kindIcon) moved
// to the shared module both sub-apps use (#132); re-exported here so schema
// components keep importing it from './types'.
import type { FieldKind } from '../shared/field-kinds';

export { FIELD_KINDS, kindIcon, kindLabel } from '../shared/field-kinds';
export type { FieldKind } from '../shared/field-kinds';

export interface FieldValidationOptions {
  primary?: boolean;
  required?: boolean;
  unique?: boolean;
  memberOfValueList?: number | null;
  range?: {
    min?: string;
    max?: string;
  };
}

/** Auto-enter value source (#159/#160). Only the constant source exists today;
 * the engine fills `value` on record create when the field is left empty. */
export interface FieldAutoEnterOptions {
  kind: 'constant';
  value: string;
}

export interface FieldOptions {
  system?: boolean;
  validation?: FieldValidationOptions;
  /** A value the engine populates on create when the field is left empty. */
  autoEnter?: FieldAutoEnterOptions;
  /** FK/reference constraint. Establishes a relationship edge from this field. */
  reference?: {
    name: string;
    toTable: number;
    toField: number;
  };
}

export function emptyFieldOptions(): FieldOptions {
  return {};
}

/** A user table — mirrors the server's `TableSchemaView`. `phys` is the physical
 * table name in data.db (derived from `name`; read-only in the UI for now). */
export interface TableView {
  id: number;
  name: string;
  notes: string;
  phys: string;
  position: number;
  /** Saved Relationships-graph box position; null until the box is first dragged. */
  graphX?: number | null;
  graphY?: number | null;
}

/** A field on a table — mirrors the server's `FieldSchemaView`. `position` is the
 * server-authoritative order; `phys` is the derived physical column name. */
export interface FieldView {
  id: number;
  name: string;
  notes: string;
  phys: string;
  kind: FieldKind;
  options: FieldOptions;
  position: number;
}

/** Traversal cardinality of one direction of a relationship. */
export type Cardinality = 'one' | 'many';

/** A named relationship between two table fields. Mirrors `RelationshipSchemaView`. */
export interface RelationshipView {
  id: number;
  name: string;
  fromTable: number;
  toTable: number;
  fromField: number;
  toField: number;
  /** Portal CRUD permissions (#110/#174): whether the portal may create/delete
   * records through this relationship. Toggled on the graph connector drawer;
   * persisted independently of the schema draft (like graph box positions). */
  allowCreate: boolean;
  allowDelete: boolean;
  /** Derived traversal cardinality: forward = from-table → to-table (FK owner →
   * parent, always to-one); reverse = to-table → from-table (always to-many). */
  forwardCardinality: Cardinality;
  reverseCardinality: Cardinality;
}

/** The structural fields of a relationship, as sent to the create/update
 * endpoints. Excludes the id and the referential/cardinality fields, which the
 * server derives or owns via the dedicated `/referential` route (#174). */
export type RelationshipInput = Pick<
  RelationshipView,
  'name' | 'fromTable' | 'toTable' | 'fromField' | 'toField'
>;

export interface ValueListView {
  id: number;
  name: string;
  source: 'custom' | 'field';
  config: unknown;
  position: number;
}
