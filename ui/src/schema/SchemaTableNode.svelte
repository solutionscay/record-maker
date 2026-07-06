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
    fields: SchemaGraphField[];
    relationshipCount: number;
    onTable: (id: number) => void;
    onField: (tableId: number, fieldId: number) => void;
  }

  const rowTop = 52;
  const rowHeight = 28;

  let {
    data,
    selected = false,
  }: {
    data: SchemaTableNodeData;
    selected?: boolean;
  } = $props();

  function handleTop(index: number): string {
    return `${rowTop + index * rowHeight + rowHeight / 2}px`;
  }

  function openTable() {
    data.onTable(data.table.id);
  }

  function onTableKeydown(event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    openTable();
  }

  function openField(event: MouseEvent, fieldId: number) {
    event.stopPropagation();
    data.onField(data.table.id, fieldId);
  }
</script>

<div class="tn" class:selected role="button" tabindex="0" onclick={openTable} onkeydown={onTableKeydown}>
  <header class="tn-head">
    <span class="tn-title" title={data.table.name}>{data.table.name}</span>
    <span class="tn-count">{data.relationshipCount}</span>
  </header>

  <div class="tn-fields">
    {#if data.fields.length === 0}
      <div class="tn-empty">No fields</div>
    {:else}
      {#each data.fields as field, index (field.id)}
        <button type="button" class="tn-field nodrag nopan" onclick={(e) => openField(e, field.id)}>
          <Icon name={kindIcon(field.kind)} />
          <span class="tn-field-name" title={field.name}>{field.name}</span>
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
        </button>
        <Handle
          type="target"
          id={`target-${field.id}`}
          position={Position.Left}
          class="tn-handle tn-handle--target"
          style={`top: ${handleTop(index)}`}
        />
        <Handle
          type="source"
          id={`source-${field.id}`}
          position={Position.Right}
          class="tn-handle tn-handle--source"
          style={`top: ${handleTop(index)}`}
        />
      {/each}
    {/if}
  </div>
</div>

<style>
  .tn {
    width: 238px;
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
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
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
    grid-template-columns: 16px minmax(0, 1fr) auto auto auto auto auto;
    align-items: center;
    gap: 6px;
    padding: 0 10px 0 12px;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
    text-align: left;
  }
  .tn-field:hover {
    background: rgba(0, 0, 0, 0.04);
  }
  .tn-field :global(.icon) {
    width: 14px;
    height: 14px;
    color: var(--rm-text-dim);
  }
  .tn-field-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
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
