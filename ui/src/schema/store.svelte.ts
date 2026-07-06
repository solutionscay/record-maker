// The schema-builder store (#113/#119). It keeps a full editable draft of the
// schema, separate from the last server-loaded baseline. Drawer saves only touch
// the draft; the schema-window Save applies the draft through the #107 API.

import * as api from './persist';
import { SchemaError } from './persist';
import type { FieldKind, FieldView, RelationshipView, TableView } from './types';

type FieldMap = Record<number, FieldView[]>;

const cloneTable = (t: TableView): TableView => ({ ...t });
const cloneField = (f: FieldView): FieldView => ({ ...f });
const cloneRelationship = (r: RelationshipView): RelationshipView => ({ ...r });
const cloneFields = (fields: FieldMap): FieldMap =>
  Object.fromEntries(Object.entries(fields).map(([id, fs]) => [id, fs.map(cloneField)]));

function normName(s: string): string {
  return s.trim().toLocaleLowerCase();
}

function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export class SchemaStore {
  /** Draft tables, including unsaved negative-id tables. */
  tables = $state<TableView[]>([]);
  /** The table whose fields the grid shows; null before load / when none exist. */
  selectedTableId = $state<number | null>(null);
  /** Draft fields by table id, including negative ids for unsaved fields. */
  fieldsByTable = $state<FieldMap>({});
  /** Draft relationships. Negative ids are unsaved relationships. */
  relationships = $state<RelationshipView[]>([]);
  /** Existing relationships deleted in the draft. */
  deletedRelationshipIds = $state<number[]>([]);
  /** True during the initial full schema load. */
  loading = $state(true);
  /** True while the top-level schema save is applying the draft. */
  saving = $state(false);
  /** Last error message, shown in a dismissable banner; null when clear. */
  error = $state<string | null>(null);

  private baseTables: TableView[] = [];
  private baseFieldsByTable: FieldMap = {};
  private baseRelationships: RelationshipView[] = [];
  private tempId = -1;

  /** The selected table object (derived), or null. */
  get selectedTable(): TableView | null {
    return this.tables.find((t) => t.id === this.selectedTableId) ?? null;
  }

  /** Fields for the selected table, in draft display order. */
  get fields(): readonly FieldView[] {
    return this.selectedTableId == null ? [] : (this.fieldsByTable[this.selectedTableId] ?? []);
  }

  get hasChanges(): boolean {
    return (
      !sameJson(this.tables, this.baseTables) ||
      !sameJson(this.fieldsByTable, this.baseFieldsByTable) ||
      !sameJson(this.relationships, this.baseRelationships) ||
      this.deletedRelationshipIds.length > 0
    );
  }

  get changeSummary(): string {
    if (!this.hasChanges) return 'No unsaved schema changes';
    const tableDelta = this.tables.filter((t) => !sameJson(t, this.baseTables.find((b) => b.id === t.id))).length;
    const fieldDelta = Object.values(this.fieldsByTable)
      .flat()
      .filter((f) => {
        const base = Object.values(this.baseFieldsByTable)
          .flat()
          .find((b) => b.id === f.id);
        return !sameJson(f, base);
      }).length;
    const relDelta =
      this.relationships.filter((r) => !sameJson(r, this.baseRelationships.find((b) => b.id === r.id))).length +
      this.deletedRelationshipIds.length;
    const parts = [
      tableDelta > 0 ? `${tableDelta} table${tableDelta === 1 ? '' : 's'}` : '',
      fieldDelta > 0 ? `${fieldDelta} field${fieldDelta === 1 ? '' : 's'}` : '',
      relDelta > 0 ? `${relDelta} relationship${relDelta === 1 ? '' : 's'}` : '',
    ].filter(Boolean);
    return `Unsaved: ${parts.join(', ')}`;
  }

  /** Run `op`, routing any failure into `error` so the UI can show it. */
  private async guard<T>(op: () => Promise<T>): Promise<T | null> {
    try {
      const out = await op();
      this.error = null;
      return out;
    } catch (e) {
      this.error = e instanceof SchemaError ? e.message : e instanceof Error ? e.message : String(e);
      return null;
    }
  }

  private nextId(): number {
    return this.tempId--;
  }

  /** Full load: tables, each table's fields, and relationships. */
  async load(): Promise<void> {
    this.loading = true;
    const loaded = await this.guard(async () => {
      const tables = await api.listTables();
      const fieldsByTable: FieldMap = {};
      await Promise.all(
        tables.map(async (table) => {
          fieldsByTable[table.id] = await api.listFields(table.id);
        }),
      );
      const relationships = await api.listRelationships();
      return { tables, fieldsByTable, relationships };
    });
    if (loaded) {
      this.baseTables = loaded.tables.map(cloneTable);
      this.baseFieldsByTable = cloneFields(loaded.fieldsByTable);
      this.baseRelationships = loaded.relationships.map(cloneRelationship);
      this.tables = loaded.tables.map(cloneTable);
      this.fieldsByTable = cloneFields(loaded.fieldsByTable);
      this.relationships = loaded.relationships.map(cloneRelationship);
      this.deletedRelationshipIds = [];
      if (this.selectedTableId != null && !this.tables.some((t) => t.id === this.selectedTableId)) {
        this.selectedTableId = null;
      }
    }
    this.loading = false;
  }

  discardChanges(): void {
    this.tables = this.baseTables.map(cloneTable);
    this.fieldsByTable = cloneFields(this.baseFieldsByTable);
    this.relationships = this.baseRelationships.map(cloneRelationship);
    this.deletedRelationshipIds = [];
    this.error = null;
  }

  selectTable(tableId: number): void {
    this.selectedTableId = tableId;
  }

  // ── table draft mutations ───────────────────────────────────────────────

  saveTableDraft(id: number | null, name: string, notes: string): TableView | null {
    const cleanName = name.trim();
    if (!cleanName) return this.fail('Table name is required.');
    if (this.tables.some((t) => t.id !== id && normName(t.name) === normName(cleanName))) {
      return this.fail(`A table named "${cleanName}" already exists.`);
    }
    if (id == null) {
      const table: TableView = { id: this.nextId(), name: cleanName, notes: notes.trim(), phys: '' };
      this.tables = [...this.tables, table];
      this.fieldsByTable = { ...this.fieldsByTable, [table.id]: [] };
      this.selectedTableId = table.id;
      this.error = null;
      return table;
    }
    let updated: TableView | null = null;
    this.tables = this.tables.map((t) => {
      if (t.id !== id) return t;
      updated = { ...t, name: cleanName, notes: notes.trim() };
      return updated;
    });
    this.error = null;
    return updated;
  }

  deleteTableDraft(id: number): boolean {
    if (id > 0) {
      this.error = 'Existing tables cannot be deleted until schema impact review is available.';
      return false;
    }
    this.tables = this.tables.filter((t) => t.id !== id);
    const { [id]: _removed, ...remainingFields } = this.fieldsByTable;
    this.fieldsByTable = remainingFields;
    this.relationships = this.relationships.filter((r) => r.fromTable !== id && r.toTable !== id);
    if (this.selectedTableId === id) this.selectedTableId = this.tables[0]?.id ?? null;
    this.error = null;
    return true;
  }

  // ── field draft mutations ───────────────────────────────────────────────

  saveFieldDraft(tableId: number, id: number | null, name: string, kind: FieldKind, notes: string): FieldView | null {
    const cleanName = name.trim();
    if (!cleanName) return this.fail('Field name is required.');
    const fields = this.fieldsByTable[tableId] ?? [];
    if (fields.some((f) => f.id !== id && normName(f.name) === normName(cleanName))) {
      return this.fail(`A field named "${cleanName}" already exists in this table.`);
    }
    if (id == null) {
      const field: FieldView = {
        id: this.nextId(),
        name: cleanName,
        notes: notes.trim(),
        phys: '',
        kind,
        position: fields.length,
      };
      this.fieldsByTable = { ...this.fieldsByTable, [tableId]: [...fields, field] };
      this.error = null;
      return field;
    }
    let updated: FieldView | null = null;
    this.fieldsByTable = {
      ...this.fieldsByTable,
      [tableId]: fields.map((f) => {
        if (f.id !== id) return f;
        updated = { ...f, name: cleanName, kind, notes: notes.trim() };
        return updated;
      }),
    };
    this.error = null;
    return updated;
  }

  deleteFieldDraft(tableId: number, fieldId: number): boolean {
    if (fieldId > 0) {
      this.error = 'Existing fields cannot be deleted until schema impact review is available.';
      return false;
    }
    const fields = this.fieldsByTable[tableId] ?? [];
    this.fieldsByTable = {
      ...this.fieldsByTable,
      [tableId]: fields.filter((f) => f.id !== fieldId).map((f, i) => ({ ...f, position: i })),
    };
    this.relationships = this.relationships.filter((r) => r.fromField !== fieldId && r.toField !== fieldId);
    this.error = null;
    return true;
  }

  reorder(fieldIds: number[]): void {
    const tableId = this.selectedTableId;
    if (tableId == null) return;
    const fields = this.fieldsByTable[tableId] ?? [];
    const byId = new Map(fields.map((f) => [f.id, f]));
    if (fieldIds.length !== fields.length || fieldIds.some((id) => !byId.has(id))) return;
    this.fieldsByTable = {
      ...this.fieldsByTable,
      [tableId]: fieldIds.map((id, position) => ({ ...byId.get(id)!, position })),
    };
  }

  // ── relationship draft mutations ────────────────────────────────────────

  saveRelationshipDraft(id: number | null, rel: Omit<RelationshipView, 'id'>): RelationshipView | null {
    if (!rel.name.trim()) return this.fail('Relationship name is required.');
    if (!this.tableById(rel.fromTable) || !this.tableById(rel.toTable)) return this.fail('Choose both tables.');
    if (!this.fieldById(rel.fromTable, rel.fromField) || !this.fieldById(rel.toTable, rel.toField)) {
      return this.fail('Choose fields that belong to the selected tables.');
    }
    if (
      this.relationships.some(
        (r) => r.id !== id && r.fromTable === rel.fromTable && normName(r.name) === normName(rel.name),
      )
    ) {
      return this.fail('Relationship names must be unique for the source table.');
    }
    const cleanRel = { ...rel, name: rel.name.trim() };
    if (id == null) {
      const created: RelationshipView = { id: this.nextId(), ...cleanRel };
      this.relationships = [...this.relationships, created];
      this.error = null;
      return created;
    }
    let updated: RelationshipView | null = null;
    this.relationships = this.relationships.map((r) => {
      if (r.id !== id) return r;
      updated = { id, ...cleanRel };
      return updated;
    });
    this.error = null;
    return updated;
  }

  deleteRelationshipDraft(id: number): boolean {
    this.relationships = this.relationships.filter((r) => r.id !== id);
    if (id > 0 && !this.deletedRelationshipIds.includes(id)) {
      this.deletedRelationshipIds = [...this.deletedRelationshipIds, id];
    }
    this.error = null;
    return true;
  }

  tableById(id: number): TableView | null {
    return this.tables.find((t) => t.id === id) ?? null;
  }

  fieldById(tableId: number, fieldId: number): FieldView | null {
    return (this.fieldsByTable[tableId] ?? []).find((f) => f.id === fieldId) ?? null;
  }

  // ── commit ──────────────────────────────────────────────────────────────

  async saveAll(): Promise<boolean> {
    if (!this.validateDraft()) return false;
    this.saving = true;
    try {
      const tableIdMap = new Map<number, number>();
      const fieldIdMap = new Map<number, number>();
      const resolveTable = (id: number) => tableIdMap.get(id) ?? id;
      const resolveField = (id: number) => fieldIdMap.get(id) ?? id;

      for (const id of this.deletedRelationshipIds) {
        await api.deleteRelationship(id);
      }

      for (const table of this.tables.filter((t) => t.id > 0)) {
        const base = this.baseTables.find((t) => t.id === table.id);
        if (base && !sameJson(table, base)) await api.updateTable(table.id, table.name, table.notes);
      }

      for (const table of this.tables.filter((t) => t.id < 0)) {
        const created = await api.createTableWithNotes(table.name, table.notes);
        tableIdMap.set(table.id, created.id);
      }

      for (const table of this.tables) {
        const sourceTableId = table.id;
        const targetTableId = resolveTable(table.id);
        const fields = this.fieldsByTable[sourceTableId] ?? [];
        for (const field of fields.filter((f) => f.id > 0)) {
          const base = this.baseFieldsByTable[sourceTableId]?.find((f) => f.id === field.id);
          if (base && !sameJson(field, base)) {
            await api.updateField(targetTableId, field.id, field.name, field.kind, field.notes);
          }
        }
        for (const field of fields.filter((f) => f.id < 0)) {
          const created = await api.createFieldWithNotes(targetTableId, field.name, field.kind, field.notes);
          fieldIdMap.set(field.id, created.id);
        }
        if (fields.length > 0) {
          await api.reorderFields(targetTableId, fields.map((f) => resolveField(f.id)));
        }
      }

      for (const rel of this.relationships.filter((r) => r.id > 0)) {
        const base = this.baseRelationships.find((r) => r.id === rel.id);
        const body = this.relationshipBody(rel, resolveTable, resolveField);
        const baseBody = base ? this.relationshipBody(base, (id) => id, (id) => id) : null;
        if (!baseBody || !sameJson(body, baseBody)) await api.updateRelationship(rel.id, body);
      }
      for (const rel of this.relationships.filter((r) => r.id < 0)) {
        await api.createRelationship(this.relationshipBody(rel, resolveTable, resolveField));
      }

      await this.load();
      this.error = null;
      return true;
    } catch (e) {
      this.error = e instanceof SchemaError ? e.message : e instanceof Error ? e.message : String(e);
      return false;
    } finally {
      this.saving = false;
    }
  }

  private relationshipBody(
    rel: RelationshipView,
    tableId: (id: number) => number,
    fieldId: (id: number) => number,
  ): Omit<RelationshipView, 'id'> {
    return {
      name: rel.name,
      fromTable: tableId(rel.fromTable),
      toTable: tableId(rel.toTable),
      fromField: fieldId(rel.fromField),
      toField: fieldId(rel.toField),
    };
  }

  private validateDraft(): boolean {
    const tableNames = new Set<string>();
    for (const table of this.tables) {
      const name = normName(table.name);
      if (!name) return this.fail('Every table needs a name.') !== null;
      if (tableNames.has(name)) return this.fail(`Duplicate table name: ${table.name}`) !== null;
      tableNames.add(name);
    }

    for (const table of this.tables) {
      const fieldNames = new Set<string>();
      for (const field of this.fieldsByTable[table.id] ?? []) {
        const name = normName(field.name);
        if (!name) return this.fail(`Every field in ${table.name} needs a name.`) !== null;
        if (fieldNames.has(name)) return this.fail(`Duplicate field name in ${table.name}: ${field.name}`) !== null;
        fieldNames.add(name);
      }
    }

    for (const rel of this.relationships) {
      if (!rel.name.trim()) return this.fail('Every relationship needs a name.') !== null;
      if (!this.fieldById(rel.fromTable, rel.fromField) || !this.fieldById(rel.toTable, rel.toField)) {
        return this.fail(`Relationship "${rel.name}" points at a missing field.`) !== null;
      }
    }
    return true;
  }

  private fail<T>(message: string): T | null {
    this.error = message;
    return null;
  }
}
