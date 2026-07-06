<script lang="ts">
  import type { SchemaStore } from './store.svelte';
  import type { TableView } from './types';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    table,
    onclose,
  }: {
    store: SchemaStore;
    table: TableView | null;
    onclose: () => void;
  } = $props();

  let name = $state('');
  let notes = $state('');

  $effect(() => {
    name = table?.name ?? '';
    notes = table?.notes ?? '';
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function save() {
    const saved = store.saveTableDraft(table?.id ?? null, name, notes);
    if (saved) onclose();
  }

  function remove() {
    if (table && store.deleteTableDraft(table.id)) onclose();
  }
</script>

<aside class="td">
  <header class="td-head">
    <span class="td-title">{table ? 'Table details' : 'New table'}</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  <div class="td-body">
    <label class="sc-micro td-label" for="td-name">Table name</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="td-name" class="sc-input" bind:value={name} autofocus />

    <label class="sc-micro td-label" for="td-notes">Notes</label>
    <textarea id="td-notes" class="sc-textarea" rows="5" bind:value={notes}></textarea>

    {#if table}
      <span class="sc-micro td-label">Physical name</span>
      <code class="td-code">{table.phys || 'Created when schema is saved'}</code>
    {/if}

    <p class="sc-hint td-note">Drawer Save updates the draft. The schema is not applied until the window Save.</p>
  </div>

  <footer class="td-foot">
    {#if table}
      <button
        type="button"
        class="sc-btn sc-btn--danger td-delete"
        onclick={remove}
        disabled={table.id > 0}
        title={table.id > 0 ? 'Deletion needs impact review before it is enabled' : 'Delete draft table'}
      >
        <Icon name="delete" />Delete table
      </button>
    {/if}
    <span class="td-spacer"></span>
    <button type="button" class="sc-btn" onclick={onclose}>Cancel</button>
    <button type="button" class="sc-btn sc-btn--primary" onclick={save} disabled={name.trim().length === 0}>Save</button>
  </footer>
</aside>

<style>
  .td {
    position: absolute;
    top: 0;
    right: 0;
    z-index: 20;
    width: 360px;
    max-width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 0.5px solid var(--rm-border);
    background: var(--rm-inspector-bg);
    box-shadow: -12px 0 32px rgba(0, 0, 0, 0.14);
    animation: td-slide 0.16s ease-out;
  }
  @keyframes td-slide {
    from {
      transform: translateX(14px);
      opacity: 0.4;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }
  .td-head,
  .td-foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .td-head {
    justify-content: space-between;
    padding-right: 12px;
  }
  .td-foot {
    border-top: 0.5px solid var(--rm-border);
    border-bottom: 0;
  }
  .td-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .td-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px;
  }
  .td-label {
    display: block;
    margin: 0 0 6px;
  }
  .td-label:not(:first-child) {
    margin-top: 16px;
  }
  .td-code {
    display: block;
    font-size: 12px;
    padding: 8px 10px;
    border-radius: 7px;
    background: rgba(0, 0, 0, 0.05);
    color: var(--rm-text);
    word-break: break-all;
  }
  .td-note {
    margin: 14px 0 0;
    line-height: 1.45;
  }
  .td-delete {
    color: var(--rm-danger);
  }
  .td-spacer {
    flex: 1 1 auto;
  }
</style>
