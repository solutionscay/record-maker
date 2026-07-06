<script lang="ts">
  // Single-object identity section: a field's binding + read-only toggle, or a
  // text label's content. Field identity comes from the render model's
  // server-resolved fieldId — never re-derived from the binding string
  // client-side (#134) — so the root passes it in alongside the object.
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import FieldSelect from '../FieldSelect.svelte';
  import {
    setObjectBinding as persistBinding,
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

  async function setSelectedBinding(nextFieldId: number): Promise<void> {
    if (selected.kind !== 'field' || !Number.isFinite(nextFieldId)) return;
    llog('persist', 'inspector: set field binding', { id: selected.id, fieldId: nextFieldId });
    try {
      const view = await persistBinding(layoutId, selected.id, nextFieldId, doc.rec);
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
  <span class="side-label">{selected.kind === 'text' ? 'Text' : 'Binding'}</span>
  {#if selected.kind === 'field'}
    <FieldSelect
      fields={doc.fields}
      value={fieldId}
      placeholder="Unresolved"
      title="Bound field"
      onselect={(id) => setSelectedBinding(id)}
    />
    {#if fieldId === null && selected.binding}
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
