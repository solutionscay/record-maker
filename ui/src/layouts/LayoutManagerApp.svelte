<script lang="ts">
  // Layout Manager (#149/#151). Two kinds of layout:
  //  • Default — the Form/List/Table trio every table is born with (#57).
  //    Grouped one row per table; the enabled views show as pills and are edited
  //    in a drawer of switches (enable/disable, never delete). At least one view
  //    per table stays on (server-guarded).
  //  • Custom — anything made via "New layout". A standalone single-view
  //    layout, freely renamed / deleted.
  // Both groups drag-reorder from their row handle only. Actions commit
  // immediately (no draft/save step), like the Tables tab. Structure mirrors the
  // schema builder: titled header, scrolling body, fixed footer with Done.
  import { onMount } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import NewLayoutDrawer from './NewLayoutDrawer.svelte';
  import DefaultViewsDrawer from './DefaultViewsDrawer.svelte';
  import CustomLayoutDrawer from './CustomLayoutDrawer.svelte';
  import {
    listLayouts,
    listTables,
    reorderLayouts,
    type LayoutManagerView,
    type TableOption,
  } from './persist';

  let layouts = $state<LayoutManagerView[]>([]);
  let tables = $state<TableOption[]>([]);
  let loading = $state(true);
  let error = $state('');
  let newOpen = $state(false);
  let editingTableId = $state<number | null>(null);
  let editingCustomId = $state<number | null>(null);
  let dragId = $state<number | null>(null);
  let dragTableId = $state<number | null>(null);

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

  // The group the "Edit views" drawer is editing, re-derived from live layouts so
  // its switches reflect each immediate toggle.
  const editingGroup = $derived(
    editingTableId === null ? null : (defaultGroups.find((g) => g.tableId === editingTableId) ?? null),
  );

  // The custom layout the "Edit layout" drawer is editing.
  const editingCustom = $derived(
    editingCustomId === null ? null : (customs.find((l) => l.id === editingCustomId) ?? null),
  );

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

  // Patch one view in place (from the Edit-views drawer's immediate toggle).
  function viewUpdated(updated: LayoutManagerView) {
    layouts = layouts.map((x) => (x.id === updated.id ? updated : x));
  }

  // Patch a renamed custom layout in place (from the Edit-layout drawer).
  function renamed(updated: LayoutManagerView) {
    layouts = layouts.map((l) => (l.id === updated.id ? updated : l));
  }

  // Drop a deleted custom layout and close its drawer.
  function deleted(id: number) {
    layouts = layouts.filter((x) => x.id !== id);
    editingCustomId = null;
  }

  function created(l: LayoutManagerView) {
    layouts = [...layouts, l];
    newOpen = false;
  }

  // ── drag-to-reorder (handle-initiated only) ──────────────────────────────
  // The drag starts on the .lm-handle (draggable), never the whole row. Use the
  // parent row as the drag image so the ghost is the full row, not the grip.
  function rowDragImage(e: DragEvent) {
    const row = (e.currentTarget as HTMLElement).closest('.lm-row') as HTMLElement | null;
    if (row && e.dataTransfer) e.dataTransfer.setDragImage(row, 16, Math.min(20, row.offsetHeight / 2));
  }

  function onCustomDragStart(id: number, e: DragEvent) {
    dragId = id;
    e.dataTransfer?.setData('text/plain', `c:${id}`);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
    rowDragImage(e);
  }
  function onCustomDragOver(overId: number, e: DragEvent) {
    e.preventDefault();
    if (dragId === null || dragId === overId) return;
    const order = customs.slice();
    const from = order.findIndex((l) => l.id === dragId);
    const to = order.findIndex((l) => l.id === overId);
    if (from === -1 || to === -1) return;
    const [moved] = order.splice(from, 1);
    order.splice(to, 0, moved);
    // Reassemble: defaults keep their order, customs take the new one.
    layouts = [...layouts.filter((l) => l.isDefault), ...order];
  }

  function onDefaultDragStart(tableId: number, e: DragEvent) {
    dragTableId = tableId;
    e.dataTransfer?.setData('text/plain', `d:${tableId}`);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
    rowDragImage(e);
  }
  function onDefaultDragOver(overTableId: number, e: DragEvent) {
    e.preventDefault();
    if (dragTableId === null || dragTableId === overTableId) return;
    const order = defaultGroups.map((g) => g.tableId);
    const from = order.indexOf(dragTableId);
    const to = order.indexOf(overTableId);
    if (from === -1 || to === -1) return;
    const [moved] = order.splice(from, 1);
    order.splice(to, 0, moved);
    // Each table's trio moves as a contiguous block; customs keep their order.
    const reordered = order.flatMap((tid) => layouts.filter((l) => l.isDefault && l.tableId === tid));
    layouts = [...reordered, ...layouts.filter((l) => !l.isDefault)];
  }

  async function onDragEnd() {
    dragId = null;
    dragTableId = null;
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
      <span class="lm-apptitle">Manage Layouts</span>
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
      <button type="button" class="lm-x" title="Close" aria-label="Close" onclick={done}>
        <Icon name="close" />
      </button>
    </div>
  </header>

  <div class="lm-body">
    {#if loading}
      <p class="sc-note sc-hint">Loading layouts...</p>
    {:else if layouts.length === 0}
      <div class="sc-empty lm-empty">
        <p class="sc-empty-title">No layouts yet</p>
        <p class="sc-hint">Create a table first, then add layouts for it here.</p>
      </div>
    {:else}
      <!-- Default layouts: one row per table, views as pills, edited in a drawer. -->
      <div class="lm-section">Default layouts</div>
      <div class="lm-colhead sc-colhead">
        <span aria-hidden="true"></span>
        <span class="sc-micro">Table</span>
        <span class="sc-micro">Views</span>
        <span aria-hidden="true"></span>
        <span aria-hidden="true"></span>
      </div>
      {#each defaultGroups as group (group.tableId)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="lm-row"
          class:dragging={dragTableId === group.tableId}
          ondragover={(e) => onDefaultDragOver(group.tableId, e)}
        >
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            class="lm-handle"
            title="Drag to reorder"
            draggable="true"
            ondragstart={(e) => onDefaultDragStart(group.tableId, e)}
            ondragend={onDragEnd}
          ></span>
          <button type="button" class="lm-name" onclick={() => openDefault(group)} title="Open in Layout Mode">
            {group.tableName}
          </button>
          <span class="lm-pills">
            {#each group.views as v (v.id)}
              <span class="lm-pill" class:off={!v.enabled}>{viewLabel(v.view)}</span>
            {/each}
          </span>
          <span aria-hidden="true"></span>
          <span class="lm-actions">
            <button
              type="button"
              class="sc-btn sc-btn--icon sc-btn--ghost"
              title="Edit views"
              onclick={() => (editingTableId = group.tableId)}
            >
              <Icon name="edit" />
            </button>
          </span>
        </div>
      {/each}

      <!-- Custom layouts: standalone, renamable, deletable, reorderable. -->
      {#if customs.length > 0}
        <div class="lm-section">Custom layouts</div>
        <div class="lm-colhead sc-colhead">
          <span aria-hidden="true"></span>
          <span class="sc-micro">Layout Name</span>
          <span class="sc-micro">View</span>
          <span class="sc-micro">Associated Table</span>
          <span aria-hidden="true"></span>
        </div>
        {#each customs as l (l.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="lm-row"
            class:dragging={dragId === l.id}
            ondragover={(e) => onCustomDragOver(l.id, e)}
          >
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <span
              class="lm-handle"
              title="Drag to reorder"
              draggable="true"
              ondragstart={(e) => onCustomDragStart(l.id, e)}
              ondragend={onDragEnd}
            ></span>
            <button type="button" class="lm-name" onclick={() => openLayout(l.id)} title="Open in Layout Mode">
              {l.name}
            </button>
            <span class="lm-view">{viewLabel(l.view)}</span>
            <span class="lm-table">{l.tableName}</span>
            <span class="lm-actions">
              <button
                type="button"
                class="sc-btn sc-btn--icon sc-btn--ghost"
                title="Edit layout"
                onclick={() => (editingCustomId = l.id)}
              >
                <Icon name="edit" />
              </button>
            </span>
          </div>
        {/each}
      {/if}
    {/if}
  </div>

  {#if error}
    <div class="lm-error" role="alert">
      <svg class="lm-error-ico" aria-hidden="true"><use href="#icon-find" /></svg>
      <span>{error}</span>
      <button type="button" class="lm-error-x" title="Dismiss" onclick={() => (error = '')}>
        <svg class="lm-error-ico" aria-hidden="true"><use href="#icon-minus" /></svg>
      </button>
    </div>
  {/if}

  {#if newOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="lm-scrim" onclick={() => (newOpen = false)}></div>
    <NewLayoutDrawer {tables} onclose={() => (newOpen = false)} oncreate={created} />
  {:else if editingGroup}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="lm-scrim" onclick={() => (editingTableId = null)}></div>
    <DefaultViewsDrawer
      tableName={editingGroup.tableName}
      views={editingGroup.views}
      onclose={() => (editingTableId = null)}
      onupdated={viewUpdated}
    />
  {:else if editingCustom}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="lm-scrim" onclick={() => (editingCustomId = null)}></div>
    <CustomLayoutDrawer
      layout={editingCustom}
      onclose={() => (editingCustomId = null)}
      onrenamed={renamed}
      ondeleted={deleted}
    />
  {/if}
</div>

<style>
  .lm {
    position: relative;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--rm-control-bg);
  }
  .lm-head {
    flex: none;
  }
  .lm-title {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .lm-apptitle {
    font-size: 13px;
    font-weight: 700;
    color: var(--rm-text);
  }
  .lm-head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  /* Upper-right close (X) — dismisses the pane, like the classic Manage dialogs. */
  .lm-x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: var(--rm-radius);
    background: var(--rm-control-bg);
    color: var(--rm-text-dim);
    cursor: pointer;
    box-shadow: var(--rm-elev-1);
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .lm-x:hover {
    color: var(--rm-text);
    border-color: var(--rm-border-strong);
  }
  .lm-x :global(.icon) {
    width: 15px;
    height: 15px;
  }
  .lm-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
  }

  /* Floating error toast — same pattern as the schema builder's .sb-error. */
  .lm-error {
    position: fixed;
    left: 50%;
    bottom: 24px;
    transform: translateX(-50%);
    z-index: 60;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: min(38rem, calc(100vw - 2rem));
    padding: 9px 10px 9px 14px;
    border-radius: 9px;
    background: var(--rm-danger);
    color: #fff;
    font-size: 12.5px;
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.22);
  }
  .lm-error-ico {
    width: 1em;
    height: 1em;
    fill: currentColor;
    flex: none;
  }
  .lm-error-x {
    margin-left: 4px;
    padding: 2px;
    border: 0;
    border-radius: 0;
    background: rgba(255, 255, 255, 0.18);
    color: #fff;
    line-height: 0;
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .lm-error-x:hover {
    background: rgba(255, 255, 255, 0.3);
  }

  /* Section divider (Default / Custom). */
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

  /* One shared column grid for BOTH groups, so their columns line up:
     handle · name · view(s) · associated table · actions. */
  .lm-colhead,
  .lm-row {
    display: grid;
    grid-template-columns: 24px minmax(0, 1.4fr) minmax(0, 1.7fr) minmax(0, 1.3fr) 72px;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 14px;
  }
  /* the colhead's sticky/height/border/bg come from .sc-colhead */
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
  .lm-handle:active {
    cursor: grabbing;
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

  /* Default-row view pills — static display of enabled/disabled state; editing
     happens in the Edit-views drawer (the pencil). */
  .lm-pills {
    display: flex;
    gap: 6px;
  }
  .lm-pill {
    height: 22px;
    padding: 0 10px;
    display: inline-flex;
    align-items: center;
    border-radius: 11px;
    border: 0.5px solid transparent;
    background: var(--rm-accent);
    color: #fff;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .lm-pill.off {
    background: var(--rm-control-bg);
    border-color: var(--rm-border);
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

  .lm-scrim {
    position: absolute;
    inset: 0;
    z-index: 15;
    background: rgba(20, 22, 28, 0.14);
  }
</style>
