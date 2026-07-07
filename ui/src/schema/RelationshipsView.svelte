<script lang="ts">
  import {
    Background,
    BackgroundVariant,
    Controls,
    SvelteFlow,
    type Edge,
    type EdgeTypes,
    type Node,
    type NodeTypes,
  } from '@xyflow/svelte';
  import '@xyflow/svelte/dist/style.css';
  import type { SchemaStore } from './store.svelte';
  import Icon from '../lib/Icon.svelte';
  import { fieldBadgeInfo } from './fieldBadges';
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

  const canCreate = $derived(store.tables.some((t) => (store.fieldsByTable[t.id] ?? []).length > 0));

  const nodeTypes: NodeTypes = { schemaTable: SchemaTableNode };
  const edgeTypes: EdgeTypes = { schemaRelationship: SchemaRelationshipEdge };

  type SchemaNode = Node<SchemaTableNodeData, 'schemaTable'>;
  type SchemaEdge = Edge<SchemaRelationshipEdgeData, 'schemaRelationship'>;

  function relationshipCount(tableId: number): number {
    return store.relationships.filter((r) => r.fromTable === tableId || r.toTable === tableId).length;
  }

  // The graph is a relationship diagram, not a column dump: only fields that
  // are a primary key, an FK, or referenced by another field earn a row here
  // (#142). Everything else stays visible in the Fields list.
  function keyFieldRows(table: TableView): SchemaGraphField[] {
    return (store.fieldsByTable[table.id] ?? [])
      .map((field) => ({
        id: field.id,
        name: field.name,
        kind: field.kind,
        ...fieldBadgeInfo(store, table.id, field),
      }))
      .filter((field) => field.primary || field.fkNames.length > 0 || field.keyNames.length > 0);
  }

  // Table boxes now size to their content (no fixed width/truncation, #143),
  // so a fixed grid spacing would let wide boxes overlap their neighbors.
  // Estimate each table's rendered width from its longest visible name and
  // space every column by the widest table in the whole graph.
  function estimatedNodeWidth(table: TableView): number {
    const names = [table.name, ...keyFieldRows(table).map((f) => f.name)];
    const longest = names.reduce((max, name) => Math.max(max, name.length), 10);
    return Math.min(520, Math.max(190, longest * 7 + 150));
  }

  const columnSpacing = $derived(
    Math.max(340, ...store.tables.map((t) => estimatedNodeWidth(t) + 70)),
  );

  function nodePosition(index: number): { x: number; y: number } {
    const columns = Math.max(1, Math.min(3, Math.ceil(Math.sqrt(Math.max(1, store.tables.length)))));
    return {
      x: (index % columns) * columnSpacing,
      y: Math.floor(index / columns) * 260,
    };
  }

  function nodeX(tableId: number): number {
    const index = store.tables.findIndex((t) => t.id === tableId);
    return index === -1 ? 0 : nodePosition(index).x;
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
    store.tables.map((table, index) => {
      const totalFieldCount = (store.fieldsByTable[table.id] ?? []).length;
      const fields = keyFieldRows(table);
      return {
        id: `table-${table.id}`,
        type: 'schemaTable',
        position: nodePosition(index),
        data: {
          table,
          fields,
          hiddenFieldCount: totalFieldCount - fields.length,
          relationshipCount: relationshipCount(table.id),
          onTable: ontable,
          onField: (tableId: number, fieldId: number) => onfield(fieldId, tableId),
        },
        draggable: false,
      };
    }),
  );

  const edges = $derived.by<SchemaEdge[]>(() =>
    store.relationships.filter(validRelationship).map((rel) => {
      // The source table's handle must face toward the target table (and vice
      // versa) or Svelte Flow's smooth-step router loops the path around both
      // boxes instead of connecting them directly (#139). Each field carries a
      // handle on both sides (SchemaTableNode), so pick left/right per edge
      // from where the two tables actually land in the grid.
      const fromOnLeft = nodeX(rel.fromTable) <= nodeX(rel.toTable);
      const sourceSide = fromOnLeft ? 'right' : 'left';
      const targetSide = fromOnLeft ? 'left' : 'right';
      return {
        id: `relationship-${rel.id}`,
        type: 'schemaRelationship',
        source: `table-${rel.fromTable}`,
        target: `table-${rel.toTable}`,
        sourceHandle: `source-${sourceSide}-${rel.fromField}`,
        targetHandle: `target-${targetSide}-${rel.toField}`,
        data: { relationshipId: rel.id, label: `${rel.name} - many to one`, onOpen: onedit },
        // Crow's-foot notation (#143): a fork at the source (the FK/"many"
        // side) and a tick at the target (the referenced/"one" side),
        // defined once as custom SVG markers below. Svelte Flow wraps these
        // in url('#...') itself, so these must be bare marker ids, not
        // already-wrapped url(#...) strings.
        markerStart: 'rm-crowfoot-many',
        markerEnd: 'rm-crowfoot-one',
        class: 'rv-edge',
      };
    }),
  );

  function openEdge(edge: SchemaEdge) {
    if (edge.data) onedit(edge.data.relationshipId);
  }
</script>

<div class="rv">
  <!-- Crow's-foot marker defs (#143): referenced by id from anywhere in the
       document via url(#...), so this can live outside Svelte Flow's own
       <svg> canvas. Fork = "many" (source/FK end); tick pair = "one"
       (target/referenced end). Neutral gray regardless of hover/selected
       state, matching the line's own default (unselected) color. -->
  <svg width="0" height="0" style="position: absolute" aria-hidden="true">
    <defs>
      <marker id="rm-crowfoot-many" markerWidth="14" markerHeight="12" refX="0" refY="6" orient="auto">
        <path d="M0,6 L13,0.5 M0,6 L13,11.5 M0,6 L13,6" fill="none" stroke="#60646c" stroke-width="1.4" stroke-linecap="round" />
      </marker>
      <marker id="rm-crowfoot-one" markerWidth="10" markerHeight="12" refX="9" refY="6" orient="auto">
        <path d="M3,0.5 L3,11.5 M7,0.5 L7,11.5" fill="none" stroke="#60646c" stroke-width="1.4" stroke-linecap="round" />
      </marker>
    </defs>
  </svg>

  <header class="sc-viewhead rv-head">
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
      <p class="sc-note sc-hint">Loading relationships...</p>
    {:else if store.tables.length === 0}
      <div class="sc-empty rv-empty">
        <p class="sc-empty-title">No tables yet</p>
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
      <div class="sc-empty rv-empty rv-empty--overlay">
        <p class="sc-empty-title">No fields available</p>
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
      <div class="sc-empty rv-empty rv-empty--overlay">
        <p class="sc-empty-title">No relationships yet</p>
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
  /* Header bar / empty-state chrome comes from the shared .sc-* classes
     (schema.css); this view adds only its stickiness and overlay placement. */
  .rv-head {
    position: sticky;
    top: 0;
    z-index: 2;
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
  .rv-empty {
    position: relative;
    z-index: 2;
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
    border-radius: 0;
    overflow: hidden;
  }
</style>
