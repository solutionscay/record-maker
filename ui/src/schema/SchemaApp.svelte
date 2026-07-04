<script lang="ts">
  // Schema-builder root (#113). Owns the single store and the three-pane surface:
  // the table list (left), the field grid for the selected table (center), and a
  // master-detail field drawer (right) that opens when a field is edited. UI-only
  // drawer state lives here; all schema truth lives in the store.
  import { SchemaStore } from './store.svelte';
  import TableList from './TableList.svelte';
  import FieldGrid from './FieldGrid.svelte';
  import FieldDrawer from './FieldDrawer.svelte';
  import Icon from '../lib/Icon.svelte';

  const store = new SchemaStore();
  void store.load();

  // The drawer targets a field by id (not object) so it survives the store
  // replacing field objects on edit, and auto-closes if that field is deleted.
  let drawerFieldId = $state<number | null>(null);
  const drawerField = $derived(store.fields.find((f) => f.id === drawerFieldId) ?? null);

  function openField(id: number) {
    drawerFieldId = id;
  }
  function closeDrawer() {
    drawerFieldId = null;
  }
</script>

<div class="sb" class:has-drawer={drawerField}>
  <TableList {store} />

  <section class="sb-main">
    {#if store.loading}
      <div class="sb-center"><p class="sb-muted">Loading schema…</p></div>
    {:else if !store.selectedTable}
      <div class="sb-center">
        <p class="sb-empty-title">No tables yet</p>
        <p class="sb-muted">Create your first table in the list on the left to start adding fields.</p>
      </div>
    {:else}
      <FieldGrid {store} onedit={openField} openFieldId={drawerFieldId} />
    {/if}
  </section>

  {#if drawerField}
    <FieldDrawer {store} field={drawerField} onclose={closeDrawer} />
  {/if}
</div>

{#if store.error}
  <div class="sb-error" role="alert">
    <svg class="icon" aria-hidden="true"><use href="#icon-find" /></svg>
    <span>{store.error}</span>
    <button type="button" class="sb-error-x" title="Dismiss" onclick={() => (store.error = null)}>
      <Icon name="minus" />
    </button>
  </div>
{/if}

<style>
  .sb {
    display: grid;
    grid-template-columns: 248px 1fr;
    height: 100%;
    min-height: 0;
    background: var(--rm-workspace-bg);
  }
  .sb.has-drawer {
    grid-template-columns: 248px 1fr 340px;
  }
  .sb-main {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .sb-center {
    margin: auto;
    max-width: 24rem;
    text-align: center;
    padding: 2rem;
  }
  .sb-empty-title {
    margin: 0 0 6px;
    font-size: 15px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .sb-muted {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--rm-text-dim);
  }
  /* Error banner — bottom-center toast over the surface. */
  .sb-error {
    position: fixed;
    left: 50%;
    bottom: 44px;
    transform: translateX(-50%);
    z-index: 50;
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
</style>
