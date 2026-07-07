<script lang="ts">
  // Layout Manager (#149): a flat, reorderable list of every layout in the
  // solution. Actions commit immediately (no draft/save step, unlike the
  // schema builder) — this is closer to the Tables tab than to a field drawer.
  import { onMount } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import { confirmDanger } from './confirm';
  import NewLayoutDrawer from './NewLayoutDrawer.svelte';
  import {
    deleteLayout,
    listLayouts,
    listTables,
    renameLayout,
    reorderLayouts,
    type LayoutManagerView,
    type TableOption,
  } from './persist';

  let layouts = $state<LayoutManagerView[]>([]);
  let tables = $state<TableOption[]>([]);
  let loading = $state(true);
  let error = $state('');
  let newOpen = $state(false);
  let renamingId = $state<number | null>(null);
  let renameValue = $state('');
  let dragId = $state<number | null>(null);

  onMount(load);

  async function load() {
    loading = true;
    try {
      const [ls, ts] = await Promise.all([listLayouts(), listTables()]);
      layouts = ls;
      tables = ts;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function done() {
    window.location.href = '/';
  }

  function openLayout(id: number) {
    window.location.href = `/design/${id}`;
  }

  function startRename(l: LayoutManagerView) {
    renamingId = l.id;
    renameValue = l.name;
  }

  async function commitRename(id: number) {
    const name = renameValue.trim();
    renamingId = null;
    if (!name) return;
    try {
      const updated = await renameLayout(id, name);
      layouts = layouts.map((l) => (l.id === id ? updated : l));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function remove(l: LayoutManagerView) {
    const ok = await confirmDanger(`Delete layout "${l.name}"? This cannot be undone.`, 'Delete layout');
    if (!ok) return;
    try {
      await deleteLayout(l.id);
      layouts = layouts.filter((x) => x.id !== l.id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function created(l: LayoutManagerView) {
    layouts = [...layouts, l];
    newOpen = false;
  }

  // ── drag-to-reorder ──────────────────────────────────────────────────────
  function onDragStart(id: number, e: DragEvent) {
    dragId = id;
    e.dataTransfer?.setData('text/plain', String(id));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDragOver(overId: number, e: DragEvent) {
    e.preventDefault();
    if (dragId === null || dragId === overId) return;
    const from = layouts.findIndex((l) => l.id === dragId);
    const to = layouts.findIndex((l) => l.id === overId);
    if (from === -1 || to === -1) return;
    const next = layouts.slice();
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    layouts = next;
  }

  async function onDragEnd() {
    dragId = null;
    try {
      layouts = await reorderLayouts(layouts.map((l) => l.id));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      await load();
    }
  }
</script>

<div class="lm">
  <header class="sc-viewhead lm-head">
    <div class="lm-title">
      <span class="sc-micro">Layouts</span>
      <span class="sc-count">{layouts.length} defined</span>
    </div>
    <div class="lm-head-actions">
      <button
        type="button"
        class="sc-btn sc-btn--primary"
        onclick={() => (newOpen = true)}
        disabled={tables.length === 0}
        title={tables.length === 0 ? 'Create a table first' : ''}
      >
        <Icon name="plus" />New layout
      </button>
      <button type="button" class="sc-btn" onclick={done}>Done</button>
    </div>
  </header>

  {#if error}
    <p class="sc-note lm-error">{error}</p>
  {/if}

  <div class="lm-list">
    {#if loading}
      <p class="sc-note sc-hint">Loading layouts...</p>
    {:else if layouts.length === 0}
      <div class="sc-empty lm-empty">
        <p class="sc-empty-title">No layouts yet</p>
        <p class="sc-hint">Create a table first, then add layouts for it here.</p>
      </div>
    {:else}
      <div class="lm-colhead sc-colhead">
        <span aria-hidden="true"></span>
        <span class="sc-micro">Layout Name</span>
        <span class="sc-micro">Associated Table</span>
        <span aria-hidden="true"></span>
      </div>
      {#each layouts as l (l.id)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="lm-row"
          class:dragging={dragId === l.id}
          draggable="true"
          ondragstart={(e) => onDragStart(l.id, e)}
          ondragover={(e) => onDragOver(l.id, e)}
          ondragend={onDragEnd}
        >
          <span class="lm-handle" title="Drag to reorder" aria-hidden="true"></span>
          {#if renamingId === l.id}
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="sc-cell lm-rename"
              bind:value={renameValue}
              onblur={() => commitRename(l.id)}
              onkeydown={(e) => {
                if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur();
                if (e.key === 'Escape') renamingId = null;
              }}
              autofocus
            />
          {:else}
            <button type="button" class="lm-name" onclick={() => openLayout(l.id)} title="Open in Layout Mode">
              {l.name}
            </button>
          {/if}
          <span class="lm-table">{l.tableName}</span>
          <span class="lm-actions">
            <button
              type="button"
              class="sc-btn sc-btn--icon sc-btn--ghost"
              title="Rename"
              onclick={() => startRename(l)}
            >
              <Icon name="edit" />
            </button>
            <button
              type="button"
              class="sc-btn sc-btn--icon sc-btn--ghost sc-btn--danger"
              title="Delete"
              onclick={() => remove(l)}
            >
              <Icon name="delete" />
            </button>
          </span>
        </div>
      {/each}
    {/if}
  </div>
</div>

{#if newOpen}
  <NewLayoutDrawer {tables} onclose={() => (newOpen = false)} oncreate={created} />
{/if}

<style>
  .lm {
    position: relative;
    height: 100%;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .lm-head {
    position: sticky;
    top: 0;
    z-index: 2;
  }
  .lm-title {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .lm-head-actions {
    display: flex;
    gap: 8px;
  }
  .lm-error {
    color: var(--rm-danger);
  }
  .lm-list {
    flex: 1 1 auto;
    min-height: 0;
  }
  .lm-colhead,
  .lm-row {
    display: grid;
    grid-template-columns: 24px minmax(0, 1.4fr) minmax(0, 1.4fr) 68px;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 18px;
  }
  .lm-row {
    min-height: var(--sc-row-h);
    border-bottom: 0.5px solid var(--rm-border);
    transition: background 0.12s ease;
  }
  .lm-row:hover {
    background: rgba(0, 0, 0, 0.02);
  }
  .lm-row.dragging {
    opacity: 0.4;
  }
  .lm-handle {
    width: 10px;
    height: 16px;
    flex: none;
    cursor: grab;
    background-image: radial-gradient(circle, var(--rm-text-dim) 1.1px, transparent 1.2px);
    background-size: 4px 4px;
    background-repeat: repeat;
  }
  .lm-name {
    min-width: 0;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 0;
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .lm-name:hover {
    color: var(--rm-accent);
    text-decoration: underline;
  }
  .lm-rename {
    min-width: 0;
  }
  .lm-table {
    min-width: 0;
    font-size: 12px;
    color: var(--rm-text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
  }
</style>
