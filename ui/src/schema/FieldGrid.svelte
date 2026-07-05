<script lang="ts">
  // Level 2 of the drill-down (#113): the field grid for the open table. A
  // breadcrumb (‹ Tables) + a table-switcher head the level; the grid is a
  // spreadsheet-like fast path (inline rename/retype, drag-reorder, delete, add
  // row); the gear on a row opens the field-detail drawer. The parent owns drag
  // state; each row (FieldRow) isolates its own inline buffers.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind } from './types';
  import { FIELD_KINDS } from './types';
  import FieldRow from './FieldRow.svelte';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    onback,
    onswitch,
    onedit,
    openFieldId,
  }: {
    store: SchemaStore;
    onback: () => void;
    onswitch: (id: number) => void;
    onedit: (id: number) => void;
    openFieldId: number | null;
  } = $props();

  // Inline "add field" row.
  let newName = $state('');
  let newKind = $state<FieldKind>('text');
  let addBusy = $state(false);

  async function addField() {
    const name = newName.trim();
    if (!name || addBusy) return;
    addBusy = true;
    const field = await store.addField(name, newKind);
    addBusy = false;
    if (field) {
      newName = '';
      newKind = 'text';
    }
  }

  // Drag-to-reorder. Track the dragged field, the row being hovered, and whether
  // the insertion goes before/after it (computed from the pointer position).
  let dragId = $state<number | null>(null);
  let overId = $state<number | null>(null);
  let overPos = $state<'before' | 'after'>('before');

  function onDragStart(id: number) {
    dragId = id;
  }
  function onDragOver(id: number, pos: 'before' | 'after') {
    if (dragId == null) return;
    if (id !== overId) overId = id;
    if (pos !== overPos) overPos = pos;
  }
  function onDragEnd() {
    dragId = null;
    overId = null;
  }
  function onDrop() {
    const from = dragId;
    const target = overId;
    const pos = overPos;
    dragId = null;
    overId = null;
    if (from == null || target == null || from === target) return;

    const ids = store.fields.map((f) => f.id);
    const fromIdx = ids.indexOf(from);
    if (fromIdx < 0) return;
    ids.splice(fromIdx, 1);
    let targetIdx = ids.indexOf(target);
    if (targetIdx < 0) targetIdx = ids.length - 1;
    const insertIdx = pos === 'after' ? targetIdx + 1 : targetIdx;
    ids.splice(insertIdx, 0, from);

    // Skip the round-trip if nothing actually moved.
    const current = store.fields.map((f) => f.id);
    if (ids.length === current.length && ids.every((v, i) => v === current[i])) return;
    void store.reorder(ids);
  }
</script>

