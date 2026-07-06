<script lang="ts">
  import {
    Background,
    BackgroundVariant,
    Controls,
    MarkerType,
    SvelteFlow,
    type Edge,
    type EdgeTypes,
    type Node,
    type NodeTypes,
  } from '@xyflow/svelte';
  import '@xyflow/svelte/dist/style.css';
  import type { SchemaStore } from './store.svelte';
  import Icon from '../lib/Icon.svelte';
  import SchemaRelationshipEdge, { type SchemaRelationshipEdgeData } from './SchemaRelationshipEdge.svelte';
  import SchemaTableNode, { type SchemaGraphField, type SchemaTableNodeData } from './SchemaTableNode.svelte';
  import type { RelationshipView, TableView } from './types';

  let {
    store,
    onnew,
    onedit,
    ontable,
    onfield,
  }: {
    store: SchemaStore;
    onnew: () => void;
    onedit: (id: number) => void;
    ontable: (id: number) => void;
    onfield: (id: number, tableId?: number) => void;
  } = $props();

  function tableName(id: number): string {
    return store.tableById(id)?.name ?? 'Missing table';
  }

  function fieldName(tableId: number, fieldId: number): string {
    return store.fieldById(tableId, fieldId)?.name ?? 'Missing field';
  }

  const canCreate = $derived(store.tables.some((t) => (store.fieldsByTable[t.id] ?? []).length > 0));

  const nodeTypes: NodeTypes = { schemaTable: SchemaTableNode };
  const edgeTypes: EdgeTypes = { schemaRelationship: SchemaRelationshipEdge };

  type SchemaNode = Node<SchemaTableNodeData, 'schemaTable'>;
  type SchemaEdge = Edge<SchemaRelationshipEdgeData, 'schemaRelationship'>;

  function relationshipCount(tableId: number): number {
    return store.relationships.filter((r) => r.fromTable === tableId || r.toTable === tableId).length;
  }

  function fieldRows(table: TableView): SchemaGraphField[] {
    return (store.fieldsByTable[table.id] ?? []).map((field) => {
      const from = store.relationships.filter((r) => r.fromTable === table.id && r.fromField === field.id);
      const to = store.relationships.filter((r) => r.toTable === table.id && r.toField === field.id);
      return {
        id: field.id,
        name: field.name,
        kind: field.kind,
        fkNames: from.map((r) => `${r.name} -> ${tableName(r.toTable)}.${fieldName(r.toTable, r.toField)}`),
        keyNames: to.map((r) => `${tableName(r.fromTable)}.${fieldName(r.fromTable, r.fromField)} -> ${r.name}`),
      };
    });
  }

  function nodePosition(index: number): { x: number; y: number } {
    const columns = Math.max(1, Math.min(3, Math.ceil(Math.sqrt(Math.max(1, store.tables.length)))));
    return {
      x: (index % columns) * 340,
      y: Math.floor(index / columns) * 260,
    };
  }

  function validRelationship(rel: RelationshipView): boolean {
    return (
      store.tableById(rel.fromTable) != null &&
      store.tableById(rel.toTable) != null &&
      store.fieldById(rel.fromTable, rel.fromField) != null &&
      store.fieldById(rel.toTable, rel.toField) != null
    );
  }

  const nodes = $derived.by<SchemaNode[]>(() =>
    store.tables.map((table, index) => ({
      id: `table-${table.id}`,
      type: 'schemaTable',
      position: nodePosition(index),
      data: {
        table,
        fields: fieldRows(table),
        relationshipCount: relationshipCount(table.id),
        onTable: ontable,
        onField: (tableId: number, fieldId: number) => onfield(fieldId, tableId),
      },
      draggable: false,
    })),
  );

  const edges = $derived.by<SchemaEdge[]>(() =>
    store.relationships.filter(validRelationship).map((rel) => ({
      id: `relationship-${rel.id}`,
      type: 'schemaRelationship',
      source: `table-${rel.fromTable}`,
      target: `table-${rel.toTable}`,
      sourceHandle: `source-${rel.fromField}`,
      targetHandle: `target-${rel.toField}`,
      data: { relationshipId: rel.id, label: `${rel.name} - many to one`, onOpen: onedit },
      markerEnd: { type: MarkerType.ArrowClosed },
      class: 'rv-edge',
    })),
  );

  function openEdge(edge: SchemaEdge) {
    if (edge.data) onedit(edge.data.relationshipId);
  }
