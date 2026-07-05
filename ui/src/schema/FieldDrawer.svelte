<script lang="ts">
  // The field-detail drawer (#113) — master-detail, mirroring Layout Mode's
  // inspector rhythm (head / body / foot, 18px padding, hairline dividers). A
  // left rail of categories (Field name / Type / Advanced) drives the right pane;
  // a logical preview chip (`Name · Type`, never DDL) sits pinned at the top.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind, FieldView } from './types';
  import { FIELD_KINDS, kindIcon, kindLabel } from './types';
  import { confirmDanger } from './confirm';
  import Icon from '../lib/Icon.svelte';

  let { store, field, onclose }: { store: SchemaStore; field: FieldView; onclose: () => void } = $props();

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  type Category = 'name' | 'type' | 'advanced';
  let category = $state<Category>('name');
  const CATEGORIES: { key: Category; label: string }[] = [
    { key: 'name', label: 'Field name' },
    { key: 'type', label: 'Type' },
    { key: 'advanced', label: 'Advanced' },
  ];

  function commitName(el: HTMLInputElement) {
    const v = el.value.trim();
    if (v && v !== field.name) void store.renameField(field.id, v);
    else el.value = field.name;
  }

  let typeQuery = $state('');
  const filteredKinds = $derived(
    FIELD_KINDS.filter((k) => k.label.toLowerCase().includes(typeQuery.trim().toLowerCase())),
  );
  function pickKind(kind: FieldKind) {
    if (kind !== field.kind) void store.retypeField(field.id, kind);
  }

  async function remove() {
    const ok = await confirmDanger(`Delete the “${field.name}” field? This cannot be undone.`, 'Delete field');
    if (ok) {
      void store.deleteField(field.id);
      onclose();
    }
  }
</script>

<aside class="fd">
  <header class="fd-head">
    <span class="fd-title">Field details</span>
    <button type="button" class="sc-btn sc-btn--icon sc-btn--ghost" title="Close" onclick={onclose}>
      <Icon name="minus" />
    </button>
  </header>

  <div class="fd-chip">
    <Icon name={kindIcon(field.kind)} />
    <span class="fd-chip-name">{field.name || 'Untitled'}</span>
    <span class="fd-chip-sep">·</span>
    <span class="fd-chip-kind">{kindLabel(field.kind)}</span>
  </div>

  <div class="fd-body">
    <nav class="fd-rail">
      {#each CATEGORIES as c (c.key)}
        <button type="button" class="fd-rail-item" class:active={category === c.key} onclick={() => (category = c.key)}>
          {c.label}
        </button>
      {/each}
    </nav>

    <div class="fd-pane">
      {#if category === 'name'}
        <label class="sc-micro fd-plabel" for="fd-name">Field name</label>
        <input
          id="fd-name"
          class="sc-input"
          value={field.name}
          onblur={(e) => commitName(e.currentTarget)}
          onkeydown={(e) => {
            if (e.key === 'Enter') e.currentTarget.blur();
            else if (e.key === 'Escape') {
              e.currentTarget.value = field.name;
              e.currentTarget.blur();
            }
          }}
        />
        <p class="sc-hint fd-note">The name you'll use on layouts and in calculations.</p>
      {:else if category === 'type'}
        <label class="sc-micro fd-plabel" for="fd-type-q">Type</label>
        <input id="fd-type-q" class="sc-input" placeholder="Search types…" bind:value={typeQuery} />
        <ul class="fd-typelist">
          {#each filteredKinds as k (k.kind)}
            <li>
              <button type="button" class="fd-type-opt" class:active={k.kind === field.kind} onclick={() => pickKind(k.kind)}>
                <Icon name={k.icon} />
                <span>{k.label}</span>
                {#if k.kind === field.kind}<span class="fd-tick">✓</span>{/if}
              </button>
            </li>
          {:else}
            <li class="sc-hint fd-note">No matching type.</li>
          {/each}
        </ul>
      {:else}
        <span class="sc-micro fd-plabel">Physical name</span>
        <code class="fd-code">{field.phys}</code>
        <p class="sc-hint fd-note">The physical column name in storage, derived from the field name. Read-only for now.</p>
      {/if}
    </div>
  </div>

  <footer class="fd-foot">
    <button type="button" class="sc-btn sc-btn--danger fd-delete" onclick={remove}>
      <Icon name="delete" />Delete field
    </button>
  </footer>
</aside>

<style>
  .fd {
    position: absolute;
    top: 0;
    right: 0;
    z-index: 20;
    width: 340px;
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
  /* Head / chip / body / foot mirror the inspector's rhythm. */
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
    display: grid;
    grid-template-columns: 112px 1fr;
    overflow: hidden;
  }
  .fd-rail {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 8px;
    overflow: auto;
  }
  .fd-rail-item {
    text-align: left;
    padding: 7px 9px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--rm-text);
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }
  .fd-rail-item:hover {
    background: rgba(0, 0, 0, 0.05);
  }
  .fd-rail-item.active {
    background: var(--rm-accent-soft);
    color: var(--rm-accent);
    font-weight: 600;
  }
  .fd-pane {
    min-width: 0;
    padding: 14px 18px;
    overflow: auto;
    border-left: 0.5px solid var(--rm-border);
  }
  .fd-plabel {
    display: block;
    margin-bottom: 6px;
  }
  .fd-note {
    margin: 8px 0 0;
    line-height: 1.45;
  }
  .fd-typelist {
    list-style: none;
    margin: 10px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .fd-type-opt {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--rm-text);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }
  .fd-type-opt :global(.icon) {
    color: var(--rm-text-dim);
    flex: none;
  }
  .fd-type-opt:hover {
    background: rgba(0, 0, 0, 0.05);
  }
  .fd-type-opt.active {
    background: var(--rm-accent-soft);
    color: var(--rm-accent);
    font-weight: 600;
  }
  .fd-type-opt.active :global(.icon) {
    color: var(--rm-accent);
  }
  .fd-tick {
    margin-left: auto;
    color: var(--rm-accent);
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
  .fd-foot {
    flex: none;
    padding: 12px 18px;
    border-top: 0.5px solid var(--rm-border);
  }
  /* Resting red text; the shared --danger hover fills it in. */
  .fd-delete {
    color: var(--rm-danger);
  }
</style>
