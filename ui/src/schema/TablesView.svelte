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

  let dragId = $state<number | null>(null);
  let overId = $state<number | null>(null);
  let overPos = $state<'before' | 'after'>('before');

  function onDragStart(id: number) {
    dragId = id;
  }
  function onDragOver(id: number, pos: 'before' | 'after') {
    if (dragId == null) return;
    if (id !== overId) overId = id;
    if (pos !== overPos) overPos = pos;
  }
  function onDragEnd() {
    dragId = null;
    overId = null;
  }
  function onDrop() {
    const from = dragId;
    const target = overId;
    const pos = overPos;
    dragId = null;
    overId = null;
    if (from == null || target == null || from === target) return;

    const ids = store.tables.map((t) => t.id);
    const fromIdx = ids.indexOf(from);
    if (fromIdx < 0) return;
    ids.splice(fromIdx, 1);
    let targetIdx = ids.indexOf(target);
    if (targetIdx < 0) targetIdx = ids.length - 1;
    ids.splice(pos === 'after' ? targetIdx + 1 : targetIdx, 0, from);
    store.reorderTables(ids);
  }
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
      {@const isDragging = dragId === table.id}
      {@const isDropBefore = overId === table.id && overPos === 'before' && dragId != null && dragId !== table.id}
      {@const isDropAfter = overId === table.id && overPos === 'after' && dragId != null && dragId !== table.id}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="tv-row"
        class:dragging={isDragging}
        ondragover={(e) => {
          e.preventDefault();
          if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
          const r = e.currentTarget.getBoundingClientRect();
          onDragOver(table.id, e.clientY < r.top + r.height / 2 ? 'before' : 'after');
        }}
        ondrop={(e) => {
          e.preventDefault();
          onDrop();
        }}
      >
        {#if isDropBefore}<div class="tv-dropline top"></div>{/if}
        {#if isDropAfter}<div class="tv-dropline bottom"></div>{/if}

        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span
          class="tv-handle"
          title="Drag to reorder"
          draggable="true"
          ondragstart={(e) => {
            e.dataTransfer?.setData('text/plain', String(table.id));
            if (e.dataTransfer) {
              e.dataTransfer.effectAllowed = 'move';
              const rowEl = e.currentTarget.closest('.tv-row');
              if (rowEl) {
                const r = rowEl.getBoundingClientRect();
                e.dataTransfer.setDragImage(rowEl, e.clientX - r.left, e.clientY - r.top);
              }
            }
            onDragStart(table.id);
          }}
          ondragend={onDragEnd}
          onclick={(e) => e.stopPropagation()}
          aria-hidden="true"
        >
          <svg class="tv-handle-ico" viewBox="0 0 16 16"><circle cx="6" cy="4" r="1" /><circle cx="10" cy="4" r="1" /><circle cx="6" cy="8" r="1" /><circle cx="10" cy="8" r="1" /><circle cx="6" cy="12" r="1" /><circle cx="10" cy="12" r="1" /></svg>
        </span>

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
  .tv-colhead,
  .tv-row {
    display: grid;
    grid-template-columns: 34px minmax(0, 1.35fr) minmax(0, 1.9fr) 34px 28px;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 18px;
  }
  .tv-row {
    position: relative;
    min-height: var(--sc-row-h);
    border-bottom: 0.5px solid var(--rm-border);
    transition: background 0.12s ease;
  }
  .tv-row:hover {
    background: rgba(0, 0, 0, 0.02);
  }
  .tv-row.dragging {
    opacity: 0.35;
  }
  .tv-handle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 24px;
    width: 24px;
    cursor: grab;
    color: var(--rm-text-dim);
    border-radius: 5px;
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }
  .tv-handle:hover {
    background: rgba(0, 0, 0, 0.06);
    color: var(--rm-text);
  }
  .tv-handle-ico {
    width: 16px;
    height: 16px;
    fill: currentColor;
    opacity: 0.6;
  }
  .tv-dropline {
    position: absolute;
    left: 8px;
    right: 8px;
    height: 2px;
    background: var(--rm-accent);
    border-radius: 2px;
    z-index: 3;
    pointer-events: none;
  }
  .tv-dropline::before {
    content: '';
    position: absolute;
    left: -3px;
    top: -2px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--rm-accent);
  }
  .tv-dropline.top {
    top: -1px;
  }
  .tv-dropline.bottom {
    bottom: -1px;
  }
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
