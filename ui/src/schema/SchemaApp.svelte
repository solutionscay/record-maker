<script lang="ts">
  // Schema-builder root (#113/#119). Tables, fields, and relationships edit a
  // local draft; the footer Save applies the draft through the schema API.
  import { SchemaStore } from './store.svelte';
  import TablesView from './TablesView.svelte';
  import TableDrawer from './TableDrawer.svelte';
  import FieldGrid from './FieldGrid.svelte';
  import FieldDrawer from './FieldDrawer.svelte';
  import RelationshipsView from './RelationshipsView.svelte';
  import RelationshipDrawer from './RelationshipDrawer.svelte';

  const store = new SchemaStore();
  void store.load();

  type Tab = 'tables' | 'fields' | 'relationships';
  let tab = $state<Tab>('tables');

  let tableDrawerId = $state<number | null | undefined>(undefined);
  let fieldDrawer = $state<{ tableId: number; id: number | null } | null>(null);
  let relationshipDrawerId = $state<number | null | undefined>(undefined);

  const tableDrawerOpen = $derived(tableDrawerId !== undefined);
  const drawerTable = $derived(tableDrawerId == null ? null : (store.tableById(tableDrawerId) ?? null));
  const drawerField = $derived(fieldDrawer?.id == null ? null : store.fieldById(fieldDrawer.tableId, fieldDrawer.id));
  const drawerRelationship = $derived(
    relationshipDrawerId == null ? null : (store.relationships.find((r) => r.id === relationshipDrawerId) ?? null),
  );

  function closeDrawers() {
    tableDrawerId = undefined;
    fieldDrawer = null;
    relationshipDrawerId = undefined;
  }

  function openTable(id: number) {
    closeDrawers();
    tab = 'fields';
    store.selectTable(id);
  }
  function goFields() {
    closeDrawers();
    tab = 'fields';
    if (store.selectedTableId == null && store.tables.length > 0) store.selectTable(store.tables[0].id);
  }
  function goTables() {
    closeDrawers();
    tab = 'tables';
  }
  function goRelationships() {
    closeDrawers();
    tab = 'relationships';
  }

  function newTable() {
    closeDrawers();
    tableDrawerId = null;
  }
  function editTable(id: number) {
    closeDrawers();
    tableDrawerId = id;
  }
  function newField() {
    const tableId = store.selectedTableId ?? store.tables[0]?.id;
    if (tableId == null) return;
    store.selectTable(tableId);
    closeDrawers();
    fieldDrawer = { tableId, id: null };
  }
  function editField(id: number, tableId = store.selectedTableId) {
    if (tableId == null) return;
    closeDrawers();
    store.selectTable(tableId);
    fieldDrawer = { tableId, id };
  }
  function newRelationship() {
    closeDrawers();
    relationshipDrawerId = null;
  }
  function editRelationship(id: number) {
    closeDrawers();
    relationshipDrawerId = id;
  }

  async function saveSchema() {
    const ok = await store.saveAll();
    if (ok) closeDrawers();
  }

  function discard() {
    store.discardChanges();
    closeDrawers();
  }

  function done() {
    if (store.hasChanges && !window.confirm('Discard unsaved schema changes?')) return;
    window.location.href = '/';
  }
</script>

<div class="sb">
  <header class="sb-head">
    <nav class="sb-tabs" aria-label="Schema sections">
      <button type="button" class="sb-tab" class:active={tab === 'tables'} onclick={goTables}>Tables</button>
      <button type="button" class="sb-tab" class:active={tab === 'fields'} onclick={goFields}>Fields</button>
      <button type="button" class="sb-tab" class:active={tab === 'relationships'} onclick={goRelationships}>Relationships</button>
    </nav>
  </header>

  <div class="sb-body">
    {#if tab === 'tables'}
      <TablesView {store} onopen={openTable} onnew={newTable} onedit={editTable} />
    {:else if tab === 'fields'}
      <FieldGrid {store} onswitch={openTable} onedit={editField} onnew={newField} onnotables={goTables} openFieldId={fieldDrawer?.id ?? null} />
    {:else}
      <RelationshipsView {store} onnew={newRelationship} onedit={editRelationship} ontable={editTable} onfield={editField} />
    {/if}

    {#if tableDrawerOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sb-scrim" onclick={closeDrawers}></div>
      <TableDrawer {store} table={drawerTable} onclose={closeDrawers} />
    {:else if fieldDrawer}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sb-scrim" onclick={closeDrawers}></div>
      <FieldDrawer {store} tableId={fieldDrawer.tableId} field={drawerField} onclose={closeDrawers} />
    {:else if relationshipDrawerId !== undefined}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sb-scrim" onclick={closeDrawers}></div>
      <RelationshipDrawer {store} relationship={drawerRelationship} onclose={closeDrawers} />
    {/if}
  </div>

  <footer class="sb-foot">
    <span class="sc-hint sb-status">{store.changeSummary}</span>
    <button type="button" class="sc-btn" onclick={discard} disabled={!store.hasChanges || store.saving}>Discard</button>
    <button type="button" class="sc-btn sc-btn--primary" onclick={saveSchema} disabled={!store.hasChanges || store.saving}>
      {store.saving ? 'Saving...' : 'Save'}
    </button>
    <button type="button" class="sc-btn" onclick={done}>Done</button>
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
  .sb-tab:hover:not(.active) {
    color: var(--rm-text);
  }
  .sb-tab.active {
    background: var(--rm-segment-active-bg);
    color: var(--rm-text);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.14);
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
    gap: 10px;
    padding: 10px 18px;
    border-top: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .sb-status {
    margin-right: auto;
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
