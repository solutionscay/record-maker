<script lang="ts">
  // Arrange section (#83): group/ungroup, align, distribute & resize-to-match,
  // and z-order. The pure geometry/reorder math lives in ../arrange; group /
  // ungroup run through the shared command layer (../actions) so the Inspector
  // buttons and the canvas keyboard/context menu share one implementation. In a
  // single-object selection only the Order grid applies (`multi` off).
  import type { EditorDoc, ObjectDoc } from '../doc.svelte';
  import Icon from '../Icon.svelte';
  import {
    canGroupSelection,
    canUngroupSelection,
    groupSelected as groupSelectedAction,
    ungroupSelected as ungroupSelectedAction,
  } from '../actions';
  import {
    alignGeometries,
    distributeGeometries,
    resizeMatchGeometries,
    zOrderChanges,
    type AlignEdge,
    type ZCmd,
  } from '../arrange';
  import { applyGeometryMany, applyZChanges } from './persist-ops';
  import { llog } from '../log';

  let {
    doc,
    layoutId = '',
    multi = false,
    busy = $bindable(false),
  }: { doc: EditorDoc; layoutId?: string; multi?: boolean; busy?: boolean } = $props();

  let selectedIds = $derived([...doc.selection]);
  let selectedObjects = $derived(
    selectedIds.map((id) => doc.getObject(id)).filter((o): o is Readonly<ObjectDoc> => !!o),
  );

  // Arrange gating (#83): align/z-order need ≥2 (the multi-panel is already ≥2);
  // distribute needs ≥3; resize-to-match needs ≥2 objects that aren't lines (a
  // line's w/h encode its direction, so matching size would distort it).
  let canDistribute = $derived(selectedIds.length >= 3);
  let resizableCount = $derived(selectedObjects.filter((o) => o.kind !== 'line').length);
  let canResizeMatch = $derived(resizableCount >= 2);
  let canGroup = $derived(canGroupSelection(doc));
  let canUngroup = $derived(canUngroupSelection(doc));

  function align(edge: AlignEdge): void {
    if (selectedObjects.length < 2) return;
    void applyGeometryMany(doc, layoutId, alignGeometries(selectedObjects, edge));
  }

  function distribute(axis: 'h' | 'v'): void {
    void applyGeometryMany(doc, layoutId, distributeGeometries(selectedObjects, axis));
  }

  function resizeMatch(dim: 'w' | 'h' | 'both'): void {
    void applyGeometryMany(doc, layoutId, resizeMatchGeometries(selectedObjects, dim));
  }

  /** Rewrite the stacking order of every part that holds a selected object, then
   * persist the changed `z` values as ONE undo step. */
  async function zorder(cmd: ZCmd): Promise<void> {
    if (selectedIds.length === 0) return;
    const sel = new Set(selectedIds);
    const changed = zOrderChanges(
      doc.renderModel.parts.map((part) => part.objects.map((o) => o.id)), // already back→front by (z, id)
      sel,
      cmd,
      (id) => doc.getObject(id)?.z,
    );
    if (changed.length === 0) return;
    llog('persist', 'inspector: z-order', { cmd, changed });
    await applyZChanges(doc, layoutId, changed);
  }

  async function groupSelectedObjects(): Promise<void> {
    if (busy) return;
    busy = true;
    await groupSelectedAction(doc, layoutId);
    busy = false;
  }

  async function ungroupSelectedObjects(): Promise<void> {
    if (busy) return;
    busy = true;
    await ungroupSelectedAction(doc, layoutId);
    busy = false;
  }
</script>

<section class="insp-sec">
  <span class="side-label">Arrange</span>
  {#if multi}
    <div class="fmt-sub">Group</div>
    <div class="group-row">
      <button type="button" class="grp-btn" title="Group selected objects" disabled={!canGroup || busy} onclick={groupSelectedObjects}>Group</button>
      <button type="button" class="grp-btn" title="Ungroup selected group" disabled={!canUngroup || busy} onclick={ungroupSelectedObjects}>Ungroup</button>
    </div>
    <div class="fmt-sub">Align</div>
    <div class="arr-grid">
      <button type="button" class="arr-btn" title="Align left edges" onclick={() => align('left')}><Icon name="obj-align-left" /></button>
      <button type="button" class="arr-btn" title="Align horizontal centers" onclick={() => align('hcenter')}><Icon name="obj-align-hcenter" /></button>
      <button type="button" class="arr-btn" title="Align right edges" onclick={() => align('right')}><Icon name="obj-align-right" /></button>
      <button type="button" class="arr-btn" title="Align top edges" onclick={() => align('top')}><Icon name="obj-align-top" /></button>
      <button type="button" class="arr-btn" title="Align vertical middles" onclick={() => align('vmiddle')}><Icon name="obj-align-vmiddle" /></button>
      <button type="button" class="arr-btn" title="Align bottom edges" onclick={() => align('bottom')}><Icon name="obj-align-bottom" /></button>
    </div>
    <div class="fmt-sub">Distribute &amp; resize</div>
    <div class="arr-grid">
      <button type="button" class="arr-btn" title="Distribute horizontally (equal gaps)" disabled={!canDistribute} onclick={() => distribute('h')}><Icon name="obj-distribute-h" /></button>
      <button type="button" class="arr-btn" title="Distribute vertically (equal gaps)" disabled={!canDistribute} onclick={() => distribute('v')}><Icon name="obj-distribute-v" /></button>
      <button type="button" class="arr-btn" title="Match width (widest)" disabled={!canResizeMatch} onclick={() => resizeMatch('w')}><Icon name="obj-same-width" /></button>
      <button type="button" class="arr-btn" title="Match height (tallest)" disabled={!canResizeMatch} onclick={() => resizeMatch('h')}><Icon name="obj-same-height" /></button>
    </div>
  {/if}
  <div class="fmt-sub">Order</div>
  <div class="arr-grid">
    <button type="button" class="arr-btn" title="Bring to front" onclick={() => zorder('front')}><Icon name="z-front" /></button>
    <button type="button" class="arr-btn" title="Bring forward" onclick={() => zorder('forward')}><Icon name="z-forward" /></button>
    <button type="button" class="arr-btn" title="Send backward" onclick={() => zorder('backward')}><Icon name="z-backward" /></button>
    <button type="button" class="arr-btn" title="Send to back" onclick={() => zorder('back')}><Icon name="z-back" /></button>
  </div>
</section>
