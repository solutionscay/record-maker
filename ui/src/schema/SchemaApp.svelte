<script lang="ts">
  // Schema-builder root (#113). Drill-down surface: Tables (level 1) → Fields
  // (level 2) → Field Detail (a drawer over the fields). The store holds all
  // schema truth; this owns only the current level and which field the drawer
  // targets.
  import { SchemaStore } from './store.svelte';
  import TablesView from './TablesView.svelte';
  import FieldGrid from './FieldGrid.svelte';
  import FieldDrawer from './FieldDrawer.svelte';

  const store = new SchemaStore();
  void store.load();

  type Screen = 'tables' | 'fields';
  let screen = $state<Screen>('tables');

  // The drawer targets a field by id (not object) so it survives the store
  // replacing field objects on edit, and auto-closes if that field is deleted.
  let drawerFieldId = $state<number | null>(null);
  const drawerField = $derived(store.fields.find((f) => f.id === drawerFieldId) ?? null);

  // Show the fields level only when a real table is open; otherwise fall back to
  // the tables level (e.g. the open table was deleted from elsewhere).
  const onFields = $derived(screen === 'fields' && store.selectedTable != null);

  async function openTable(id: number) {
    drawerFieldId = null;
    screen = 'fields';
    await store.selectTable(id);
  }
  function backToTables() {
    drawerFieldId = null;
    screen = 'tables';
  }
  function openField(id: number) {
    drawerFieldId = id;
  }
  function closeDrawer() {
    drawerFieldId = null;
  }
</script>

<div class="sb">
  {#if onFields}
    <FieldGrid {store} onback={backToTables} onswitch={openTable} onedit={openField} openFieldId={drawerFieldId} />
  {:else}
    <TablesView {store} onopen={openTable} />
  {/if}

  {#if drawerField}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="sb-scrim" onclick={closeDrawer}></div>
    <FieldDrawer {store} field={drawerField} onclose={closeDrawer} />
  {/if}
</div>

{#if store.error}
  <div class="sb-error" role="alert">
    <svg class="icon" aria-hidden="true"><use href="#icon-find" /></svg>
    <span>{store.error}</span>
    <button type="button" class="sb-error-x" title="Dismiss" onclick={() => (store.error = null)}>
      <svg class="icon" aria-hidden="true"><use href="#icon-minus" /></svg>
    </button>
  </div>
{/if}

<style>
  .sb {
    position: relative;
    height: 100%;
    min-height: 0;
    overflow: hidden;
    background: var(--rm-workspace-bg);
  }
  /* Field-detail drawer scrim — dims the fields level and closes on click. */
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
  .sb-error .icon {
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
  }
  .sb-error-x:hover {
    background: rgba(255, 255, 255, 0.3);
  }
  .sb-error-x .icon {
    width: 1em;
    height: 1em;
    fill: currentColor;
  }
</style>
