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

  type SortBy = 'custom' | 'name' | 'type';
  let sortBy = $state<SortBy>('custom');
  const canReorder = $derived(sortBy === 'custom');
  const displayFields = $derived.by<FieldView[]>(() => {
    const fs = store.fields;
    if (sortBy === 'name') return [...fs].sort((a, b) => a.name.localeCompare(b.name));
    if (sortBy === 'type') return [...fs].sort((a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name));
    return fs;
  });

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
    <p class="sc-hint">Create a table before defining fields.</p>
    <button type="button" class="sc-btn" onclick={onnotables}>Go to Tables</button>
  </div>
{:else}
  <div class="fg">
    <header class="fg-head">
      <div class="fg-group">
        <label class="sc-micro" for="fg-table">Table</label>
        <select
          id="fg-table"
          class="sc-select fg-table-select"
          value={store.selectedTableId}
          onchange={(e) => onswitch(Number(e.currentTarget.value))}
        >
          {#each store.tables as t (t.id)}
            <option value={t.id}>{t.name}</option>
          {/each}
        </select>
        <span class="sc-count">
          {store.fields.length}
          {store.fields.length === 1 ? 'field' : 'fields'} defined
        </span>
      </div>
      <div class="fg-group">
        <label class="sc-micro" for="fg-viewby">View by</label>
        <select id="fg-viewby" class="sc-select fg-viewby" bind:value={sortBy}>
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
          <span class="sc-micro">Field name</span>
          <span class="sc-micro">Type</span>
          <span class="sc-micro">Physical name</span>
          <span class="fg-c-actions" aria-hidden="true"></span>
        </div>

        {#if store.loadingFields}
          <p class="fg-note sc-hint">Loading fields…</p>
        {:else if store.fields.length === 0}
          <p class="fg-note sc-hint">No fields yet — add the first one below.</p>
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

        <div class="fg-add">
          <span class="fg-c-handle" aria-hidden="true"></span>
          <input
            class="sc-input"
            placeholder="New field name"
            bind:value={newName}
            disabled={addBusy}
            onkeydown={(e) => {
              if (e.key === 'Enter') addField();
            }}
            aria-label="New field name"
          />
          <select class="sc-select" bind:value={newKind} disabled={addBusy} aria-label="New field type">
            {#each FIELD_KINDS as k (k.kind)}
              <option value={k.kind}>{k.label}</option>
            {/each}
          </select>
          <span></span>
          <button
            type="button"
            class="sc-btn sc-btn--primary"
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
    padding: 12px 18px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .fg-group {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .fg-table-select {
    width: auto;
    min-width: 150px;
    font-weight: 600;
  }
  .fg-viewby {
    width: auto;
    min-width: 130px;
  }
  .fg-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
  }
  /* One shared column template for the header, rows, and add row. */
  .fg-colhead,
  .fg-add,
  :global(.fg-row) {
    display: grid;
    grid-template-columns: 34px minmax(0, 1.6fr) 160px minmax(0, 1fr) 84px;
    align-items: center;
    gap: 12px;
    padding: 0 18px;
  }
  .fg-colhead {
    position: sticky;
    top: 0;
    z-index: 2;
    height: var(--sc-head-h);
    border-bottom: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .fg-note {
    margin: 0;
    padding: 16px 18px;
  }
  .fg-add {
    height: 56px;
    border-top: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .fg-blank {
    margin: auto;
    padding: 3rem;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .fg-blank-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .fg-blank .sc-btn {
    margin-top: 6px;
  }
</style>