<div class="fg">
  <header class="fg-head">
    <button type="button" class="fg-back" onclick={onback}>
      <Icon name="prev" />Tables
    </button>
    <span class="fg-crumb-sep">/</span>
    <!-- Table switcher: jump between tables without leaving the fields level. -->
    <div class="fg-switch">
      <select
        class="fg-switch-select"
        value={store.selectedTableId}
        onchange={(e) => onswitch(Number(e.currentTarget.value))}
        aria-label="Switch table"
      >
        {#each store.tables as t (t.id)}
          <option value={t.id}>{t.name}</option>
        {/each}
      </select>
    </div>
    <span class="fg-sub">
      {store.fields.length}
      {store.fields.length === 1 ? 'field' : 'fields'} · stored as
      <code>{store.selectedTable?.phys}</code>
    </span>
  </header>

  <div class="fg-scroll">
    <div class="fg-grid">
      <div class="fg-colhead">
        <span class="fg-c-handle" aria-hidden="true"></span>
        <span>Field</span>
        <span>Type</span>
        <span>Physical name</span>
        <span class="fg-c-actions" aria-hidden="true"></span>
      </div>

      {#if store.loadingFields}
        <p class="fg-note">Loading fields…</p>
      {:else if store.fields.length === 0}
        <p class="fg-note">No fields yet — add the first one below.</p>
      {/if}

      {#each store.fields as field (field.id)}
        <FieldRow
          {store}
          {field}
          active={field.id === openFieldId}
          dragging={field.id === dragId}
          dropBefore={overId === field.id && overPos === 'before' && dragId != null && dragId !== field.id}
          dropAfter={overId === field.id && overPos === 'after' && dragId != null && dragId !== field.id}
          onedit={() => onedit(field.id)}
          ondragstartrow={() => onDragStart(field.id)}
          ondragoverrow={(pos) => onDragOver(field.id, pos)}
          ondroprow={onDrop}
          ondragendrow={onDragEnd}
        />
      {/each}

      <!-- Add-field row -->
      <div class="fg-add">
        <span class="fg-c-handle" aria-hidden="true"></span>
        <input
          class="fg-add-name"
          placeholder="New field name"
          bind:value={newName}
          disabled={addBusy}
          onkeydown={(e) => {
            if (e.key === 'Enter') addField();
          }}
          aria-label="New field name"
        />
        <select class="fg-add-kind" bind:value={newKind} disabled={addBusy} aria-label="New field type">
          {#each FIELD_KINDS as k (k.kind)}
            <option value={k.kind}>{k.label}</option>
          {/each}
        </select>
        <span></span>
        <button
          type="button"
          class="fg-add-btn"
          onclick={addField}
          disabled={addBusy || newName.trim().length === 0}
        >
          Add
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .fg {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    background: var(--rm-workspace-bg);
  }
  .fg-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 16px 22px 12px;
  }
  .fg-back {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    padding: 5px 10px 5px 7px;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .fg-back:hover {
    background: #f0f0f2;
  }
  .fg-back :global(.icon) {
    flex: none;
  }
  .fg-crumb-sep {
    color: var(--rm-text-dim);
  }
  .fg-switch-select {
    font: inherit;
    font-size: 17px;
    font-weight: 700;
    color: var(--rm-text);
    padding: 3px 26px 3px 8px;
    border: 0.5px solid transparent;
    border-radius: 8px;
    background-color: transparent;
    background-image: url("data:image/svg+xml;charset=utf-8,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='7' viewBox='0 0 10 7'%3E%3Cpath d='M1 1.5 5 5.5 9 1.5' fill='none' stroke='%238a8a8e' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 8px center;
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
  }
  .fg-switch-select:hover {
    border-color: var(--rm-border);
    background-color: var(--rm-control-bg);
  }
  .fg-switch-select:focus {
    outline: none;
    border-color: var(--rm-accent);
  }
  .fg-sub {
    margin-left: auto;
    font-size: 12.5px;
    color: var(--rm-text-dim);
  }
  .fg-sub code {
    font-size: 11.5px;
    padding: 1px 5px;
    border-radius: 5px;
    background: rgba(0, 0, 0, 0.05);
  }
  .fg-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 0 22px 22px;
  }
  .fg-grid {
    max-width: 860px;
    margin: 0 auto;
    background: var(--rm-control-bg);
    border: 0.5px solid var(--rm-border);
    border-radius: 12px;
    box-shadow: var(--rm-shadow-card);
    overflow: hidden;
  }
  /* One shared column template for the header, rows, and add row. */
  .fg-colhead,
  .fg-add,
  :global(.fg-row) {
    display: grid;
    grid-template-columns: 34px minmax(0, 1.6fr) 150px minmax(0, 1fr) 84px;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
  }
  .fg-colhead {
    height: 38px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--rm-text-dim);
    border-bottom: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .fg-note {
    margin: 0;
    padding: 18px 16px;
    font-size: 13px;
    color: var(--rm-text-dim);
  }
  .fg-add {
    height: 52px;
    border-top: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .fg-add-name,
  .fg-add-kind {
    font: inherit;
    font-size: 13px;
    padding: 6px 9px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
  }
  .fg-add-name:focus {
    outline: none;
    border-color: var(--rm-accent);
    box-shadow: 0 0 0 3px var(--rm-accent-soft);
  }
  .fg-add-btn {
    font: inherit;
    font-size: 12.5px;
    font-weight: 600;
    padding: 7px 0;
    border: 0.5px solid transparent;
    border-radius: 7px;
    background: var(--rm-accent);
    color: #fff;
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(10, 132, 255, 0.35);
  }
  .fg-add-btn:disabled {
    background: #c7c7cc;
    box-shadow: none;
    cursor: default;
  }
</style>
