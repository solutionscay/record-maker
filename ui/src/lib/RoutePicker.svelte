<script lang="ts">
  import type { RelatedRouteChoice } from './model';
  import Icon from './Icon.svelte';

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

  let open = $state(false);
  let query = $state('');
  let highlight = $state(0);
  let root = $state<HTMLDivElement | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let list = $state<HTMLUListElement | null>(null);

  function hopLabel(count: number): string {
    return `${count} ${count === 1 ? 'hop' : 'hops'}`;
  }

  function routePath(route: RelatedRouteChoice): string {
    return route.hops.map((hop) => hop.name).join(' → ');
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
  let selectedRoute = $derived(routes.find((route) => route.path === value) ?? null);
  let filtered = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return orderedRoutes;
    return orderedRoutes.filter((route) =>
      `${route.tableName} ${route.path} ${routePath(route)}`.toLowerCase().includes(needle),
    );
  });

  function openPopover(): void {
    if (routes.length === 0) return;
    query = '';
    open = true;
    const current = filtered.findIndex((route) => route.path === value);
    highlight = current >= 0 ? current : 0;
  }

  function close(): void {
    open = false;
  }

  function choose(route: RelatedRouteChoice | undefined): void {
    if (!route) return;
    onchange?.(route.path);
    close();
  }

  function onTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      openPopover();
    }
  }

  function onInputKeydown(event: KeyboardEvent): void {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        if (filtered.length) highlight = (highlight + 1) % filtered.length;
        break;
      case 'ArrowUp':
        event.preventDefault();
        if (filtered.length) highlight = (highlight - 1 + filtered.length) % filtered.length;
        break;
      case 'Enter':
        event.preventDefault();
        choose(filtered[highlight]);
        break;
      case 'Escape':
        event.preventDefault();
        close();
        break;
    }
  }

  $effect(() => {
    if (highlight >= filtered.length) highlight = Math.max(0, filtered.length - 1);
  });
  $effect(() => {
    if (open) input?.focus();
  });
  $effect(() => {
    if (!open || !list) return;
    (list.children[highlight] as HTMLElement | undefined)?.scrollIntoView({ block: 'nearest' });
  });
  $effect(() => {
    if (!open) return;
    function onDown(event: PointerEvent): void {
      if (root && !root.contains(event.target as Node)) close();
    }
    document.addEventListener('pointerdown', onDown, true);
    return () => document.removeEventListener('pointerdown', onDown, true);
  });
</script>

