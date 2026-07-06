<script lang="ts">
  import type { SchemaStore } from './store.svelte';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    onnew,
    onedit,
  }: {
    store: SchemaStore;
    onnew: () => void;
    onedit: (id: number) => void;
  } = $props();

  function tableName(id: number): string {
    return store.tableById(id)?.name ?? 'Missing table';
  }

  function fieldName(tableId: number, fieldId: number): string {
    return store.fieldById(tableId, fieldId)?.name ?? 'Missing field';
  }

  const canCreate = $derived(store.tables.some((t) => (store.fieldsByTable[t.id] ?? []).length > 0));
</script>

<div class="rv">
  <header class="rv-head">
    <div class="rv-title">
      <span class="sc-micro">Relationships</span>
      <span class="sc-count">{store.relationships.length} defined</span>
    </div>
    <button type="button" class="sc-btn sc-btn--primary" onclick={onnew} disabled={!canCreate}>
      <Icon name="plus" />New relationship
    </button>
  </header>

  <div class="rv-list">
    {#if store.loading}
      <p class="rv-note sc-hint">Loading relationships...</p>
    {:else if !canCreate}
      <div class="rv-empty">
        <p class="rv-empty-title">No fields available</p>
        <p class="sc-hint">Create fields before defining relationships.</p>
      </div>
    {:else if store.relationships.length === 0}
      <div class="rv-empty">
        <p class="rv-empty-title">No relationships yet</p>
        <p class="sc-hint">Connect a source field to a target field.</p>
        <button type="button" class="sc-btn sc-btn--primary" onclick={onnew}>
          <Icon name="plus" />New relationship
        </button>
      </div>
    {/if}

    {#each store.relationships as rel (rel.id)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="rv-row" onclick={() => onedit(rel.id)}>
        <svg class="rv-ico" aria-hidden="true"><use href="#icon-field" /></svg>
        <span class="rv-main">
          <span class="rv-name">{rel.name}</span>
          <span class="rv-path">
            {tableName(rel.fromTable)}.{fieldName(rel.fromTable, rel.fromField)}
            ->
            {tableName(rel.toTable)}.{fieldName(rel.toTable, rel.toField)}
          </span>
        </span>
        <span class="rv-cardinality">many to one</span>
        <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Edit relationship" onclick={(e) => {
          e.stopPropagation();
          onedit(rel.id);
        }}>
          <Icon name="settings" />
        </button>
      </div>
    {/each}
  </div>
</div>

<style>
  .rv {
    height: 100%;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .rv-head {
    position: sticky;
    top: 0;
    z-index: 2;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--sc-head-h);
    padding: 0 18px;
    border-bottom: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .rv-title {
    display: inline-flex;
    align-items: center;
    gap: 10px;
  }
  .rv-list {
    flex: 1 1 auto;
    min-height: 0;
  }
  .rv-note {
    margin: 0;
    padding: 16px 18px;
  }
  .rv-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 24px;
    text-align: center;
  }
  .rv-empty-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .rv-row {
    display: flex;
    align-items: center;
    gap: 11px;
    min-height: var(--sc-row-h);
    padding: 7px 18px;
    border-top: 0.5px solid var(--rm-border);
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .rv-row:hover {
    background: var(--rm-accent-soft);
  }
  .rv-ico {
    width: 15px;
    height: 15px;
    fill: currentColor;
    flex: none;
    color: var(--rm-text-dim);
  }
  .rv-main {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .rv-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .rv-path {
    font-size: 11.5px;
    color: var(--rm-text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rv-cardinality {
    flex: none;
    font-size: 11.5px;
    color: var(--rm-text-dim);
  }
</style>
