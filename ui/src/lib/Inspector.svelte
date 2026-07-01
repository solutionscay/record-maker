<script lang="ts">
  // The Layout-mode INSPECTOR island (issue #62 follow-up) — the selection-aware
  // Format panel (header + Binding / Style / Text / Band sections + a pinned
  // delete), mounted into the right `#layout-inspector` node and SHARING the
  // canvas's EditorDoc store with the rail-tools island. Like the rail, it
  // reads/writes ONLY through the store + persist helpers; it never touches the
  // parity-checked canvas DOM. Styling follows the "modern Mac" design ref.
  import type { EditorDoc } from './doc.svelte';
  import {
    deleteObject as persistDeleteObject,
    deletePart as persistDeletePart,
    movePart as persistMovePart,
    setObjectBinding as persistBinding,
    setObjectContent as persistContent,
    setObjectProps as persistProps,
    setObjectReadOnly as persistReadOnly,
    setPartHeight as persistPartHeight,
    setPartKind as persistPartKind,
  } from './persist';
  import { llog, lerror } from './log';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  const PART_KINDS: { id: string; label: string }[] = [
    { id: 'header', label: 'Header' },
    { id: 'body', label: 'Body' },
    { id: 'footer', label: 'Footer' },
    { id: 'subsummary', label: 'Sub-summary' },
    { id: 'grandsummary', label: 'Grand summary' },
  ];
  const KIND_LABEL: Record<string, string> = {
    field: 'Field',
    text: 'Text',
    rect: 'Rectangle',
    ellipse: 'Ellipse',
    line: 'Line',
  };

  let busy = $state(false);

  // ── Selection-aware derived state ─────────────────────────────────────────

  let selectedIds = $derived([...doc.selection]);
  let selectedId = $derived(selectedIds[0] ?? null);
  let selected = $derived(selectedId === null ? undefined : doc.getObject(selectedId));
  let selectedProps = $derived(parseProps(selected?.props ?? ''));
  let canFillLine = $derived(
    !!selected && (selected.kind === 'field' || selected.kind === 'rect' || selected.kind === 'ellipse' || selected.kind === 'line'),
  );
  let canTextFormat = $derived(!!selected && (selected.kind === 'field' || selected.kind === 'text'));
  let selectedBindingFieldId = $derived(selected?.kind === 'field' ? fieldIdForBinding(selected.binding) : null);
  let selectedPartId = $derived(doc.selectedPartId);
  let selectedPart = $derived(selectedPartId === null ? undefined : doc.getPart(selectedPartId));
  // A form offers header/body/footer only; summaries are List/Table (Issue 3). The
  // current kind stays listed so an existing band always shows its own value.
  let partKinds = $derived(
    PART_KINDS.filter(
      (p) =>
        doc.view !== 'form' ||
        (p.id !== 'subsummary' && p.id !== 'grandsummary') ||
        p.id === selectedPart?.kind,
    ),
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

  // Header title/subtitle (design: "Field" · "Text · Name").
  let headerTitle = $derived(
    selected ? (KIND_LABEL[selected.kind] ?? 'Object') : selectedPart ? 'Band' : 'Inspector',
  );
  let headerSub = $derived(
    selected
      ? selected.kind === 'field'
        ? doc.fields.find((f) => f.id === selectedBindingFieldId)?.name || selected.binding || ''
        : selected.kind === 'text'
          ? 'Label'
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

  function partKindLabel(kind: string): string {
    return PART_KINDS.find((p) => p.id === kind)?.label ?? kind;
  }

  function fieldIdForBinding(binding: string): number | null {
    const fieldName = binding.split('.').at(-1)?.toLowerCase() ?? '';
    const found = doc.fields.find((f) => f.name.toLowerCase() === fieldName);
    return found?.id ?? null;
  }

  function parseProps(raw: string): Record<string, unknown> {
    if (!raw) return {};
    try {
      const p: unknown = JSON.parse(raw);
      return p && typeof p === 'object' ? (p as Record<string, unknown>) : {};
    } catch {
      return {};
    }
  }
  function colorValue(v: unknown, fallback: string): string {
    return typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v) ? v : fallback;
  }
  function numberValue(v: unknown, fallback: number): number {
    return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
  }
  function boolValue(v: unknown): boolean {
    return v === true;
  }
  function alignValue(v: unknown): string {
    return typeof v === 'string' && ['left', 'center', 'right'].includes(v) ? v : 'left';
  }

  function isSingletonPartKind(kind: string): boolean {
    return kind === 'header' || kind === 'body' || kind === 'footer';
  }

  function canSetSelectedPartKind(kind: string): boolean {
    if (!selectedPart) return false;
    if (selectedPart.kind === kind) return true;
    // A form allows only header/body/footer — summary bands are List/Table (Issue 3).
    if (doc.view === 'form' && (kind === 'subsummary' || kind === 'grandsummary')) return false;
    if (selectedPart.kind === 'body') return false;
    if (isSingletonPartKind(kind) && doc.parts.some((p) => p.id !== selectedPart.id && p.kind === kind)) return false;
    if (kind === 'grandsummary') {
      const body = doc.parts.find((p) => p.kind === 'body');
      if (!body) return false;
      const wantsTrailing = selectedPart.position > body.position;
      return !doc.parts.some(
        (p) => p.id !== selectedPart.id && p.kind === 'grandsummary' && (p.position > body.position) === wantsTrailing,
      );
    }
    return true;
  }

  // ── Object / Style / Text handlers ────────────────────────────────────────

  async function setStyle(key: string, value: string | number | boolean): Promise<void> {
    if (selectedId === null) return;
    const next = { ...selectedProps, [key]: value };
    llog('persist', 'inspector: set style', { id: selectedId, key, value });
    // Optimistic + undoable document change; the canvas's shapeStyle then refreshes
    // from the server's single-source derivation.
    doc.setObjectProps(selectedId, JSON.stringify(next));
    doc.mark();
    try {
      const styles = await persistProps(layoutId, selectedId, next);
      doc.setObjectStyles(selectedId, styles);
    } catch (e) {
      lerror('persist', 'set style failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function setSelectedBinding(nextFieldId: number): Promise<void> {
    if (selectedId === null || selected?.kind !== 'field' || !Number.isFinite(nextFieldId)) return;
    llog('persist', 'inspector: set field binding', { id: selectedId, fieldId: nextFieldId });
    try {
      const view = await persistBinding(layoutId, selectedId, nextFieldId, doc.rec);
      doc.setProp(selectedId, 'binding', view.binding);
      doc.refreshResolved(view);
      doc.mark();
    } catch (e) {
      lerror('persist', 'set field binding failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function setSelectedContent(content: string): Promise<void> {
    if (selectedId === null || selected?.kind !== 'text') return;
    llog('persist', 'inspector: set text content', { id: selectedId });
    doc.setProp(selectedId, 'content', content);
    doc.mark();
    try {
      const view = await persistContent(layoutId, selectedId, content);
      doc.setProp(selectedId, 'content', view.content);
    } catch (e) {
      lerror('persist', 'set text content failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function setSelectedReadOnly(readOnly: boolean): Promise<void> {
    if (selectedId === null) return;
    llog('persist', 'inspector: set read-only', { id: selectedId, readOnly });
    doc.setProp(selectedId, 'readOnly', readOnly);
    doc.mark();
    try {
      const view = await persistReadOnly(layoutId, selectedId, readOnly, doc.rec);
      doc.setProp(selectedId, 'readOnly', view.readOnly);
      doc.refreshResolved(view);
    } catch (e) {
      lerror('persist', 'set read-only failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteSelectedObjects(): Promise<void> {
    const ids = selectedIds;
    if (ids.length === 0 || busy) return;
    busy = true;
    llog('persist', 'inspector: delete object(s)', { ids });
    try {
      await Promise.all(ids.map((id) => persistDeleteObject(layoutId, id)));
      for (const id of ids) doc.removeObject(id);
      doc.mark();
    } catch (e) {
      lerror('persist', 'delete object failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  // ── Band inspector ──────────────────────────────────────────────────────

  async function setSelectedPartKind(kind: string): Promise<void> {
    if (!selectedPart || !canSetSelectedPartKind(kind)) return;
    const id = selectedPart.id;
    llog('persist', 'inspector: set band kind', { id, kind });
    doc.setPartKind(id, kind);
    doc.mark();
    try {
      await persistPartKind(layoutId, id, kind);
    } catch (e) {
      lerror('persist', 'set band kind failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
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
      lerror('persist', 'set band height failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
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
      lerror('persist', 'move band failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
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
      lerror('persist', 'delete band failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
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
  {#if selected}
    {#if selected.kind === 'field' || selected.kind === 'text'}
      <section class="insp-sec">
        <span class="side-label">{selected.kind === 'text' ? 'Text' : 'Binding'}</span>
        {#if selected.kind === 'field'}
          <select
            class="ctl-select"
            value={selectedBindingFieldId ?? ''}
            disabled={doc.fields.length === 0}
            onchange={(e) => setSelectedBinding(Number(e.currentTarget.value))}
          >
            {#if selectedBindingFieldId === null}
              <option value="" disabled>Unresolved</option>
            {/if}
            {#each doc.fields as f (f.id)}
              <option value={f.id}>{f.name}</option>
            {/each}
          </select>
          {#if selectedBindingFieldId === null && selected.binding}
            <span class="le-hint">{selected.binding}</span>
          {/if}
          <div class="insp-row">
            <span>Read-only</span>
            <label class="toggle">
              <input
                type="checkbox"
                checked={selected.readOnly}
                onchange={(e) => setSelectedReadOnly(e.currentTarget.checked)}
              />
              <span class="toggle-track"><span class="toggle-knob"></span></span>
            </label>
          </div>
        {:else}
          <input
            class="ctl-input"
            type="text"
            value={selected.content}
            onchange={(e) => setSelectedContent(e.currentTarget.value)}
          />
        {/if}
      </section>
    {/if}

    {#if canFillLine}
      <div class="insp-div"></div>
      <section class="insp-sec">
        <span class="side-label">Style</span>
        <div class="insp-row">
          <span>Fill</span>
          <input
            class="swatch"
            type="color"
            value={colorValue(selectedProps.fill, '#f7f8fa')}
            onchange={(e) => setStyle('fill', e.currentTarget.value)}
          />
        </div>
        <div class="insp-row">
          <span>Border</span>
          <div class="insp-ctls">
            <input
              class="ctl-num"
              type="number"
              min="0"
              max="12"
              value={numberValue(selectedProps.strokeWidth, 1)}
              onchange={(e) => setStyle('strokeWidth', Number(e.currentTarget.value))}
            />
            <input
              class="swatch"
              type="color"
              value={colorValue(selectedProps.stroke, '#d3d8de')}
              onchange={(e) => setStyle('stroke', e.currentTarget.value)}
            />
          </div>
        </div>
      </section>
    {/if}

    {#if canTextFormat}
      <div class="insp-div"></div>
      <section class="insp-sec">
        <span class="side-label">Text</span>
        <div class="insp-row">
          <span>Size</span>
          <input
            class="ctl-num"
            type="number"
            min="6"
            max="96"
            value={numberValue(selectedProps.fontSize, 13)}
            onchange={(e) => setStyle('fontSize', Number(e.currentTarget.value))}
          />
        </div>
        <div class="seg-row">
          <div class="seg">
            <button
              type="button"
              class="seg-btn"
              class:active={boolValue(selectedProps.bold)}
              title="Bold"
              onclick={() => setStyle('bold', !boolValue(selectedProps.bold))}
            ><b>B</b></button>
            <button
              type="button"
              class="seg-btn"
              class:active={boolValue(selectedProps.italic)}
              title="Italic"
              onclick={() => setStyle('italic', !boolValue(selectedProps.italic))}
            ><i>I</i></button>
            <button
              type="button"
              class="seg-btn"
              class:active={boolValue(selectedProps.underline)}
              title="Underline"
              onclick={() => setStyle('underline', !boolValue(selectedProps.underline))}
            ><u>U</u></button>
          </div>
          <div class="seg">
            {#each ['left', 'center', 'right'] as a}
              <button
                type="button"
                class="seg-btn"
                class:active={alignValue(selectedProps.align) === a}
                title={`Align ${a}`}
                onclick={() => setStyle('align', a)}
              >{a === 'left' ? 'L' : a === 'center' ? 'C' : 'R'}</button>
            {/each}
          </div>
        </div>
        <div class="insp-row">
          <span>Color</span>
          <input
            class="swatch"
            type="color"
            value={colorValue(selectedProps.textColor, '#1b1b1f')}
            onchange={(e) => setStyle('textColor', e.currentTarget.value)}
          />
        </div>
        {#if selected.kind === 'text'}
          <!-- Text objects have a background fill too (Issue 7); the server's
               object_style() renders `background:{fill}` for them. -->
          <div class="insp-row">
            <span>Background</span>
            <input
              class="swatch"
              type="color"
              value={colorValue(selectedProps.fill, '#ffffff')}
              onchange={(e) => setStyle('fill', e.currentTarget.value)}
            />
          </div>
        {/if}
      </section>
    {/if}
  {:else if selectedPart}
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
  {:else}
    <p class="insp-empty">Select an object or band to edit it.</p>
  {/if}
</div>

{#if selected}
  <footer class="insp-foot">
    <button
      type="button"
      class="insp-delete"
      title="Delete selected object"
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

<style>
  /* Format inspector — mirrors the design ref's right panel. Reuses the global
     `.side-label` and the shared --rm-* palette (defined on body). */
  .insp-head {
    padding: 16px 18px 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    border-bottom: 0.5px solid var(--rm-border);
  }
  .insp-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--rm-text);
  }
  .insp-sub {
    min-width: 0;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--rm-text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .insp-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .insp-sec {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .insp-div {
    height: 0.5px;
    background: var(--rm-border);
  }
  .insp-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 13px;
    color: var(--rm-text);
  }
  .insp-ctls {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .insp-empty {
    margin: 0;
    font-size: 12px;
    color: var(--rm-text-dim);
  }
  /* Controls */
  .ctl-select,
  .ctl-input {
    width: 100%;
    font: inherit;
    font-size: 13px;
    color: var(--rm-text);
    padding: 8px 11px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
  }
  .ctl-select-auto {
    width: auto;
    min-width: 120px;
  }
  .ctl-num {
    width: 66px;
    font: inherit;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: var(--rm-text);
    padding: 5px 8px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
  }
  .ctl-num:disabled {
    opacity: 0.5;
  }
  /* Band reorder buttons (Issue 4). */
  .ord-btn {
    width: 30px;
    height: 26px;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    font-size: 13px;
    line-height: 1;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .ord-btn:hover:not(:disabled) {
    background: #f0f0f2;
  }
  .ord-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  /* Color swatch — a 26px rounded chip. */
  .swatch {
    width: 26px;
    height: 26px;
    padding: 0;
    border: 1px solid var(--rm-border-strong);
    border-radius: 7px;
    background: var(--rm-control-bg);
    cursor: pointer;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
  }
  .swatch::-webkit-color-swatch-wrapper {
    padding: 0;
  }
  .swatch::-webkit-color-swatch {
    border: 0;
    border-radius: 6px;
  }
  .swatch:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  /* iOS-style toggle. */
  .toggle {
    position: relative;
    display: inline-flex;
    cursor: pointer;
  }
  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .toggle-track {
    width: 36px;
    height: 21px;
    border-radius: 21px;
    background: var(--rm-segment-track);
    transition: background 0.15s ease;
  }
  .toggle-knob {
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
  .toggle input:checked + .toggle-track {
    background: var(--rm-accent);
  }
  .toggle input:checked + .toggle-track .toggle-knob {
    left: 17px;
  }
  /* Segmented controls (B/I/U, L/C/R). */
  .seg-row {
    display: flex;
    gap: 10px;
  }
  .seg {
    flex: 1;
    display: inline-flex;
    background: var(--rm-segment-track);
    border-radius: 7px;
    padding: 2px;
  }
  .seg-btn {
    flex: 1;
    text-align: center;
    padding: 5px 0;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--rm-text-dim);
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    line-height: 1;
  }
  .seg-btn.active {
    background: var(--rm-segment-active-bg);
    color: var(--rm-text);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.14);
  }
  /* Pinned delete footer. */
  .insp-foot {
    margin-top: auto;
    padding: 14px 18px;
    border-top: 0.5px solid var(--rm-border);
  }
  .insp-delete {
    width: 100%;
    text-align: center;
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    color: var(--rm-danger);
    padding: 8px;
    border-radius: 8px;
    border: 0.5px solid var(--rm-border);
    background: var(--rm-control-bg);
    cursor: pointer;
  }
  .insp-delete:hover:not(:disabled) {
    background: #fff5f5;
  }
  .insp-delete:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .le-hint {
    font-size: 11px;
    color: var(--rm-text-dim);
  }
</style>