<div class="rp {className}" bind:this={root}>
  <button
    type="button"
    class="rp-trigger"
    class:rp-placeholder={!selectedRoute}
    disabled={routes.length === 0}
    title="Related table shown by this portal"
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={() => (open ? close() : openPopover())}
    onkeydown={onTriggerKeydown}
  >
    <span class="rp-current">
      <Icon name="view-list" />
      <span class="rp-current-copy">
        <span class="rp-current-table">{selectedRoute?.tableName ?? 'No related tables'}</span>
        {#if selectedRoute}
          <span class="rp-current-meta">{hopLabel(selectedRoute.hops.length)} · {routePath(selectedRoute)}</span>
        {/if}
      </span>
    </span>
    <svg class="rp-caret" width="10" height="7" viewBox="0 0 10 7" aria-hidden="true"
      ><path d="M1 1.5 5 5.5 9 1.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg
    >
  </button>

  {#if open}
    <div class="rp-pop">
      <input
        class="rp-input"
        type="text"
        placeholder="Find a related table…"
        bind:this={input}
        bind:value={query}
        onkeydown={onInputKeydown}
      />
      <ul class="rp-list" role="listbox" bind:this={list}>
        {#each filtered as route, index (route.path)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            class="rp-option"
            class:rp-active={index === highlight}
            class:rp-selected={route.path === value}
            role="option"
            aria-selected={route.path === value}
            onpointerenter={() => (highlight = index)}
            onclick={() => choose(route)}
          >
            <span class="rp-route-main">
              <span class="rp-table">{route.tableName}</span>
              <span class="rp-depth">{hopLabel(route.hops.length)}</span>
            </span>
            <span class="rp-path">
              {#each route.hops as hop, hopIndex (hop.relationshipId + ':' + hop.direction)}
                {#if hopIndex > 0}<span class="rp-arrow">→</span>{/if}
                <span class="rp-hop-card">{hop.cardinality === 'toMany' ? '∞' : '1'}</span>
                <span>{hop.name}</span>
              {/each}
            </span>
          </li>
        {/each}
        {#if filtered.length === 0}
          <li class="rp-none">No matching related tables</li>
        {/if}
      </ul>
    </div>
  {/if}
</div>

<style>
  .rp { position: relative; width: 100%; min-width: 0; }
  .rp-trigger { display: flex; align-items: center; gap: 8px; width: 100%; padding: 7px 9px; border: .5px solid var(--rm-border); border-radius: 7px; background: var(--rm-control-bg); color: var(--rm-text); font: inherit; text-align: left; cursor: pointer; box-shadow: 0 1px 2px rgba(0,0,0,.04); }
  .rp-trigger:disabled { opacity: .5; cursor: not-allowed; }
  .rp-current { display: flex; align-items: center; gap: 8px; min-width: 0; flex: 1 1 auto; }
  .rp-current-copy { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .rp-current-table { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 550; }
  .rp-current-meta { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 10.5px; color: var(--rm-text-dim); }
  .rp-placeholder .rp-current-table { color: var(--rm-text-dim); font-weight: 400; }
  .rp-caret { flex: 0 0 auto; color: #8a8a8e; }
  .rp-pop { position: absolute; top: calc(100% + 5px); left: 0; z-index: 50; box-sizing: border-box; width: 100%; max-width: 100%; padding: 6px; border: .5px solid var(--rm-border); border-radius: 10px; background: var(--rm-panel-bg, var(--rm-control-bg)); box-shadow: 0 10px 30px rgba(0,0,0,.2); }
  .rp-input { box-sizing: border-box; width: 100%; margin-bottom: 6px; padding: 7px 9px; border: .5px solid var(--rm-border); border-radius: 6px; background: var(--rm-control-bg); color: var(--rm-text); font: inherit; font-size: 13px; }
  .rp-input:focus { outline: none; border-color: var(--rm-accent); box-shadow: 0 0 0 2px rgba(10,132,255,.22); }
  .rp-list { list-style: none; max-height: 280px; overflow-y: auto; margin: 0; padding: 0; }
  .rp-option { display: flex; flex-direction: column; gap: 4px; padding: 8px 9px; border-radius: 6px; color: var(--rm-text); cursor: pointer; }
  .rp-option.rp-selected:not(.rp-active) { background: rgba(10,132,255,.1); }
  .rp-option.rp-active { background: var(--rm-accent); color: #fff; }
  .rp-route-main { display: flex; align-items: baseline; gap: 8px; }
  .rp-table { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; font-weight: 600; }
  .rp-depth { margin-left: auto; flex: 0 0 auto; font-size: 10.5px; opacity: .68; }
  .rp-path { display: flex; flex-wrap: wrap; align-items: center; gap: 3px 4px; min-width: 0; font-size: 10.5px; line-height: 1.35; color: var(--rm-text-dim); }
  .rp-path > span:not(.rp-hop-card):not(.rp-arrow) { min-width: 0; overflow-wrap: anywhere; }
  .rp-active .rp-path { color: inherit; opacity: .78; }
  .rp-hop-card { display: inline-flex; justify-content: center; min-width: 16px; padding: 0 3px; border: 1px solid currentColor; border-radius: 999px; font-size: 9px; line-height: 14px; opacity: .72; }
  .rp-arrow { opacity: .55; }
  .rp-none { padding: 12px 8px; color: var(--rm-text-dim); font-size: 12px; text-align: center; }
</style>
