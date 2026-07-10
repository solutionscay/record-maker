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
  import { canAddPartKind as canAddPartKindRule, partKindAllowedInView } from './part-rules';
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
    { id: 'portal', label: 'Portal (related list)', icon: 'view-list' },
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
  let routePath = $state('');
  let partKind = $state('body');
  let busy = $state(false);

  // Default the Portal route picker to the first declared route so arming the
  // tool never starts on a blank selection when routes exist.
  $effect(() => {
    if (routePath === '' && doc.relatedRoutes.length > 0) routePath = doc.relatedRoutes[0].path;
  });

  // A form is a single-record view: sub/grand summaries are report-only, so a form
  // layout offers header/body/footer only (Issue 3). List/Table keep all five.
  let partKinds = $derived(PART_KINDS.filter((p) => partKindAllowedInView(doc.view, p.id)));

  // A placement (or leaving the Field tool some other way) drops `doc.activeTool`
  // back to 'pointer', but this component's own `fieldIds` is a separate copy that
  // doesn't hear about it, so a multi-select used to survive across placements:
  // re-arming the Field tool reopened the picker with the just-placed fields still
  // checked. Clear it on the 'field' -> other transition. `prevTool` is a plain
  // (non-reactive) variable so this effect tracks only `doc.activeTool`, not
  // `fieldIds`; otherwise it would fight the hydration-default effect below
  // (each clearing what the other just set) in an infinite loop. Starts
  // undefined (not read from `doc` at declaration time) so the first run never
  // reads `doc.activeTool` outside the effect's own reactive tracking.
  let prevTool: ToolKind | undefined;
  $effect(() => {
    const tool = doc.activeTool;
    if (prevTool === 'field' && tool !== 'field') clearFieldSelection();
    prevTool = tool;
  });

  // Default the Field dropdown to the first field the FIRST time the model
  // hydrates, so the picker doesn't open on a bare placeholder. `hasDefaulted`
  // (a plain, non-reactive flag, same trick as `prevTool`) makes this fire only
  // once: `fieldIds.length === 0` on its own can't tell "nothing picked yet"
  // apart from "the user explicitly cleared the selection" (clearFieldSelection,
  // below), so a rule that re-defaults on every emptiness would silently
  // overwrite an explicit clear on the next tick — the picker would look
  // permanently pinned to the first field with no way to deselect it.
  //
  // The Field tool ALWAYS places PRIMARY/base-table fields (`doc.fields`). Portal
  // column authoring lives in the portal inspector's Columns picker (#168), not
  // here — the rail never retargets its picker to a related table.
  let hasDefaulted = false;
  $effect(() => {
    const first = doc.fields[0]?.id;
    if (!hasDefaulted && fieldIds.length === 0 && first !== undefined) {
      fieldIds = [first];
      hasDefaulted = true;
    }
  });

  /** Deselect all fields for placement. The one function that clears
   * `fieldIds`, so every call site (leaving the Field tool, the picker's own
   * Clear button, future callers) agrees on what "cleared" means and keeps the
   * store in sync when the Field tool is armed. */
  function clearFieldSelection(): void {
    fieldIds = [];
    if (doc.activeTool === 'field') doc.setTool('field', [], createLabel);
  }

  $effect(() => {
    if (!canAddPartKind(partKind)) {
      partKind = PART_KINDS.find((p) => canAddPartKind(p.id))?.id ?? partKind;
    }
  });

  // ── Mode / create zones ─────────────────────────────────────────────────

  function pickTool(t: ToolKind): void {
    llog('tool', 'rail: pick tool', { tool: t, fieldIds, createLabel, routePath });
    doc.setTool(t, t === 'field' ? fieldIds : null, createLabel, t === 'portal' ? routePath : '');
  }
  function onFieldChange(): void {
    if (doc.activeTool === 'field') doc.setTool('field', fieldIds, createLabel);
  }
  function onRouteChange(): void {
    if (doc.activeTool === 'portal') doc.setTool('portal', null, true, routePath);
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

  // Band legality lives in ./part-rules (shared with the Band inspector's kind
  // select) — this only binds it to the current layout.
  function canAddPartKind(kind: string): boolean {
    return canAddPartKindRule(doc.view, doc.parts, kind);
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
        onclear={clearFieldSelection}
        dragToPlace
        title="Field to place; Shift-click range, Ctrl/Cmd-click individual, or drag a row onto the canvas"
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
  {#if doc.activeTool === 'portal'}
    <div class="le-control le-control-stack">
      <span>Related table</span>
      {#if doc.relatedRoutes.length === 0}
        <span class="le-hint">No relationships defined for this table.</span>
      {:else}
        <select
          class="le-select"
          bind:value={routePath}
          onchange={onRouteChange}
          title="Relationship route the portal shows (FK-first — routes are declared, never created here)"
        >
          {#each doc.relatedRoutes as r (r.relationshipId)}
            <option value={r.path}>{r.name} → {r.tableName}</option>
          {/each}
        </select>
      {/if}
    </div>
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
  /* Fuller look: each rail zone (Tools / History / Zoom) is a lifted card so the
     rail reads as grouped material, matching the server-rendered .rail-card. */
  .le-zone {
    display: flex;
    flex-direction: column;
    gap: 9px;
    padding: 13px 13px 14px;
    border: 0.5px solid var(--rm-border);
    border-radius: var(--rm-radius-lg);
    background: var(--rm-card-bg);
    box-shadow: var(--rm-elev-1);
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
    border-radius: var(--rm-radius);
    background: var(--rm-control-bg);
    color: var(--rm-text);
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    box-shadow: var(--rm-elev-1);
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
  /* .le-select / .le-icon-btn are shared vocabulary now — see
     ui/src/shared/controls.css (#132). */
  .le-combo-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 34px;
    gap: 7px;
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
