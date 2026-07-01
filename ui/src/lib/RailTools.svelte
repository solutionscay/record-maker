<script lang="ts">
  // The Layout-mode rail TOOLS (issue #62) — the design-mode counterpart of the
  // server-rendered Navigate zone, mounted into the sidebar's `#layout-tools`
  // node and SHARING the canvas's EditorDoc store. Four zones:
  //   • Mode   — select/move mode for existing canvas objects.
  //   • Create — a tool palette (#48): arm a tool, click the canvas to place an
  //     object (the canvas's interaction layer does the placing); a Field dropdown
  //     binds the field a placement uses; a Part control appends a band.
  //   • Style  — selection-aware appearance swatches (#47/#49): fill / line colour
  //     and line width, editing the selected object's `props` bag. The server
  //     re-derives the shape style and we feed it back for live canvas feedback.
  //   • Zoom   — canvas zoom out / readout / in (a viewport concern).
  // It reads/writes ONLY through the store + persist helpers; it never touches the
  // parity-checked canvas DOM.
  import type { EditorDoc, ToolKind } from './doc.svelte';
  import {
    createPart,
    deleteObject as persistDeleteObject,
    deletePart as persistDeletePart,
    setObjectProps as persistProps,
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
  let partKind = $state('body');
  let busy = $state(false);

  // Default the Field dropdown to the first field once the model has hydrated.
  $effect(() => {
    if (fieldId === null && doc.fields.length > 0) fieldId = doc.fields[0].id;
  });

  // ── Mode / create zones ─────────────────────────────────────────────────

  function pickTool(t: ToolKind): void {
    llog('tool', 'rail: pick tool', { tool: t, fieldId });
    doc.setTool(t, t === 'field' ? fieldId : null);
  }
  function onFieldChange(): void {
    if (doc.activeTool === 'field') doc.setTool('field', fieldId);
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
  let canStyle = $derived(!!selected);
  let selectedPartId = $derived(doc.selectedPartId);
  let selectedPart = $derived(selectedPartId === null ? undefined : doc.getPart(selectedPartId));

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

  async function setStyle(key: string, value: string | number): Promise<void> {
    if (selectedId === null) return;
    const next = { ...selectedProps, [key]: value };
    llog('persist', 'rail: set style', { id: selectedId, key, value });
    // Optimistic + undoable document change; the canvas's shapeStyle then refreshes
    // from the server's single-source derivation.
    doc.setObjectProps(selectedId, JSON.stringify(next));
    doc.mark();
    try {
      const shapeStyle = await persistProps(layoutId, selectedId, next);
      doc.setShapeStyle(selectedId, shapeStyle);
    } catch (e) {
      lerror('persist', 'set style failed', e);
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
  <span class="side-label">Mode</span>
  <div class="le-tools le-tools-single">
    {#each MODE_TOOLS as t (t.id)}
      <button
        type="button"
        class:active={doc.activeTool === t.id}
        aria-pressed={doc.activeTool === t.id}
        title={t.label}
        onclick={() => pickTool(t.id)}
      >{t.glyph}</button>
    {/each}
  </div>
</section>

<section class="le-zone">
  <span class="side-label">Create</span>
  <div class="le-tools">
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
  <div class="le-part-row">
    <select class="le-select" bind:value={partKind} title="Band kind">
      {#each PART_KINDS as p (p.id)}
        <option value={p.id}>{p.label}</option>
      {/each}
    </select>
    <button type="button" class="le-icon-btn" title="Add band" onclick={addPart} disabled={busy}>+</button>
  </div>
</section>

<section class="le-zone">
  <span class="side-label">Style</span>
  <label class="le-control">
    <span>Fill</span>
    <input
      type="color"
      value={colorValue(selectedProps.fill, '#f7f8fa')}
      disabled={!canStyle}
      onchange={(e) => setStyle('fill', e.currentTarget.value)}
    />
  </label>
  <label class="le-control">
    <span>Line</span>
    <input
      type="color"
      value={colorValue(selectedProps.stroke, '#d3d8de')}
      disabled={!canStyle}
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
      disabled={!canStyle}
      onchange={(e) => setStyle('strokeWidth', Number(e.currentTarget.value))}
    />
  </label>
  {#if !canStyle}
    <span class="le-hint">Select an object to style it.</span>
  {/if}
  <button
    type="button"
    class="le-danger-btn"
    title="Delete selected object"
    disabled={selectedIds.length === 0 || busy}
    onclick={deleteSelectedObjects}
  >Delete object</button>
</section>

<section class="le-zone">
  <span class="side-label">Band</span>
  <label class="le-control">
    <span>Kind</span>
    <select
      class="le-compact-select"
      value={selectedPart?.kind ?? partKind}
      disabled={!selectedPart}
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
      min={selectedPart ? doc.minPartHeight(selectedPart.id) : 1}
      value={selectedPart?.height ?? 0}
      disabled={!selectedPart}
      onchange={(e) => setSelectedPartHeight(Number(e.currentTarget.value))}
    />
  </label>
  <button
    type="button"
    class="le-danger-btn"
    title="Delete selected band"
    disabled={!selectedPart || busy}
    onclick={deleteSelectedPart}
  >Delete band</button>
  {#if !selectedPart}
    <span class="le-hint">Select a band to edit it.</span>
  {/if}
</section>

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
  .le-tools-single {
    grid-template-columns: 1fr;
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
  .le-part-row {
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
