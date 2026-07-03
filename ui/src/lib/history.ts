// Undo/redo replay layer (#84) — the only module that knows BOTH the diff model
// and `persist`. The store (doc.svelte.ts) mutates in-memory and returns the Step
// it undid/redid; this module translates that Step into the network writes that
// make the server match, then serializes those writes so rapid Cmd+Z presses
// reach the engine in undo/redo order.
//
// Two entry points — runUndo/runRedo — are called by BOTH the keybinding
// (interaction.ts) and the toolbar buttons (RailTools.svelte): one path, one
// serialization. Each mutates the store SYNCHRONOUSLY (so canUndo/canRedo and
// selection update instantly and a rapid second press is never dropped), then
// CAPTURES a concrete plan from the post-mutation store state before enqueuing
// the network replay. Capturing the values up front makes every step's writes
// immune to interference from a later step already applied in memory.

import type { EditorDoc, Step } from './doc.svelte';
import type { RestoreObjectRequest } from './persist';
import * as persist from './persist';

/** One serial chain per session. Guarantees the server sees steps in undo/redo
 * order, defeating the rapid-Cmd+Z reordering race. A rejected replay is swallowed
 * (already surfaced via doc.setError inside execPlan) so the chain never wedges. */
let chain: Promise<void> = Promise.resolve();

export function runUndo(doc: EditorDoc, layoutId: string): void {
  const step = doc.undo();
  if (step) enqueue(doc, layoutId, buildPlan(doc, step));
}

export function runRedo(doc: EditorDoc, layoutId: string): void {
  const step = doc.redo();
  if (step) enqueue(doc, layoutId, buildPlan(doc, step));
}

function enqueue(doc: EditorDoc, layoutId: string, plan: ReplayPlan): void {
  chain = chain.then(() => execPlan(doc, layoutId, plan)).catch(() => {});
}

// ── the plan (captured synchronously from post-undo/redo state) ──────────────

interface ReplayPlan {
  rec: number;
  restores: RestoreObjectRequest[]; // life present
  reparents: { id: number; partId: number; x: number; y: number }[]; // object.partId
  geometry: { id: number; x: number; y: number; w: number; h: number }[]; // object x/y/w/h, minus reparented
  z: { id: number; z: number }[]; // object.z (whole step, one batch)
  props: { id: number; props: Record<string, unknown> }[]; // object.props
  content: { id: number; content: string }[]; // object.content
  binding: { id: number; binding: string }[]; // object.binding (verbatim string)
  readOnly: { id: number; readOnly: boolean }[]; // object.readOnly
  partHeight: { id: number; height: number }[];
  partKind: { id: number; kind: string }[];
  partProps: { id: number; props: Record<string, unknown> }[];
  deletes: number[]; // life absent
}

/** Scan the Step once to learn WHICH (target,id,prop) cells moved, then read the
 * resulting VALUE for each from the already-mutated store, deduping per cell. */
function buildPlan(doc: EditorDoc, step: Step): ReplayPlan {
  const plan: ReplayPlan = {
    rec: doc.rec,
    restores: [],
    reparents: [],
    geometry: [],
    z: [],
    props: [],
    content: [],
    binding: [],
    readOnly: [],
    partHeight: [],
    partKind: [],
    partProps: [],
    deletes: [],
  };

  const geomIds = new Set<number>();
  const reparentIds = new Set<number>();
  const zIds = new Set<number>();
  const propIds = new Set<number>();
  const contentIds = new Set<number>();
  const bindingIds = new Set<number>();
  const roIds = new Set<number>();
  const lifeIds = new Set<number>();
  const pH = new Set<number>();
  const pK = new Set<number>();
  const pP = new Set<number>();

  for (const d of step) {
    if (d.target === 'life') {
      lifeIds.add(d.id);
    } else if (d.target === 'object') {
      switch (d.prop) {
        case 'x':
        case 'y':
        case 'w':
        case 'h':
          geomIds.add(d.id);
          break;
        case 'partId':
          reparentIds.add(d.id);
          break;
        case 'z':
          zIds.add(d.id);
          break;
        case 'props':
          propIds.add(d.id);
          break;
        case 'content':
          contentIds.add(d.id);
          break;
        case 'binding':
          bindingIds.add(d.id);
          break;
        case 'readOnly':
          roIds.add(d.id);
          break;
        case 'kind':
          console.error('history: object.kind diff is unreachable');
          break;
      }
    } else {
      // part
      if (d.prop === 'height') pH.add(d.id);
      else if (d.prop === 'kind') pK.add(d.id);
      else if (d.prop === 'props') pP.add(d.id);
      // 'position' is never emitted (applyPartPositions is non-undoable) — ignore
    }
  }

  for (const id of lifeIds) {
    const o = doc.getObject(id);
    if (o) {
      plan.restores.push({
        id: o.id,
        partId: o.partId,
        kind: o.kind,
        x: o.x,
        y: o.y,
        w: o.w,
        h: o.h,
        z: o.z,
        readOnly: o.readOnly,
        binding: o.binding,
        content: o.content,
        props: o.props,
      });
    } else {
      plan.deletes.push(id);
    }
  }
  for (const id of reparentIds) {
    const o = doc.getObject(id);
    if (o) plan.reparents.push({ id, partId: o.partId, x: o.x, y: o.y });
  }
  for (const id of geomIds) {
    if (reparentIds.has(id)) continue;
    const o = doc.getObject(id);
    if (o) plan.geometry.push({ id, x: o.x, y: o.y, w: o.w, h: o.h });
  }
  for (const id of zIds) {
    const o = doc.getObject(id);
    if (o) plan.z.push({ id, z: o.z });
  }
  for (const id of propIds) {
    const o = doc.getObject(id);
    if (o) plan.props.push({ id, props: JSON.parse(o.props || '{}') });
  }
  for (const id of contentIds) {
    const o = doc.getObject(id);
    if (o) plan.content.push({ id, content: o.content });
  }
  for (const id of bindingIds) {
    const o = doc.getObject(id);
    if (o) plan.binding.push({ id, binding: o.binding });
  }
  for (const id of roIds) {
    const o = doc.getObject(id);
    if (o) plan.readOnly.push({ id, readOnly: o.readOnly });
  }
  for (const id of pH) {
    const p = doc.getPart(id);
    if (p) plan.partHeight.push({ id, height: p.height });
  }
  for (const id of pK) {
    const p = doc.getPart(id);
    if (p) plan.partKind.push({ id, kind: p.kind });
  }
  for (const id of pP) {
    const p = doc.getPart(id);
    if (p) plan.partProps.push({ id, props: JSON.parse(p.props || '{}') });
  }
  return plan;
}

