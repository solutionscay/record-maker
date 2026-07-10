<script lang="ts">
  import type { RelatedRouteChoice, RelatedRouteHopChoice } from './model';

  let {
    routes,
    value = '',
    onchange,
    class: className = '',
  }: {
    routes: readonly RelatedRouteChoice[];
    value?: string;
    onchange?: (path: string) => void;
    class?: string;
  } = $props();

  /** A relationship id alone is not directional for a self-reference, so the
   * picker keys a hop by both pieces even though #179 currently authors the
   * ordinary reverse-then-forward join-table shape. */
  function hopKey(h: RelatedRouteHopChoice): string {
    return `${h.relationshipId}:${h.direction}`;
  }

  function sameHop(a: RelatedRouteHopChoice, b: RelatedRouteHopChoice): boolean {
    return hopKey(a) === hopKey(b);
  }

  function uniqueHops(hops: RelatedRouteHopChoice[]): RelatedRouteHopChoice[] {
    const seen = new Set<string>();
    return hops.filter((h) => {
      const key = hopKey(h);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }

  let selectedRoute = $derived(routes.find((r) => r.path === value) ?? null);
  let firstOptions = $derived(uniqueHops(routes.flatMap((r) => r.hops.slice(0, 1))));
  let selectedFirst = $derived(selectedRoute?.hops[0] ?? firstOptions[0] ?? null);
  let secondOptions = $derived(
    selectedFirst
      ? uniqueHops(
          routes
            .filter((r) => r.hops.length > 1 && sameHop(r.hops[0], selectedFirst!))
            .map((r) => r.hops[1]),
        )
      : [],
  );

  function chooseFirst(key: string): void {
    const route = routes.find((r) => r.hops.length === 1 && hopKey(r.hops[0]) === key);
    if (route) onchange?.(route.path);
  }

  function chooseSecond(key: string): void {
    if (!selectedFirst) return;
    if (key === '') {
      chooseFirst(hopKey(selectedFirst));
      return;
    }
    const route = routes.find(
      (r) =>
        r.hops.length === 2 &&
        sameHop(r.hops[0], selectedFirst!) &&
        hopKey(r.hops[1]) === key,
    );
    if (route) onchange?.(route.path);
  }
</script>

<div class="route-picker {className}">
  <select
    class="ctl-input route-hop"
    value={selectedFirst ? hopKey(selectedFirst) : ''}
    title="First relationship hop"
    onchange={(e) => chooseFirst(e.currentTarget.value)}
  >
    {#each firstOptions as hop (hopKey(hop))}
      <option value={hopKey(hop)}>
        {hop.name} ({hop.cardinality === 'toMany' ? '∞' : '1'}) → {hop.tableName}
      </option>
    {/each}
  </select>

  {#if selectedFirst && secondOptions.length > 0}
    <select
      class="ctl-input route-hop"
      value={selectedRoute?.hops[1] ? hopKey(selectedRoute.hops[1]) : ''}
      title="Optional second relationship hop"
      onchange={(e) => chooseSecond(e.currentTarget.value)}
    >
      <option value="">Use {selectedFirst.tableName}</option>
      {#each secondOptions as hop (hopKey(hop))}
        <option value={hopKey(hop)}>
          {hop.name} ({hop.cardinality === 'toMany' ? '∞' : '1'}) → {hop.tableName}
        </option>
      {/each}
    </select>
  {/if}

  {#if selectedRoute}
    <span class="le-hint route-path">{selectedRoute.path}</span>
  {/if}
</div>

<style>
  .route-picker {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .route-hop {
    width: 100%;
  }
  .route-path {
    overflow-wrap: anywhere;
  }
</style>
