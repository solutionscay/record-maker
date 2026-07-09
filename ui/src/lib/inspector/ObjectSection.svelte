<script lang="ts">
  // Single-object identity section: a field's binding + read-only toggle, or a
  // text label's content. Field identity comes from the render model's
  // server-resolved fieldId — never re-derived from the binding string
  // client-side (#134) — so the root passes it in alongside the object.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import FieldSelect from '../FieldSelect.svelte';
  import {
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
    try {
      const view = await persistBindingPath(layoutId, selected.id, path, doc.rec);
      doc.setProp(selected.id, 'binding', view.binding);
      doc.refreshResolved(view);
      doc.mark();
    } catch (e) {
      reportPersistError(doc, 'set portal route', e);
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
