<script lang="ts">
  // Level 1 of the drill-down (#113): the full-width Tables list. Click a table to
  // drill into its fields; create inline (then drill straight in); rename inline;
  // delete (with confirm). All ops go through the store, which reflects server
  // truth.
  import type { SchemaStore } from './store.svelte';
  import { confirmDanger } from './confirm';
  import Icon from '../lib/Icon.svelte';

  let { store, onopen }: { store: SchemaStore; onopen: (id: number) => void } = $props();

  // Inline "new table" row.
  let creating = $state(false);
  let newName = $state('');
  let creatingBusy = $state(false);

  // Inline rename (id being renamed + draft).
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
      onopen(table.id); // drill straight into the new table's fields
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
  <div class="tv-inner">
    <header class="tv-head">
      <h1 class="tv-title">Tables</h1>
      <button type="button" class="tv-new" onclick={startCreate} disabled={creating}>
        <Icon name="plus" />New table
      </button>
    </header>

    <div class="tv-card">
      {#if store.loading}
        <p class="tv-note">Loading tables…</p>
      {:else if store.tables.length === 0 && !creating}
        <div class="tv-empty">
          <p class="tv-empty-title">No tables yet</p>
          <p class="tv-note">Create your first table to start defining fields.</p>
          <button type="button" class="tv-empty-btn" onclick={startCreate}>
            <Icon name="plus" />New table
          </button>
        </div>
      {/if}

      {#each store.tables as table (table.id)}
        {#if renamingId === table.id}
          <div class="tv-row">
            <svg class="icon tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="tv-input"
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
            <svg class="icon tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
            <span class="tv-name">{table.name}</span>
            <code class="tv-phys">{table.phys}</code>
            <span class="tv-actions">
              <button
                type="button"
                class="tv-act"
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
                class="tv-act danger"
                title="Delete table"
                onclick={(e) => {
                  e.stopPropagation();
                  remove(table.id, table.name);
                }}
              >
                <Icon name="delete" />
              </button>
            </span>
            <svg class="icon tv-chev" aria-hidden="true"><use href="#icon-next" /></svg>
          </div>
        {/if}
      {/each}

      {#if creating}
        <div class="tv-row">
          <svg class="icon tv-ico" aria-hidden="true"><use href="#icon-app" /></svg>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="tv-input"
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
</div>

<style>
  .tv {
    height: 100%;
    min-height: 0;
    overflow: auto;
  }
  .tv-inner {
    max-width: 760px;
    margin: 0 auto;
    padding: 26px 22px 40px;
  }
  .tv-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 14px;
  }
  .tv-title {
    margin: 0;
    font-size: 21px;
    font-weight: 700;
    color: var(--rm-text);
  }
  .tv-new,
  .tv-empty-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: inherit;
    font-size: 12.5px;
    font-weight: 600;
    padding: 8px 13px;
    border: 0.5px solid transparent;
    border-radius: 8px;
    background: var(--rm-accent);
    color: #fff;
    cursor: pointer;
    box-shadow: 0 1px 3px rgba(10, 132, 255, 0.4);
  }
  .tv-new:disabled {
    background: #c7c7cc;
    box-shadow: none;
    cursor: default;
  }
  .tv-new :global(.icon),
  .tv-empty-btn :global(.icon) {
    flex: none;
  }
  .tv-card {
    background: var(--rm-control-bg);
    border: 0.5px solid var(--rm-border);
    border-radius: 12px;
    box-shadow: var(--rm-shadow-card);
    overflow: hidden;
  }
  .tv-note {
    margin: 0;
    padding: 16px;
    font-size: 13px;
    color: var(--rm-text-dim);
  }
  .tv-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 44px 24px;
    text-align: center;
  }
  .tv-empty-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .tv-empty .tv-note {
    padding: 0;
  }
  .tv-empty-btn {
    margin-top: 6px;
  }
  .tv-row {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 0 12px 0 14px;
    height: 50px;
    border-top: 0.5px solid var(--rm-border);
  }
  .tv-row:first-child {
    border-top: 0;
  }
  .tv-row.is-link {
    cursor: pointer;
  }
  .tv-row.is-link:hover {
    background: var(--rm-accent-soft);
  }
  .tv-ico {
    width: 16px;
    height: 16px;
    fill: currentColor;
    flex: none;
    color: var(--rm-text-dim);
  }
  .tv-row.is-link:hover .tv-ico {
    color: var(--rm-accent);
  }
  .tv-name {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 13.5px;
    font-weight: 600;
    color: var(--rm-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tv-phys {
    flex: none;
    font-size: 11.5px;
    color: var(--rm-text-dim);
  }
  .tv-actions {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    opacity: 0;
  }
  .tv-row.is-link:hover .tv-actions,
  .tv-actions:focus-within {
    opacity: 1;
  }
  .tv-act {
    display: inline-flex;
    align-items: center;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    padding: 4px 8px;
    border: 0.5px solid var(--rm-border);
    border-radius: 6px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    line-height: 1.4;
    cursor: pointer;
  }
  .tv-act:hover {
    background: #f0f0f2;
  }
  .tv-act.danger {
    padding: 4px 6px;
    line-height: 0;
    color: var(--rm-text-dim);
  }
  .tv-act.danger:hover {
    background: var(--rm-danger);
    color: #fff;
    border-color: transparent;
  }
  .tv-chev {
    width: 16px;
    height: 16px;
    fill: currentColor;
    flex: none;
    color: var(--rm-text-dim);
    opacity: 0.6;
  }
  .tv-input {
    flex: 1 1 auto;
    min-width: 0;
    font: inherit;
    font-size: 13.5px;
    padding: 6px 9px;
    border: 1px solid var(--rm-accent);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
  }
  .tv-input:focus {
    outline: none;
    box-shadow: 0 0 0 3px var(--rm-accent-soft);
  }
</style>
