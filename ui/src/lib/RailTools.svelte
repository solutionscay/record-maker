<script lang="ts">
  // The Layout-mode rail TOOLS (issue #62) — the design-mode counterpart of the
  // server-rendered Navigate zone, mounted into the sidebar's `#layout-tools`
  // node and SHARING the canvas's EditorDoc store. It is a compact tool palette
  // plus contextual inspectors for selected objects/bands.
  // It reads/writes ONLY through the store + persist helpers; it never touches the
  // parity-checked canvas DOM.
  import type { EditorDoc, ToolKind } from './doc.svelte';
  import {
    createPart,
    deleteObject as persistDeleteObject,
    deletePart as persistDeletePart,
    setObjectBinding as persistBinding,
    setObjectContent as persistContent,
    setObjectProps as persistProps,
    setObjectReadOnly as persistReadOnly,
    setPartHeight as persistPartHeight,
    setPartKind as persistPartKind,
  } from './persist';
  import { llog, lerror } from './log';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  const MODE_TOOLS: { id: ToolKind; label: string; glyph: string }[] = [
    { id: 'pointer', label: 'Select / move objects', glyph: '➚' },
  ];
  const CREATE_TOOLS: { id: ToolKind; label: string; glyph: string }[] = [
    { id: 'text', label: 'Text label', glyph: 'T' },
    { id: 'field', label: 'Field', glyph: '▭' },
    { id: 'rect', label: 'Rectangle', glyph: '▢' },
    { id: 'ellipse', label: 'Ellipse', glyph: '◯' },
    { id: 'line', label: 'Line', glyph: '╱' },
  ];
  const PART_KINDS: { id: string; label: string }[] = [
    { id: 'header', label: 'Header' },
    { id: 'body', label: 'Body' },
    { id: 'footer', label: 'Footer' },
    { id: 'subsummary', label: 'Sub-summary' },
    { id: 'grandsummary', label: 'Grand summary' },
  ];

  let fieldId = $state<number | null>(null);
  let createLabel = $state(true);
  let partKind = $state('body');
  let busy = $state(false);

  // Default the Field dropdown to the first field once the model has hydrated.
  $effect(() => {
    if (fieldId === null && doc.fields.length > 0) fieldId = doc.fields[0].id;
  });

  // ── Mode / create zones ─────────────────────────────────────────────────

  function pickTool(t: ToolKind): void {
    llog('tool', 'rail: pick tool', { tool: t, fieldId, createLabel });
    doc.setTool(t, t === 'field' ? fieldId : null, createLabel);
  }
  function onFieldChange(): void {
    if (doc.activeTool === 'field') doc.setTool('field', fieldId, createLabel);
  }
  function onCreateLabelChange(): void {
    if (doc.activeTool === 'field') doc.setTool('field', fieldId, createLabel);
  }
  async function addPart(): Promise<void> {
    if (busy) return;
    busy = true;
    llog('create', 'rail: add band', { kind: partKind });
    try {
      doc.addPart(await createPart(layoutId, partKind, 80));
    } catch (e) {
      lerror('create', 'add band failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  // ── Style zone (selection-aware) ─────────────────────────────────────────

  let selectedIds = $derived([...doc.selection]);
  let selectedId = $derived(selectedIds[0] ?? null);
  let selected = $derived(selectedId === null ? undefined : doc.getObject(selectedId));
  let selectedProps = $derived(parseProps(selected?.props ?? ''));
  let selectedKind = $derived(selected?.kind ?? '');
  let canFillLine = $derived(
    !!selected && (selected.kind === 'field' || selected.kind === 'rect' || selected.kind === 'ellipse' || selected.kind === 'line'),
  );
  let canTextFormat = $derived(!!selected && (selected.kind === 'field' || selected.kind === 'text'));
  let selectedBindingFieldId = $derived(selected?.kind === 'field' ? fieldIdForBinding(selected.binding) : null);
  let selectedPartId = $derived(doc.selectedPartId);
  let selectedPart = $derived(selectedPartId === null ? undefined : doc.getPart(selectedPartId));

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

  async function setStyle(key: string, value: string | number | boolean): Promise<void> {
    if (selectedId === null) return;
    const next = { ...selectedProps, [key]: value };
    llog('persist', 'rail: set style', { id: selectedId, key, value });
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
    llog('persist', 'rail: set field binding', { id: selectedId, fieldId: nextFieldId });
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
    llog('persist', 'rail: set text content', { id: selectedId });
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
    llog('persist', 'rail: set read-only', { id: selectedId, readOnly });
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
    llog('persist', 'rail: delete object(s)', { ids });
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
    if (!selectedPart) return;
    const id = selectedPart.id;
    llog('persist', 'rail: set band kind', { id, kind });
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
    llog('persist', 'rail: set band height', { id, height: next });
    doc.setPartHeight(id, next);
    doc.mark();
    try {
      await persistPartHeight(layoutId, id, next);
    } catch (e) {
      lerror('persist', 'set band height failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function deleteSelectedPart(): Promise<void> {
    if (!selectedPart || busy) return;
    const id = selectedPart.id;
    busy = true;
    llog('persist', 'rail: delete band', { id });
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

  // ── Zoom zone ────────────────────────────────────────────────────────────

  function zoomBy(delta: number): void {
    doc.setZoom(doc.zoom + delta);
  }
</script>

<section class="le-zone">
  <span class="side-label">Tools</span>
  <div class="le-tools">
    {#each MODE_TOOLS as t (t.id)}
      <button
        type="button"
        class:active={doc.activeTool === t.id}
        aria-pressed={doc.activeTool === t.id}
        title={t.label}
        onclick={() => pickTool(t.id)}
      >{t.glyph}</button>
    {/each}
    {#each CREATE_TOOLS as t (t.id)}
      <button
        type="button"
        class:active={doc.activeTool === t.id}
        aria-pressed={doc.activeTool === t.id}
        title={t.label}
        onclick={() => pickTool(t.id)}
      >{t.glyph}</button>
    {/each}
  </div>
  {#if doc.activeTool === 'field'}
    <label class="le-control le-control-stack">
      <span>Field to place</span>
      <select
        class="le-select"
        bind:value={fieldId}
        onchange={onFieldChange}
        disabled={doc.fields.length === 0}
        title="Field to place"
      >
        {#each doc.fields as f (f.id)}
          <option value={f.id}>{f.name}</option>
        {/each}
      </select>
    </label>
    <label class="le-check">
      <input
        type="checkbox"
        bind:checked={createLabel}
        onchange={onCreateLabelChange}
      />
      <span>Create label</span>
    </label>
  {/if}
  <div class="le-combo-row">
    <select class="le-select" bind:value={partKind} title="Band kind">
      {#each PART_KINDS as p (p.id)}
        <option value={p.id}>{p.label}</option>
      {/each}
    </select>
    <button type="button" class="le-icon-btn" title="Add band" onclick={addPart} disabled={busy}>+</button>
  </div>
</section>

<section class="le-zone">
  <span class="side-label">Object</span>
  {#if selected}
    <div class="le-object-head">
      <span>{selectedKind}</span>
      {#if selected.kind === 'field'}
        <label class="le-check">
          <input
            type="checkbox"
            checked={selected.readOnly}
            onchange={(e) => setSelectedReadOnly(e.currentTarget.checked)}
          />
          <span>Read-only</span>
        </label>
      {/if}
    </div>
    {#if selected.kind === 'field'}
      <label class="le-control le-control-stack">
        <span>Binding</span>
        <select
          class="le-select"
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
      </label>
    {:else if selected.kind === 'text'}
      <label class="le-control le-control-stack">
        <span>Text</span>
        <input
          class="le-text-input"
          type="text"
          value={selected.content}
          onchange={(e) => setSelectedContent(e.currentTarget.value)}
        />
      </label>
    {/if}
    <button
      type="button"
      class="le-danger-btn"
      title="Delete selected object"
      disabled={selectedIds.length === 0 || busy}
      onclick={deleteSelectedObjects}
    >Delete object</button>
  {:else}
    <span class="le-hint">Select an object to edit it.</span>
  {/if}
</section>

<section class="le-zone">
  <span class="side-label">Style</span>
  <label class="le-control">
    <span>Fill</span>
    <input
      type="color"
      value={colorValue(selectedProps.fill, '#f7f8fa')}
      disabled={!canFillLine}
      onchange={(e) => setStyle('fill', e.currentTarget.value)}
    />
  </label>
  <label class="le-control">
    <span>Line</span>
    <input
      type="color"
      value={colorValue(selectedProps.stroke, '#d3d8de')}
      disabled={!canFillLine}
      onchange={(e) => setStyle('stroke', e.currentTarget.value)}
    />
  </label>
  <label class="le-control">
    <span>Width</span>
    <input
      type="number"
      min="0"
      max="12"
      value={numberValue(selectedProps.strokeWidth, 1)}
      disabled={!canFillLine}
      onchange={(e) => setStyle('strokeWidth', Number(e.currentTarget.value))}
    />
  </label>
  {#if !canFillLine}
    <span class="le-hint">Select a field or shape.</span>
  {/if}
</section>

{#if canTextFormat}
  <section class="le-zone">
    <span class="side-label">Text</span>
    <label class="le-control">
      <span>Color</span>
      <input
        type="color"
        value={colorValue(selectedProps.textColor, '#1b1b1f')}
        onchange={(e) => setStyle('textColor', e.currentTarget.value)}
      />
    </label>
    <label class="le-control">
      <span>Size</span>
      <input
        type="number"
        min="6"
        max="96"
        value={numberValue(selectedProps.fontSize, 13)}
        onchange={(e) => setStyle('fontSize', Number(e.currentTarget.value))}
      />
    </label>
    <div class="le-toggle-row">
      <button
        type="button"
        class:active={boolValue(selectedProps.bold)}
        title="Bold"
        onclick={() => setStyle('bold', !boolValue(selectedProps.bold))}
      >B</button>
      <button
        type="button"
        class:active={boolValue(selectedProps.italic)}
        title="Italic"
        onclick={() => setStyle('italic', !boolValue(selectedProps.italic))}
      ><i>I</i></button>
      <button
        type="button"
        class:active={boolValue(selectedProps.underline)}
        title="Underline"
        onclick={() => setStyle('underline', !boolValue(selectedProps.underline))}
      ><u>U</u></button>
    </div>
    <div class="le-align-row" title="Text alignment">
      {#each ['left', 'center', 'right'] as a}
        <button
          type="button"
          class:active={alignValue(selectedProps.align) === a}
          onclick={() => setStyle('align', a)}
        >{a === 'left' ? 'L' : a === 'center' ? 'C' : 'R'}</button>
      {/each}
    </div>
  </section>
{/if}

{#if selectedPart}
  <section class="le-zone">
    <span class="side-label">Band</span>
  <label class="le-control">
    <span>Kind</span>
    <select
      class="le-compact-select"
      value={selectedPart.kind}
      onchange={(e) => setSelectedPartKind(e.currentTarget.value)}
    >
      {#each PART_KINDS as p (p.id)}
        <option value={p.id}>{p.label}</option>
      {/each}
    </select>
  </label>
  <label class="le-control">
    <span>Height</span>
    <input
      type="number"
      min={doc.minPartHeight(selectedPart.id)}
      value={selectedPart.height}
      onchange={(e) => setSelectedPartHeight(Number(e.currentTarget.value))}
    />
  </label>
  <button
    type="button"
    class="le-danger-btn"
    title="Delete selected band"
    disabled={busy}
    onclick={deleteSelectedPart}
  >Delete band</button>
  </section>
{/if}

<section class="le-zone">
  <span class="side-label">Zoom</span>
  <div class="le-zoom">
    <button type="button" class="le-icon-btn" title="Zoom out" onclick={() => zoomBy(-0.1)}>−</button>
    <span class="le-zoom-num">{Math.round(doc.zoom * 100)}%</span>
    <button type="button" class="le-icon-btn" title="Zoom in" onclick={() => zoomBy(0.1)}>+</button>
  </div>
</section>

<style>
  /* Rail tools live inside the server-styled `.sidebar`; reuse its `.side-label`
     vocabulary (a global class) and add only what's specific to the tools. */
  .le-zone {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .le-tools {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.3rem;
  }
  .le-tools button {
    height: 30px;
    padding: 0;
    border: 1px solid #ccc;
    border-radius: 0.35rem;
    background: #fff;
    cursor: pointer;
    font-size: 0.95rem;
    line-height: 1;
  }
  .le-tools button:hover {
    background: #eee;
  }
  .le-tools button.active {
    background: #1f6feb;
    border-color: #1f6feb;
    color: #fff;
  }
  .le-select {
    width: 100%;
    min-width: 0;
    font: inherit;
    font-size: 0.78rem;
    padding: 0.25rem;
  }
  .le-compact-select {
    min-width: 0;
    width: 7rem;
    font: inherit;
    font-size: 0.74rem;
    padding: 0.2rem;
  }
  .le-combo-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 28px;
    gap: 0.3rem;
  }
  .le-icon-btn {
    height: 28px;
    width: 100%;
    padding: 0;
    border: 1px solid #ccc;
    border-radius: 0.35rem;
    background: #fff;
    cursor: pointer;
    line-height: 1;
  }
  .le-icon-btn:hover:not(:disabled) {
    background: #eee;
  }
  .le-icon-btn:disabled {
    color: #bbb;
    cursor: not-allowed;
  }
  .le-danger-btn {
    min-height: 28px;
    padding: 0.25rem 0.4rem;
    border: 1px solid #d0a5a5;
    border-radius: 0.35rem;
    background: #fff;
    color: #8a1f1f;
    cursor: pointer;
    font: inherit;
    font-size: 0.74rem;
  }
  .le-danger-btn:hover:not(:disabled) {
    background: #fff0f0;
  }
  .le-danger-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .le-control {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0.4rem;
    align-items: center;
    font-size: 0.76rem;
    color: #444;
  }
  .le-control-stack {
    grid-template-columns: 1fr;
    gap: 0.2rem;
  }
  .le-object-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.4rem;
    min-width: 0;
    font-size: 0.76rem;
    color: #555;
  }
  .le-object-head > span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-transform: capitalize;
  }
  .le-check {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    white-space: nowrap;
    font-size: 0.7rem;
    color: #444;
  }
  .le-check input {
    width: 14px;
    height: 14px;
    margin: 0;
  }
  .le-text-input {
    min-width: 0;
    width: 100%;
    font: inherit;
    font-size: 0.78rem;
    padding: 0.25rem;
    border: 1px solid #ccc;
    border-radius: 0.3rem;
  }
  .le-control input[type='color'] {
    width: 34px;
    height: 26px;
    padding: 1px;
    border: 1px solid #ccc;
    border-radius: 0.3rem;
    background: #fff;
  }
  .le-control input[type='number'] {
    width: 52px;
    font: inherit;
    padding: 0.2rem 0.3rem;
    border: 1px solid #ccc;
    border-radius: 0.3rem;
  }
  .le-control input:disabled {
    opacity: 0.5;
  }
  .le-toggle-row,
  .le-align-row {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0.25rem;
  }
  .le-toggle-row button,
  .le-align-row button {
    height: 26px;
    padding: 0;
    border: 1px solid #ccc;
    border-radius: 0.3rem;
    background: #fff;
    cursor: pointer;
    font: inherit;
    font-size: 0.72rem;
  }
  .le-toggle-row button.active,
  .le-align-row button.active {
    background: #1f6feb;
    border-color: #1f6feb;
    color: #fff;
  }
  .le-hint {
    font-size: 0.68rem;
    color: #999;
  }
  .le-zoom {
    display: grid;
    grid-template-columns: 28px 1fr 28px;
    gap: 0.25rem;
    align-items: center;
  }
  .le-zoom-num {
    text-align: center;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
</style>
