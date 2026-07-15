// Clipboard controller (#85/#48, split in #135): cut / copy / paste / duplicate
// over the session clipboard (../clipboard.svelte). Paste and duplicate are
// policy wrappers over ONE clone-creation flow (`#materializeClones`), differing
// only in offset, restack/readOnly fidelity, and pointer forcing.

import type { ClipboardObject } from '../clipboard.svelte';
import { clipboard } from '../clipboard.svelte';
import type { ObjectView } from '../model';
import type { NewObjectRequest } from '../persist';
import { createObject, deleteObject, setObjectReadOnly, setObjectsZ } from '../persist';
import { deleteSelected as deleteSelectedAction, isDeleting } from '../actions';
import { GRID, clampOrigin, snapToGrid } from '../canvas-edit';
import { llog, lerror } from '../log';
import { parseProps } from '../object-props';
import type { CanvasContext } from './context';

export class ClipboardController {
  readonly #ctx: CanvasContext;

  constructor(ctx: CanvasContext) {
    this.#ctx = ctx;
  }

  /** Snapshot the current object selection into the session clipboard. Reads
   *  structural fields from getObject (ObjectDoc) and fieldId from the render
   *  model's ObjectView projection. No store mutation, no persist, not undoable. */
  copySelection(): boolean {
    const seed = [...this.#ctx.doc.selection];
    if (seed.length === 0) return false;

    // Pull a copied portal's authored columns + their caption labels (its owned
    // children, #168/#169) along with the frame, so paste re-creates them under
    // the new portal. A child selected on its own is copied as-is.
    const ids = new Set<number>(seed);
    for (const id of seed) {
      if (this.#ctx.doc.getObject(id)?.kind === 'portal') {
        for (const c of this.#ctx.doc.childObjectIds(id)) ids.add(c);
      }
    }

    const objects: ClipboardObject[] = [];
    for (const id of ids) {
      const d = this.#ctx.doc.getObject(id); // ObjectDoc: id/kind/parentObjectId/partId/x/y/w/h/z/readOnly/binding/content/props
      if (!d) continue;
      const v = this.#ctx.objectView(id); // ObjectView: fieldId
      objects.push({
        id: d.id,
        kind: d.kind,
        parentObjectId: d.parentObjectId,
        partId: d.partId,
        x: d.x,
        y: d.y,
        w: d.w,
        h: d.h,
        z: d.z,
        readOnly: d.readOnly,
        binding: d.binding,
        content: d.content,
        props: d.props, // string, verbatim
        fieldId: v?.fieldId ?? null,
      });
    }
    if (objects.length === 0) return false;
    clipboard.write({ objects });
    llog('clipboard', 'copied objects', { count: objects.length, ids });
    return true;
  }

  /** Cut = copy the selection, then run the shared delete command. One atomic
   *  undo step (the delete's single mark()). */
  async cutSelected(): Promise<void> {
    if (isDeleting()) return;
    if (!this.copySelection()) return; // capture BEFORE removal: after removeObject, fieldId is gone
    await deleteSelectedAction(this.#ctx.doc, this.#ctx.layoutId); // bulk delete → removeObject×N → ONE mark() → detach moveable
  }

  /** Paste every clipboard object as a NEW server object into its source part at
   *  a cascade offset, preserving relative layout / z / readOnly (the clone flow's
   *  paste policy). */
  async paste(): Promise<void> {
    const payload = clipboard.payload;
    if (!payload || payload.objects.length === 0) return;
    if (this.#ctx.placing || isDeleting() || this.#ctx.gesturing || this.#ctx.placement.isDrawing) return;

    const step = clipboard.nextPasteStep(); // 1,2,3…
    const newIds = await this.#materializeClones(payload.objects, {
      offset: { mode: 'cascade', desired: step * GRID }, // n * 8px down-right, before in-band capping
      restack: true, // pasted group lands ON TOP, readOnly restored
      forcePointer: true, // a still-armed draw tool would clear moveable's target
      label: 'paste',
    });
    if (newIds) llog('clipboard', 'pasted objects', { count: newIds.length, step, newIds });
  }

  /** Duplicate the current selection (#48) at a fixed one-step offset so the
   * copies land visibly next to the originals rather than exactly on top of
   * them, then select the copies (not the originals) — the usual "duplicate
   * leaves you holding the new one" convention. Same clone flow as paste, minus
   * the z-restack/readOnly pass: duplicates keep their server-assigned stacking
   * and don't carry the per-object read-only flag (low-priority gap; nothing in
   * the editor exposes setting that flag yet — field-editability-in-layout-mode). */
  async duplicateSelected(): Promise<void> {
    const ids = new Set(this.#ctx.doc.selection);
    if (ids.size === 0 || this.#ctx.placing) return;
    // A duplicated portal brings its authored columns + caption labels (its owned
    // children, #168/#169) so the copy is a complete, working portal.
    for (const id of [...ids]) {
      if (this.#ctx.doc.getObject(id)?.kind === 'portal') {
        for (const c of this.#ctx.doc.childObjectIds(id)) ids.add(c);
      }
    }
    // ObjectView (unlike the store's own ObjectDoc) carries the resolved fieldId
    // a clone needs, but not which part it's in — that's implicit in which
    // PartView it's nested under in renderModel, so pair the two here.
    const clips: ClipboardObject[] = [];
    for (const part of this.#ctx.doc.renderModel.parts) {
      for (const view of part.objects) {
        if (!ids.has(view.id)) continue;
        clips.push({
          id: view.id,
          kind: view.kind,
          parentObjectId: view.parentObjectId ?? null,
          partId: part.id,
          x: view.x,
          y: view.y,
          w: view.w,
          h: view.h,
          z: view.z,
          readOnly: view.readOnly,
          binding: view.binding,
          content: view.content,
          props: view.props,
          fieldId: view.fieldId,
        });
      }
    }
    const newIds = await this.#materializeClones(clips, {
      offset: { mode: 'fixed', dx: GRID * 2, dy: GRID * 2 },
      restack: false,
      forcePointer: false,
      label: 'duplicate',
    });
    if (newIds) llog('create', 'duplicated object(s)', { from: [...ids], created: newIds });
  }

  canPaste(): boolean {
    return clipboard.hasContent;
  }

  /** Materialize `clips` as NEW server objects — the ONE clone-creation flow
   *  behind both paste and duplicate, which differ only in policy: the offset
   *  (per-part capped cascade vs fixed step), whether the clones are restacked on
   *  top with readOnly restored, and whether the pointer tool is forced.
   *  All-or-nothing: either every clone is created+added under one mark(), or
   *  none is and any server rows created before a failure are rolled back.
   *  Returns the new ids, or null when nothing was materialized. */
  async #materializeClones(
    clips: ClipboardObject[],
    policy: {
      offset: { mode: 'cascade'; desired: number } | { mode: 'fixed'; dx: number; dy: number };
      restack: boolean;
      forcePointer: boolean;
      label: string;
    },
  ): Promise<number[] | null> {
    if (clips.length === 0) return null;
    const doc = this.#ctx.doc;
    this.#ctx.placing = true;
    const model = doc.renderModel;

    // Create in ascending source-z order so server insert order tracks original
    // stacking; the restack pass (paste) then makes it exact.
    const ordered = [...clips].sort((a, b) => a.z - b.z);

    // Resolve each clip's target part (its source part, or the last band if that
    // part is gone), then one (dx,dy) per clip from the offset policy.
    const resolved = ordered.map((c) => {
      const part = doc.getPart(c.partId) ?? model.parts.at(-1);
      return { c, partId: part?.id ?? c.partId, partH: part?.height ?? Number.MAX_SAFE_INTEGER };
    });
    const offset = this.#cloneOffsets(resolved, model.width, policy.offset);

    // One create request per clip, offset by its part's shared (dx,dy). The
    // parent link is resolved separately per phase: a portal (or any top-level
    // object) sends `null`; a child column/label sends its NEW parent id.
    const buildReq = (c: ClipboardObject, partId: number, parentObjectId: number | null): NewObjectRequest => {
      const { dx, dy } = offset.get(partId) ?? { dx: 0, dy: 0 };
      const caps = doc.capsFor(c.kind);
      return {
        partId,
        kind: c.kind,
        x: clampOrigin(snapToGrid(c.x + dx)),
        y: clampOrigin(snapToGrid(c.y + dy)),
        w: c.w,
        h: c.h,
        rec: doc.rec,
        fieldId: caps.bindable ? c.fieldId : null,
        // The binding is what actually recreates the value object: send it so a
        // field whose fieldId is null (unresolved binding / empty table) still
        // clones instead of 400ing, and so the copy keeps its exact binding. A
        // portal also lives in the `binding` slot — it holds the relationship
        // ROUTE — but is `bindable:false` (the field-binding inspector must not
        // target it), so carry its binding explicitly or create rejects the clone
        // with "portal needs a route". Matches how the portal TOOL sends its route
        // on placement (placement.ts). `fieldId`/`createLabel` stay gated on
        // `bindable`: a portal has no field and no caption to spawn.
        binding: caps.bindable || c.kind === 'portal' ? c.binding : null,
        createLabel: caps.bindable ? false : undefined, // NEVER auto-spawn a caption
        content: caps.contentSlot ? c.content : null,
        props: c.props ? parseProps(c.props) : null, // string → object for the wire
        parentObjectId,
      };
    };

    // Partition into parents and their owned children (#168/#169). A child whose
    // parent is ALSO being cloned re-parents onto the new parent; an orphan child
    // (parent not in this run) is created top-level, exactly as before. Parents
    // must be created FIRST so their fresh ids exist to point the children at.
    const clonedIds = new Set(resolved.map((r) => r.c.id));
    const isChild = (c: ClipboardObject) => c.parentObjectId !== null && clonedIds.has(c.parentObjectId);
    const parentRes = resolved.filter((r) => !isChild(r.c));
    const childRes = resolved.filter((r) => isChild(r.c));

    const created: { view: ObjectView; partId: number; clip: ClipboardObject }[] = [];
    const landedIds: number[] = []; // every server row created this run — for rollback
    let committed = false; // true once addObject + mark() fold the clones into store/undo
    // Reject if any create in a phase failed; the catch rolls back landed rows.
    const throwIfAnyRejected = (results: PromiseSettledResult<ObjectView[]>[]) => {
      if (results.some((r) => r.status === 'rejected')) {
        const firstRej = results.find((r) => r.status === 'rejected') as PromiseRejectedResult | undefined;
        throw firstRej?.reason ?? new Error(`${policy.label} failed`);
      }
    };
    try {
      // 1. Phase 1 — persist every PARENT (fresh ids). allSettled so a partial
      //    failure rolls back cleanly. Map each source id → its new id so the
      //    children can point at the clone rather than the original portal.
      const parentPlans = parentRes.map(({ c, partId }) => ({ partId, clip: c, req: buildReq(c, partId, null) }));
      const parentResults = await Promise.allSettled(parentPlans.map((p) => createObject(this.#ctx.layoutId, p.req)));
      for (const r of parentResults) if (r.status === 'fulfilled') for (const v of r.value) landedIds.push(v.id);
      throwIfAnyRejected(parentResults);
      const srcToNew = new Map<number, number>(); // source object id → new clone id
      parentResults.forEach((r, i) => {
        const views = (r as PromiseFulfilledResult<ObjectView[]>).value;
        for (const v of views) created.push({ view: v, partId: parentPlans[i].partId, clip: parentPlans[i].clip });
        if (views[0]) srcToNew.set(parentPlans[i].clip.id, views[0].id);
      });

      // 2. Phase 2 — persist the CHILDREN, each re-parented onto its new portal.
      if (childRes.length) {
        const childPlans = childRes.map(({ c, partId }) => ({
          partId,
          clip: c,
          req: buildReq(c, partId, srcToNew.get(c.parentObjectId as number) ?? null),
        }));
        const childResults = await Promise.allSettled(childPlans.map((p) => createObject(this.#ctx.layoutId, p.req)));
        for (const r of childResults) if (r.status === 'fulfilled') for (const v of r.value) landedIds.push(v.id);
        throwIfAnyRejected(childResults);
        childResults.forEach((r, i) => {
          for (const v of (r as PromiseFulfilledResult<ObjectView[]>).value) {
            created.push({ view: v, partId: childPlans[i].partId, clip: childPlans[i].clip });
          }
        });
      }

      // 3. Restore ascending source-z order across BOTH phases so the restack pass
      //    and the store insert preserve the group's internal stacking (a portal's
      //    columns/labels stack above its frame).
      created.sort((a, b) => a.clip.z - b.clip.z || a.clip.id - b.clip.id);

      // 4. Paste-only z / readOnly fidelity: place the cloned group ON TOP of each
      //    target part in preserved relative order, and restore readOnly. Persist
      //    first, then mirror into the doc so store + server agree; all diffs fold
      //    into ONE step. A failure here also unwinds through the catch (rows
      //    created in step 1 are rolled back), so a network drop mid-run never
      //    leaves phantom rows.
      const targetZ = policy.restack ? await this.#applyCloneZAndReadOnly(created) : null;

      // 5. Add every view (undoable life diffs) THEN a single mark() = one undo step.
      for (const { view, partId } of created) doc.addObject(view, partId);
      if (targetZ) {
        for (const { view, clip } of created) {
          // mirror z/readOnly into the doc
          const z = targetZ.get(view.id);
          if (z !== undefined && view.z !== z) doc.setProp(view.id, 'z', z);
          if (view.readOnly !== clip.readOnly) doc.setProp(view.id, 'readOnly', clip.readOnly);
        }
      }
      doc.mark(); // ← ONE atomic undo step
      committed = true; // past here the clones live in store + undo; never roll back

      // 6. Optionally force pointer mode (a still-armed draw tool clears moveable's
      //    target), then select the clones; rely on the reactive moveable sync
      //    (like the draw finish).
      if (policy.forcePointer) doc.setTool('pointer');
      const newIds = created.map((c) => c.view.id);
      doc.selectOnly(newIds);
      this.#ctx.hover.pin(newIds.at(-1) ?? null); // pin hover so the target sync resolves right
      return newIds;
    } catch (e) {
      // Roll back any rows that landed before the run committed, so store and
      // server never diverge (no phantom rows surfacing on the next reload).
      if (!committed && landedIds.length) {
        await Promise.allSettled(landedIds.map((id) => deleteObject(this.#ctx.layoutId, id)));
      }
      lerror('clipboard', `failed to ${policy.label}`, e);
      this.#ctx.reportError(e);
      return null;
    } finally {
      this.#ctx.placing = false;
    }
  }

  /** One (dx,dy) per target part for a clone run. `fixed` shifts every clip by
   *  the same delta (duplicate). `cascade` computes ONE capped delta per part:
   *  every object in a band shifts together, so relative layout is preserved
   *  exactly, and the delta is capped so the group's far edge stays in-band on
   *  BOTH axes (a per-object clamp would collapse offsets near an edge). Each
   *  capped delta is floored to a whole GRID step: keeps the paste grid-aligned
   *  and guarantees the far edge never snaps a few px past the in-band cap. */
  #cloneOffsets(
    resolved: { c: ClipboardObject; partId: number; partH: number }[],
    canvasWidth: number,
    policy: { mode: 'cascade'; desired: number } | { mode: 'fixed'; dx: number; dy: number },
  ): Map<number, { dx: number; dy: number }> {
    const offset = new Map<number, { dx: number; dy: number }>();
    if (policy.mode === 'fixed') {
      for (const { partId } of resolved) offset.set(partId, { dx: policy.dx, dy: policy.dy });
      return offset;
    }
    const ext = new Map<number, { maxX: number; maxY: number; partH: number }>();
    for (const { c, partId, partH } of resolved) {
      const e = ext.get(partId) ?? { maxX: 0, maxY: 0, partH };
      e.maxX = Math.max(e.maxX, c.x + c.w);
      e.maxY = Math.max(e.maxY, c.y + c.h);
      ext.set(partId, e);
    }
    const capToGrid = (max: number) =>
      Math.max(0, Math.floor(Math.min(policy.desired, max) / GRID) * GRID);
    for (const [partId, e] of ext) {
      offset.set(partId, {
        dx: capToGrid(canvasWidth - e.maxX),
        dy: capToGrid(e.partH - e.maxY),
      });
    }
    return offset;
  }

  /** Assign each pasted clone a z that stacks the whole group on top of its
   *  target part in preserved relative order, and persist z + readOnly. Returns
   *  the chosen z per created view id for the doc-mirror step. */
  async #applyCloneZAndReadOnly(
    created: { view: ObjectView; partId: number; clip: ClipboardObject }[],
  ): Promise<Map<number, number>> {
    // Next free z per target part = 1 + max z currently in that part.
    const nextZ = new Map<number, number>();
    const model = this.#ctx.doc.renderModel;
    for (const p of model.parts) {
      const maxZ = p.objects.reduce((m, o) => Math.max(m, o.z), -1);
      nextZ.set(p.id, maxZ + 1);
    }
    const targetZ = new Map<number, number>();
    const zItems: { id: number; z: number }[] = [];
    const roItems: { id: number; readOnly: boolean }[] = [];
    // created is already in ascending source-z order (creation order), so ranking
    // by iteration preserves the group's internal stacking.
    for (const c of created) {
      const z = nextZ.get(c.partId) ?? c.view.z;
      nextZ.set(c.partId, z + 1);
      targetZ.set(c.view.id, z);
      if (z !== c.view.z) zItems.push({ id: c.view.id, z });
      if (c.clip.readOnly !== c.view.readOnly) roItems.push({ id: c.view.id, readOnly: c.clip.readOnly });
    }
    if (zItems.length) await setObjectsZ(this.#ctx.layoutId, zItems);
    // Read-only flags are independent per object — persist them in parallel
    // instead of a sequential await loop.
    await Promise.all(roItems.map((r) => setObjectReadOnly(this.#ctx.layoutId, r.id, r.readOnly, this.#ctx.doc.rec)));
    return targetZ;
  }
}
