<script lang="ts">
  // Field drawer (#113/#119). Edits stay local until this drawer's Save updates
  // the schema draft; the server is not touched until the schema window Save.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind, FieldView } from './types';
  import { FIELD_KINDS, kindIcon, kindLabel } from './types';
  import Icon from '../lib/Icon.svelte';

  let {
    store,
    tableId,
    field,
    onclose,
  }: {
    store: SchemaStore;
    tableId: number;
    field: FieldView | null;
    onclose: () => void;
  } = $props();

  let name = $state('');
  let kind = $state<FieldKind>('text');
  let notes = $state('');

  $effect(() => {
    name = field?.name ?? '';
    kind = field?.kind ?? 'text';
    notes = field?.notes ?? '';
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function save() {
    const saved = store.saveFieldDraft(tableId, field?.id ?? null, name, kind, notes);
    if (saved) onclose();
  }

  function remove() {
    if (field && store.deleteFieldDraft(tableId, field.id)) onclose();
  }
</script>

<aside class="fd">
  <header class="fd-head">
    <span class="fd-title">{field ? 'Field details' : 'New field'}</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  <div class="fd-chip">
    <Icon name={kindIcon(kind)} />
    <span class="fd-chip-name">{name || 'Untitled'}</span>
    <span class="fd-chip-sep">.</span>
    <span class="fd-chip-kind">{kindLabel(kind)}</span>
  </div>

  <div class="fd-body">
    <label class="sc-micro fd-label" for="fd-name">Field name</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="fd-name" class="sc-input" bind:value={name} autofocus />

    <label class="sc-micro fd-label" for="fd-kind">Type</label>
    <select id="fd-kind" class="sc-select" bind:value={kind}>
      {#each FIELD_KINDS as k (k.kind)}
        <option value={k.kind}>{k.label}</option>
      {/each}
    </select>

    <label class="sc-micro fd-label" for="fd-notes">Notes</label>
    <textarea id="fd-notes" class="sc-textarea" rows="5" bind:value={notes}></textarea>

    {#if field}
      <span class="sc-micro fd-label">Physical name</span>
      <code class="fd-code">{field.phys || 'Created when schema is saved'}</code>
    {/if}

    <p class="sc-hint fd-note">Drawer Save updates the draft. The schema is not applied until the window Save.</p>
  </div>

  <footer class="fd-foot">
    {#if field}
      <button
        type="button"
        class="sc-btn sc-btn--danger fd-delete"
        onclick={remove}
        disabled={field.id > 0}
        title={field.id > 0 ? 'Deletion needs impact review before it is enabled' : 'Delete draft field'}
      >
        <Icon name="delete" />Delete field
      </button>
    {/if}
    <span class="fd-spacer"></span>
    <button type="button" class="sc-btn" onclick={onclose}>Cancel</button>
    <button type="button" class="sc-btn sc-btn--primary" onclick={save} disabled={name.trim().length === 0}>Save</button>
  </footer>
</aside>

<style>
  .fd {
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
    animation: fd-slide 0.16s ease-out;
  }
  @keyframes fd-slide {
    from {
      transform: translateX(14px);
      opacity: 0.4;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }
  .fd-head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 12px 18px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .fd-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--rm-text);
  }
  .fd-chip {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 14px 18px 0;
    padding: 0 11px;
    height: 34px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    font-size: 12.5px;
    box-shadow: var(--sc-shadow);
  }
  .fd-chip :global(.icon) {
    color: var(--rm-accent);
    flex: none;
  }
  .fd-chip-name {
    font-weight: 600;
    color: var(--rm-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fd-chip-sep,
  .fd-chip-kind {
    color: var(--rm-text-dim);
  }
  .fd-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px;
  }
  .fd-label {
    display: block;
    margin: 0 0 6px;
  }
  .fd-label:not(:first-child) {
    margin-top: 16px;
  }
  .fd-code {
    display: block;
    font-size: 12px;
    padding: 8px 10px;
    border-radius: 7px;
    background: rgba(0, 0, 0, 0.05);
    color: var(--rm-text);
    word-break: break-all;
  }
  .fd-note {
    margin: 14px 0 0;
    line-height: 1.45;
  }
  .fd-foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 0.5px solid var(--rm-border);
  }
  .fd-delete {
    color: var(--rm-danger);
  }
  .fd-spacer {
    flex: 1 1 auto;
  }
</style>
