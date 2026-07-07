<script lang="ts">
  import { BaseEdge, EdgeLabel, Position, getSmoothStepPath } from '@xyflow/svelte';

  export interface SchemaRelationshipEdgeData extends Record<string, unknown> {
    relationshipId: number;
    label: string;
    onOpen: (id: number) => void;
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
  const labelX = $derived(path[1]);
  const labelY = $derived(path[2]);

  function open(event: MouseEvent) {
    event.stopPropagation();
    if (data) data.onOpen(data.relationshipId);
  }
</script>

<BaseEdge path={edgePath} {markerStart} {markerEnd} class={`re-path${selected ? ' selected' : ''}`} />
<EdgeLabel x={labelX} y={labelY} transparent>
  <button type="button" class="re-label nodrag nopan" onclick={open}>
    {data?.label ?? 'relationship'}
  </button>
</EdgeLabel>

<style>
  :global(.svelte-flow__edge .re-path) {
    stroke: #60646c;
    stroke-width: 1.5;
  }
  :global(.svelte-flow__edge:hover .re-path),
  :global(.svelte-flow__edge .re-path.selected) {
    stroke: var(--rm-accent);
    stroke-width: 2;
  }
  .re-label {
    max-width: 180px;
    height: 24px;
    padding: 0 8px;
    border: 0.5px solid var(--rm-border);
    border-radius: 0;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    box-shadow: var(--sc-shadow);
    font: inherit;
    font-size: 11px;
    font-weight: 700;
    line-height: 22px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .re-label:hover {
    border-color: var(--rm-accent);
    color: var(--rm-accent);
  }
</style>
