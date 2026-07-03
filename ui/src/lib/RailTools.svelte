<script lang="ts">
  // The Layout-mode rail TOOLS (issue #62) — the design-mode counterpart of the
  // server-rendered Navigate zone, mounted into the sidebar's `#layout-tools`
  // node and SHARING the canvas's EditorDoc store. It is a compact tool palette
  // plus the band-add combo and the zoom control. The selection-aware inspectors
  // (Object / Style / Text / Band) live in the right-panel Inspector island.
  // It reads/writes ONLY through the store + persist helpers; it never touches the
  // parity-checked canvas DOM.
  import type { EditorDoc, ToolKind } from './doc.svelte';
  import { createPart } from './persist';
  import { runUndo, runRedo } from './history';
  import { llog, lerror } from './log';
  import Icon from './Icon.svelte';
  import FieldSelect from './FieldSelect.svelte';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  // Each tool carries the id of the shared sprite symbol (#72) it renders; the
  // line tool reuses `minus` (a horizontal rule) as its glyph.
  const MODE_TOOLS: { id: ToolKind; label: string; icon: string }[] = [
    { id: 'pointer', label: 'Select / move objects', icon: 'pointer' },
  ];
  const CREATE_TOOLS: { id: ToolKind; label: string; icon: string }[] = [
    { id: 'text', label: 'Text label', icon: 'text' },
    { id: 'field', label: 'Field', icon: 'field' },
    { id: 'rect', label: 'Rectangle', icon: 'rect' },
    { id: 'ellipse', label: 'Ellipse', icon: 'ellipse' },
    { id: 'line', label: 'Line', icon: 'minus' },
  ];
  const PART_KINDS: { id: string; label: string }[] = [
    { id: 'header', label: 'Header' },
    { id: 'body', label: 'Body' },
    { id: 'footer', label: 'Footer' },
    { id: 'subsummary', label: 'Sub-summary' },
    { id: 'grandsummary', label: 'Grand summary' },
  ];

  let fieldIds = $state<number[]>([]);
  let createLabel = $state(true);
  let partKind = $state('body');
  let busy = $state(false);

  let usedPartKinds = $derived(new Set(doc.parts.map((p) => p.kind)));
  // A form is a single-record view: sub/grand summaries are report-only, so a form
  // layout offers header/body/footer only (Issue 3). List/Table keep all five.
  let partKinds = $derived(
    PART_KINDS.filter((p) => doc.view !== 'form' || (p.id !== 'subsummary' && p.id !== 'grandsummary')),
  );

  // Default the Field dropdown to the first field once the model has hydrated.
  $effect(() => {
    if (fieldIds.length === 0 && doc.fields.length > 0) fieldIds = [doc.fields[0].id];
  });

  $effect(() => {
    if (!canAddPartKind(partKind)) {
      partKind = PART_KINDS.find((p) => canAddPartKind(p.id))?.id ?? partKind;
    }
  });

  // ── Mode / create zones ─────────────────────────────────────────────────

  function pickTool(t: ToolKind): void {
    llog('tool', 'rail: pick tool', { tool: t, fieldIds, createLabel });
    doc.setTool(t, t === 'field' ? fieldIds : null, createLabel);
  }
  function onFieldChange(): void {
    if (doc.activeTool === 'field') doc.setTool('field', fieldIds, createLabel);
  }
  function onCreateLabelChange(): void {
    if (doc.activeTool === 'field') doc.setTool('field', fieldIds, createLabel);
  }
  async function addPart(): Promise<void> {
    if (busy || !canAddPartKind(partKind)) return;
    busy = true;
    llog('create', 'rail: add band', { kind: partKind });
    try {
      const { part, positions } = await createPart(layoutId, partKind, 80);
      doc.addPart(part, positions);
    } catch (e) {
      lerror('create', 'add band failed', e);
      doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function isSingletonPartKind(kind: string): boolean {
    return kind === 'header' || kind === 'body' || kind === 'footer';
  }

  function canAddPartKind(kind: string): boolean {
    // A form allows only header/body/footer — summary bands are List/Table (Issue 3).
    if (doc.view === 'form' && (kind === 'subsummary' || kind === 'grandsummary')) return false;
    if (isSingletonPartKind(kind) && usedPartKinds.has(kind)) return false;
    if (kind === 'grandsummary') {
      const body = doc.parts.find((p) => p.kind === 'body');
      if (!body) return false;
      const leading = doc.parts.some((p) => p.kind === 'grandsummary' && p.position < body.position);
      const trailing = doc.parts.some((p) => p.kind === 'grandsummary' && p.position > body.position);
      return !(leading && trailing);
    }
    return true;
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
        aria-label={t.label}
        onclick={() => pickTool(t.id)}
      ><Icon name={t.icon} /></button>
    {/each}
    {#each CREATE_TOOLS as t (t.id)}
      <button
        type="button"
        class:active={doc.activeTool === t.id}
        aria-pressed={doc.activeTool === t.id}
        title={t.label}
        aria-label={t.label}
        onclick={() => pickTool(t.id)}
      ><Icon name={t.icon} /></button>
    {/each}
  </div>
  {#if doc.activeTool === 'field'}
    <div class="le-control le-control-stack">
      <span>Field to place</span>
      <FieldSelect
        fields={doc.fields}
        value={fieldIds[0] ?? null}
        values={fieldIds}
        multi
        onselect={(id) => {
          fieldIds = [id];
          onFieldChange();
        }}
        onselectMany={(ids) => {
          fieldIds = ids;
          onFieldChange();
        }}
        title="Field to place; Shift-click range, Ctrl/Cmd-click individual"
      />
    </div>
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
      {#each partKinds as p (p.id)}
        <option value={p.id} disabled={!canAddPartKind(p.id)}>{p.label}</option>
      {/each}
    </select>
    <button type="button" class="le-icon-btn" title="Add band" aria-label="Add band" onclick={addPart} disabled={busy || !canAddPartKind(partKind)}><Icon name="plus" /></button>
  </div>
</section>

<section class="le-zone">
  <span class="side-label">History</span>
  <div class="le-history">
    <button type="button" class="le-icon-btn" title="Undo" aria-label="Undo"
            onclick={() => runUndo(doc, layoutId)} disabled={!doc.canUndo}><Icon name="undo" /></button>
    <button type="button" class="le-icon-btn" title="Redo" aria-label="Redo"
            onclick={() => runRedo(doc, layoutId)} disabled={!doc.canRedo}><Icon name="redo" /></button>
  </div>
</section>

<section class="le-zone">
  <span class="side-label">Zoom</span>
  <div class="le-zoom">
    <button type="button" class="le-icon-btn" title="Zoom out" aria-label="Zoom out" onclick={() => zoomBy(-0.1)}><Icon name="minus" /></button>
    <span class="le-zoom-num">{Math.round(doc.zoom * 100)}%</span>
    <button type="button" class="le-icon-btn" title="Zoom in" aria-label="Zoom in" onclick={() => zoomBy(0.1)}><Icon name="plus" /></button>
  </div>
</section>

<style>
  /* Rail tools live inside the server-styled `.sidebar`; reuse its `.side-label`
     vocabulary (a global class) and the shared --rm-* palette (defined on body). */
  .le-zone {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .le-tools {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 7px;
  }
  .le-tools button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 36px;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .le-tools button:hover:not(.active) {
    background: #f0f0f2;
  }
  .le-tools button.active {
    background: var(--rm-accent);
    border-color: var(--rm-accent);
    color: #fff;
    box-shadow: 0 1px 3px rgba(10, 132, 255, 0.4);
  }
  .le-select {
    width: 100%;
    min-width: 0;
    font: inherit;
    font-size: 13px;
    color: var(--rm-text);
    padding: 7px 26px 7px 10px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
    appearance: none;
    -webkit-appearance: none;
    background-color: var(--rm-control-bg);
    background-image: url("data:image/svg+xml;charset=utf-8,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='7' viewBox='0 0 10 7'%3E%3Cpath d='M1 1.5 5 5.5 9 1.5' fill='none' stroke='%238a8a8e' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 9px center;
  }
  .le-combo-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 34px;
    gap: 7px;
  }
  .le-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 34px;
    width: 100%;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    line-height: 1;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }
  .le-icon-btn:hover:not(:disabled) {
    background: #f0f0f2;
  }
  .le-icon-btn:disabled {
    color: #bbb;
    cursor: not-allowed;
    box-shadow: none;
  }
  .le-control {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0.4rem;
    align-items: center;
    font-size: 13px;
    color: var(--rm-text);
  }
  .le-control-stack {
    grid-template-columns: 1fr;
    gap: 6px;
  }
  .le-check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    font-size: 12px;
    color: var(--rm-text);
  }
  .le-check input {
    width: 14px;
    height: 14px;
    margin: 0;
    accent-color: var(--rm-accent);
  }
  .le-history {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .le-zoom {
    display: grid;
    grid-template-columns: 34px 1fr 34px;
    gap: 6px;
    align-items: center;
  }
  .le-zoom-num {
    text-align: center;
    font-size: 13px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
</style>
