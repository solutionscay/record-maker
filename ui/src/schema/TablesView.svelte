<script lang="ts">
  // Tables tab (#113/#119): rows navigate to fields, while create/edit/delete
  // live in the table drawer. No row-level inline schema mutations.
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
  <header class="tv-head">
    <span class="sc-micro">Tables</span>
    <button type="button" class="sc-btn sc-btn--primary" onclick={onnew}>
      <Icon name="plus" />New table
    </button>
  </header>

  <div class="tv-list">
    {#if store.loading}
      <p class="tv-note sc-hint">Loading tables...</p>
    {:else if store.tables.length === 0}
      <div class="tv-empty">
        <p class="tv-empty-title">No tables yet</p>
        <p class="sc-hint">Create your first table to start defining fields.</p>
        <button type="button" class="sc-btn sc-btn--primary" onclick={onnew}>
          <Icon name="plus" />New table
        </button>
      </div>
    {/if}

    {#each store.tables as table (table.id)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="tv-row is-link" onclick={() => onopen(table.id)}>
        <svg class="tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
        <span class="tv-main">
          <span class="tv-name">{table.name}</span>
          {#if table.notes}
            <span class="tv-notes">{table.notes}</span>
          {/if}
        </span>
        <code class="sc-phys">{table.phys || 'draft'}</code>
        <span class="tv-actions">
          <button
            type="button"
            class="sc-btn sc-btn--icon sc-btn--ghost"
            title="Edit table"
            onclick={(e) => {
              e.stopPropagation();
              onedit(table.id);
            }}
          >
            <Icon name="settings" />
          </button>
        </span>
        <svg class="tv-chev" aria-hidden="true"><use href="#icon-next" /></svg>
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
  .tv-head {
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
  .tv-list {
    flex: 1 1 auto;
    min-height: 0;
  }
  .tv-note {
    margin: 0;
    padding: 16px 18px;
  }
  .tv-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 24px;
    text-align: center;
  }
  .tv-empty-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .tv-empty .sc-btn {
    margin-top: 6px;
  }
  .tv-row {
    display: flex;
    align-items: center;
    gap: 11px;
    min-height: var(--sc-row-h);
    padding: 7px 12px 7px 18px;
    border-top: 0.5px solid var(--rm-border);
  }
  .tv-row:first-child {
    border-top: 0;
  }
  .tv-row.is-link {
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .tv-row.is-link:hover {
    background: var(--rm-accent-soft);
  }
  .tv-ico,
  .tv-chev {
    width: 15px;
    height: 15px;
    fill: currentColor;
    flex: none;
    color: var(--rm-text-dim);
  }
  .tv-main {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tv-name {
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
    flex: none;
    display: inline-flex;
    align-items: center;
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .tv-row.is-link:hover .tv-actions,
  .tv-actions:focus-within {
    opacity: 1;
  }
</style>
