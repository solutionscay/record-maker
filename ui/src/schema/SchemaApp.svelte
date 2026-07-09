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
  import NoTablesEmpty from './NoTablesEmpty.svelte';
  import { confirmDanger } from './confirm';
  import { onMount } from 'svelte';

  const store = new SchemaStore();
  void store.load();

  onMount(() => {
    function handleBeforeUnload(e: BeforeUnloadEvent) {
      if (store.hasChanges && !(window as any).schemaAllowNavigation) {
        e.preventDefault();
        e.returnValue = '';
      }
    }
    window.addEventListener('beforeunload', handleBeforeUnload);

    // Expose dirty guard for global mode-switching shortcuts (#164)
    (window as any).schemaHasChanges = () => store.hasChanges;
    (window as any).schemaChangeSummary = () => store.changeSummary;

    return () => {
      window.removeEventListener('beforeunload', handleBeforeUnload);
      delete (window as any).schemaHasChanges;
      delete (window as any).schemaChangeSummary;
    };
  });

  type Tab = 'tables' | 'fields' | 'relationships';
  let tab = $state<Tab>('tables');

  let tableDrawerId = $state<number | null | undefined>(undefined);
  let fieldDrawer = $state<{ tableId: number; id: number | null } | null>(null);
  let relationshipDrawerId = $state<number | null>(null);

  const tableDrawerOpen = $derived(tableDrawerId !== undefined);
  const drawerTable = $derived(tableDrawerId == null ? null : (store.tableById(tableDrawerId) ?? null));
  const drawerField = $derived(fieldDrawer?.id == null ? null : store.fieldById(fieldDrawer.tableId, fieldDrawer.id));
  const drawerRelationship = $derived(
    relationshipDrawerId == null ? null : (store.relationships.find((r) => r.id === relationshipDrawerId) ?? null),
  );

  function closeDrawers() {
    tableDrawerId = undefined;
    fieldDrawer = null;
    relationshipDrawerId = null;
  }

  // The graph's SOLE editing affordance (#174): clicking a connector opens the
  // relationship drawer to toggle its portal permission flags.
  function openRelationship(id: number) {
    closeDrawers();
    relationshipDrawerId = id;
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
  async function saveSchema() {
    const ok = await store.saveAll();
    if (ok) closeDrawers();
  }

  async function discard() {
    if (store.hasChanges) {
      const ok = await confirmDanger(
        'Are you sure you want to discard all unsaved schema changes? This cannot be undone.',
        'Discard changes?',
        'Discard changes',
        'Keep editing',
      );
      if (!ok) return;
    }
    store.discardChanges();
    closeDrawers();
  }

  async function done() {
    if (store.hasChanges) {
      const ok = await confirmDanger(
        `You have unsaved changes: ${store.changeSummary}. If you leave now, your changes will be discarded.`,
        'Unsaved schema changes',
        'Discard changes',
        'Keep editing',
      );
      if (!ok) return;
    }
    (window as any).schemaAllowNavigation = true;
    window.location.href = '/';
  }
</script>

<div class="sb">
  <header class="sb-head">
    <span class="sb-apptitle">
      Manage Database
      {#if store.hasChanges}
        <span class="sb-apptitle-dot" title="Unsaved changes"></span>
      {/if}
    </span>
    <nav class="sb-tabs" aria-label="Schema sections">
      <button type="button" class="sb-tab" class:active={tab === 'tables'} onclick={goTables}>Tables</button>
      <button type="button" class="sb-tab" class:active={tab === 'fields'} onclick={goFields}>Fields</button>
      <button type="button" class="sb-tab" class:active={tab === 'relationships'} onclick={goRelationships}>Relationships</button>
    </nav>
    <button type="button" class="sb-x" title="Close" aria-label="Close" onclick={done}>
      <svg class="icon" aria-hidden="true"><use href="#icon-close" /></svg>
    </button>
  </header>

  <div class="sb-body">
    {#if !store.loading && store.tables.length === 0}
      <!-- One shared empty state across all three tabs until a table exists. -->
      <NoTablesEmpty onnew={newTable} />
    {:else if tab === 'tables'}
      <TablesView {store} onopen={openTable} onnew={newTable} onedit={editTable} />
    {:else if tab === 'fields'}
      <FieldGrid {store} onswitch={openTable} onedit={editField} onnew={newField} openFieldId={fieldDrawer?.id ?? null} />
    {:else}
      <RelationshipsView {store} selectedId={relationshipDrawerId} onselect={openRelationship} />
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
    {:else if relationshipDrawerId != null}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sb-scrim" onclick={closeDrawers}></div>
      <RelationshipDrawer {store} relationship={drawerRelationship} onclose={closeDrawers} />
    {/if}
  </div>

  <footer class="sb-foot">
    <span class="sc-hint sb-status" class:sb-status--dirty={store.hasChanges}>
      {#if store.hasChanges}
        <span class="sb-status-dot"></span>
      {/if}
      {store.changeSummary}
    </span>
    <button type="button" class="sc-btn" onclick={discard} disabled={!store.hasChanges || store.saving}>Discard</button>
    <button type="button" class="sc-btn sc-btn--primary" onclick={saveSchema} disabled={!store.hasChanges || store.saving}>
      {store.saving ? 'Saving...' : 'Save'}
    </button>
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
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 10px 16px;
    border-bottom: 0.5px solid var(--rm-border);
    background: var(--rm-toolbar-bg);
  }
  .sb-apptitle {
    justify-self: start;
    font-size: 13px;
    font-weight: 700;
    color: var(--rm-text);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .sb-apptitle-dot {
    position: relative;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #ff9f0a;
    box-shadow: 0 0 6px #ff9f0a;
    display: block;
  }
  .sb-apptitle-dot::after {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #ff9f0a;
    box-shadow: 0 0 8px #ff9f0a;
    animation: sb-pulse 2.2s infinite ease-out;
  }
  /* Upper-right close (X) — dismisses the pane, like the classic Manage dialogs. */
  .sb-x {
    justify-self: end;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: var(--rm-radius);
    background: var(--rm-control-bg);
    color: var(--rm-text-dim);
    cursor: pointer;
    box-shadow: var(--rm-elev-1);
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease;
  }
  .sb-x:hover {
    color: var(--rm-text);
    border-color: var(--rm-border-strong);
  }
  .sb-x .icon {
    width: 15px;
    height: 15px;
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
    border-radius: 0;
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
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .sb-status--dirty {
    color: #ff9f0a;
    font-weight: 500;
  }
  .sb-status-dot {
    position: relative;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #ff9f0a;
    box-shadow: 0 0 6px #ff9f0a;
    display: block;
  }
  .sb-status-dot::after {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #ff9f0a;
    box-shadow: 0 0 8px #ff9f0a;
    animation: sb-pulse 2.2s infinite ease-out;
  }
  @keyframes sb-pulse {
    0% {
      transform: translate(-50%, -50%) scale(1);
      opacity: 1;
      box-shadow: 0 0 4px rgba(255, 159, 10, 0.8);
    }
    100% {
      transform: translate(-50%, -50%) scale(2.8);
      opacity: 0;
      box-shadow: 0 0 16px rgba(255, 159, 10, 0);
    }
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
    border-radius: 0;
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
