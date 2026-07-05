<script lang="ts">
  // The Fields tab (#113): a Table selector + "N fields defined" + a View-by
  // control head the grid (inspired by the classic database-definition dialog).
  // The grid is a spreadsheet-like fast path (inline rename/retype, drag-reorder
  // in custom order, delete, add row); the gear on a row opens the field-detail
  // drawer. The parent owns drag state; each row isolates its own inline buffers.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind, FieldView } from './types';
  import { FIELD_KINDS } from './types';
  import FieldRow from './FieldRow.svelte';

  let {
    store,
    onswitch,
    onedit,
    onnotables,
    openFieldId,
  }: {
    store: SchemaStore;
    onswitch: (id: number) => void;
    onedit: (id: number) => void;
    onnotables: () => void;
    openFieldId: number | null;
  } = $props();

  // View-by: custom (the stored order — draggable) or a display-only sort.
  type SortBy = 'custom' | 'name' | 'type';
  let sortBy = $state<SortBy>('custom');
  const canReorder = $derived(sortBy === 'custom');
  const displayFields = $derived.by<FieldView[]>(() => {
    const fs = store.fields;
    if (sortBy === 'name') return [...fs].sort((a, b) => a.name.localeCompare(b.name));
    if (sortBy === 'type') return [...fs].sort((a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name));
    return fs;
  });

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

  // Drag-to-reorder (custom order only). Track the dragged field, the hovered
  // row, and whether the insertion goes before/after it (from the pointer).
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

    const current = store.fields.map((f) => f.id);
    if (ids.length === current.length && ids.every((v, i) => v === current[i])) return;
    void store.reorder(ids);
  }
</script>

{#if store.tables.length === 0}
  <div class="fg-blank">
    <p class="fg-blank-title">No tables yet</p>
    <p class="fg-blank-sub">Create a table before defining fields.</p>
    <button type="button" class="fg-blank-btn" onclick={onnotables}>Go to Tables</button>
  </div>
{:else}
  <div class="fg">
    <header class="fg-head">
      <div class="fg-head-group">
        <label class="fg-hlabel" for="fg-table">Table</label>
        <select
          id="fg-table"
          class="fg-select"
          value={store.selectedTableId}
          onchange={(e) => onswitch(Number(e.currentTarget.value))}
        >
          {#each store.tables as t (t.id)}
            <option value={t.id}>{t.name}</option>
          {/each}
        </select>
        <span class="fg-count">
          {store.fields.length}
          {store.fields.length === 1 ? 'field' : 'fields'} defined
        </span>
      </div>
      <div class="fg-head-group">
        <label class="fg-hlabel" for="fg-viewby">View by</label>
        <select id="fg-viewby" class="fg-select" bind:value={sortBy}>
          <option value="custom">Custom order</option>
          <option value="name">Field name</option>
          <option value="type">Type</option>
        </select>
      </div>
    </header>

    <div class="fg-scroll">
      <div class="fg-grid">
        <div class="fg-colhead">
          <span class="fg-c-handle" aria-hidden="true"></span>
          <span>Field name</span>
          <span>Type</span>
          <span>Physical name</span>
          <span class="fg-c-actions" aria-hidden="true"></span>
        </div>

        {#if store.loadingFields}
          <p class="fg-note">Loading fields…</p>
        {:else if store.fields.length === 0}
          <p class="fg-note">No fields yet — add the first one below.</p>
        {/if}

        {#each displayFields as field (field.id)}
          <FieldRow
            {store}
            {field}
            reorderable={canReorder}
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
{/if}

<style>
  .fg {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .fg-head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 18px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .fg-head-group {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .fg-hlabel {
    font-size: 12px;
    font-weight: 600;
    color: var(--rm-text-dim);
  }
  .fg-select {
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    color: var(--rm-text);
    padding: 6px 26px 6px 9px;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
    appearance: none;
    -webkit-appearance: none;
    background-color: var(--rm-control-bg);
    background-image: url("data:image/svg+xml;charset=utf-8,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='7' viewBox='0 0 10 7'%3E%3Cpath d='M1 1.5 5 5.5 9 1.5' fill='none' stroke='%238a8a8e' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 9px center;
    cursor: pointer;
  }
  .fg-select:focus {
    outline: none;
    border-color: var(--rm-accent);
  }
  .fg-count {
    font-size: 12px;
    color: var(--rm-text-dim);
    white-space: nowrap;
  }
  .fg-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px;
  }
  .fg-grid {
    background: var(--rm-control-bg);
    border: 0.5px solid var(--rm-border);
    border-radius: 10px;
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
    height: 36px;
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
  .fg-blank {
    margin: auto;
    padding: 3rem;
    text-align: center;
  }
  .fg-blank-title {
    margin: 0 0 6px;
    font-size: 15px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .fg-blank-sub {
    margin: 0 0 14px;
    font-size: 13px;
    color: var(--rm-text-dim);
  }
  .fg-blank-btn {
    font: inherit;
    font-size: 12.5px;
    font-weight: 600;
    padding: 8px 14px;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .fg-blank-btn:hover {
    background: #f0f0f2;
  }
</style>