</script>

<div class="rv">
  <header class="rv-head">
    <div class="rv-title">
      <span class="sc-micro">Relationships</span>
      <span class="sc-count">{store.relationships.length} defined</span>
    </div>
    <button type="button" class="sc-btn sc-btn--primary" onclick={onnew} disabled={!canCreate}>
      <Icon name="plus" />New relationship
    </button>
  </header>

  <div class="rv-graph">
    {#if store.loading}
      <p class="rv-note sc-hint">Loading relationships...</p>
    {:else if store.tables.length === 0}
      <div class="rv-empty">
        <p class="rv-empty-title">No tables yet</p>
        <p class="sc-hint">Create tables and fields before viewing relationships.</p>
      </div>
    {:else if !canCreate}
      <SvelteFlow
        {nodes}
        edges={[]}
        {nodeTypes}
        {edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.28, maxZoom: 1 }}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable
        deleteKey={null}
        onnodeclick={({ node }) => ontable(node.data.table.id)}
      >
        <Controls />
        <Background variant={BackgroundVariant.Dots} gap={18} size={1} />
      </SvelteFlow>
      <div class="rv-empty rv-empty--overlay">
        <p class="rv-empty-title">No fields available</p>
        <p class="sc-hint">Create fields before defining relationships.</p>
      </div>
    {:else if store.relationships.length === 0}
      <SvelteFlow
        {nodes}
        edges={[]}
        {nodeTypes}
        {edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.28, maxZoom: 1 }}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable
        deleteKey={null}
        onnodeclick={({ node }) => ontable(node.data.table.id)}
      >
        <Controls />
        <Background variant={BackgroundVariant.Dots} gap={18} size={1} />
      </SvelteFlow>
      <div class="rv-empty rv-empty--overlay">
        <p class="rv-empty-title">No relationships yet</p>
        <p class="sc-hint">Connect a source field to a target field. The graph will show that edge here.</p>
        <button type="button" class="sc-btn sc-btn--primary" onclick={onnew}>
          <Icon name="plus" />New relationship
        </button>
      </div>
    {:else}
      <SvelteFlow
        {nodes}
        {edges}
        {nodeTypes}
        {edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.18, maxZoom: 1 }}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable
        deleteKey={null}
        onnodeclick={({ node }) => ontable(node.data.table.id)}
        onedgeclick={({ edge }) => openEdge(edge)}
      >
        <Controls />
        <Background variant={BackgroundVariant.Dots} gap={18} size={1} />
      </SvelteFlow>
    {/if}
  </div>
</div>

<style>
  .rv {
    height: 100%;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .rv-head {
    position: sticky;
    top: 0;
    z-index: 2;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--sc-head-h);
    padding: 0 18px;
    border-bottom: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .rv-title {
    display: inline-flex;
    align-items: center;
    gap: 10px;
  }
  .rv-graph {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
    background: #f7f7f8;
  }
  .rv-note {
    margin: 0;
    padding: 16px 18px;
  }
  .rv-empty {
    position: relative;
    z-index: 2;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 24px;
    text-align: center;
  }
  .rv-empty--overlay {
    position: absolute;
    left: 50%;
    bottom: 24px;
    transform: translateX(-50%);
    width: min(360px, calc(100% - 48px));
    padding: 14px 18px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: color-mix(in srgb, var(--rm-control-bg) 92%, transparent);
    box-shadow: var(--sc-shadow);
  }
  .rv-empty-title {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--rm-text);
  }
  :global(.rv-graph .svelte-flow) {
    width: 100%;
    height: 100%;
    --xy-node-border-default: 0;
    --xy-node-boxshadow-default: none;
    --xy-edge-stroke-default: var(--rm-text-dim);
    --xy-edge-stroke-width-default: 1.4;
    --xy-controls-button-background-color-default: var(--rm-control-bg);
    --xy-controls-button-border-color-default: var(--rm-border);
    --xy-controls-button-color-default: var(--rm-text);
  }
  :global(.rv-graph .svelte-flow__node) {
    border: 0;
    background: transparent;
    box-shadow: none;
  }
  :global(.rv-graph .svelte-flow__edge-labels) {
    position: absolute;
    inset: 0;
    z-index: 8;
    pointer-events: none;
  }
  :global(.rv-graph .svelte-flow__controls) {
    box-shadow: var(--sc-shadow);
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    overflow: hidden;
  }
</style>
