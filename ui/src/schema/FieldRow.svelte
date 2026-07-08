<script lang="ts">
  // One field row in the grid (#113/#119/#122). It is display-only except for
  // the drag handle and explicit edit action.
  import type { FieldView } from './types';
  import { kindIcon, kindLabel } from './types';
  import type { FieldBadgeInfo } from './fieldBadges';
  import Icon from '../lib/Icon.svelte';

  let {
    field,
    badges,
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
    field: FieldView;
    badges: FieldBadgeInfo | null;
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

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  bind:this={rowEl}
  class="fg-row"
  class:active
  class:dragging
  ondragover={onDragOver}
  ondrop={onDrop}
>
  {#if dropBefore}<div class="fg-dropline top"></div>{/if}
  {#if dropAfter}<div class="fg-dropline bottom"></div>{/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <span
    class="fg-handle"
    class:disabled={!reorderable}
    title={reorderable ? 'Drag to reorder' : 'Switch to Custom order to reorder'}
    draggable={reorderable}
    ondragstart={onDragStart}
    ondragend={ondragendrow}
    onclick={(e) => e.stopPropagation()}
    aria-hidden="true"
  >
    <svg class="fg-handle-ico" viewBox="0 0 16 16"><circle cx="6" cy="4" r="1" /><circle cx="10" cy="4" r="1" /><circle cx="6" cy="8" r="1" /><circle cx="10" cy="8" r="1" /><circle cx="6" cy="12" r="1" /><circle cx="10" cy="12" r="1" /></svg>
  </span>

  <span class="fg-name-cell">
    <span class="fg-name">{field.name}</span>
  </span>

  <span class="fg-type">
    <Icon name={kindIcon(field.kind)} />
    <span>{kindLabel(field.kind)}</span>
  </span>

  <span class="fg-settings">
    {#if badges?.primary}
      <span class="fg-badge fg-badge--primary" title="Primary ID">ID</span>
    {/if}
    {#if badges?.required}
      <span class="fg-badge fg-badge--required" title="Required">REQ</span>
    {/if}
    {#if badges?.unique}
      <span class="fg-badge fg-badge--unique" title="Unique">UNIQ</span>
    {/if}
    {#if badges?.autoEnter}
      <span class="fg-badge fg-badge--auto" title={`Auto-enter: constant "${badges.autoEnter.value}"`}>AUTO</span>
    {/if}
    {#if badges && badges.keyNames.length > 0}
      <span class="fg-badge" title={`Referenced by ${badges.keyNames.join(', ')}`}>KEY</span>
    {/if}
    {#if badges && badges.fkNames.length > 0}
      <span class="fg-badge fg-badge--fk" title={`References ${badges.fkNames.join(', ')}`}>FK</span>
    {/if}
  </span>

  <span class="fg-notes" title={field.notes || 'No notes'}>{field.notes || 'No notes'}</span>

  <span class="fg-actions">
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Edit field" onclick={onedit}>
      <Icon name="edit" />
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
  :global(.fg-row:hover) {
    background: rgba(0, 0, 0, 0.02);
  }
  :global(.fg-row.active) {
    background: rgba(0, 0, 0, 0.04);
  }
  :global(.fg-row.dragging) {
    opacity: 0.35;
  }
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
  .fg-handle.disabled {
    cursor: default;
    opacity: 0.3;
  }
  .fg-handle-ico {
    width: 16px;
    height: 16px;
    fill: currentColor;
    opacity: 0.6;
  }
  .fg-name-cell {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .fg-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .fg-settings {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    min-width: 0;
  }
  .fg-badge {
    flex: none;
    height: 18px;
    min-width: 22px;
    padding: 0 5px;
    border: 0.5px solid var(--rm-border);
    border-radius: 5px;
    background: var(--rm-segment-active-bg);
    color: var(--rm-text);
    font-size: 10px;
    font-weight: 700;
    line-height: 17px;
    text-align: center;
  }
  .fg-badge--primary {
    border-color: transparent;
    background: rgba(255, 159, 10, 0.16);
    color: #8a5a00;
  }
  .fg-badge--required {
    border-color: transparent;
    background: rgba(255, 69, 58, 0.12);
    color: var(--rm-danger);
  }
  .fg-badge--unique {
    border-color: transparent;
    background: rgba(175, 82, 222, 0.12);
    color: #7a2fa0;
  }
  .fg-badge--fk {
    border-color: transparent;
    background: rgba(52, 199, 89, 0.14);
    color: #247a38;
  }
  .fg-badge--auto {
    border-color: transparent;
    background: var(--rm-accent-soft, rgba(10, 132, 255, 0.12));
    color: #174ea6;
  }
  .fg-type {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--rm-text-dim);
    font-size: 13px;
  }
  .fg-type :global(.icon) {
    flex: none;
  }
  .fg-notes {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--rm-text-dim);
  }
  .fg-actions {
    display: flex;
    justify-content: center;
  }
</style>
