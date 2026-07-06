<script lang="ts">
  import type { SchemaStore } from './store.svelte';
  import type { RelationshipView } from './types';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    relationship,
    onclose,
  }: {
    store: SchemaStore;
    relationship: RelationshipView | null;
    onclose: () => void;
  } = $props();

  function firstTable(): number {
    return store.tables[0]?.id ?? 0;
  }
  function firstField(tableId: number): number {
    return store.fieldsByTable[tableId]?.[0]?.id ?? 0;
  }

  let name = $state('');
  let fromTable = $state(0);
  let toTable = $state(0);
  let fromField = $state(0);
  let toField = $state(0);

  const fromFields = $derived(store.fieldsByTable[fromTable] ?? []);
  const toFields = $derived(store.fieldsByTable[toTable] ?? []);
  const canSave = $derived(name.trim().length > 0 && fromField !== 0 && toField !== 0);

  $effect(() => {
    name = relationship?.name ?? '';
    const nextFromTable = relationship?.fromTable ?? firstTable();
    const nextToTable = relationship?.toTable ?? firstTable();
    fromTable = nextFromTable;
    toTable = nextToTable;
    fromField = relationship?.fromField ?? firstField(nextFromTable);
    toField = relationship?.toField ?? firstField(nextToTable);
  });

  $effect(() => {
    if (!fromFields.some((f) => f.id === fromField)) fromField = firstField(fromTable);
    if (!toFields.some((f) => f.id === toField)) toField = firstField(toTable);
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function save() {
    const saved = store.saveRelationshipDraft(relationship?.id ?? null, {
      name,
      fromTable,
      toTable,
      fromField,
      toField,
    });
    if (saved) onclose();
  }

  function remove() {
    if (relationship && store.deleteRelationshipDraft(relationship.id)) onclose();
  }
</script>

<aside class="rd">
  <header class="rd-head">
    <span class="rd-title">{relationship ? 'Relationship details' : 'New relationship'}</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  <div class="rd-body">
    <label class="sc-micro rd-label" for="rd-name">Name</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="rd-name" class="sc-input" bind:value={name} autofocus />

    <span class="sc-micro rd-section">Source</span>
    <label class="sc-micro rd-label" for="rd-from-table">Table</label>
    <select id="rd-from-table" class="sc-select" value={fromTable} onchange={(e) => {
      fromTable = Number(e.currentTarget.value);
      fromField = firstField(fromTable);
    }}>
      {#each store.tables as table (table.id)}
        <option value={table.id}>{table.name}</option>
      {/each}
    </select>

    <label class="sc-micro rd-label" for="rd-from-field">Field</label>
    <select id="rd-from-field" class="sc-select" value={fromField} onchange={(e) => (fromField = Number(e.currentTarget.value))}>
      {#each fromFields as field (field.id)}
        <option value={field.id}>{field.name}</option>
      {/each}
    </select>

    <span class="sc-micro rd-section">Target</span>
    <label class="sc-micro rd-label" for="rd-to-table">Table</label>
    <select id="rd-to-table" class="sc-select" value={toTable} onchange={(e) => {
      toTable = Number(e.currentTarget.value);
      toField = firstField(toTable);
    }}>
      {#each store.tables as table (table.id)}
        <option value={table.id}>{table.name}</option>
      {/each}
    </select>

    <label class="sc-micro rd-label" for="rd-to-field">Field</label>
    <select id="rd-to-field" class="sc-select" value={toField} onchange={(e) => (toField = Number(e.currentTarget.value))}>
      {#each toFields as field (field.id)}
        <option value={field.id}>{field.name}</option>
      {/each}
    </select>

    <p class="sc-hint rd-note">Relationships are saved to the draft first and applied with the schema window Save.</p>
  </div>

  <footer class="rd-foot">
    {#if relationship}
      <button type="button" class="sc-btn sc-btn--danger rd-delete" onclick={remove}>
        <Icon name="delete" />Delete relationship
      </button>
    {/if}
    <span class="rd-spacer"></span>
    <button type="button" class="sc-btn" onclick={onclose}>Cancel</button>
    <button type="button" class="sc-btn sc-btn--primary" onclick={save} disabled={!canSave}>Save</button>
  </footer>
</aside>

<style>
  .rd {
    position: absolute;
    top: 0;
    right: 0;
    z-index: 20;
    width: 380px;
    max-width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 0.5px solid var(--rm-border);
    background: var(--rm-inspector-bg);
    box-shadow: -12px 0 32px rgba(0, 0, 0, 0.14);
  }
  .rd-head,
  .rd-foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .rd-head {
    justify-content: space-between;
    padding-right: 12px;
  }
  .rd-foot {
    border-top: 0.5px solid var(--rm-border);
    border-bottom: 0;
  }
  .rd-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .rd-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px;
  }
  .rd-section {
    display: block;
    margin: 18px 0 8px;
  }
  .rd-label {
    display: block;
    margin: 10px 0 6px;
  }
  .rd-note {
    margin: 14px 0 0;
    line-height: 1.45;
  }
  .rd-delete {
    color: var(--rm-danger);
  }
  .rd-spacer {
    flex: 1 1 auto;
  }
</style>
