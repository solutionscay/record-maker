<script lang="ts">
  // Tables tab (#113/#119/#121): a grid of user tables. Edit opens the table
  // drawer; the chevron alone drills into the table's fields.
  import type { SchemaStore } from './store.svelte';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    onopen,
    onnew,
    onedit,
  }: {
    store: SchemaStore;
    onopen: (id: number) => void;
    onnew: () => void;
    onedit: (id: number) => void;
  } = $props();
</script>

<div class="tv">
  <header class="sc-viewhead tv-head">
    <span class="sc-micro">Tables</span>
    <button type="button" class="sc-btn sc-btn--primary" onclick={onnew}>
      <Icon name="plus" />New table
    </button>
  </header>

  <div class="tv-list">
    {#if store.loading}
      <p class="sc-note sc-hint">Loading tables...</p>
    {:else if store.tables.length === 0}
      <div class="sc-empty tv-empty">
        <p class="sc-empty-title">No tables yet</p>
        <p class="sc-hint">Create your first table to start defining fields.</p>
        <button type="button" class="sc-btn sc-btn--primary" onclick={onnew}>
          <Icon name="plus" />New table
        </button>
      </div>
    {/if}

    {#if !store.loading && store.tables.length > 0}
      <div class="tv-colhead sc-colhead">
        <span class="tv-c-icon" aria-hidden="true"></span>
        <span class="sc-micro">Table</span>
        <span class="sc-micro">Notes</span>
        <span class="tv-c-actions" aria-hidden="true"></span>
        <span class="tv-c-nav" aria-hidden="true"></span>
      </div>
    {/if}

    {#each store.tables as table (table.id)}
      <div class="tv-row">
        <svg class="tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
        <span class="tv-name">{table.name}</span>
        <span class="tv-notes" title={table.notes || 'No notes'}>{table.notes || 'No notes'}</span>
        <span class="tv-actions">
          <button
            type="button"
            class="sc-btn sc-btn--icon sc-btn--ghost"
            title="Edit table"
            onclick={() => onedit(table.id)}
          >
            <Icon name="edit" />
          </button>
        </span>
        <button type="button" class="tv-nav" title="Show fields" onclick={() => onopen(table.id)}>
          <svg class="tv-chev" aria-hidden="true"><use href="#icon-next" /></svg>
        </button>
      </div>
    {/each}
  </div>
</div>

<style>
  .tv {
    height: 100%;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  /* Header bar / col-header / empty-state chrome comes from the shared .sc-*
     classes (schema.css); this view adds only its stickiness and grid. */
  .tv-head {
    position: sticky;
    top: 0;
    z-index: 2;
  }
  .tv-list {
    flex: 1 1 auto;
    min-height: 0;
  }
  .tv-empty .sc-btn {
    margin-top: 6px;
  }
  .tv-colhead,
  .tv-row {
    display: grid;
    grid-template-columns: 34px minmax(0, 1.35fr) minmax(0, 1.9fr) 34px 28px;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 18px;
  }
  .tv-row {
    min-height: var(--sc-row-h);
    border-bottom: 0.5px solid var(--rm-border);
    transition: background 0.12s ease;
  }
  .tv-row:hover {
    background: rgba(0, 0, 0, 0.02);
  }
  .tv-ico,
  .tv-chev {
    width: 15px;
    height: 15px;
    fill: currentColor;
    flex: none;
    color: var(--rm-text-dim);
  }
  .tv-name {
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tv-notes {
    font-size: 11.5px;
    color: var(--rm-text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tv-actions {
    display: flex;
    justify-content: center;
  }
  .tv-nav {
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--rm-text-dim);
    cursor: pointer;
  }
  .tv-nav:hover {
    background: rgba(0, 0, 0, 0.06);
    color: var(--rm-text);
  }
</style>
