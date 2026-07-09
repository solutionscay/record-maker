<script lang="ts">
  import { BaseEdge, Position, getSmoothStepPath } from '@xyflow/svelte';

  export interface SchemaRelationshipEdgeData extends Record<string, unknown> {
    relationshipId: number;
    name: string;
    /** Portal create/delete permission state, shown as chips on the line (#174). */
    allowCreate: boolean;
    allowDelete: boolean;
    /** Driven by the graph's own selected-relationship state (elementsSelectable
     * is off, so the framework never sets the `selected` prop here). */
    selected: boolean;
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
  // the cardinality; no floating name label (#145) — a name repeated on every
  // connector was noise the shapes already convey. The name is still a native
  // hover tooltip, and the graph-level onedgeclick (RelationshipsView) opens the
  // relationship connector drawer when you click the line, via BaseEdge's own
  // wide invisible interaction stroke (#174). The only chips on the line are the
  // portal create/delete permission indicators below.
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

  // Selection is graph-managed (elementsSelectable is off), so highlight from
  // our own data flag rather than the framework's `selected` prop.
  const active = $derived(selected || Boolean(data?.selected));

  // One chip per enabled permission — a clean line stays clean; the presence of
  // a chip is what signals "portal create/delete is reachable here" (#174).
  const chips = $derived(
    [
      data?.allowCreate ? { key: 'C', cls: 'create', title: 'Portal can create records through this relationship' } : null,
      data?.allowDelete ? { key: 'D', cls: 'delete', title: 'Portal can delete records through this relationship' } : null,
    ].filter((c): c is { key: string; cls: string; title: string } => c != null),
  );
  const CHIP_W = 16;
  const CHIP_STEP = 20;
  const chipStartX = $derived(-((chips.length * CHIP_STEP - (CHIP_STEP - CHIP_W)) / 2));
</script>

<g>
  <title>{data?.name ?? 'relationship'}</title>
  <BaseEdge path={edgePath} {markerStart} {markerEnd} class={`re-path${active ? ' selected' : ''}`} />
  {#if chips.length > 0}
    <g class="re-badges" transform={`translate(${labelX} ${labelY})`} aria-hidden="true">
      {#each chips as chip, i (chip.key)}
        <g class={`re-chip re-chip--${chip.cls}`} transform={`translate(${chipStartX + i * CHIP_STEP} 0)`}>
          <title>{chip.title}</title>
          <rect x="0" y="-7" width={CHIP_W} height="14" rx="4" />
          <text x={CHIP_W / 2} y="0.5" dominant-baseline="central" text-anchor="middle">{chip.key}</text>
        </g>
      {/each}
    </g>
  {/if}
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
  /* Compact create/delete permission chips on the connector — same tinted-pill
     language as the table-node field badges (SchemaTableNode's .tn-badge). */
  .re-badges {
    pointer-events: none;
  }
  .re-chip text {
    font-size: 9px;
    font-weight: 700;
  }
  .re-chip--create rect {
    fill: rgba(52, 199, 89, 0.16);
  }
  .re-chip--create text {
    fill: #247a38;
  }
  .re-chip--delete rect {
    fill: rgba(255, 69, 58, 0.14);
  }
  .re-chip--delete text {
    fill: var(--rm-danger);
  }
</style>
