<script lang="ts">
  // Single-object identity section: a field's binding + read-only toggle, or a
  // text label's content. Field identity comes from the render model's
  // server-resolved fieldId — never re-derived from the binding string
  // client-side (#134) — so the root passes it in alongside the object.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import type { ObjectView } from '../model';
  import FieldSelect from '../FieldSelect.svelte';
  import { defaultBox } from '../create';
  import {
    createObject,
    deleteObjects,
    setObjectBinding as persistBinding,
    setObjectBindingPath as persistBindingPath,
    setObjectContent as persistContent,
    setObjectReadOnly as persistReadOnly,
  } from '../persist';
  import { llog } from '../log';
  import { reportPersistError } from './persist-ops';

  let {
    doc,
    layoutId = '',
    selected,
    fieldId,
  }: { doc: EditorDoc; layoutId?: string; selected: Readonly<ObjectDoc>; fieldId: number | null } = $props();

  // SELECT-TO-SCOPE (#168/#169): a selected field that is a portal COLUMN (it has a
  // parent portal) binds to the portal's RELATED table, not the primary one — so its
  // Binding row must offer the related fields and rebind ROUTE-RELATIVE. When the
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
    const seg = (selected.binding.split('.').pop() ?? '').toLowerCase();
    return columnPortal.fields.find((f) => f.name.toLowerCase() === seg)?.id ?? null;
  });

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
        const path = `${scope.path}.${f.name}`;
        llog('persist', 'inspector: set portal column binding', { id: selected.id, path });
        view = await persistBindingPath(layoutId, selected.id, path, doc.rec);
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
      const view = await persistBindingPath(layoutId, selected.id, path, doc.rec);
      doc.setProp(selected.id, 'binding', view.binding);
      doc.refreshResolved(view);
      doc.mark();
    } catch (e) {
      reportPersistError(doc, 'set portal route', e);
    }
  }

  // ── Portal columns (#168): author a portal's columns from ITS inspector, not the
  // left rail. When a portal is selected, resolve its bound route (so the picker
  // offers the related table's fields) and enumerate its authored column children
  // (the field objects whose `parentObjectId` is this portal), so the user can
  // append and remove columns here. FK-first: a column binds a declared route field.
  let busyCols = $state(false);

  /** The declared route the selected portal is bound to (its related-table fields
   * feed the column picker), or null when the selection isn't a bound portal. */
  let portalRoute = $derived(
    selected.kind === 'portal'
      ? (doc.relatedRoutes.find((r) => r.path === selected.binding) ?? null)
      : null,
  );

  /** Every object owned by the selected portal (columns AND their spawned caption
   * labels), pulled from the render model. Empty unless a portal is selected. */
  let portalChildren = $derived.by<ObjectView[]>(() => {
    if (selected.kind !== 'portal') return [];
    const out: ObjectView[] = [];
    for (const p of doc.renderModel.parts) {
      for (const o of p.objects) {
        if (o.parentObjectId === selected.id) out.push(o);
      }
    }
    return out;
  });

  /** The portal's authored columns (field children) in visual order — the same
   * `(x, y, z, id)` order the server enumerates them in for the header row. */
  let portalColumns = $derived(
    portalChildren
      .filter((o) => o.kind === 'field')
      .slice()
      .sort((a, b) => a.x - b.x || a.y - b.y || a.z - b.z || a.id - b.id),
  );

  /** A column's display name: its binding's trailing segment mapped back to the
   * related field by name, falling back to the resolved label or the raw segment. */
  function columnName(o: ObjectView): string {
    const seg = (o.binding.split('.').pop() ?? '').toLowerCase();
    const f = portalRoute?.fields.find((c) => c.name.toLowerCase() === seg);
    return f?.name ?? o.label ?? seg;
  }

  /** Append a column to the selected portal: POST the create-object route with
   * `parentObjectId` = the portal id and the chosen related field, so the server
   * builds the route-relative binding (`<route>.<field>`) and creates the column
   * child (plus its caption label). New columns land ROW-RELATIVE — the next slot
   * to the right of the last column — reusing the Field tool's default box. */
  async function addColumn(fieldId: number): Promise<void> {
    if (selected.kind !== 'portal' || busyCols) return;
    const size = defaultBox('field');
    const last = portalColumns.at(-1);
    const x = last ? last.x + last.w : selected.x;
    const y = last ? last.y : selected.y;
    busyCols = true;
    llog('persist', 'inspector: add portal column', { portal: selected.id, fieldId });
    try {
      const views = await createObject(layoutId, {
        partId: selected.partId,
        kind: 'field',
        x,
        y,
        w: size.w,
        h: size.h,
        fieldId,
        createLabel: true,
        parentObjectId: selected.id,
        rec: doc.rec,
      });
      for (const v of views) doc.addObject(v, selected.partId);
      doc.mark();
    } catch (e) {
      reportPersistError(doc, 'add portal column', e);
    } finally {
      busyCols = false;
    }
  }

  /** Remove a column from the selected portal — deleting the field child AND the
   * header label the server spawned for it, as one undoable step. A portal column's
   * label is a top header: `create_field_object` places it directly ABOVE the value
   * (`x = col.x`, `y = max(0, col.y - col.h)`), not the left caption a top-level
   * field gets — so pair on that geometry. */
  async function removeColumn(col: ObjectView): Promise<void> {
    if (busyCols) return;
    const labelY = Math.max(0, col.y - col.h);
    const label = portalChildren.find(
      (o) => o.kind === 'text' && o.y === labelY && o.x === col.x,
    );
    const ids = label ? [col.id, label.id] : [col.id];
    busyCols = true;
    llog('persist', 'inspector: remove portal column', { portal: selected.id, ids });
    try {
      await deleteObjects(layoutId, ids);
      for (const id of ids) doc.removeObject(id);
      doc.mark();
    } catch (e) {
      reportPersistError(doc, 'remove portal column', e);
    } finally {
      busyCols = false;
    }
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
    >{selected.kind === 'text' ? 'Text' : selected.kind === 'portal' ? 'Related list' : 'Binding'}</span
  >
  {#if selected.kind === 'field'}
    {#if columnPortal}
      <span class="le-hint">Column of {columnPortal.tableName} (portal)</span>
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
      <select
        class="ctl-input"
        value={selected.binding}
        title="Relationship route the portal shows"
        onchange={(e) => setSelectedRoute(e.currentTarget.value)}
      >
        {#each doc.relatedRoutes as r (r.relationshipId)}
          <option value={r.path}>{r.name} → {r.tableName}</option>
        {/each}
      </select>
    {/if}
    {#if selected.binding}
      <span class="le-hint">{selected.binding}</span>
    {/if}
  {:else}
    <input
      class="ctl-input"
      type="text"
      value={selected.content}
      onchange={(e) => setSelectedContent(e.currentTarget.value)}
    />
  {/if}
</section>

{#if selected.kind === 'portal'}
  <section class="insp-sec">
    <span class="side-label">Columns</span>
    {#if !portalRoute}
      <span class="le-hint">Bind a related list route above to add columns.</span>
    {:else}
      <FieldSelect
        fields={portalRoute.fields}
        value={null}
        placeholder="Add column…"
        title="Add a column from {portalRoute.tableName}"
        onselect={(id) => addColumn(id)}
      />
      {#if portalColumns.length === 0}
        <span class="le-hint">No columns yet — pick a field above to add one.</span>
      {:else}
        <ul class="col-list">
          {#each portalColumns as col (col.id)}
            <li class="col-row">
              <span class="col-name">{columnName(col)}</span>
              <button
                type="button"
                class="col-del"
                title="Remove column"
                aria-label="Remove column"
                disabled={busyCols}
                onclick={() => removeColumn(col)}>×</button
              >
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </section>
{/if}
