<script lang="ts">
  import type { RelatedRouteChoice } from './model';

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

  function hopLabel(count: number): string {
    return `${count} ${count === 1 ? 'hop' : 'hops'}`;
  }

  function routePath(route: RelatedRouteChoice): string {
    return route.hops.map((hop) => hop.name).join(' → ');
  }

  function optionLabel(route: RelatedRouteChoice): string {
    return `${route.tableName} — ${hopLabel(route.hops.length)} · ${routePath(route)}`;
  }

  let orderedRoutes = $derived(
    routes
      .slice()
      .sort(
        (a, b) =>
          a.hops.length - b.hops.length ||
          a.tableName.localeCompare(b.tableName) ||
          a.path.localeCompare(b.path),
      ),
  );
  let selectedRoute = $derived(routes.find((r) => r.path === value) ?? null);
</script>

<div class="route-picker {className}">
  <select
    class="ctl-input route-select"
    value={value}
    title="Related table shown by this portal"
    aria-label="Related table"
    onchange={(e) => onchange?.(e.currentTarget.value)}
  >
    {#each orderedRoutes as route (route.path)}
      <option value={route.path}>{optionLabel(route)}</option>
    {/each}
  </select>

  {#if selectedRoute}
    <span class="le-hint route-path">
      {hopLabel(selectedRoute.hops.length)} via {routePath(selectedRoute)}
    </span>
  {/if}
</div>

<style>
  .route-picker {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .route-select {
    width: 100%;
  }
  .route-path {
    overflow-wrap: anywhere;
  }
</style>
