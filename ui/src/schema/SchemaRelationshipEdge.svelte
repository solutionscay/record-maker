<script lang="ts">
  import { BaseEdge, Position, getSmoothStepPath } from '@xyflow/svelte';

  export interface SchemaRelationshipEdgeData extends Record<string, unknown> {
    relationshipId: number;
    name: string;
  }

  let {
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition = Position.Right,
    targetPosition = Position.Left,
    markerStart,
    markerEnd,
    data,
    selected = false,
  }: {
    sourceX: number;
    sourceY: number;
    targetX: number;
    targetY: number;
    sourcePosition?: Position;
    targetPosition?: Position;
    markerStart?: string;
    markerEnd?: string;
    data?: SchemaRelationshipEdgeData;
    selected?: boolean;
  } = $props();

  // Crow's-foot notation (fork/tick, set via markerStart/markerEnd) carries
  // the cardinality; no visible label (#145) — a name repeated as a floating
  // caption on every connector was noise the shapes already convey. The name
  // is still there as a native hover tooltip, and the graph-level
  // onedgeclick (RelationshipsView) still opens the edit drawer when you
  // click the line, via BaseEdge's own wide invisible interaction stroke.
  const path = $derived(
    getSmoothStepPath({
      sourceX,
      sourceY,
      sourcePosition,
      targetX,
      targetY,
      targetPosition,
      borderRadius: 10,
    }),
  );
  const edgePath = $derived(path[0]);
</script>

<g>
  <title>{data?.name ?? 'relationship'}</title>
  <BaseEdge path={edgePath} {markerStart} {markerEnd} class={`re-path${selected ? ' selected' : ''}`} />
</g>

<style>
  :global(.svelte-flow__edge .re-path) {
    stroke: #60646c;
    stroke-width: 1.5;
    cursor: pointer;
  }
  :global(.svelte-flow__edge:hover .re-path),
  :global(.svelte-flow__edge .re-path.selected) {
    stroke: var(--rm-accent);
    stroke-width: 2;
  }
</style>