// ── execPlan — phase ordering (failure-mode core) ────────────────────────────
//
// Phases awaited in order; parallel within a phase. Restores first (rows other
// phases reference), deletes last (never delete a row an earlier phase touched).
// Reparents before geometry: an object with a partId diff is persisted only via
// setObjectPart (which carries x,y) and is excluded from `geometry`, so no rect
// is double-written or lost (band-settle never changes w/h — resize can't cross
// bands, interaction.ts).

async function execPlan(doc: EditorDoc, layoutId: string, plan: ReplayPlan): Promise<void> {
  const errors: unknown[] = [];
  const guard = (p: Promise<unknown>) =>
    p.catch((e) => {
      errors.push(e);
    });

  // PHASE A — restores (atomic, one transaction), before any reference.
  if (plan.restores.length) {
    await guard(
      persist
        .restoreObjects(layoutId, plan.restores, plan.rec)
        .then((views) => views.forEach((v) => doc.refreshResolved(v))),
    );
  }

  // PHASE B — reparents before geometry.
  await Promise.all(
    plan.reparents.map((r) => guard(persist.setObjectPart(layoutId, r.id, r.partId, r.x, r.y))),
  );

  // PHASE C — geometry, z (single batch), scalars/props/part edits, in parallel.
  const jobs: Promise<unknown>[] = [];
  for (const g of plan.geometry)
    jobs.push(guard(persist.setObjectGeometry(layoutId, g.id, { x: g.x, y: g.y, w: g.w, h: g.h })));
  if (plan.z.length) jobs.push(guard(persist.setObjectsZ(layoutId, plan.z)));
  for (const p of plan.props)
    jobs.push(guard(persist.setObjectProps(layoutId, p.id, p.props).then((s) => doc.setObjectStyles(p.id, s))));
  for (const c of plan.content)
    jobs.push(guard(persist.setObjectContent(layoutId, c.id, c.content).then((v) => doc.refreshResolved(v))));
  for (const b of plan.binding)
    jobs.push(
      guard(persist.setObjectBindingPath(layoutId, b.id, b.binding, plan.rec).then((v) => doc.refreshResolved(v))),
    );
  for (const r of plan.readOnly)
    jobs.push(
      guard(persist.setObjectReadOnly(layoutId, r.id, r.readOnly, plan.rec).then((v) => doc.refreshResolved(v))),
    );
  for (const h of plan.partHeight) jobs.push(guard(persist.setPartHeight(layoutId, h.id, h.height)));
  for (const k of plan.partKind) jobs.push(guard(persist.setPartKind(layoutId, k.id, k.kind)));
  for (const p of plan.partProps)
    jobs.push(guard(persist.setPartProps(layoutId, p.id, p.props).then((v) => doc.setPartStyle(p.id, v.partStyle))));
  await Promise.all(jobs);

  // PHASE D — deletes, last.
  await Promise.all(plan.deletes.map((id) => guard(persist.deleteObject(layoutId, id))));

  if (errors.length) doc.setError('History sync failed — reload the layout to reconcile.');
}
