<script lang="ts">
  // Schema-builder root (#113). A full-width, sidebar-less window with in-window
  // tabs — Tables / Fields / Relationships — inspired by the classic
  // database-definition dialog. The store holds all schema truth; this owns only
  // the active tab and which field the drawer targets. Relationships is PR 2.
  import { SchemaStore } from './store.svelte';
  import TablesView from './TablesView.svelte';
  import FieldGrid from './FieldGrid.svelte';
  import FieldDrawer from './FieldDrawer.svelte';

  const store = new SchemaStore();
  void store.load();

  type Tab = 'tables' | 'fields';
  let tab = $state<Tab>('tables');

  let drawerFieldId = $state<number | null>(null);
  const drawerField = $derived(store.fields.find((f) => f.id === drawerFieldId) ?? null);

  async function openTable(id: number) {
    drawerFieldId = null;
    tab = 'fields';
    await store.selectTable(id);
  }
  async function goFields() {
    tab = 'fields';
    if (store.selectedTableId == null && store.tables.length > 0) {
      await store.selectTable(store.tables[0].id);
    }
  }
  function goTables() {
    tab = 'tables';
  }
  function openField(id: number) {
    drawerFieldId = id;
  }
  function closeDrawer() {
    drawerFieldId = null;
  }
</script>

<div class="sb">
  <header class="sb-head">
    <nav class="sb-tabs" aria-label="Schema sections">
      <button type="button" class="sb-tab" class:active={tab === 'tables'} onclick={goTables}>Tables</button>
      <button type="button" class="sb-tab" class:active={tab === 'fields'} onclick={goFields}>Fields</button>
      <button type="button" class="sb-tab" disabled title="Coming soon">Relationships</button>
    </nav>
  </header>

  <div class="sb-body">
    {#if tab === 'tables'}
      <TablesView {store} onopen={openTable} />
    {:else}
      <FieldGrid {store} onswitch={openTable} onedit={openField} onnotables={goTables} openFieldId={drawerFieldId} />
    {/if}

    {#if drawerField}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sb-scrim" onclick={closeDrawer}></div>
      <FieldDrawer {store} field={drawerField} onclose={closeDrawer} />
    {/if}
  </div>

  <footer class="sb-foot">
    <span class="sc-hint">Changes are saved as you make them.</span>
    <button type="button" class="sc-btn sc-btn--primary" onclick={() => (window.location.href = '/')}>Done</button>
  </footer>
</div>

{#if store.error}
  <div class="sb-error" role="alert">
    <svg class="sb-error-ico" aria-hidden="true"><use href="#icon-find" /></svg>
    <span>{store.error}</span>
    <button type="button" class="sb-error-x" title="Dismiss" onclick={() => (store.error = null)}>
      <svg class="sb-error-ico" aria-hidden="true"><use href="#icon-minus" /></svg>
    </button>
  </div>
{/if}

<style>
  .sb {
    position: relative;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--rm-control-bg);
  }
  .sb-head {
    flex: none;
    display: flex;
    justify-content: center;
    padding: 10px 16px;
    border-bottom: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  /* Centered segmented tabs (matches the shell's .view-switch / .modes). */
  .sb-tabs {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: 8px;
    background: var(--rm-segment-track);
  }
  .sb-tab {
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    padding: 5px 16px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--rm-text-dim);
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .sb-tab:hover:not(:disabled):not(.active) {
    color: var(--rm-text);
  }
  .sb-tab.active {
    background: var(--rm-segment-active-bg);
    color: var(--rm-text);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.14);
  }
  .sb-tab:disabled {
    color: #bcbcc1;
    cursor: default;
  }
  .sb-body {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .sb-foot {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 14px;
    padding: 10px 18px;
    border-top: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .sb-scrim {
    position: absolute;
    inset: 0;
    z-index: 15;
    background: rgba(20, 22, 28, 0.14);
  }
  .sb-error {
    position: fixed;
    left: 50%;
    bottom: 44px;
    transform: translateX(-50%);
    z-index: 60;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: min(38rem, calc(100vw - 2rem));
    padding: 9px 10px 9px 14px;
    border-radius: 9px;
    background: var(--rm-danger);
    color: #fff;
    font-size: 12.5px;
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.22);
  }
  .sb-error-ico {
    width: 1em;
    height: 1em;
    fill: currentColor;
    flex: none;
  }
  .sb-error-x {
    margin-left: 4px;
    padding: 2px;
    border: 0;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.18);
    color: #fff;
    line-height: 0;
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .sb-error-x:hover {
    background: rgba(255, 255, 255, 0.3);
  }
</style>
