<script lang="ts">
  // Self-contained drawer shell (does NOT import the schema builder's
  // SchemaDrawer.svelte): that component is only ever used within the
  // schema-builder Vite entry today, so cross-importing it here would make it
  // a module shared across two entries, which pushes its scoped CSS into an
  // orphaned chunk no askama template links (see layout-manager.css's own
  // top comment for the same constraint on the CSS side). A few dozen lines
  // of drawer chrome is cheaper than that footgun.
  import Icon from '../lib/Icon.svelte';
  import { createLayout, type LayoutManagerView, type TableOption } from './persist';

  let {
    tables,
    onclose,
    oncreate,
  }: {
    tables: TableOption[];
    onclose: () => void;
    oncreate: (layout: LayoutManagerView) => void;
  } = $props();

  let name = $state('');
  let tableId = $state(0);
  let view = $state('form');
  let error = $state('');
  let saving = $state(false);

  $effect(() => {
    tableId = tables[0]?.id ?? 0;
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  async function create() {
    if (!name.trim() || !tableId) return;
    saving = true;
    error = '';
    try {
      const layout = await createLayout(name.trim(), tableId, view);
      oncreate(layout);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<aside class="nld">
  <header class="nld-head">
    <span class="nld-title">New layout</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  <div class="nld-body">
    <label class="sc-micro nld-label" for="nl-name">Layout name</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="nl-name" class="sc-input" bind:value={name} autofocus />

    <label class="sc-micro nld-label" for="nl-table">Table</label>
    <select id="nl-table" class="sc-select" bind:value={tableId}>
      {#each tables as t (t.id)}
        <option value={t.id}>{t.name}</option>
      {/each}
    </select>

    <label class="sc-micro nld-label" for="nl-view">View</label>
    <select id="nl-view" class="sc-select" bind:value={view}>
      <option value="form">Form</option>
      <option value="list">List</option>
      <option value="table">Table</option>
    </select>

    {#if error}
      <p class="sc-hint nld-error">{error}</p>
    {/if}
  </div>

  <footer class="nld-foot">
    <span class="nld-spacer"></span>
    <button type="button" class="sc-btn" onclick={onclose}>Cancel</button>
    <button
      type="button"
      class="sc-btn sc-btn--primary"
      onclick={create}
      disabled={saving || !name.trim() || !tableId}
    >
      {saving ? 'Creating…' : 'Create'}
    </button>
  </footer>
</aside>

<style>
  /* Same recipe as the schema builder's SchemaDrawer.svelte shell — kept a
     separate local copy for the reason noted above. */
  .nld {
    position: absolute;
    top: 0;
    right: 0;
    z-index: 20;
    max-width: 100%;
    width: 360px;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 0.5px solid var(--rm-border);
    background: var(--rm-inspector-bg);
    box-shadow: -12px 0 32px rgba(0, 0, 0, 0.14);
    animation: nld-slide 0.16s ease-out;
  }
  @keyframes nld-slide {
    from {
      transform: translateX(14px);
      opacity: 0.4;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }
  .nld-head,
  .nld-foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
  }
  .nld-head {
    justify-content: space-between;
    padding-right: 12px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .nld-foot {
    border-top: 0.5px solid var(--rm-border);
  }
  .nld-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .nld-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px;
  }
  .nld-label {
    display: block;
    margin: 0 0 6px;
  }
  .nld-label:not(:first-child) {
    margin-top: 16px;
  }
  .nld-spacer {
    flex: 1 1 auto;
  }
  .nld-error {
    margin: 14px 0 0;
    color: var(--rm-danger);
  }
</style>
