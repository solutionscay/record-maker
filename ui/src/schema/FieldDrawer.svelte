<script lang="ts">
  // Field drawer (#113/#119). Edits stay local until this drawer's Save updates
  // the schema draft; the server is not touched until the schema window Save.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind, FieldOptions, FieldView } from './types';
  import { FIELD_KINDS, kindIcon, kindLabel } from './types';
  import Icon from '../lib/Icon.svelte';
  import { untrack } from 'svelte';

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
  let primary = $state(false);
  let required = $state(false);
  let unique = $state(false);
  let memberOfEnabled = $state(false);
  let memberOfValueList = $state<number | null>(null);
  let rangeMin = $state('');
  let rangeMax = $state('');
  let referenceEnabled = $state(false);
  let referenceName = $state('');
  let referenceToTable = $state<number | null>(null);
  let referenceToField = $state<number | null>(null);
  let hydrationKey = '';

  const hasRange = $derived(kind === 'number' || kind === 'date' || kind === 'time' || kind === 'timestamp');
  const rangeInputType = $derived(kind === 'number' ? 'number' : kind === 'date' ? 'date' : kind === 'time' ? 'time' : 'datetime-local');
  const referenceFields = $derived(referenceToTable == null ? [] : (store.fieldsByTable[referenceToTable] ?? []));
  const memberOfValid = $derived(
    !memberOfEnabled || (memberOfValueList != null && store.valueLists.some((list) => list.id === memberOfValueList)),
  );
  const referenceValid = $derived(
    !referenceEnabled ||
      (referenceName.trim().length > 0 &&
        referenceToTable != null &&
        referenceToField != null &&
        store.fieldById(referenceToTable, referenceToField) != null),
  );

  $effect(() => {
    const fieldSignature =
      field == null
        ? 'new'
        : `${field.id}:${field.name}:${field.kind}:${field.notes}:${JSON.stringify(field.options ?? {})}`;
    const nextHydrationKey = `${tableId}:${fieldSignature}`;
    if (nextHydrationKey === hydrationKey) return;
    hydrationKey = nextHydrationKey;

    untrack(() => {
      const nextReferenceToTable = field?.options?.reference?.toTable ?? store.tables[0]?.id ?? null;
      name = field?.name ?? '';
      kind = field?.kind ?? 'text';
      notes = field?.notes ?? '';
      primary = field?.options?.validation?.primary ?? false;
      required = field?.options?.validation?.required ?? false;
      unique = field?.options?.validation?.unique ?? false;
      memberOfValueList = field?.options?.validation?.memberOfValueList ?? store.valueLists[0]?.id ?? null;
      memberOfEnabled = field?.options?.validation?.memberOfValueList != null;
      rangeMin = field?.options?.validation?.range?.min ?? '';
      rangeMax = field?.options?.validation?.range?.max ?? '';
      referenceEnabled = field?.options?.reference != null;
      referenceName = field?.options?.reference?.name ?? '';
      referenceToTable = nextReferenceToTable;
      referenceToField =
        field?.options?.reference?.toField ??
        (nextReferenceToTable == null ? null : (store.fieldsByTable[nextReferenceToTable]?.[0]?.id ?? null));
    });
  });

  $effect(() => {
    if (!memberOfEnabled) return;
    if (memberOfValueList == null || !store.valueLists.some((list) => list.id === memberOfValueList)) {
      memberOfValueList = store.valueLists[0]?.id ?? null;
    }
  });

  $effect(() => {
    if (!referenceEnabled) return;
    if (!referenceName.trim()) referenceName = (name.trim() || 'relationship').toLocaleLowerCase().replace(/\s+/g, '_');
    if (referenceToTable == null || !store.tableById(referenceToTable)) {
      referenceToTable = store.tables[0]?.id ?? null;
    }
    const fields = referenceToTable == null ? [] : (store.fieldsByTable[referenceToTable] ?? []);
    if (referenceToField == null || !fields.some((f) => f.id === referenceToField)) {
      referenceToField = fields[0]?.id ?? null;
    }
  });

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onclose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function optionsDraft(): FieldOptions {
    const validation: NonNullable<FieldOptions['validation']> = {};
    if (primary) validation.primary = true;
    if (primary || required) validation.required = true;
    if (primary || unique) validation.unique = true;
    if (memberOfEnabled && memberOfValueList != null) validation.memberOfValueList = memberOfValueList;
    if (hasRange && (rangeMin.trim() || rangeMax.trim())) {
      validation.range = {};
      if (rangeMin.trim()) validation.range.min = rangeMin.trim();
      if (rangeMax.trim()) validation.range.max = rangeMax.trim();
    }
    const options: FieldOptions = Object.keys(validation).length > 0 ? { validation } : {};
    if (referenceEnabled && referenceToTable != null && referenceToField != null) {
      options.reference = {
        name: referenceName.trim(),
        toTable: referenceToTable,
        toField: referenceToField,
      };
    }
    return options;
  }

  function save() {
    const saved = store.saveFieldDraft(tableId, field?.id ?? null, name, kind, notes, optionsDraft());
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

    <section class="fd-section" aria-labelledby="fd-validation">
      <span id="fd-validation" class="sc-micro fd-label">Validation</span>
      <label class="fd-check">
        <input type="checkbox" bind:checked={primary} />
        <span>Primary ID</span>
      </label>
      <label class="fd-check">
        <input type="checkbox" checked={primary || required} disabled={primary} onchange={(e) => (required = e.currentTarget.checked)} />
        <span>Required</span>
      </label>
      <label class="fd-check">
        <input type="checkbox" checked={primary || unique} disabled={primary} onchange={(e) => (unique = e.currentTarget.checked)} />
        <span>Unique</span>
      </label>
      <label class="fd-check">
        <input type="checkbox" bind:checked={memberOfEnabled} disabled={store.valueLists.length === 0} />
        <span>Member of value list</span>
      </label>
      {#if memberOfEnabled}
        <label>
          <span class="sc-hint">Value list</span>
          <select
            class="sc-select"
            value={memberOfValueList ?? ''}
            onchange={(e) => (memberOfValueList = Number(e.currentTarget.value))}
            disabled={store.valueLists.length === 0}
          >
            {#each store.valueLists as list (list.id)}
              <option value={list.id}>{list.name}</option>
            {/each}
          </select>
        </label>
      {/if}
      {#if hasRange}
        <div class="fd-range">
          <label>
            <span class="sc-hint">Min</span>
            <input class="sc-input" type={rangeInputType} bind:value={rangeMin} />
          </label>
          <label>
            <span class="sc-hint">Max</span>
            <input class="sc-input" type={rangeInputType} bind:value={rangeMax} />
          </label>
        </div>
      {/if}
    </section>

    <section class="fd-section" aria-labelledby="fd-reference">
      <span id="fd-reference" class="sc-micro fd-label">Reference</span>
      <label class="fd-check">
        <input type="checkbox" bind:checked={referenceEnabled} />
        <span>References another field</span>
      </label>
      {#if referenceEnabled}
        <div class="fd-ref">
          <label>
            <span class="sc-hint">Relationship name</span>
            <input class="sc-input" bind:value={referenceName} />
          </label>
          <label>
            <span class="sc-hint">Target table</span>
            <select
              class="sc-select"
              value={referenceToTable ?? ''}
              onchange={(e) => {
                referenceToTable = Number(e.currentTarget.value);
                referenceToField = store.fieldsByTable[referenceToTable]?.[0]?.id ?? null;
              }}
            >
              {#each store.tables as table (table.id)}
                <option value={table.id}>{table.name}</option>
              {/each}
            </select>
          </label>
          <label>
            <span class="sc-hint">Target field</span>
            <select
              class="sc-select"
              value={referenceToField ?? ''}
              onchange={(e) => (referenceToField = Number(e.currentTarget.value))}
              disabled={referenceFields.length === 0}
            >
              {#each referenceFields as target (target.id)}
                <option value={target.id}>{target.name}</option>
              {/each}
            </select>
          </label>
        </div>
      {/if}
    </section>

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
    <button type="button" class="sc-btn sc-btn--primary" onclick={save} disabled={name.trim().length === 0 || !memberOfValid || !referenceValid}>Save</button>
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
  .fd-section {
    margin-top: 16px;
  }
  .fd-check {
    height: 28px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--rm-text);
  }
  .fd-check input {
    width: 14px;
    height: 14px;
    accent-color: var(--rm-accent);
  }
  .fd-range {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 10px;
    margin-top: 8px;
  }
  .fd-range label {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .fd-ref {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 8px;
  }
  .fd-ref label {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
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
