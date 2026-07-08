// The schema-builder store (#113/#119). It keeps a full editable draft of the
// schema, separate from the last server-loaded baseline. Drawer saves only touch
// the draft; the schema-window Save applies the draft through the #107 API.

import * as api from './persist';
import { HttpError } from '../shared/http';
import {
  emptyFieldOptions,
  type FieldKind,
  type FieldOptions,
  type FieldView,
  type RelationshipView,
  type TableView,
  type ValueListView,
} from './types';

type FieldMap = Record<number, FieldView[]>;

const cloneTable = (t: TableView): TableView => ({ ...t });
const cloneField = (f: FieldView): FieldView => ({ ...f });
const cloneRelationship = (r: RelationshipView): RelationshipView => ({ ...r });
const cloneValueList = (v: ValueListView): ValueListView => ({ ...v });
const cloneFields = (fields: FieldMap): FieldMap =>
  Object.fromEntries(Object.entries(fields).map(([id, fs]) => [id, fs.map(cloneField)]));

function normName(s: string): string {
  return s.trim().toLocaleLowerCase();
}

function sameJson(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

function withoutReference(options: FieldOptions): FieldOptions {
  if (!options.reference) return options;
  const { reference: _reference, ...rest } = options;
  return rest;
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
  /** Existing fields deleted in the draft, applied (column dropped) on save. */
  deletedFields = $state<{ tableId: number; fieldId: number }[]>([]);
  /** Value lists available for member-of-list field validation. */
  valueLists = $state<ValueListView[]>([]);
  /** True during the initial full schema load. */
  loading = $state(true);
  /** True while the top-level schema save is applying the draft. */
  saving = $state(false);
  /** Last error message, shown in a dismissable banner; null when clear. */
  error = $state<string | null>(null);

  private baseTables: TableView[] = [];
  private baseFieldsByTable: FieldMap = {};
  private baseRelationships: RelationshipView[] = [];
  private baseValueLists: ValueListView[] = [];
  private tempId = -1;

  /** The selected table object (derived), or null. */
  get selectedTable(): TableView | null {
    return this.tables.find((t) => t.id === this.selectedTableId) ?? null;
  }

  /** Fields for the selected table, in draft display order. */
  get fields(): readonly FieldView[] {
    return this.selectedTableId == null ? [] : (this.fieldsByTable[this.selectedTableId] ?? []);
  }

  /** Memoized draft-vs-baseline dirty check. A `$derived.by` class field so the
   * whole-draft JSON serialization runs once per draft change instead of on
   * every reactive read of `hasChanges`/`changeSummary`. */
  readonly #hasChanges: boolean = $derived.by(
    () =>
      !sameJson(this.tables, this.baseTables) ||
      !sameJson(this.fieldsByTable, this.baseFieldsByTable) ||
      !sameJson(this.relationships, this.baseRelationships) ||
      this.deletedRelationshipIds.length > 0 ||
      this.deletedFields.length > 0,
  );

  /** Memoized change summary — same caching rationale as `#hasChanges`. */
  readonly #changeSummary: string = $derived.by(() => {
    if (!this.#hasChanges) return 'No unsaved schema changes';
    const tableDelta = this.tables.filter((t) => !sameJson(t, this.baseTables.find((b) => b.id === t.id))).length;
    const fieldDelta = Object.values(this.fieldsByTable)
      .flat()
      .filter((f) => {
        const base = Object.values(this.baseFieldsByTable)
          .flat()
          .find((b) => b.id === f.id);
        return !sameJson(f, base);
      }).length + this.deletedFields.length;
    const relDelta =
      this.relationships.filter((r) => !sameJson(r, this.baseRelationships.find((b) => b.id === r.id))).length +
      this.deletedRelationshipIds.length;
    const parts = [
      tableDelta > 0 ? `${tableDelta} table${tableDelta === 1 ? '' : 's'}` : '',
      fieldDelta > 0 ? `${fieldDelta} field${fieldDelta === 1 ? '' : 's'}` : '',
      relDelta > 0 ? `${relDelta} relationship${relDelta === 1 ? '' : 's'}` : '',
    ].filter(Boolean);
    return `Unsaved: ${parts.join(', ')}`;
  });

  get hasChanges(): boolean {
    return this.#hasChanges;
  }

  get changeSummary(): string {
    return this.#changeSummary;
  }

  /** Run `op`, routing any failure into `error` so the UI can show it. */
  private async guard<T>(op: () => Promise<T>): Promise<T | null> {
    try {
      const out = await op();
      this.error = null;
      return out;
    } catch (e) {
      this.error = e instanceof HttpError ? e.message : e instanceof Error ? e.message : String(e);
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
      const valueLists = await api.listValueLists();
      return { tables, fieldsByTable, relationships, valueLists };
    });
    if (loaded) {
      this.baseTables = loaded.tables.map(cloneTable);
      this.baseFieldsByTable = cloneFields(loaded.fieldsByTable);
      this.baseRelationships = loaded.relationships.map(cloneRelationship);
      this.baseValueLists = loaded.valueLists.map(cloneValueList);
      this.tables = loaded.tables.map(cloneTable);
      this.fieldsByTable = cloneFields(loaded.fieldsByTable);
      this.relationships = loaded.relationships.map(cloneRelationship);
      this.valueLists = loaded.valueLists.map(cloneValueList);
      this.deletedRelationshipIds = [];
      this.deletedFields = [];
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
    this.valueLists = this.baseValueLists.map(cloneValueList);
    this.deletedRelationshipIds = [];
    this.deletedFields = [];
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
      const nextPos = Math.max(0, ...this.tables.map((t) => t.position)) + 1;
      const table: TableView = {
        id: this.nextId(),
        name: cleanName,
        notes: notes.trim(),
        phys: '',
        position: nextPos,
      };
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

  /** Save a table box's Relationships-graph position. Pure view-state: applied
   * to BOTH the draft and the baseline so arranging the diagram never registers
   * as an unsaved schema change, and persisted immediately (unsaved negative-id
   * tables just move locally until they're first saved). */
  persistGraphPosition(tableId: number, x: number, y: number): void {
    const apply = (t: TableView): TableView => (t.id === tableId ? { ...t, graphX: x, graphY: y } : t);
    this.tables = this.tables.map(apply);
    this.baseTables = this.baseTables.map(apply);
    if (tableId > 0) void this.guard(() => api.setTableGraphPosition(tableId, x, y));
  }

  // ── field draft mutations ───────────────────────────────────────────────

  saveFieldDraft(
    tableId: number,
    id: number | null,
    name: string,
    kind: FieldKind,
    notes: string,
    options: FieldOptions,
  ): FieldView | null {
    const cleanName = name.trim();
    if (!cleanName) return this.fail('Field name is required.');
    const fields = this.fieldsByTable[tableId] ?? [];
    if (fields.some((f) => f.id !== id && normName(f.name) === normName(cleanName))) {
      return this.fail(`A field named "${cleanName}" already exists in this table.`);
    }
    if (options.validation?.primary && fields.some((f) => f.id !== id && f.options.validation?.primary)) {
      return this.fail('A table can only have one Primary ID field.');
    }
    const reference = options.reference;
    const memberOfValueList = options.validation?.memberOfValueList;
    if (memberOfValueList != null && !this.valueLists.some((list) => list.id === memberOfValueList)) {
      return this.fail('Choose a valid value list.');
    }
    if (reference) {
      if (!reference.name.trim()) return this.fail('Reference relationships need a name.');
      if (!this.tableById(reference.toTable) || !this.fieldById(reference.toTable, reference.toField)) {
        return this.fail('Choose a valid reference target.');
      }
      if (
        this.relationships.some(
          (r) =>
            r.fromTable === tableId &&
            r.fromField !== (id ?? Number.NaN) &&
            normName(r.name) === normName(reference.name),
        )
      ) {
        return this.fail('Relationship names must be unique for the source table.');
      }
    }
    if (id == null) {
      const field: FieldView = {
        id: this.nextId(),
        name: cleanName,
        notes: notes.trim(),
        phys: '',
        kind,
        options: emptyFieldOptions(),
        position: fields.length,
      };
      field.options = options;
      this.fieldsByTable = { ...this.fieldsByTable, [tableId]: [...fields, field] };
      this.error = null;
      return field;
    }
    let updated: FieldView | null = null;
    this.fieldsByTable = {
      ...this.fieldsByTable,
      [tableId]: fields.map((f) => {
        if (f.id !== id) return f;
        updated = { ...f, name: cleanName, kind, notes: notes.trim(), options };
        return updated;
      }),
    };
    this.error = null;
    return updated;
  }

  deleteFieldDraft(tableId: number, fieldId: number): boolean {
    const field = this.fieldById(tableId, fieldId);
    if (field?.options?.system) {
      this.error = 'The system primary key cannot be deleted.';
      return false;
    }
    // Remove every relationship the field participates in (as FK or target),
    // tracking persisted ones for deletion so the server's field-delete cascade
    // has nothing left to drop. deleteRelationshipDraft handles both sides.
    for (const rel of this.relationships.filter((r) => r.fromField === fieldId || r.toField === fieldId)) {
      this.deleteRelationshipDraft(rel.id);
    }
    const fields = this.fieldsByTable[tableId] ?? [];
    this.fieldsByTable = {
      ...this.fieldsByTable,
      [tableId]: fields.filter((f) => f.id !== fieldId).map((f, i) => ({ ...f, position: i })),
    };
    // Persisted fields are dropped on save; unsaved (negative-id) ones just vanish.
    if (fieldId > 0 && !this.deletedFields.some((d) => d.fieldId === fieldId)) {
      this.deletedFields = [...this.deletedFields, { tableId, fieldId }];
    }
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

  reorderTables(tableIds: number[]): void {
    const byId = new Map(this.tables.map((t) => [t.id, t]));
    if (tableIds.length !== this.tables.length || tableIds.some((id) => !byId.has(id))) return;
    this.tables = tableIds.map((id, position) => ({ ...byId.get(id)!, position }));
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
      // Field ids already persisted (updated or created) in the structural pass,
      // so the options pass never re-sends an identical update.
      const persisted = new Set<number>();

      // Independent deletes/updates run in parallel; each ordered group below
      // still completes before the next (renames land before creates, creates
      // fill the id maps before anything resolves through them).
      await Promise.all(this.deletedRelationshipIds.map((id) => api.deleteRelationship(id)));

      // Field deletes run after relationship deletes so the server-side
      // field-delete cascade (which also clears meta_relationship rows) finds
      // them already gone; the drop is idempotent from the client's view.
      await Promise.all(this.deletedFields.map((d) => api.deleteField(d.tableId, d.fieldId)));

      await Promise.all(
        this.tables
          .filter((t) => t.id > 0)
          .filter((t) => {
            const base = this.baseTables.find((b) => b.id === t.id);
            return base && !sameJson(t, base);
          })
          .map((t) => api.updateTable(t.id, t.name, t.notes)),
      );

      const newTables = this.tables.filter((t) => t.id < 0);
      const createdTables = await Promise.all(
        newTables.map((t) => api.createTableWithNotes(t.name, t.notes)),
      );
      newTables.forEach((t, i) => tableIdMap.set(t.id, createdTables[i].id));

      // Structural field pass, tables in parallel: update existing fields
      // (references stripped — their targets may not exist yet), then create new
      // fields (filling the field id map), then persist the order.
      await Promise.all(
        this.tables.map(async (table) => {
          const targetTableId = resolveTable(table.id);
          const fields = this.fieldsByTable[table.id] ?? [];
          const changed = fields.filter((f) => {
            if (f.id < 0) return false;
            const base = this.baseFieldsByTable[table.id]?.find((b) => b.id === f.id);
            return base && !sameJson(f, base);
          });
          await Promise.all(
            changed.map((f) =>
              api
                .updateField(targetTableId, f.id, f.name, f.kind, f.notes, withoutReference(f.options))
                .then(() => {
                  persisted.add(f.id);
                }),
            ),
          );
          const newFields = fields.filter((f) => f.id < 0);
          const createdFields = await Promise.all(
            newFields.map((f) =>
              api.createFieldWithDetails(targetTableId, f.name, f.kind, f.notes, withoutReference(f.options)),
            ),
          );
          newFields.forEach((f, i) => {
            fieldIdMap.set(f.id, createdFields[i].id);
            persisted.add(f.id);
          });
          const reorderIds = fields.filter((f) => !f.options?.system).map((f) => resolveField(f.id));
          if (reorderIds.length > 0) {
            await api.reorderFields(targetTableId, reorderIds);
          }
        }),
      );

      // Options pass, now that every table/field id resolves: re-send only the
      // fields whose resolved options differ from BOTH the baseline and what the
      // structural pass just persisted (i.e. reference-carrying options).
      await Promise.all(
        this.tables.flatMap((table) => {
          const targetTableId = resolveTable(table.id);
          return (this.fieldsByTable[table.id] ?? []).flatMap((field) => {
            const finalOptions = this.resolvedFieldOptions(field.options, resolveTable, resolveField);
            const baseOptions = this.baseFieldsByTable[table.id]?.find((f) => f.id === field.id)?.options ?? {};
            if (sameJson(finalOptions, baseOptions)) return [];
            if (persisted.has(field.id) && sameJson(finalOptions, withoutReference(field.options))) return [];
            return [
              api.updateField(
                targetTableId,
                resolveField(field.id),
                field.name,
                field.kind,
                field.notes,
                finalOptions,
              ),
            ];
          });
        }),
      );

      await Promise.all(
        this.relationships
          .filter((r) => r.id > 0)
          .flatMap((rel) => {
            const base = this.baseRelationships.find((r) => r.id === rel.id);
            const body = this.relationshipBody(rel, resolveTable, resolveField);
            const baseBody = base ? this.relationshipBody(base, (id) => id, (id) => id) : null;
            return !baseBody || !sameJson(body, baseBody) ? [api.updateRelationship(rel.id, body)] : [];
          }),
      );
      await Promise.all(
        this.relationships
          .filter((r) => r.id < 0)
          .map((rel) => api.createRelationship(this.relationshipBody(rel, resolveTable, resolveField))),
      );

      if (this.tables.length > 0) {
        await api.reorderTables(this.tables.map((t) => resolveTable(t.id)));
      }

      // Reload rather than patching the baseline from responses: on any failure
      // above the draft must stay intact and the baseline must stay the server's
      // truth, and the reload is what guarantees that equivalence.
      await this.load();
      this.error = null;
      return true;
    } catch (e) {
      this.error = e instanceof HttpError ? e.message : e instanceof Error ? e.message : String(e);
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

  private resolvedFieldOptions(
    options: FieldOptions,
    tableId: (id: number) => number,
    fieldId: (id: number) => number,
  ): FieldOptions {
    if (!options.reference) return options;
    return {
      ...options,
      reference: {
        ...options.reference,
        toTable: tableId(options.reference.toTable),
        toField: fieldId(options.reference.toField),
      },
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
      let primaryField: string | null = null;
      for (const field of this.fieldsByTable[table.id] ?? []) {
        const name = normName(field.name);
        if (!name) return this.fail(`Every field in ${table.name} needs a name.`) !== null;
        if (fieldNames.has(name)) return this.fail(`Duplicate field name in ${table.name}: ${field.name}`) !== null;
        if (field.options.validation?.primary) {
          if (primaryField != null) {
            return this.fail(`Table "${table.name}" can only have one Primary ID field.`) !== null;
          }
          primaryField = field.name;
        }
        const memberOfValueList = field.options.validation?.memberOfValueList;
        if (memberOfValueList != null && !this.valueLists.some((list) => list.id === memberOfValueList)) {
          return this.fail(`Field "${field.name}" references a missing value list.`) !== null;
        }
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
