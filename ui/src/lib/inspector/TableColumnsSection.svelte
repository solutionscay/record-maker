<script lang="ts">
  // Layout-selected Table column manager (#117). It appears only in the Layout
  // inspector for a `table` layout; selecting a row hands control back to the
  // ordinary per-object Field inspector.
  import type { EditorDoc } from '../doc.svelte';
  import Icon from '../Icon.svelte';
  import { kindIcon } from '../../shared/field-kinds';
  import {
    createObject,
    setObjectProps as persistProps,
    setObjectReadOnly as persistReadOnly,
  } from '../persist';
  import {
    projectTableColumns,
    withTableColumnSettings,
    type TableColumnSettings,
    type TableFieldState,
  } from '../table-columns';
  import { llog } from '../log';
  import { reportPersistError } from './persist-ops';

  let { doc, layoutId = '' }: { doc: EditorDoc; layoutId?: string } = $props();

  let query = $state('');
  let busy = $state(false);
  let draggedFieldId = $state<number | null>(null);
  let projection = $derived(projectTableColumns(doc.renderModel));
  let normalizedQuery = $derived(query.trim().toLowerCase());
  let visible = $derived(
    normalizedQuery
      ? projection.visible.filter((row) => row.field.name.toLowerCase().includes(normalizedQuery))
      : projection.visible,
  );
  let available = $derived(
    normalizedQuery
      ? projection.available.filter((row) => row.field.name.toLowerCase().includes(normalizedQuery))
      : projection.available,
  );

  function inspect(row: TableFieldState): void {
    if (row.primaryObjectId === null) return;
    doc.setTool('pointer');
    doc.selectOnly([row.primaryObjectId]);
  }

  /** Apply per-object table-column patches as one undo step, then persist each
   * object's complete props bag. Keeping the existing bag preserves formatting
   * and every unrelated object setting. */
  async function persistPatches(
    patches: Map<number, Partial<TableColumnSettings>>,
    label: string,
  ): Promise<void> {
    if (patches.size === 0) return;
    const nexts = new Map<number, Record<string, unknown>>();
    for (const [id, patch] of patches) {
      const object = doc.getObject(id);
      if (!object) continue;
      const next = withTableColumnSettings(object.props, patch);
      nexts.set(id, next);
      doc.setObjectProps(id, JSON.stringify(next));
    }
    doc.mark();
    llog('persist', `inspector: ${label}`, { ids: [...nexts.keys()] });
    try {
      await Promise.all(
        [...nexts].map(async ([id, next]) => {
          const styles = await persistProps(layoutId, id, next);
          doc.setObjectStyles(id, styles);
        }),
      );
    } catch (error) {
      reportPersistError(doc, label, error);
    }
  }

  function addPatch(
    patches: Map<number, Partial<TableColumnSettings>>,
    ids: readonly number[],
    patch: Partial<TableColumnSettings>,
  ): void {
    for (const id of ids) patches.set(id, { ...(patches.get(id) ?? {}), ...patch });
  }

  function orderedPatches(rows: readonly TableFieldState[]): Map<number, Partial<TableColumnSettings>> {
    const patches = new Map<number, Partial<TableColumnSettings>>();
    rows.forEach((row, order) => addPatch(patches, row.visibleObjectIds, { visible: true, order }));
    return patches;
  }

  async function hide(row: TableFieldState): Promise<void> {
    if (busy) return;
    busy = true;
    const remaining = projection.visible.filter((item) => item.field.id !== row.field.id);
    const patches = orderedPatches(remaining);
    addPatch(patches, row.objectIds, { visible: false });
    await persistPatches(patches, `hide table column ${row.field.name}`);
    busy = false;
  }

  async function add(row: TableFieldState): Promise<void> {
    if (busy || projection.bodyPartId === null) return;
    busy = true;
    try {
      // An already-authored hidden object keeps its geometry, formatting, and
      // identity. Normalize every visible row at the same time so the new final
      // order is explicit and deterministic.
      if (row.objectIds.length > 0) {
        const nextRows = [...projection.visible, { ...row, visibleObjectIds: row.objectIds }];
        const patches = orderedPatches(nextRows);
        addPatch(patches, row.objectIds, { visible: true, order: nextRows.length - 1 });
        await persistPatches(patches, `show table column ${row.field.name}`);
        if (row.field.system) {
          const views = await Promise.all(
            row.objectIds.map((id) => persistReadOnly(layoutId, id, true, doc.rec)),
          );
          for (const view of views) {
            doc.setProp(view.id, 'readOnly', true);
            doc.refreshResolved(view);
          }
          doc.mark();
        }
        return;
      }

      // Normalize legacy columns before inserting an explicitly ordered new one;
      // otherwise an explicit order would sort ahead of every legacy object.
      await persistPatches(orderedPatches(projection.visible), 'normalize table column order');

      const body = doc.getPart(projection.bodyPartId);
      const previousId = projection.visible.at(-1)?.primaryObjectId ?? null;
      const previous = previousId === null ? null : doc.getObject(previousId);
      const w = previous?.w ?? 200;
      const h = previous?.h ?? 24;
      let x = previous?.x ?? 96;
      let y = previous ? previous.y + Math.max(32, previous.h + doc.gridSize) : 16;
      if (body && y + h > body.height) {
        x = previous ? previous.x + previous.w + Math.max(8, doc.gridSize) : 96;
        y = Math.min(16, Math.max(0, body.height - h));
      }
      let created = await createObject(layoutId, {
        partId: projection.bodyPartId,
        kind: 'field',
        x,
        y,
        w,
        h: body ? Math.min(h, Math.max(1, body.height - y)) : h,
        fieldId: row.field.id,
        createLabel: false,
        props: { tableColumn: { visible: true, order: projection.visible.length } },
        rec: doc.rec,
      });
      // Value-only creation avoids manufacturing a caption object for a table
      // column. The system primary key still keeps its immutable/read-only rule.
      if (row.field.system) {
        created = await Promise.all(
          created.map((view) => persistReadOnly(layoutId, view.id, true, doc.rec)),
        );
      }
      for (const view of created) doc.addObject(view, projection.bodyPartId);
      doc.mark();
      llog('create', 'inspector: add table column', {
        fieldId: row.field.id,
        objectIds: created.map((object) => object.id),
      });
    } catch (error) {
      reportPersistError(doc, `add table column ${row.field.name}`, error);
    } finally {
      busy = false;
    }
  }

  async function reorder(draggedId: number, targetId: number): Promise<void> {
    if (busy || draggedId === targetId || normalizedQuery) return;
    const rows = projection.visible.slice();
    const from = rows.findIndex((row) => row.field.id === draggedId);
    const to = rows.findIndex((row) => row.field.id === targetId);
    if (from < 0 || to < 0) return;
    const [moved] = rows.splice(from, 1);
    rows.splice(to, 0, moved);
    busy = true;
    await persistPatches(orderedPatches(rows), 'reorder table columns');
    busy = false;
  }

  function dragStart(event: DragEvent, row: TableFieldState): void {
    if (busy || normalizedQuery) {
      event.preventDefault();
      return;
    }
    draggedFieldId = row.field.id;
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/plain', String(row.field.id));
    }
  }

  function dragOver(event: DragEvent): void {
    if (draggedFieldId === null || busy || normalizedQuery) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
  }

  function drop(event: DragEvent, row: TableFieldState): void {
    event.preventDefault();
    const source = draggedFieldId;
    draggedFieldId = null;
    if (source !== null) void reorder(source, row.field.id);
  }
