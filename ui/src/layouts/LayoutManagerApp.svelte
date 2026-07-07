<script lang="ts">
  // Layout Manager (#149/#151). Two kinds of layout:
  //  • Default — the Form/List/Table trio every table is born with (#57).
  //    Grouped one row per table; each view is a toggle chip (enable/disable,
  //    never delete). At least one view per table stays on (server-guarded).
  //  • Custom — anything made via "New layout". A standalone single-view
  //    layout, freely renamed / deleted / drag-reordered.
  // Actions commit immediately (no draft/save step), like the Tables tab.
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
    setLayoutEnabled,
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

  const VIEW_ORDER: Record<string, number> = { form: 0, list: 1, table: 2 };

  // Default layouts grouped one row per table, views in Form/List/Table order.
  type DefaultGroup = { tableId: number; tableName: string; views: LayoutManagerView[] };
  const defaultGroups = $derived.by<DefaultGroup[]>(() => {
    const byTable = new Map<number, DefaultGroup>();
    for (const l of layouts) {
      if (!l.isDefault) continue;
      let g = byTable.get(l.tableId);
      if (!g) {
        g = { tableId: l.tableId, tableName: l.tableName, views: [] };
        byTable.set(l.tableId, g);
      }
      g.views.push(l);
    }
    for (const g of byTable.values()) {
      g.views.sort((a, b) => (VIEW_ORDER[a.view] ?? 9) - (VIEW_ORDER[b.view] ?? 9));
    }
    return [...byTable.values()];
  });

  // Custom layouts, in the global drag order.
  const customs = $derived(layouts.filter((l) => !l.isDefault));

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

  // Open a table's default layout for design — its first enabled view, so the
  // Layout-mode view tabs land somewhere real.
  function openDefault(group: DefaultGroup) {
    const target = group.views.find((v) => v.enabled) ?? group.views[0];
    if (target) openLayout(target.id);
  }

  function viewLabel(view: string): string {
    if (view === 'form') return 'Form';
    if (view === 'list') return 'List';
    return 'Table';
  }

  async function toggleView(l: LayoutManagerView) {
    error = '';
    try {
      const updated = await setLayoutEnabled(l.id, !l.enabled);
      layouts = layouts.map((x) => (x.id === l.id ? updated : x));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
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

  // ── drag-to-reorder (custom layouts only) ────────────────────────────────
  function onDragStart(id: number, e: DragEvent) {
    dragId = id;
    e.dataTransfer?.setData('text/plain', String(id));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDragOver(overId: number, e: DragEvent) {
    e.preventDefault();
    if (dragId === null || dragId === overId) return;
    const order = customs.slice();
    const from = order.findIndex((l) => l.id === dragId);
    const to = order.findIndex((l) => l.id === overId);
    if (from === -1 || to === -1) return;
    const [moved] = order.splice(from, 1);
    order.splice(to, 0, moved);
    // Reassemble the full list: defaults keep their order, customs take the new one.
    layouts = [...layouts.filter((l) => l.isDefault), ...order];
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
      <span class="sc-count">
        {defaultGroups.length} {defaultGroups.length === 1 ? 'table' : 'tables'}{customs.length > 0
          ? ` · ${customs.length} custom`
          : ''}
      </span>
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
      <!-- Default layouts: one row per table, views as toggle chips. -->
      <div class="lm-colhead lm-colhead--default sc-colhead">
        <span class="sc-micro">Table</span>
        <span class="sc-micro">Views</span>
        <span aria-hidden="true"></span>
      </div>
      {#each defaultGroups as group (group.tableId)}
        <div class="lm-row lm-row--default">
          <button
            type="button"
            class="lm-name"
            onclick={() => openDefault(group)}
            title="Open in Layout Mode"
          >
            {group.tableName}
          </button>
          <span class="lm-chips">
            {#each group.views as v (v.id)}
              <button
                type="button"
                class="lm-chip"
                class:on={v.enabled}
                onclick={() => toggleView(v)}
                title={v.enabled ? `Disable ${viewLabel(v.view)} view` : `Enable ${viewLabel(v.view)} view`}
                aria-pressed={v.enabled}
              >
                {viewLabel(v.view)}
              </button>
            {/each}
          </span>
          <span class="lm-hint">default layout</span>
        </div>
      {/each}

      <!-- Custom layouts: standalone, renamable, deletable, reorderable. -->
      {#if customs.length > 0}
        <div class="lm-section">Custom layouts</div>
        <div class="lm-colhead lm-colhead--custom sc-colhead">
          <span aria-hidden="true"></span>
          <span class="sc-micro">Layout Name</span>
          <span class="sc-micro">View</span>
          <span class="sc-micro">Associated Table</span>
          <span aria-hidden="true"></span>
        </div>
        {#each customs as l (l.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="lm-row lm-row--custom"
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
            <span class="lm-view">{viewLabel(l.view)}</span>
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

  /* Section divider between defaults and customs. */
  .lm-section {
    padding: 9px 18px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--rm-text-dim);
    background: var(--rm-toolbar-bg);
    border-top: 0.5px solid var(--rm-border);
    border-bottom: 0.5px solid var(--rm-border);
  }

  .lm-colhead,
  .lm-row {
    display: grid;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 18px;
  }
  .lm-colhead--default,
  .lm-row--default {
    grid-template-columns: minmax(0, 1.1fr) minmax(0, 2fr) 110px;
  }
  .lm-colhead--custom,
  .lm-row--custom {
    grid-template-columns: 24px minmax(0, 1.4fr) 72px minmax(0, 1.4fr) 68px;
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

  /* Toggle chips (default rows). */
  .lm-chips {
    display: flex;
    gap: 6px;
  }
  .lm-chip {
    height: 22px;
    padding: 0 10px;
    border-radius: 11px;
    border: 0.5px solid var(--rm-border);
    background: var(--rm-control-bg);
    color: var(--rm-text-dim);
    font: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.02em;
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .lm-chip:hover {
    border-color: var(--rm-accent);
  }
  .lm-chip.on {
    background: var(--rm-accent);
    border-color: transparent;
    color: #fff;
  }
  .lm-hint {
    justify-self: end;
    font-size: 11px;
    color: var(--rm-text-dim);
  }

  /* Custom-row cells. */
  .lm-view {
    justify-self: start;
    height: 16px;
    padding: 1px 6px 0;
    border-radius: 4px;
    background: rgba(10, 132, 255, 0.12);
    color: var(--rm-accent);
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
    line-height: 15px;
    white-space: nowrap;
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
