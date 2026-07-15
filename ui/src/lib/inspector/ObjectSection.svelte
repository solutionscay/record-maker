<script lang="ts">
  // Single-object identity section: a field's binding + read-only toggle, or a
  // text label's content. Field identity comes from the render model's
  // server-resolved fieldId — never re-derived from the binding string
  // client-side (#134) — so the root passes it in alongside the object.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import FieldSelect from '../FieldSelect.svelte';
  import RoutePicker from '../RoutePicker.svelte';
  import {
    setObjectBinding as persistBinding,
    setObjectBindingPath as persistBindingPath,
    setObjectContent as persistContent,
    setObjectReadOnly as persistReadOnly,
  } from '../persist';
  import { llog } from '../log';
  import { MAX_PORTAL_ROW_COUNT, parseProps, portalRowCount } from '../object-props';
  import { reportPersistError, writeObjectProps } from './persist-ops';

  let {
    doc,
    layoutId = '',
    selected,
    fieldId,
  }: { doc: EditorDoc; layoutId?: string; selected: Readonly<ObjectDoc>; fieldId: number | null } = $props();

  // SELECT-TO-SCOPE (#168/#169): a selected field that is a portal COLUMN (it has a
  // parent portal) binds to a table along the portal route, not the primary one —
  // so its Binding row offers the grouped route fields and rebinds route-relative. When the
  // sole selection is this column, `doc.scopedPortal` resolves its owning portal
  // (route path + related fields). A plain top-level field leaves this null and
  // keeps the primary-table picker + fieldId rebind untouched.
  let columnPortal = $derived(
    selected.kind === 'field' && selected.parentObjectId !== null ? doc.scopedPortal : null,
  );
  // The server resolves a column's binding against the PRIMARY record (its last
  // segment rarely matches a primary field), so the model's `fieldId` is usually
  // null for a column. For the picker's current-selection highlight only, map the
  // binding's trailing field-name segment back to a related field id by name.
  let columnFieldId = $derived.by(() => {
    if (!columnPortal) return fieldId;
    const exact = columnPortal.fields.find(
      (f) => `${f.routePath ?? columnPortal.path}.${f.name}`.toLowerCase() === selected.binding.toLowerCase(),
    );
    if (exact) return exact.id;
    const seg = (selected.binding.split('.').pop() ?? '').toLowerCase();
    return columnPortal.fields.find((f) => f.name.toLowerCase() === seg)?.id ?? null;
  });
  let columnField = $derived(
    columnPortal ? (columnPortal.fields.find((field) => field.id === columnFieldId) ?? null) : null,
  );

  async function setSelectedBinding(nextFieldId: number): Promise<void> {
    if (selected.kind !== 'field' || !Number.isFinite(nextFieldId)) return;
    // A portal column rebinds ROUTE-RELATIVE: resolve the picked RELATED field by id
    // and write `<route>.<field>` verbatim through the binding-path endpoint (the
    // fieldId endpoint only knows the primary table). Top-level fields keep the
    // fieldId rebind so the server stays the single source of the primary dot-path.
    const scope = columnPortal;
    try {
      let view;
      if (scope) {
        const f = scope.fields.find((c) => c.id === nextFieldId);
        if (!f) return;
        const path = `${f.routePath ?? scope.path}.${f.name}`;
        const readOnly = f.system || (f.routePath !== undefined && f.routePath !== scope.path);
        llog('persist', 'inspector: set portal column binding', { id: selected.id, path });
        view = await persistBindingPath(layoutId, selected.id, path, doc.rec);
        if (selected.readOnly !== readOnly) {
          view = await persistReadOnly(layoutId, selected.id, readOnly, doc.rec);
          doc.setProp(selected.id, 'readOnly', readOnly);
        }
      } else {
        llog('persist', 'inspector: set field binding', { id: selected.id, fieldId: nextFieldId });
        view = await persistBinding(layoutId, selected.id, nextFieldId, doc.rec);
      }
      doc.setProp(selected.id, 'binding', view.binding);
      doc.refreshResolved(view);
      doc.mark();
    } catch (e) {
      reportPersistError(doc, 'set field binding', e);
    }
  }

  async function setSelectedContent(content: string): Promise<void> {
    if (selected.kind !== 'text') return;
    llog('persist', 'inspector: set text content', { id: selected.id });
    doc.setProp(selected.id, 'content', content);
    doc.mark();
    try {
      const view = await persistContent(layoutId, selected.id, content);
      doc.setProp(selected.id, 'content', view.content);
    } catch (e) {
      reportPersistError(doc, 'set text content', e);
    }
  }

  async function setSelectedRoute(path: string): Promise<void> {
    // Re-anchor a portal to another declared relationship route (#168). The path
    // is written VERBATIM through the binding-path endpoint (FK-first: the route
    // is picked from the layout's declared routes, never authored).
    if (selected.kind !== 'portal' || !path) return;
    llog('persist', 'inspector: set portal route', { id: selected.id, path });
    // Optimistic: update the reactive binding synchronously so the one-way
    // <select value={selected.binding}> stays on the picked option. Without this,
    // Svelte re-asserts the pre-round-trip binding on the change-event flush and
    // the control snaps back to the portal's original route (mirrors the
    // setProp-before-await order in PartSection / FormatSection).
    doc.setProp(selected.id, 'binding', path);
    try {
      const view = await persistBindingPath(layoutId, selected.id, path, doc.rec, true);
      doc.setProp(selected.id, 'binding', view.binding);
      doc.refreshResolved(view);
      doc.mark();
    } catch (e) {
      reportPersistError(doc, 'set portal route', e);
    }
  }

  // ── Portal columns (#168): ADD a portal's columns from its inspector (the left
  // rail is for top-level tools). When a portal is selected, resolve its bound
  // route so the picker offers the related table's fields. FK-first: a column
  // binds a declared route field.
  //
  // Adding uses the SAME gesture as the rail's "Field to place": a multi-select
  // list you select then DRAG onto the portal (dragToPlace + a portal-column drag
  // payload). The canvas drop handler (placement.ts) does the parent-aware create.
  // Columns (and their labels) are ordinary objects — remove them on the canvas
  // like any other object; there is no bespoke column manager here.
  /** Fields selected in the Columns picker, staged for the drag onto the portal. */
  let colFieldIds = $state<number[]>([]);
  // Clear the staged column selection when the selected portal changes, so ids
  // picked for one portal don't linger into another portal's (different) route.
  let lastPortalId = -1;
  $effect(() => {
    const id = selected.kind === 'portal' ? selected.id : -1;
    if (id !== lastPortalId) {
      lastPortalId = id;
      colFieldIds = [];
    }
  });

  /** The declared route the selected portal is bound to (all traversed-table
   * fields feed the grouped column picker), or null when not a bound portal. */
  let portalRoute = $derived(
    selected.kind === 'portal'
      ? (doc.relatedRoutes.find((r) => r.path === selected.binding) ?? null)
      : null,
  );
  let portalProps = $derived(parseProps(selected.props));
  let portalRows = $derived(portalRowCount(portalProps));

  async function setPortalRows(value: number): Promise<void> {
    if (selected.kind !== 'portal') return;
    const rowCount = Math.min(MAX_PORTAL_ROW_COUNT, Math.max(1, Math.round(value || 1)));
    const next = { ...portalProps, rowCount };
    llog('persist', 'inspector: set portal row count', { id: selected.id, rowCount });
    await writeObjectProps(doc, layoutId, selected.id, next, 'set portal row count');
  }

  async function setSelectedReadOnly(readOnly: boolean): Promise<void> {
    llog('persist', 'inspector: set read-only', { id: selected.id, readOnly });
    doc.setProp(selected.id, 'readOnly', readOnly);
    doc.mark();
    try {
      const view = await persistReadOnly(layoutId, selected.id, readOnly, doc.rec);
      doc.setProp(selected.id, 'readOnly', view.readOnly);
      doc.refreshResolved(view);
    } catch (e) {
      reportPersistError(doc, 'set read-only', e);
    }
  }
