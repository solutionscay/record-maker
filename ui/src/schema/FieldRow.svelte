<script lang="ts">
  // One field row in the grid (#113). Isolates its own inline-rename buffer (so a
  // store update elsewhere can't clobber mid-edit) and commits rename/retype/
  // delete through the store. Reorder is driven by the parent via drag callbacks;
  // this row owns the draggable handle, a full-row drag ghost, and the insertion
  // line that shows exactly where the field will land.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind, FieldView } from './types';
  import { FIELD_KINDS, kindIcon } from './types';
  import { confirmDanger } from './confirm';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    field,
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

  // Inline rename: one-way bound to `field.name`, so Svelte only rewrites the
  // input when the server value changes — it never clobbers what's being typed.
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
    e.dataTransfer?.setData('text/plain', String(field.id));
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Drag the WHOLE row as the ghost (not just the little handle), aligned to
      // the cursor — so it's obvious what's moving.
      const r = rowEl.getBoundingClientRect();
      e.dataTransfer.setDragImage(rowEl, e.clientX - r.left, e.clientY - r.top);
    }
    ondragstartrow();
  }
  function onDragOver(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    // Insert before or after this row depending on which half the cursor is over.
    const r = rowEl.getBoundingClientRect();
    ondragoverrow(e.clientY < r.top + r.height / 2 ? 'before' : 'after');
  }
  function onDrop(e: DragEvent) {
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
    title="Drag to reorder"
    draggable="true"
    ondragstart={onDragStart}
    ondragend={ondragendrow}
    aria-hidden="true"
  >
    <svg class="icon" viewBox="0 0 16 16"><circle cx="6" cy="4" r="1" /><circle cx="10" cy="4" r="1" /><circle cx="6" cy="8" r="1" /><circle cx="10" cy="8" r="1" /><circle cx="6" cy="12" r="1" /><circle cx="10" cy="12" r="1" /></svg>
  </span>

  <input
    class="fg-name"
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
    <select
      class="fg-type-select"
      value={field.kind}
      onchange={(e) => retype(e.currentTarget.value)}
      aria-label="Field type"
    >
      {#each FIELD_KINDS as k (k.kind)}
        <option value={k.kind}>{k.label}</option>
      {/each}
    </select>
  </span>

  <code class="fg-phys" title={field.phys}>{field.phys}</code>

  <span class="fg-actions">
    <button type="button" class="fg-act" class:on={active} title="Field details" onclick={onedit}>
      <Icon name="settings" />
    </button>
    <button type="button" class="fg-act danger" title="Delete field" onclick={remove}>
      <Icon name="delete" />
    </button>
  </span>
</div>

<style>
  :global(.fg-row) {
    position: relative;
    height: 46px;
    border-top: 0.5px solid var(--rm-border);
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
  /* Insertion indicator — a crisp accent line with an end cap, sitting on the
     boundary the dragged field will drop onto. */
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
    cursor: grab;
    color: var(--rm-text-dim);
    border-radius: 5px;
  }
  .fg-handle:hover {
    background: rgba(0, 0, 0, 0.06);
    color: var(--rm-text);
  }
  .fg-handle:active {
    cursor: grabbing;
  }
  .fg-handle .icon {
    width: 16px;
    height: 16px;
    fill: currentColor;
    opacity: 0.6;
  }
  .fg-name {
    min-width: 0;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    padding: 6px 8px;
    border: 1px solid transparent;
    border-radius: 7px;
    background: transparent;
    color: var(--rm-text);
  }
  .fg-name:hover {
    border-color: var(--rm-border);
    background: var(--rm-control-bg);
  }
  .fg-name:focus {
    outline: none;
    border-color: var(--rm-accent);
    background: var(--rm-control-bg);
    box-shadow: 0 0 0 3px var(--rm-accent-soft);
  }
  .fg-type {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--rm-text-dim);
  }
  .fg-type-select {
    min-width: 0;
    flex: 1 1 auto;
    font: inherit;
    font-size: 13px;
    padding: 5px 6px;
    border: 1px solid transparent;
    border-radius: 7px;
    background: transparent;
    color: var(--rm-text);
  }
  .fg-type-select:hover {
    border-color: var(--rm-border);
    background: var(--rm-control-bg);
  }
  .fg-type-select:focus {
    outline: none;
    border-color: var(--rm-accent);
  }
  .fg-phys {
    min-width: 0;
    font-size: 11.5px;
    color: var(--rm-text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fg-actions {
    display: inline-flex;
    justify-content: flex-end;
    gap: 4px;
  }
  .fg-act {
    padding: 5px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--rm-text-dim);
    line-height: 0;
    cursor: pointer;
    opacity: 0;
  }
  :global(.fg-row:hover) .fg-act,
  .fg-act.on,
  .fg-act:focus-visible {
    opacity: 1;
  }
  .fg-act:hover {
    background: rgba(0, 0, 0, 0.06);
    color: var(--rm-text);
  }
  .fg-act.on {
    background: var(--rm-accent);
    color: #fff;
  }
  .fg-act.danger:hover {
    background: var(--rm-danger);
    color: #fff;
  }
</style>
