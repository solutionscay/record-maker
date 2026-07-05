<script lang="ts">
  // The Tables tab (#113): a full-width list. Click a table to open its fields;
  // create inline (then open it); rename inline; delete (with confirm). All ops go
  // through the store, which reflects server truth.
  import type { SchemaStore } from './store.svelte';
  import { confirmDanger } from './confirm';
  import Icon from '../lib/Icon.svelte';

  let { store, onopen }: { store: SchemaStore; onopen: (id: number) => void } = $props();

  let creating = $state(false);
  let newName = $state('');
  let creatingBusy = $state(false);

  let renamingId = $state<number | null>(null);
  let renameDraft = $state('');

  function startCreate() {
    creating = true;
    newName = '';
  }
  async function commitCreate() {
    const name = newName.trim();
    if (!name) {
      creating = false;
      return;
    }
    creatingBusy = true;
    const table = await store.createTable(name);
    creatingBusy = false;
    if (table) {
      creating = false;
      onopen(table.id);
    }
  }
  function cancelCreate() {
    creating = false;
    newName = '';
  }

  function startRename(id: number, current: string) {
    renamingId = id;
    renameDraft = current;
  }
  async function commitRename(id: number) {
    const name = renameDraft.trim();
    renamingId = null;
    if (name) await store.renameTable(id, name);
  }

  async function remove(id: number, name: string) {
    const ok = await confirmDanger(
      `Delete the “${name}” table and all its fields and records? This cannot be undone.`,
      'Delete table',
    );
    if (ok) await store.deleteTable(id);
  }
</script>

<div class="tv">
  <header class="tv-head">
    <span class="sc-micro">Tables</span>
    <button type="button" class="sc-btn sc-btn--primary" onclick={startCreate} disabled={creating}>
      <Icon name="plus" />New table
    </button>
  </header>

  <div class="tv-list">
    {#if store.loading}
      <p class="tv-note sc-hint">Loading tables…</p>
    {:else if store.tables.length === 0 && !creating}
      <div class="tv-empty">
        <p class="tv-empty-title">No tables yet</p>
        <p class="sc-hint">Create your first table to start defining fields.</p>
        <button type="button" class="sc-btn sc-btn--primary" onclick={startCreate}>
          <Icon name="plus" />New table
        </button>
      </div>
    {/if}

    {#each store.tables as table (table.id)}
      {#if renamingId === table.id}
        <div class="tv-row">
          <svg class="tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="sc-input tv-rename"
            autofocus
            bind:value={renameDraft}
            onblur={() => commitRename(table.id)}
            onkeydown={(e) => {
              if (e.key === 'Enter') e.currentTarget.blur();
              else if (e.key === 'Escape') renamingId = null;
            }}
          />
        </div>
      {:else}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="tv-row is-link" onclick={() => onopen(table.id)}>
          <svg class="tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
          <span class="tv-name">{table.name}</span>
          <code class="sc-phys">{table.phys}</code>
          <span class="tv-actions">
            <button
              type="button"
              class="sc-btn sc-btn--ghost tv-rename-btn"
              title="Rename table"
              onclick={(e) => {
                e.stopPropagation();
                startRename(table.id, table.name);
              }}
            >
              Rename
            </button>
            <button
              type="button"
              class="sc-btn sc-btn--icon sc-btn--ghost sc-btn--danger"
              title="Delete table"
              onclick={(e) => {
                e.stopPropagation();
                remove(table.id, table.name);
              }}
            >
              <Icon name="delete" />
            </button>
          </span>
          <svg class="tv-chev" aria-hidden="true"><use href="#icon-next" /></svg>
        </div>
      {/if}
    {/each}

    {#if creating}
      <div class="tv-row">
        <svg class="tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="sc-input tv-rename"
          autofocus
          placeholder="Table name"
          bind:value={newName}
          disabled={creatingBusy}
          onblur={commitCreate}
          onkeydown={(e) => {
            if (e.key === 'Enter') e.currentTarget.blur();
            else if (e.key === 'Escape') cancelCreate();
          }}
        />
      </div>
    {/if}
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
    height: var(--sc-row-h);
    padding: 0 12px 0 18px;
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
  .tv-ico {
    width: 15px;
    height: 15px;
    fill: currentColor;
    flex: none;
    color: var(--rm-text-dim);
    transition: color 0.12s ease;
  }
  .tv-row.is-link:hover .tv-ico {
    color: var(--rm-accent);
  }
  .tv-name {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tv-actions {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .tv-row.is-link:hover .tv-actions,
  .tv-actions:focus-within {
    opacity: 1;
  }
  /* The ghost "Rename" reads as a text link, not a chip. */
  .tv-rename-btn {
    padding: 0 8px;
    font-weight: 500;
  }
  .tv-chev {
    width: 15px;
    height: 15px;
    fill: currentColor;
    flex: none;
    color: var(--rm-text-dim);
    opacity: 0.55;
  }
  .tv-rename {
    max-width: 280px;
  }
</style>
