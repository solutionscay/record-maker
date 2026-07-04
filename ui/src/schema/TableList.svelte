<script lang="ts">
  // Left pane: the table list (#113). Select a table to edit its fields; create a
  // new table inline; rename in place (double-click); delete (with confirm). All
  // ops go through the store, which reflects server truth.
  import type { SchemaStore } from './store.svelte';
  import { confirmDanger } from './confirm';
  import Icon from '../lib/Icon.svelte';

  let { store }: { store: SchemaStore } = $props();

  // Inline "new table" row state.
  let creating = $state(false);
  let newName = $state('');
  let creatingBusy = $state(false);

  // Inline rename state (id being renamed + its draft).
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
    // Keep the row open on failure (error shows in the banner) so the name isn't lost.
    if (table) creating = false;
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

<aside class="tl">
  <header class="tl-head">
    <span class="tl-title">Tables</span>
    <button type="button" class="tl-add" title="New table" onclick={startCreate} disabled={creating}>
      <Icon name="plus" />
    </button>
  </header>

  <div class="tl-list">
    {#each store.tables as table (table.id)}
      <div class="tl-row" class:active={table.id === store.selectedTableId}>
        {#if renamingId === table.id}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="tl-input"
            autofocus
            bind:value={renameDraft}
            onblur={() => commitRename(table.id)}
            onkeydown={(e) => {
              if (e.key === 'Enter') e.currentTarget.blur();
              else if (e.key === 'Escape') {
                renamingId = null;
              }
            }}
          />
        {:else}
          <button
            type="button"
            class="tl-name"
            onclick={() => store.selectTable(table.id)}
            ondblclick={() => startRename(table.id, table.name)}
            title={table.name}
          >
            <Icon name="field" />
            <span class="tl-label">{table.name}</span>
          </button>
          <button
            type="button"
            class="tl-del"
            title="Delete table"
            onclick={() => remove(table.id, table.name)}
          >
            <Icon name="delete" />
          </button>
        {/if}
      </div>
    {/each}

    {#if creating}
      <div class="tl-row">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="tl-input"
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

    {#if store.tables.length === 0 && !creating}
      <p class="tl-empty">No tables yet.</p>
    {/if}
  </div>
</aside>

<style>
  .tl {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 0.5px solid var(--rm-border);
    background: var(--rm-sidebar-bg);
  }
  .tl-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 8px 14px;
  }
  .tl-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--rm-text-dim);
  }
  .tl-add {
    display: inline-flex;
    padding: 3px;
    border: 0.5px solid var(--rm-border);
    border-radius: 6px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    line-height: 0;
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .tl-add:hover:not(:disabled) {
    background: #f0f0f2;
  }
  .tl-add:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .tl-list {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 0 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .tl-row {
    display: flex;
    align-items: center;
    border-radius: 7px;
  }
  .tl-row.active {
    background: var(--rm-accent-soft);
  }
  .tl-name {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 7px 8px;
    border: 0;
    background: transparent;
    color: var(--rm-text);
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    border-radius: 7px;
  }
  .tl-row:not(.active) .tl-name:hover {
    background: rgba(0, 0, 0, 0.04);
  }
  .tl-row.active .tl-name {
    font-weight: 600;
    color: var(--rm-accent);
  }
  .tl-name :global(.icon) {
    flex: none;
    color: var(--rm-text-dim);
  }
  .tl-row.active .tl-name :global(.icon) {
    color: var(--rm-accent);
  }
  .tl-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tl-del {
    flex: none;
    padding: 5px;
    margin-right: 3px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--rm-text-dim);
    line-height: 0;
    cursor: pointer;
    opacity: 0;
  }
  .tl-row:hover .tl-del,
  .tl-del:focus-visible {
    opacity: 1;
  }
  .tl-del:hover {
    background: var(--rm-danger);
    color: #fff;
  }
  .tl-input {
    flex: 1 1 auto;
    min-width: 0;
    margin: 2px 0;
    padding: 6px 8px;
    border: 1px solid var(--rm-accent);
    border-radius: 7px;
    font: inherit;
    font-size: 13px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
  }
  .tl-input:focus {
    outline: none;
    box-shadow: 0 0 0 3px var(--rm-accent-soft);
  }
  .tl-empty {
    margin: 8px;
    font-size: 12px;
    color: var(--rm-text-dim);
  }
</style>
