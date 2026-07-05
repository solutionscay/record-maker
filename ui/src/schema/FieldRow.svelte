<script lang="ts">
  // One field row in the grid (#113). Inline rename/retype/delete through the
  // store; the input is one-way bound to field.name so Svelte only rewrites it
  // when the server value changes (never mid-edit). Reorder is driven by the
  // parent via drag callbacks; this row owns the draggable handle, a full-row
  // drag ghost, and the insertion line showing where the field will land.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind, FieldView } from './types';
  import { FIELD_KINDS, kindIcon } from './types';
  import { confirmDanger } from './confirm';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    field,
    reorderable,
    active,
    dragging,
    dropBefore,
    dropAfter,
    onedit,
    ondragstartrow,
    ondragoverrow,
    ondroprow,
    ondragendrow,
  }: {
    store: SchemaStore;
    field: FieldView;
    reorderable: boolean;
    active: boolean;
    dragging: boolean;
    dropBefore: boolean;
    dropAfter: boolean;
    onedit: () => void;
    ondragstartrow: () => void;
    ondragoverrow: (pos: 'before' | 'after') => void;
    ondroprow: () => void;
    ondragendrow: () => void;
  } = $props();

  let rowEl: HTMLDivElement;

  function commitName(el: HTMLInputElement) {
    const v = el.value.trim();
    if (v && v !== field.name) void store.renameField(field.id, v);
    else el.value = field.name;
  }
  function retype(kind: string) {
    if (kind !== field.kind) void store.retypeField(field.id, kind as FieldKind);
  }
  async function remove() {
    const ok = await confirmDanger(`Delete the “${field.name}” field? This cannot be undone.`, 'Delete field');
    if (ok) void store.deleteField(field.id);
  }

  function onDragStart(e: DragEvent) {
    if (!reorderable) return;
    e.dataTransfer?.setData('text/plain', String(field.id));
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      const r = rowEl.getBoundingClientRect();
      e.dataTransfer.setDragImage(rowEl, e.clientX - r.left, e.clientY - r.top);
    }
    ondragstartrow();
  }
  function onDragOver(e: DragEvent) {
    if (!reorderable) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    const r = rowEl.getBoundingClientRect();
    ondragoverrow(e.clientY < r.top + r.height / 2 ? 'before' : 'after');
  }
  function onDrop(e: DragEvent) {
    if (!reorderable) return;
    e.preventDefault();
    ondroprow();
  }
</script>

<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
  bind:this={rowEl}
  class="fg-row"
  class:active
  class:dragging
  role="row"
  ondragover={onDragOver}
  ondrop={onDrop}
>
  {#if dropBefore}<div class="fg-dropline top"></div>{/if}
  {#if dropAfter}<div class="fg-dropline bottom"></div>{/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <span
    class="fg-handle"
    class:disabled={!reorderable}
    title={reorderable ? 'Drag to reorder' : 'Switch to “Custom order” to reorder'}
    draggable={reorderable}
    ondragstart={onDragStart}
    ondragend={ondragendrow}
    aria-hidden="true"
  >
    <svg class="fg-handle-ico" viewBox="0 0 16 16"><circle cx="6" cy="4" r="1" /><circle cx="10" cy="4" r="1" /><circle cx="6" cy="8" r="1" /><circle cx="10" cy="8" r="1" /><circle cx="6" cy="12" r="1" /><circle cx="10" cy="12" r="1" /></svg>
  </span>

  <input
    class="sc-cell fg-name"
    value={field.name}
    onblur={(e) => commitName(e.currentTarget)}
    onkeydown={(e) => {
      if (e.key === 'Enter') e.currentTarget.blur();
      else if (e.key === 'Escape') {
        e.currentTarget.value = field.name;
        e.currentTarget.blur();
      }
    }}
    aria-label="Field name"
  />

  <span class="fg-type">
    <Icon name={kindIcon(field.kind)} />
    <select class="sc-cell fg-type-select" value={field.kind} onchange={(e) => retype(e.currentTarget.value)} aria-label="Field type">
      {#each FIELD_KINDS as k (k.kind)}
        <option value={k.kind}>{k.label}</option>
      {/each}
    </select>
  </span>

  <code class="sc-phys fg-phys" title={field.phys}>{field.phys}</code>

  <span class="fg-actions">
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost fg-gear" class:on={active} title="Field details" onclick={onedit}>
      <Icon name="settings" />
    </button>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost sc-btn--danger" title="Delete field" onclick={remove}>
      <Icon name="delete" />
    </button>
  </span>
</div>

<style>
  :global(.fg-row) {
    position: relative;
    height: var(--sc-row-h);
    border-top: 0.5px solid var(--rm-border);
    transition: background 0.12s ease;
  }
  :global(.fg-row):first-of-type {
    border-top: 0;
  }
  :global(.fg-row.active) {
    background: var(--rm-accent-soft);
  }
  :global(.fg-row.dragging) {
    opacity: 0.35;
  }
  /* Insertion indicator — an accent line with an end cap on the drop boundary. */
  .fg-dropline {
    position: absolute;
    left: 8px;
    right: 8px;
    height: 2px;
    background: var(--rm-accent);
    border-radius: 2px;
    z-index: 3;
    pointer-events: none;
  }
  .fg-dropline::before {
    content: '';
    position: absolute;
    left: -3px;
    top: -2px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--rm-accent);
  }
  .fg-dropline.top {
    top: -1px;
  }
  .fg-dropline.bottom {
    bottom: -1px;
  }
  .fg-handle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 24px;
    cursor: grab;
    color: var(--rm-text-dim);
    border-radius: 5px;
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }
  .fg-handle:hover {
    background: rgba(0, 0, 0, 0.06);
    color: var(--rm-text);
  }
  .fg-handle:active {
    cursor: grabbing;
  }
  .fg-handle.disabled {
    cursor: default;
    opacity: 0.3;
  }
  .fg-handle.disabled:hover {
    background: transparent;
    color: var(--rm-text-dim);
  }
  .fg-handle-ico {
    width: 16px;
    height: 16px;
    fill: currentColor;
    opacity: 0.6;
  }
  .fg-type {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--rm-text-dim);
  }
  .fg-type :global(.icon) {
    flex: none;
  }
  .fg-type-select {
    flex: 1 1 auto;
  }
  .fg-name {
    font-weight: 500;
  }
  .fg-phys {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fg-actions {
    display: inline-flex;
    justify-content: flex-end;
    gap: 4px;
  }
  /* Row actions stay hidden until hover to keep the grid calm. */
  .fg-actions .sc-btn {
    opacity: 0;
    transition:
      opacity 0.12s ease,
      background 0.12s ease,
      color 0.12s ease;
  }
  :global(.fg-row:hover) .fg-actions .sc-btn,
  .fg-actions .sc-btn:focus-visible,
  .fg-gear.on {
    opacity: 1;
  }
  .fg-gear.on {
    background: var(--rm-accent);
    color: #fff;
  }
</style>
