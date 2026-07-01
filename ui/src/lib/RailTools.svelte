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

  let usedPartKinds = $derived(new Set(doc.parts.map((p) => p.kind)));
  // A form is a single-record view: sub/grand summaries are report-only, so a form
  // layout offers header/body/footer only (Issue 3). List/Table keep all five.
  let partKinds = $derived(
    PART_KINDS.filter((p) => doc.view !== 'form' || (p.id !== 'subsummary' && p.id !== 'grandsummary')),
  );

  // Default the Field dropdown to the first field once the model has hydrated.
  $effect(() => {
    if (fieldId === null && doc.fields.length > 0) fieldId = doc.fields[0].id;
  });

  $effect(() => {
    if (!canAddPartKind(partKind)) {
      partKind = PART_KINDS.find((p) => canAddPartKind(p.id))?.id ?? partKind;
    }
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
    if (busy || !canAddPartKind(partKind)) return;
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
      {#each partKinds as p (p.id)}
        <option value={p.id} disabled={!canAddPartKind(p.id)}>{p.label}</option>
      {/each}
    </select>
    <button type="button" class="le-icon-btn" title="Add band" onclick={addPart} disabled={busy || !canAddPartKind(partKind)}>+</button>
  </div>
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
    height: 36px;
    padding: 0;
    border: 0.5px solid var(--rm-border);
    border-radius: 8px;
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    font-size: 14px;
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
    padding: 7px 10px;
    border: 0.5px solid var(--rm-border);
    border-radius: 7px;
    background: var(--rm-control-bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.04);
  }
  .le-combo-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 34px;
    gap: 7px;
  }
  .le-icon-btn {
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
