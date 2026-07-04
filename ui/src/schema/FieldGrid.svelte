<script lang="ts">
  // Center pane: the field grid for the selected table (#113). A spreadsheet-like
  // fast path — inline rename + retype, drag-to-reorder, delete, and an add row —
  // with an "edit" affordance per field that opens the master-detail drawer. The
  // parent owns drag state; each row (FieldRow) isolates its own inline buffers.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind } from './types';
  import { FIELD_KINDS } from './types';
  import FieldRow from './FieldRow.svelte';

  let {
    store,
    onedit,
    openFieldId,
  }: { store: SchemaStore; onedit: (id: number) => void; openFieldId: number | null } = $props();

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

  // Drag-to-reorder: track the dragged field and the row it's hovering.
  let dragId = $state<number | null>(null);
  let overId = $state<number | null>(null);

  function onDragStart(id: number) {
    dragId = id;
  }
  function onDragOver(id: number) {
    if (dragId != null && id !== overId) overId = id;
  }
  function onDragEnd() {
    dragId = null;
    overId = null;
  }
  function onDrop(id: number) {
    const from = dragId;
    dragId = null;
    overId = null;
    if (from == null || from === id) return;
    const ids = store.fields.map((f) => f.id);
    const fromIdx = ids.indexOf(from);
    if (fromIdx < 0) return;
    ids.splice(fromIdx, 1);
    const toIdx = ids.indexOf(id);
    ids.splice(toIdx < 0 ? ids.length : toIdx, 0, from);
    void store.reorder(ids);
  }
</script>

<div class="fg">
  <header class="fg-head">
    <div class="fg-titles">
      <h1 class="fg-title">{store.selectedTable?.name}</h1>
      <p class="fg-sub">
        {store.fields.length}
        {store.fields.length === 1 ? 'field' : 'fields'} · stored as
        <code>{store.selectedTable?.phys}</code>
      </p>
    </div>
  </header>

  <div class="fg-scroll">
    <div class="fg-grid" role="table" aria-label="Fields">
      <div class="fg-colhead" role="row">
        <span class="fg-c-handle" aria-hidden="true"></span>
        <span role="columnheader">Field</span>
        <span role="columnheader">Type</span>
        <span role="columnheader">Physical name</span>
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
          dropTarget={overId === field.id && dragId != null && dragId !== field.id}
          onedit={() => onedit(field.id)}
          ondragstartrow={() => onDragStart(field.id)}
          ondragoverrow={() => onDragOver(field.id)}
          ondroprow={() => onDrop(field.id)}
          ondragendrow={onDragEnd}
        />
      {/each}

      <!-- Add-field row -->
      <div class="fg-add" role="row">
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
    padding: 18px 22px 12px;
  }
  .fg-title {
    margin: 0;
    font-size: 19px;
    font-weight: 700;
    color: var(--rm-text);
  }
  .fg-sub {
    margin: 3px 0 0;
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
    grid-template-columns: 30px minmax(0, 1.6fr) 150px minmax(0, 1fr) 84px;
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
  .fg-add-name {
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
  .fg-add-kind {
    font: inherit;
    font-size: 13px;
    padding: 6px 9px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
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
