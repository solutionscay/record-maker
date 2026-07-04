// The schema-builder store (#113) — Svelte 5 runes in a `.svelte.ts` module so
// the reactive surface is universal (browser components today; headless-testable
// later, like the Layout editor's doc store). It owns the on-screen schema and
// funnels every mutation through the #107 API, then reflects the server's
// returned view — the server is authoritative, this never guesses.

import * as api from './persist';
import { SchemaError } from './persist';
import type { FieldKind, FieldView, TableView } from './types';

export class SchemaStore {
  /** All user tables, in picker order. */
  tables = $state<TableView[]>([]);
  /** The table whose fields the grid shows; null before load / when none exist. */
  selectedTableId = $state<number | null>(null);
  /** Fields of the selected table, in `position` order. */
  fields = $state<FieldView[]>([]);
  /** True during the initial tables+fields load (drives the loading state). */
  loading = $state(true);
  /** True while a per-table field fetch is in flight (table switch). */
  loadingFields = $state(false);
  /** Last error message, shown in a dismissable banner; null when clear. */
  error = $state<string | null>(null);

  /** The selected table object (derived), or null. */
  get selectedTable(): TableView | null {
    return this.tables.find((t) => t.id === this.selectedTableId) ?? null;
  }

  /** Run `op`, routing any failure into `error` so the UI can show it. Returns
   * the op's result, or null if it threw. */
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

  /** Initial load: fetch tables, then the first table's fields. */
  async load(): Promise<void> {
    this.loading = true;
    const tables = await this.guard(() => api.listTables());
    this.tables = tables ?? [];
    const first = this.tables[0]?.id ?? null;
    this.selectedTableId = first;
    if (first != null) await this.loadFields(first);
    else this.fields = [];
    this.loading = false;
  }

  /** Load (or reload) one table's fields into `fields`. */
  private async loadFields(tableId: number): Promise<void> {
    this.loadingFields = true;
    const fields = await this.guard(() => api.listFields(tableId));
    // Ignore a stale response if the selection changed while it was in flight.
    if (this.selectedTableId === tableId) this.fields = fields ?? [];
    this.loadingFields = false;
  }

  /** Select a table and load its fields (no-op if already selected). */
  async selectTable(tableId: number): Promise<void> {
    if (this.selectedTableId === tableId) return;
    this.selectedTableId = tableId;
    this.fields = [];
    await this.loadFields(tableId);
  }

  // ── tables ──────────────────────────────────────────────────────────────

  /** Create a table and select it. Returns the new table (or null on failure). */
  async createTable(name: string): Promise<TableView | null> {
    const table = await this.guard(() => api.createTable(name.trim()));
    if (!table) return null;
    this.tables = [...this.tables, table];
    await this.selectTable(table.id);
    return table;
  }

  async renameTable(id: number, name: string): Promise<void> {
    const updated = await this.guard(() => api.renameTable(id, name.trim()));
    if (!updated) return;
    this.tables = this.tables.map((t) => (t.id === id ? updated : t));
  }

  async deleteTable(id: number): Promise<void> {
    const ok = await this.guard(() => api.deleteTable(id));
    if (ok === null) return;
    const remaining = this.tables.filter((t) => t.id !== id);
    this.tables = remaining;
    if (this.selectedTableId === id) {
      const next = remaining[0]?.id ?? null;
      this.selectedTableId = next;
      if (next != null) await this.loadFields(next);
      else this.fields = [];
    }
  }

  // ── fields ──────────────────────────────────────────────────────────────

  /** Add a field to the selected table. Returns the new field (or null). */
  async addField(name: string, kind: FieldKind): Promise<FieldView | null> {
    const tableId = this.selectedTableId;
    if (tableId == null) return null;
    const field = await this.guard(() => api.createField(tableId, name.trim(), kind));
    if (!field) return null;
    this.fields = [...this.fields, field];
    return field;
  }

  async renameField(fieldId: number, name: string): Promise<void> {
    const tableId = this.selectedTableId;
    if (tableId == null) return;
    const updated = await this.guard(() => api.renameField(tableId, fieldId, name.trim()));
    if (!updated) return;
    this.fields = this.fields.map((f) => (f.id === fieldId ? updated : f));
  }

  async retypeField(fieldId: number, kind: FieldKind): Promise<void> {
    const tableId = this.selectedTableId;
    if (tableId == null) return;
    const updated = await this.guard(() => api.retypeField(tableId, fieldId, kind));
    if (!updated) return;
    this.fields = this.fields.map((f) => (f.id === fieldId ? updated : f));
  }

  async deleteField(fieldId: number): Promise<void> {
    const tableId = this.selectedTableId;
    if (tableId == null) return;
    const ok = await this.guard(() => api.deleteField(tableId, fieldId));
    if (ok === null) return;
    this.fields = this.fields.filter((f) => f.id !== fieldId);
  }

  /** Reorder fields to the given id sequence; reflects the server's ordered list. */
  async reorder(fieldIds: number[]): Promise<void> {
    const tableId = this.selectedTableId;
    if (tableId == null) return;
    const ordered = await this.guard(() => api.reorderFields(tableId, fieldIds));
    if (!ordered) return;
    if (this.selectedTableId === tableId) this.fields = ordered;
  }
}