</script>

<section class="insp-sec table-columns">
  <div class="tc-title-row">
    <span class="side-label">Table Columns</span>
    <span class="tc-count">{projection.visible.length}</span>
  </div>

  <input
    class="ctl-input tc-search"
    type="search"
    placeholder="Search fields…"
    aria-label="Search table fields"
    bind:value={query}
  />

  <div class="tc-group-title">
    <span>Visible</span>
    <span>{visible.length}</span>
  </div>
  <div class="tc-list" role="list" aria-label="Visible table columns">
    {#each visible as row (row.field.id)}
      <div
        class="tc-row"
        class:dragging={draggedFieldId === row.field.id}
        role="listitem"
        draggable={!busy && !normalizedQuery}
        ondragstart={(event) => dragStart(event, row)}
        ondragover={dragOver}
        ondrop={(event) => drop(event, row)}
        ondragend={() => (draggedFieldId = null)}
      >
        <span class="tc-handle" aria-hidden="true" title="Drag to reorder">⠿</span>
        <button
          type="button"
          class="tc-field"
          title={`Inspect ${row.field.name}`}
          onclick={() => inspect(row)}
        >
          <Icon name={kindIcon(row.field.kind, 'type-text')} />
          <span>{row.field.name}</span>
        </button>
        <button
          type="button"
          class="tc-action"
          title={`Hide ${row.field.name}`}
          aria-label={`Hide ${row.field.name}`}
          disabled={busy}
          onclick={() => hide(row)}
        ><Icon name="minus" /></button>
      </div>
    {:else}
      <span class="tc-empty">No visible columns.</span>
    {/each}
  </div>
  {#if normalizedQuery && projection.visible.length > 1}
    <span class="le-hint">Clear the search to reorder columns.</span>
  {/if}

  <div class="tc-group-title tc-available-title">
    <span>Available</span>
    <span>{available.length}</span>
  </div>
  <div class="tc-list" role="list" aria-label="Available table fields">
    {#each available as row (row.field.id)}
      <div class="tc-row tc-available" role="listitem">
        <span class="tc-dot" aria-hidden="true">○</span>
        <span class="tc-field tc-field-static">
          <Icon name={kindIcon(row.field.kind, 'type-text')} />
          <span>{row.field.name}</span>
        </span>
        <button
          type="button"
          class="tc-action"
          title={`Show ${row.field.name}`}
          aria-label={`Show ${row.field.name}`}
          disabled={busy || projection.bodyPartId === null}
          onclick={() => add(row)}
        ><Icon name="plus" /></button>
      </div>
    {:else}
      <span class="tc-empty">{normalizedQuery ? 'No matching fields.' : 'All fields are visible.'}</span>
    {/each}
  </div>

  {#if projection.bodyPartId === null}
    <span class="le-hint">Add a Body band before showing table columns.</span>
  {:else}
    <span class="le-hint">Drag visible fields to reorder. Select a name to inspect its field object.</span>
  {/if}
</section>

<style>
  .table-columns {
    gap: 10px;
  }
  .tc-title-row,
  .tc-group-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .tc-count {
    min-width: 22px;
    padding: 2px 6px;
    border-radius: 999px;
    background: var(--rm-segment-track);
    color: var(--rm-text-dim);
    font-size: 10px;
    font-weight: 700;
    text-align: center;
  }
  .tc-search {
    width: 100%;
  }
  .tc-group-title {
    margin-top: 2px;
    color: var(--rm-text-dim);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .tc-available-title {
    margin-top: 5px;
  }
  .tc-list {
    overflow: hidden;
    border: 0.5px solid var(--rm-border);
    border-radius: var(--rm-radius);
    background: var(--rm-control-bg);
  }
  .tc-row {
    min-height: 34px;
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr) 28px;
    align-items: center;
    gap: 4px;
    padding: 0 5px;
    border-bottom: 0.5px solid var(--rm-border);
    color: var(--rm-text);
  }
  .tc-row:last-child {
    border-bottom: 0;
  }
  .tc-row[draggable='true'] {
    cursor: grab;
  }
  .tc-row.dragging {
    opacity: 0.45;
  }
  .tc-handle,
  .tc-dot {
    color: var(--rm-text-dim);
    font-size: 14px;
    text-align: center;
    user-select: none;
  }
  .tc-field {
    min-width: 0;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 3px;
    border: 0;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font: inherit;
    font-size: 12px;
    text-align: left;
  }
  .tc-field:hover {
    color: var(--rm-accent);
  }
  .tc-field span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tc-field-static {
    cursor: default;
  }
  .tc-field-static:hover {
    color: inherit;
  }
  .tc-action {
    width: 26px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    border-radius: var(--rm-radius);
    background: transparent;
    color: var(--rm-text-dim);
    cursor: pointer;
  }
  .tc-action:hover:not(:disabled) {
    background: var(--rm-segment-track);
    color: var(--rm-accent);
  }
  .tc-action:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .tc-empty {
    display: block;
    padding: 10px;
    color: var(--rm-text-dim);
    font-size: 11px;
  }
</style>
