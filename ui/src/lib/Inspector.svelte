<script lang="ts">
  // The Layout-mode INSPECTOR island (issue #62 follow-up) — the selection-aware
  // Format panel, mounted into the right `#layout-inspector` node and SHARING the
  // canvas's EditorDoc store with the rail-tools island. This root only decides
  // WHICH panel applies (multi-selection / single object / band / empty) plus the
  // header and pinned delete footer; the sections themselves are child components
  // under ./inspector, each reading/writing ONLY through the store + persist
  // helpers (#135). Panel CSS is the shared ./inspector/inspector.css vocabulary.
  import type { EditorDoc, ObjectDoc } from './doc.svelte';
  import { deletePart as persistDeletePart } from './persist';
  import { deleteSelected as deleteSelectedAction } from './actions';
  import { llog } from './log';
  import ArrangeSection from './inspector/ArrangeSection.svelte';
  import StyleSection from './inspector/StyleSection.svelte';
  import TextSection from './inspector/TextSection.svelte';
  import ObjectSection from './inspector/ObjectSection.svelte';
  import PositionSection from './inspector/PositionSection.svelte';
  import SizeSection from './inspector/SizeSection.svelte';
  import FormatSection from './inspector/FormatSection.svelte';
  import PartSection from './inspector/PartSection.svelte';
  import LayoutGridSection from './inspector/LayoutGridSection.svelte';
  import { partKindLabel } from './inspector/part-kinds';
  import { reportPersistError } from './inspector/persist-ops';
  import { parseProps } from './object-props';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  const KIND_LABEL: Record<string, string> = {
    field: 'Field',
    text: 'Text',
    rect: 'Rectangle',
    ellipse: 'Ellipse',
    line: 'Line',
    portal: 'Portal',
  };

  let busy = $state(false);

  // ── Selection-aware derived state ─────────────────────────────────────────

  let selectedIds = $derived([...doc.selection]);
  let hasMultipleSelection = $derived(selectedIds.length > 1);
  let selectedId = $derived(selectedIds[0] ?? null);
  let selected = $derived(selectedId === null ? undefined : doc.getObject(selectedId));
  let selectedProps = $derived(parseProps(selected?.props ?? ''));
  let selectedObjects = $derived(
    selectedIds.map((id) => doc.getObject(id)).filter((o): o is Readonly<ObjectDoc> => !!o),
  );

  // Capability gating: a section appears only when EVERY selected object shares
  // the capability (not just the first one). The per-kind capability table is the
  // engine's, shipped in design_model and read through `doc.capsFor`, so "can
  // this kind be filled / text-formatted" is defined exactly once, server-side.
  // A single selection is just the 1-element case of the same predicate.
  let allCanFillLine = $derived(
    selectedObjects.length > 0 && selectedObjects.every((o) => doc.capsFor(o.kind).fill),
  );
  let allCanTextFormat = $derived(
    selectedObjects.length > 0 && selectedObjects.every((o) => doc.capsFor(o.kind).textFormat),
  );

  // Field identity comes from the render model's server-resolved fieldId —
  // never re-derived from the binding string client-side (#134).
  let selectedBindingFieldId = $derived(
    selected?.kind === 'field' && selectedId !== null ? (doc.getResolved(selectedId)?.fieldId ?? null) : null,
  );
  // Value format (#77/#78) is contextual by the bound field's kind (#79/#76).
  let selectedFieldKind = $derived(
    selected?.kind === 'field' ? (doc.fields.find((f) => f.id === selectedBindingFieldId)?.kind ?? null) : null,
  );
  let hasValueFormat = $derived(
    selectedFieldKind !== null && ['number', 'bool', 'date', 'time', 'timestamp'].includes(selectedFieldKind),
  );

  let selectedPartId = $derived(doc.selectedPartId);
  let selectedPart = $derived(selectedPartId === null ? undefined : doc.getPart(selectedPartId));

  // Header title/subtitle (design: "Field" · "Text · Name").
  let headerTitle = $derived(
    hasMultipleSelection ? 'Multiple items selected' : selected ? (KIND_LABEL[selected.kind] ?? 'Object') : selectedPart ? 'Band' : 'Layout',
  );
  let headerSub = $derived(
    hasMultipleSelection
      ? `${selectedIds.length} objects`
      : selected
      ? selected.kind === 'field'
        ? doc.fields.find((f) => f.id === selectedBindingFieldId)?.name || selected.binding || ''
        : selected.kind === 'text'
          ? 'Label'
          : selected.kind === 'portal'
            ? selected.binding || 'Related list'
            : ''
      : selectedPart
        ? partKindLabel(selectedPart.kind)
        : '',
  );
  let deleteLabel = $derived(
    selectedIds.length > 1
      ? 'Delete objects'
      : selected?.kind === 'field'
        ? 'Delete Field'
        : selected
          ? `Delete ${KIND_LABEL[selected.kind] ?? 'Object'}`
          : '',
  );

  // Delete runs through the shared command layer (./actions) so the Inspector
  // button and the canvas keyboard/context menu share one implementation —
  // including the post-delete canvas chrome cleanup.
  async function deleteSelectedObjects(): Promise<void> {
    if (busy) return;
    busy = true;
    await deleteSelectedAction(doc, layoutId);
    busy = false;
  }

  async function deleteSelectedPart(): Promise<void> {
    if (!selectedPart || selectedPart.kind === 'body' || busy) return;
    const id = selectedPart.id;
    busy = true;
    llog('persist', 'inspector: delete band', { id });
    try {
      await persistDeletePart(layoutId, id);
      doc.removePart(id);
    } catch (e) {
      reportPersistError(doc, 'delete band', e);
    } finally {
      busy = false;
    }
  }
