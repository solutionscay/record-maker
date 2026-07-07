<script lang="ts">
  import { Handle, Position } from '@xyflow/svelte';
  import type { FieldKind, TableView } from './types';
  import { kindIcon, kindLabel } from './types';
  import Icon from '../lib/Icon.svelte';

  export interface SchemaGraphField {
    id: number;
    name: string;
    kind: FieldKind;
    primary: boolean;
    required: boolean;
    unique: boolean;
    fkNames: string[];
    keyNames: string[];
  }

  export interface SchemaTableNodeData extends Record<string, unknown> {
    table: TableView;
    /** Only primary/FK/referenced fields — see RelationshipsView's keyFieldRows (#142). */
    fields: SchemaGraphField[];
    /** Count of this table's fields left out of `fields` because they aren't keys. */
    hiddenFieldCount: number;
    relationshipCount: number;
    onTable: (id: number) => void;
  }

  let {
    data,
    selected = false,
  }: {
    data: SchemaTableNodeData;
    selected?: boolean;
  } = $props();

  function openTable() {
    data.onTable(data.table.id);
  }

  function onTableKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    openTable();
  }
</script>

<div class="tn" class:selected role="button" tabindex="0" onclick={openTable} onkeydown={onTableKeydown}>
  <header class="tn-head">
    <span class="tn-title">{data.table.name}</span>
    <span class="tn-count">{data.relationshipCount}</span>
  </header>

  <div class="tn-fields">
    {#if data.fields.length === 0 && data.hiddenFieldCount === 0}
      <div class="tn-empty">No fields</div>
    {:else if data.fields.length === 0}
      <div class="tn-empty">No key fields ({data.hiddenFieldCount} not shown)</div>
    {:else}
      {#each data.fields as field (field.id)}
        <!-- Purely informational (#148) — stop the click here so it doesn't
             bubble up into the table box's own onclick and open the table
             drawer instead. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="tn-field" onclick={(e) => e.stopPropagation()}>
          <Icon name={kindIcon(field.kind)} />
          <span class="tn-field-name">{field.name}</span>
          <span class="tn-kind">{kindLabel(field.kind)}</span>
          {#if field.primary}
            <span class="tn-badge tn-badge--primary" title="Primary ID">id</span>
          {/if}
          {#if field.required}
            <span class="tn-badge tn-badge--required" title="Required">req</span>
          {/if}
          {#if field.unique}
            <span class="tn-badge tn-badge--unique" title="Unique">uniq</span>
          {/if}
          {#if field.keyNames.length > 0}
            <span class="tn-badge" title={`Referenced by ${field.keyNames.join(', ')}`}>key</span>
          {/if}
          {#if field.fkNames.length > 0}
            <span class="tn-badge tn-badge--fk" title={`References ${field.fkNames.join(', ')}`}>fk</span>
          {/if}
        </div>
      {/each}
      {#if data.hiddenFieldCount > 0}
        <div class="tn-more">+{data.hiddenFieldCount} more field{data.hiddenFieldCount === 1 ? '' : 's'}</div>
      {/if}
    {/if}
  </div>
  <!-- One handle quartet per box, not per field row (#147) — a relationship
       connects at the vertical center of the table regardless of which field
       it references, like a classic ER diagram, instead of jogging to
       whichever row the field happens to land on. Both sides carry a
       target+source pair (#139) so an edge can attach to whichever side
       actually faces the other table. No `top` override: Svelte Flow centers
       Left/Right handles vertically by default. -->
  <Handle type="source" id="source-left" position={Position.Left} class="tn-handle tn-handle--source" />
  <Handle type="target" id="target-left" position={Position.Left} class="tn-handle tn-handle--target" />
  <Handle type="target" id="target-right" position={Position.Right} class="tn-handle tn-handle--target" />
  <Handle type="source" id="source-right" position={Position.Right} class="tn-handle tn-handle--source" />
</div>

<style>
  .tn {
    width: max-content;
    min-width: 190px;
    min-height: 90px;
    overflow: hidden;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    box-shadow: var(--sc-shadow);
    color: var(--rm-text);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .tn:hover,
  .tn.selected {
    border-color: var(--rm-accent);
    box-shadow: var(--sc-ring);
  }
  .tn-head {
    height: 36px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px 0 12px;
    border-bottom: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .tn-title {
    flex: 1 1 auto;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 700;
  }
  .tn-count {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border: 0.5px solid var(--rm-border);
    border-radius: 5px;
    background: var(--rm-control-bg);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--rm-text-dim);
  }
  .tn-fields {
    position: relative;
    padding: 8px 0;
  }
  .tn-field {
    position: relative;
    width: 100%;
    height: 28px;
    display: grid;
    grid-template-columns: 16px auto auto auto auto auto auto;
    align-items: center;
    gap: 6px;
    padding: 0 10px 0 12px;
    text-align: left;
  }
  .tn-field :global(.icon) {
    width: 14px;
    height: 14px;
    color: var(--rm-text-dim);
  }
  .tn-field-name {
    white-space: nowrap;
    font-size: 12px;
    font-weight: 600;
  }
  .tn-kind {
    font-size: 10.5px;
    color: var(--rm-text-dim);
  }
  .tn-badge {
    height: 16px;
    padding: 1px 5px 0;
    border-radius: 4px;
    background: rgba(10, 132, 255, 0.12);
    color: var(--rm-accent);
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
    line-height: 15px;
  }
  .tn-badge--fk {
    background: rgba(52, 199, 89, 0.14);
    color: #247a38;
  }
  .tn-badge--primary {
    background: rgba(255, 159, 10, 0.16);
    color: #8a5a00;
  }
  .tn-badge--required {
    background: rgba(255, 69, 58, 0.12);
    color: var(--rm-danger);
  }
  .tn-badge--unique {
    background: rgba(175, 82, 222, 0.12);
    color: #7a2fa0;
  }
  .tn-empty {
    height: 38px;
    display: flex;
    align-items: center;
    padding: 0 12px;
    font-size: 11.5px;
    color: var(--rm-text-dim);
  }
  .tn-more {
    height: 24px;
    display: flex;
    align-items: center;
    padding: 0 12px;
    font-size: 11px;
    color: var(--rm-text-dim);
  }
  :global(.svelte-flow__handle.tn-handle) {
    width: 7px;
    height: 7px;
    min-width: 7px;
    min-height: 7px;
    border: 1.5px solid var(--rm-control-bg);
    background: var(--rm-text-dim);
  }
  :global(.svelte-flow__handle.tn-handle--source) {
    background: #247a38;
  }
  :global(.svelte-flow__handle.tn-handle--target) {
    background: var(--rm-accent);
  }
</style>