</script>

<section class="insp-sec">
  <span class="side-label"
    >{selected.kind === 'text' ? 'Text' : selected.kind === 'portal' ? 'Related table' : 'Binding'}</span
  >
  {#if selected.kind === 'field'}
    {#if columnPortal}
      <span class="le-hint">Column from {columnField?.tableName ?? columnPortal.tableName}</span>
    {/if}
    <FieldSelect
      fields={columnPortal ? columnPortal.fields : doc.fields}
      value={columnFieldId}
      placeholder="Unresolved"
      title={columnPortal ? 'Bound related field' : 'Bound field'}
      onselect={(id) => setSelectedBinding(id)}
    />
    {#if columnFieldId === null && selected.binding}
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
  {:else if selected.kind === 'portal'}
    {#if doc.relatedRoutes.length === 0}
      <span class="le-hint">No relationships defined for this table.</span>
    {:else}
      <RoutePicker
        routes={doc.relatedRoutes}
        value={selected.binding}
        onchange={setSelectedRoute}
      />
    {/if}
    <div class="insp-row">
      <span>Rows</span>
      <input
        class="ctl-num"
        type="number"
        min="1"
        max={MAX_PORTAL_ROW_COUNT}
        value={portalRows}
        onchange={(e) => setPortalRows(Number(e.currentTarget.value))}
      />
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

{#if selected.kind === 'portal' && portalRoute}
  <section class="insp-sec">
    <span class="side-label">Field to place</span>
    <FieldSelect
      fields={portalRoute.fields}
      value={colFieldIds[0] ?? null}
      values={colFieldIds}
      multi
      dragToPlace
      portalDrag={{ portalId: selected.id, route: portalRoute.path }}
      placeholder="Fields to add…"
      title="Field to place; Shift-click range, Ctrl/Cmd-click individual, or drag onto the portal"
      onselect={(id) => (colFieldIds = [id])}
      onselectMany={(ids) => (colFieldIds = ids)}
      onclear={() => (colFieldIds = [])}
    />
  </section>
{/if}