</script>

<header class="insp-head">
  <span class="insp-title">{headerTitle}</span>
  {#if headerSub}<span class="insp-sub">{headerSub}</span>{/if}
</header>

<div class="insp-body">
  {#if hasMultipleSelection}
    <ArrangeSection {doc} {layoutId} multi bind:busy />
    <div class="insp-div"></div>
    <SizeSection {doc} {layoutId} objects={selectedObjects} />
    {#if allCanFillLine || allCanTextFormat}
      <div class="insp-div"></div>
      {#if allCanFillLine}
        <StyleSection {doc} {layoutId} objects={selectedObjects} />
      {/if}
      {#if allCanTextFormat}
        {#if allCanFillLine}<div class="insp-div"></div>{/if}
        <TextSection {doc} {layoutId} objects={selectedObjects} />
      {/if}
    {/if}
  {:else if selected}
    <ArrangeSection {doc} {layoutId} bind:busy />
    <div class="insp-div"></div>
    <PositionSection {doc} {layoutId} {selected} />
    <div class="insp-div"></div>
    <SizeSection {doc} {layoutId} objects={selectedObjects} />
    <div class="insp-div"></div>
    {#if selected.kind === 'field' || selected.kind === 'text' || selected.kind === 'portal'}
      <ObjectSection {doc} {layoutId} {selected} fieldId={selectedBindingFieldId} />
    {/if}

    {#if allCanFillLine}
      <div class="insp-div"></div>
      <StyleSection {doc} {layoutId} objects={selectedObjects} />
    {/if}

    {#if allCanTextFormat}
      <div class="insp-div"></div>
      <TextSection {doc} {layoutId} objects={selectedObjects} />
    {/if}

    {#if hasValueFormat && selectedId !== null && selectedFieldKind !== null}
      <div class="insp-div"></div>
      <FormatSection {doc} {layoutId} {selectedId} props={selectedProps} fieldKind={selectedFieldKind} />
    {/if}
  {:else if selectedPart}
    <PartSection {doc} {layoutId} bind:busy />
  {:else}
    <LayoutGridSection {doc} {layoutId} />
  {/if}
</div>

{#if selectedIds.length > 0}
  <footer class="insp-foot">
    <button
      type="button"
      class="insp-delete"
      title={hasMultipleSelection ? 'Delete selected objects' : 'Delete selected object'}
      disabled={selectedIds.length === 0 || busy}
      onclick={deleteSelectedObjects}
    >{deleteLabel}</button>
  </footer>
{:else if selectedPart}
  <footer class="insp-foot">
    <button
      type="button"
      class="insp-delete"
      title="Delete selected band"
      disabled={busy || selectedPart.kind === 'body'}
      onclick={deleteSelectedPart}
    >Delete band</button>
  </footer>
{/if}
