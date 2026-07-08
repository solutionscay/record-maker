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
  import { fieldBadgeInfo } from './fieldBadges';
  import SchemaRelationshipEdge, { type SchemaRelationshipEdgeData } from './SchemaRelationshipEdge.svelte';
  import SchemaTableNode, { type SchemaGraphField, type SchemaTableNodeData } from './SchemaTableNode.svelte';
  import type { RelationshipView, TableView } from './types';

  // Strictly a read-only diagram of the schema's constraints (#142+). It never
  // creates or edits anything: no "new relationship", no opening a table or a
  // relationship to edit. Relationships are defined solely by field references
  // in the Fields tab; this view only draws the constraints they produce.
  let { store }: { store: SchemaStore } = $props();

  const hasAnyFields = $derived(store.tables.some((t) => (store.fieldsByTable[t.id] ?? []).length > 0));

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

  // Grid position is otherwise blind to relationships: two related tables can
  // land in different rows purely because of creation order, forcing their
  // edge into a long detour through left/right-only handles instead of a
  // direct line (#144). Reorder the tables fed into the grid (layout only —
  // the Tables tab keeps its own order) via BFS over the relationship graph,
  // so directly-related tables end up adjacent in the linear sequence and
  // usually land in the same row. Doesn't fully solve hub-shaped graphs with
  // more neighbors than fit in one row, but fixes the common case.
  const layoutTables = $derived.by<TableView[]>(() => {
    const byId = new Map(store.tables.map((t) => [t.id, t]));
    const neighbors = new Map<number, Set<number>>(store.tables.map((t) => [t.id, new Set<number>()]));
    for (const rel of store.relationships) {
      if (!neighbors.has(rel.fromTable) || !neighbors.has(rel.toTable)) continue;
      neighbors.get(rel.fromTable)!.add(rel.toTable);
      neighbors.get(rel.toTable)!.add(rel.fromTable);
    }
    const visited = new Set<number>();
    const ordered: TableView[] = [];
    for (const start of store.tables) {
      if (visited.has(start.id)) continue;
      const queue = [start.id];
      visited.add(start.id);
      while (queue.length > 0) {
        const id = queue.shift()!;
        ordered.push(byId.get(id)!);
        for (const neighborId of neighbors.get(id) ?? []) {
          if (!visited.has(neighborId)) {
            visited.add(neighborId);
            queue.push(neighborId);
          }
        }
      }
    }
    return ordered;
  });

  function nodePosition(index: number): { x: number; y: number } {
    const columns = Math.max(1, Math.min(3, Math.ceil(Math.sqrt(Math.max(1, store.tables.length)))));
    return {
      x: (index % columns) * columnSpacing,
      y: Math.floor(index / columns) * 260,
    };
  }

  function nodeX(tableId: number): number {
    const node = nodes.find((n) => n.id === `table-${tableId}`);
    if (node) return node.position.x;
    const index = layoutTables.findIndex((t) => t.id === tableId);
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

  let nodes = $state<SchemaNode[]>([]);

  $effect(() => {
    nodes = layoutTables.map((table, index) => {
      const totalFieldCount = (store.fieldsByTable[table.id] ?? []).length;
      const fields = keyFieldRows(table);
      const existingNode = nodes.find((n) => n.id === `table-${table.id}`);
      const position = existingNode ? existingNode.position : nodePosition(index);
      return {
        id: `table-${table.id}`,
        type: 'schemaTable',
        position,
        data: {
          table,
          fields,
          hiddenFieldCount: totalFieldCount - fields.length,
          relationshipCount: relationshipCount(table.id),
        },
        draggable: true,
      };
    });
  });

  const edges = $derived.by<SchemaEdge[]>(() =>
    store.relationships.filter(validRelationship).map((rel) => {
      // The source table's handle must face toward the target table (and vice
      // versa) or Svelte Flow's smooth-step router loops the path around both
      // boxes instead of connecting them directly (#139). Each table carries
      // one centered handle per side (SchemaTableNode, #147), so pick
      // left/right per edge from where the two tables actually land in the
      // grid.
      const fromOnLeft = nodeX(rel.fromTable) <= nodeX(rel.toTable);
      const sourceSide = fromOnLeft ? 'right' : 'left';
      const targetSide = fromOnLeft ? 'left' : 'right';
      return {
        id: `relationship-${rel.id}`,
        type: 'schemaRelationship',
        source: `table-${rel.fromTable}`,
        target: `table-${rel.toTable}`,
        sourceHandle: `source-${sourceSide}`,
        targetHandle: `target-${targetSide}`,
        data: { relationshipId: rel.id, name: rel.name },
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
</script>

<div class="rv">
  <!-- Crow's-foot marker defs (#143, reworked #146): referenced by id from
       anywhere in the document via url(#...), so this can live outside
       Svelte Flow's own <svg> canvas. Fork = "many" (source/FK end); tick
       pair = "one" (target/referenced end). userSpaceOnUse keeps the glyphs
       a fixed size regardless of the path's own stroke-width (which grows on
       hover/selected) — with the default strokeWidth units, the whole
       marker would visibly balloon on hover. Neutral gray regardless of
       hover/selected state, matching the line's own default color. -->
  <svg width="0" height="0" style="position: absolute" aria-hidden="true">
    <defs>
      <!-- Prongs touch the box at three points (top/mid/bottom) and converge
           to a single vertex further out along the line — a real fork, not
           an arrowhead. refX=0 anchors the box-touching side flush against
           the path's endpoint, so there's no gap between line and box. -->
      <marker id="rm-crowfoot-many" markerUnits="userSpaceOnUse" markerWidth="11" markerHeight="16" refX="0" refY="8" orient="auto">
        <path d="M0,0 L11,8 M0,16 L11,8 M0,8 L11,8" fill="none" stroke="#54585f" stroke-width="1.4" stroke-linecap="round" />
      </marker>
      <marker id="rm-crowfoot-one" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="13" refX="9" refY="6.5" orient="auto">
        <path d="M3,0.5 L3,12.5 M7,0.5 L7,12.5" fill="none" stroke="#54585f" stroke-width="1.4" stroke-linecap="round" />
      </marker>
    </defs>
  </svg>

  <header class="sc-viewhead rv-head">
    <div class="rv-title">
      <span class="sc-micro">Relationships</span>
      <span class="sc-count">{store.relationships.length} defined</span>
    </div>
    <span class="sc-hint rv-note">Read-only — relationships come from field references</span>
  </header>

  <div class="rv-graph">
    {#if store.loading}
      <p class="sc-note sc-hint">Loading relationships...</p>
    {:else if !hasAnyFields}
      <SvelteFlow
        bind:nodes
        edges={[]}
        {nodeTypes}
        {edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.28, maxZoom: 1 }}
        nodesDraggable={true}
        nodesConnectable={false}
        elementsSelectable={false}
        deleteKey={null}
      >
        <Controls showLock={false} />
        <Background variant={BackgroundVariant.Dots} gap={18} size={1} />
      </SvelteFlow>
      <div class="sc-empty rv-empty rv-empty--overlay">
        <p class="sc-empty-title">No fields yet</p>
        <p class="sc-hint">Add fields to your tables — a field that references another appears here as a relationship.</p>
      </div>
    {:else if store.relationships.length === 0}
      <SvelteFlow
        bind:nodes
        edges={[]}
        {nodeTypes}
        {edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.28, maxZoom: 1 }}
        nodesDraggable={true}
        nodesConnectable={false}
        elementsSelectable={false}
        deleteKey={null}
      >
        <Controls showLock={false} />
        <Background variant={BackgroundVariant.Dots} gap={18} size={1} />
      </SvelteFlow>
    {:else}
      <SvelteFlow
        bind:nodes
        {edges}
        {nodeTypes}
        {edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.18, maxZoom: 1 }}
        nodesDraggable={true}
        nodesConnectable={false}
        elementsSelectable={false}
        deleteKey={null}
      >
        <Controls showLock={false} />
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
