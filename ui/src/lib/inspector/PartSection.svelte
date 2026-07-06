<script lang="ts">
  // Band inspector: kind / height / background fill, and reorder for summary
  // bands (Issue 4). Band-kind legality lives in ../part-rules (shared with the
  // rail's add-band gating) — this only binds it to the current selection.
  import type { EditorDoc } from '../doc.svelte';
  import {
    movePart as persistMovePart,
    setPartHeight as persistPartHeight,
    setPartKind as persistPartKind,
    setPartProps as persistPartProps,
  } from '../persist';
  import { canSetPartKind, partKindAllowedInView } from '../part-rules';
  import { parseProps } from '../object-props';
  import { llog } from '../log';
  import { colorValue } from './values';
  import { PART_KINDS } from './part-kinds';
  import { reportPersistError } from './persist-ops';

  let {
    doc,
    layoutId = '',
    busy = $bindable(false),
  }: { doc: EditorDoc; layoutId?: string; busy?: boolean } = $props();

  let selectedPartId = $derived(doc.selectedPartId);
  let selectedPart = $derived(selectedPartId === null ? undefined : doc.getPart(selectedPartId));
  let selectedPartProps = $derived(parseProps(selectedPart?.props ?? ''));
  // A form offers header/body/footer only; summaries are List/Table (Issue 3). The
  // current kind stays listed so an existing band always shows its own value.
  let partKinds = $derived(
    PART_KINDS.filter((p) => partKindAllowedInView(doc.view, p.id) || p.id === selectedPart?.kind),
  );
  // Summary bands (sub/grand) are reorderable between the header and footer (Issue 4).
  let sortedParts = $derived([...doc.parts].sort((a, b) => a.position - b.position || a.id - b.id));
  let selectedPartIdx = $derived(selectedPart ? sortedParts.findIndex((p) => p.id === selectedPart.id) : -1);
  let selectedPartIsSummary = $derived(
    !!selectedPart && (selectedPart.kind === 'subsummary' || selectedPart.kind === 'grandsummary'),
  );
  let canMovePartUp = $derived(
    selectedPartIsSummary && selectedPartIdx > 0 && sortedParts[selectedPartIdx - 1].kind !== 'header',
  );
  let canMovePartDown = $derived(
    selectedPartIsSummary &&
      selectedPartIdx >= 0 &&
      selectedPartIdx < sortedParts.length - 1 &&
      sortedParts[selectedPartIdx + 1].kind !== 'footer',
  );

  function canSetSelectedPartKind(kind: string): boolean {
    return !!selectedPart && canSetPartKind(doc.view, doc.parts, selectedPart, kind);
  }

  async function setSelectedPartKind(kind: string): Promise<void> {
    if (!selectedPart || !canSetSelectedPartKind(kind)) return;
    const id = selectedPart.id;
    llog('persist', 'inspector: set band kind', { id, kind });
    doc.setPartKind(id, kind);
    doc.mark();
    try {
      await persistPartKind(layoutId, id, kind);
    } catch (e) {
      reportPersistError(doc, 'set band kind', e);
    }
  }

  async function setSelectedPartFill(value: string): Promise<void> {
    if (!selectedPart) return;
    const id = selectedPart.id;
    const next = { ...selectedPartProps, fill: value };
    llog('persist', 'inspector: set band fill', { id, value });
    // Optimistic + undoable document change; the band's partStyle then refreshes
    // from the server's single-source derivation (mirrors object setStyle).
    doc.setPartProps(id, JSON.stringify(next));
    doc.mark();
    try {
      const view = await persistPartProps(layoutId, id, next);
      doc.setPartStyle(id, view.partStyle);
    } catch (e) {
      reportPersistError(doc, 'set band fill', e);
    }
  }

  async function setSelectedPartHeight(height: number): Promise<void> {
    if (!selectedPart) return;
    const id = selectedPart.id;
    const next = Math.max(doc.minPartHeight(id), Math.round(height || 1));
    llog('persist', 'inspector: set band height', { id, height: next });
    doc.setPartHeight(id, next);
    doc.mark();
    try {
      await persistPartHeight(layoutId, id, next);
    } catch (e) {
      reportPersistError(doc, 'set band height', e);
    }
  }

  async function moveSelectedPart(up: boolean): Promise<void> {
    if (!selectedPart || busy) return;
    if (up ? !canMovePartUp : !canMovePartDown) return;
    const id = selectedPart.id;
    busy = true;
    llog('persist', 'inspector: move band', { id, up });
    try {
      const positions = await persistMovePart(layoutId, id, up);
      doc.applyPartPositions(positions);
    } catch (e) {
      reportPersistError(doc, 'move band', e);
    } finally {
      busy = false;
    }
  }
</script>

{#if selectedPart}
  <section class="insp-sec">
    <span class="side-label">Band</span>
    <div class="insp-row">
      <span>Kind</span>
      <select
        class="ctl-select ctl-select-auto"
        value={selectedPart.kind}
        onchange={(e) => setSelectedPartKind(e.currentTarget.value)}
      >
        {#each partKinds as p (p.id)}
          <option value={p.id} disabled={!canSetSelectedPartKind(p.id)}>{p.label}</option>
        {/each}
      </select>
    </div>
    <div class="insp-row">
      <span>Height</span>
      <input
        class="ctl-num"
        type="number"
        min={doc.minPartHeight(selectedPart.id)}
        value={selectedPart.height}
        onchange={(e) => setSelectedPartHeight(Number(e.currentTarget.value))}
      />
    </div>
    <!-- Band background fill (Issue 7): the server's part_style() renders
         `background:{fill}` for the band, live on the canvas and in Browse. -->
    <div class="insp-row">
      <span>Background</span>
      <input
        class="swatch"
        type="color"
        value={colorValue(selectedPartProps.fill, '#ffffff')}
        onchange={(e) => setSelectedPartFill(e.currentTarget.value)}
      />
    </div>
    {#if selectedPartIsSummary}
      <!-- Summary bands reorder between the header and footer (Issue 4). -->
      <div class="insp-row">
        <span>Order</span>
        <div class="insp-ctls">
          <button
            type="button"
            class="ord-btn"
            title="Move band up"
            disabled={busy || !canMovePartUp}
            onclick={() => moveSelectedPart(true)}
          >↑</button>
          <button
            type="button"
            class="ord-btn"
            title="Move band down"
            disabled={busy || !canMovePartDown}
            onclick={() => moveSelectedPart(false)}
          >↓</button>
        </div>
      </div>
    {/if}
  </section>
{/if}
