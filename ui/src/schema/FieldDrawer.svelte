<script lang="ts">
  // Field drawer (#113/#119). Edits stay local until this drawer's Save updates
  // the schema draft; the server is not touched until the schema window Save.
  import type { SchemaStore } from './store.svelte';
  import type { FieldKind, FieldOptions, FieldView } from './types';
  import { FIELD_KINDS, kindIcon, kindLabel } from './types';
  import Icon from '../lib/Icon.svelte';
  import SchemaDrawer from './SchemaDrawer.svelte';
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
  let required = $state(false);
  let unique = $state(false);
  let memberOfEnabled = $state(false);
  let memberOfValueList = $state<number | null>(null);
  let rangeMin = $state('');
  let rangeMax = $state('');
  let autoEnterSource = $state<'none' | 'constant'>('none');
  let autoEnterValue = $state('');
  let referenceEnabled = $state(false);
  let referenceName = $state('');
  let referenceToTable = $state<number | null>(null);
  let referenceToField = $state<number | null>(null);
  let hydrationKey = '';

  const hasRange = $derived(kind === 'number' || kind === 'date' || kind === 'time' || kind === 'timestamp');
  const rangeInputType = $derived(kind === 'number' ? 'number' : kind === 'date' ? 'date' : kind === 'time' ? 'time' : 'datetime-local');
  const autoEnterInputType = $derived(
    kind === 'number' ? 'number' : kind === 'date' ? 'date' : kind === 'time' ? 'time' : kind === 'timestamp' ? 'datetime-local' : 'text',
  );
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
      required = field?.options?.validation?.required ?? false;
      unique = field?.options?.validation?.unique ?? false;
      memberOfValueList = field?.options?.validation?.memberOfValueList ?? store.valueLists[0]?.id ?? null;
      memberOfEnabled = field?.options?.validation?.memberOfValueList != null;
      rangeMin = field?.options?.validation?.range?.min ?? '';
      rangeMax = field?.options?.validation?.range?.max ?? '';
      autoEnterSource = field?.options?.autoEnter?.kind ?? 'none';
      autoEnterValue = field?.options?.autoEnter?.value ?? '';
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

  function optionsDraft(): FieldOptions {
    const validation: NonNullable<FieldOptions['validation']> = {};
    if (required) validation.required = true;
    if (unique) validation.unique = true;
    if (memberOfEnabled && memberOfValueList != null) validation.memberOfValueList = memberOfValueList;
    if (hasRange && (rangeMin.trim() || rangeMax.trim())) {
      validation.range = {};
      if (rangeMin.trim()) validation.range.min = rangeMin.trim();
      if (rangeMax.trim()) validation.range.max = rangeMax.trim();
    }
    const options: FieldOptions = Object.keys(validation).length > 0 ? { validation } : {};
    if (autoEnterSource === 'constant') {
      options.autoEnter = { kind: 'constant', value: autoEnterValue };
    }
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

<SchemaDrawer title={field ? 'Field details' : 'New field'} {onclose}>
  {#snippet lead()}
    <div class="fd-chip">
      <Icon name={kindIcon(kind)} />
      <span class="fd-chip-name">{name || 'Untitled'}</span>
      <span class="fd-chip-sep">.</span>
      <span class="fd-chip-kind">{kindLabel(kind)}</span>
    </div>
  {/snippet}

  {#snippet footer()}
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
  {/snippet}

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
    <label class="fd-switch">
      <span class="fd-switch-text">Required</span>
      <span class="fd-toggle">
        <input type="checkbox" bind:checked={required} />
        <span class="fd-track"><span class="fd-knob"></span></span>
      </span>
    </label>
    <label class="fd-switch">
      <span class="fd-switch-text">Unique</span>
      <span class="fd-toggle">
        <input type="checkbox" bind:checked={unique} />
        <span class="fd-track"><span class="fd-knob"></span></span>
      </span>
    </label>
    <label class="fd-switch">
      <span class="fd-switch-text">Member of value list</span>
      <span class="fd-toggle">
        <input type="checkbox" bind:checked={memberOfEnabled} disabled={store.valueLists.length === 0} />
        <span class="fd-track"><span class="fd-knob"></span></span>
      </span>
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

  <section class="fd-section fd-auto" aria-labelledby="fd-auto-enter">
    <span id="fd-auto-enter" class="sc-micro fd-label">Auto-enter</span>
    <label>
      <span class="sc-hint">Source</span>
      <select class="sc-select" bind:value={autoEnterSource}>
        <option value="none">None</option>
        <option value="constant">Constant value</option>
      </select>
    </label>
    {#if autoEnterSource === 'constant'}
      <label>
        <span class="sc-hint">Value</span>
        <input class="sc-input" type={autoEnterInputType} bind:value={autoEnterValue} />
      </label>
    {/if}
  </section>

  <section class="fd-section" aria-labelledby="fd-reference">
    <span id="fd-reference" class="sc-micro fd-label">Reference</span>
    <label class="fd-switch">
      <span class="fd-switch-text">References another field</span>
      <span class="fd-toggle">
        <input type="checkbox" bind:checked={referenceEnabled} />
        <span class="fd-track"><span class="fd-knob"></span></span>
      </span>
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

  <p class="sc-hint fd-note">Drawer Save updates the draft. The schema is not applied until the window Save.</p>
</SchemaDrawer>

<style>
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
  .fd-label {
    display: block;
    margin: 0 0 6px;
  }
  .fd-label:not(:first-child) {
    margin-top: 16px;
  }
  .fd-section {
    margin-top: 16px;
  }
  /* Validation / reference options as iOS-style switches (label left, toggle
     right) — same geometry as the layout drawers' switches. */
  .fd-switch {
    min-height: 32px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
    color: var(--rm-text);
    cursor: pointer;
  }
  .fd-switch:has(input:disabled) {
    cursor: not-allowed;
  }
  .fd-switch-text {
    min-width: 0;
  }
  .fd-toggle {
    position: relative;
    display: inline-flex;
    flex: none;
  }
  .fd-toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .fd-track {
    width: 36px;
    height: 21px;
    border-radius: 21px;
    background: var(--rm-segment-track);
    transition: background 0.15s ease;
  }
  .fd-knob {
    position: absolute;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    top: 2px;
    left: 2px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
    transition: left 0.15s ease;
  }
  .fd-toggle input:checked + .fd-track {
    background: var(--rm-accent);
  }
  .fd-toggle input:checked + .fd-track .fd-knob {
    left: 17px;
  }
  .fd-toggle:has(input:disabled) .fd-track {
    opacity: 0.55;
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
  /* Auto-enter source select + value input stack like the reference fields. */
  .fd-auto label {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin-top: 8px;
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
  .fd-delete {
    color: var(--rm-danger);
  }
  .fd-spacer {
    flex: 1 1 auto;
  }
</style>
