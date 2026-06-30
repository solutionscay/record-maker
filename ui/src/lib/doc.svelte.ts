// Layout Mode editor document store (#45) — the reactive core the canvas (#46),
// inspector, and undo history all read from and write through. Built as Svelte 5
// runes in a `.svelte.ts` module so the reactivity is *universal*: it drives
// components in the browser AND can be exercised headless in the zero-dep node
// test harness (scripts/doc-check.mjs) via `vite.ssrLoadModule`.
//
// ── State scopes (the multi-user seam) ──────────────────────────────────────
// State is partitioned into three scopes so a server-authoritative, multi-user
// model can land later WITHOUT a rewrite (issue #45):
//
//   • document — the persisted/synced truth: the part list plus, per object, its
//                structural contract (#43: kind, x/y/w/h/z, read_only, binding,
//                owning part). This is exactly what #15 will POST to the engine
//                and the ONLY scope the undo/redo history tracks.
//   • session  — local, per-client UI state: selection, hover, current record,
//                and the record-resolved render projection (label/value). Never
//                synced, never undoable.
//   • presence — ephemeral, per-peer state (remote cursors/selection) for future
//                multi-user. A typed seam today; carries no behaviour yet.
//
// Only USER-SOURCED document edits enter the undo history. Hydration and (future)
// remote/programmatic updates mutate state WITHOUT recording history — that
// boundary is the document/session/presence partition doing its job.
//
// In-memory only: there is no engine POST here (that is #15). The command surface
// — moveObject / resizeObject / setProp / setPartHeight → diff → apply / undo /
// redo / mark — is shaped so #15's geometry-persist and #46's moveable/selecto
// drag bind onto it without reshaping the store.

import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import type { DesignModel, ObjectView, PartView } from './model';

// ── document types ──────────────────────────────────────────────────────────

/** The structural sub-contract of an object that the store OWNS and the undo
 * history tracks — exactly the frozen #43 properties plus the owning part.
 * Geometry is part-relative px integers; `binding` is the dot-path expression;
 * `content` is the static-text slot of a `text` object; `kind` is `'field' |
 * 'text' | 'rect' | 'line' | 'ellipse'` (a string, mirroring the open contract). */
export interface ObjectDoc {
  id: number;
  /** Owning part id (object membership in a part is carried here, not by a list). */
  partId: number;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Stacking order within the part; higher paints in front (back→front = z,id). */
  z: number;
  readOnly: boolean;
  binding: string;
  /** Static text of a `text` object — its own slot, distinct from `binding`. */
  content: string;
}

/** A layout part/band — the structural part contract (#43): id/kind/height plus
 * `position` (top→bottom order). Object membership is DERIVED: objects carry
 * their own `partId`, so a part record never holds an object list. */
export interface PartDoc {
  id: number;
  kind: string;
  height: number;
  position: number;
}

// ── session types ───────────────────────────────────────────────────────────

/** The server-resolved render projection of an object (#44/#60): whether it is a
 * bound `field` (its field id, label, live value) or a `shape` (its derived
 * appearance `shapeStyle`) — all decided server-side (the value/label for the
 * current record; the shape flag/style from kind+props). This is NOT part of the
 * document contract: it is refreshed wholesale on hydrate / record change and
 * never enters the undo history. Kept beside the document object only so the
 * canvas can render. */
export interface ObjectResolved {
  field: boolean;
  shape: boolean;
  fieldId: number | null;
  label: string;
  value: string;
  shapeStyle: string;
}

const EMPTY_RESOLVED: ObjectResolved = {
  field: false,
  shape: false,
  fieldId: null,
  label: '',
  value: '',
  shapeStyle: '',
};

// ── presence types ──────────────────────────────────────────────────────────

/** Ephemeral per-peer state for the future multi-user seam. Intentionally
 * minimal today — it exists so presence is a real, typed scope rather than a
 * retrofit. Never synced into the document, never undoable. */
export interface PeerPresence {
  /** Object ids the peer has selected. */
  selection: number[];
}

// ── command / diff types ────────────────────────────────────────────────────

/** A primitive document value a diff can carry. */
type Primitive = number | string | boolean | null;

/** The object properties an object diff may target — the editable structural
 * contract only. The record-resolved projection fields are deliberately absent,
 * so a value/label refresh can never be mistaken for a user edit. */
export type ObjectProp =
  | 'x'
  | 'y'
  | 'w'
  | 'h'
  | 'z'
  | 'readOnly'
  | 'binding'
  | 'content'
  | 'kind'
  | 'partId';

/** The part properties a part diff may target. */
export type PartProp = 'height' | 'position' | 'kind';

/** One reversible change to document state. `before`/`after` are the EXACT prior
 * and next primitive values, so a revert restores geometry/props byte-for-byte
 * (the "undo restores exact geometry" guarantee, #45). */
export type Diff =
  | { target: 'object'; id: number; prop: ObjectProp; before: Primitive; after: Primitive }
  | { target: 'part'; id: number; prop: PartProp; before: Primitive; after: Primitive };

/** One atomic undo step: an ordered group of diffs that undo/redo together.
 * `mark()` seals the open group into a step — an atomic stopping point. */
type Step = Diff[];

/** Absolute geometry to set on an object; any omitted side is left unchanged.
 * A resize handle may move x/y while changing w/h, so all four are optional. */
export type Geometry = Partial<Pick<ObjectDoc, 'x' | 'y' | 'w' | 'h'>>;

const GEOMETRY_PROPS = ['x', 'y', 'w', 'h'] as const;

/**
 * The Layout Mode editor document store. One instance per mounted editor island.
 *
 * Read it reactively via the getters (`renderModel`, `canUndo`, `selection`, …);
 * mutate it only through the command methods, which build diffs, apply them, and
 * record them into the open undo group. `mark()` seals that group; `undo`/`redo`
 * step whole groups. `hydrate` seeds state from the #44 model and records NO
 * history.
 */
export class EditorDoc {
  // ── document scope ──
  #layoutId = $state(0);
  /** Canvas width (px). Server-derived (max object right edge + margin, #44); the
   * store keeps the hydrated value — recomputing it is the engine's job (#15). */
  #width = $state(0);
  /** Flat object map, keyed by object id — the store's primary document table. */
  readonly #objects = new SvelteMap<number, ObjectDoc>();
  /** Parts in `position` order. Replaced immutably on edit so reads stay reactive. */
  #parts = $state<PartDoc[]>([]);

  // ── document scope: undo history ──
  /** Sealed undo steps, oldest→newest. Pop to undo. */
  #past = $state<Step[]>([]);
  /** Sealed redo steps, oldest→newest. Pop to redo. Cleared by any fresh edit. */
  #future = $state<Step[]>([]);
  /** The open (un-sealed) group: diffs accumulated since the last `mark()`. */
  #pending = $state<Diff[]>([]);

  // ── session scope ──
  /** Record-resolved render projection, keyed by object id (label/value/…). */
  readonly #resolved = new SvelteMap<number, ObjectResolved>();
  readonly #selection = new SvelteSet<number>();
  #hovered = $state<number | null>(null);
  #rec = $state(0);
  #total = $state(0);
  #hydrated = $state(false);

  // ── presence scope (multi-user seam; no behaviour yet) ──
  readonly #presence = new SvelteMap<string, PeerPresence>();

  // ── hydration (NOT a user edit — records no history) ──────────────────────

  /**
   * Seed the store from the #44 read model: fill the flat object map + part list
   * (document), the resolved projection (session), and reset all session/history
   * state. Hydration is programmatic, so it pushes NO undo history and leaves the
   * history empty (`canUndo === false`). Re-hydrating (e.g. record change) is the
   * supported refresh path and likewise records nothing.
   */
  hydrate(model: DesignModel): void {
    this.#layoutId = model.layoutId;
    this.#width = model.width;
    this.#rec = model.rec;
    this.#total = model.total;

    this.#objects.clear();
    this.#resolved.clear();
    const parts: PartDoc[] = [];
    model.parts.forEach((p, position) => {
      // `position` is the part's top→bottom order; the wire model carries it as
      // array order (server: ORDER BY position, id), so the index is authoritative.
      parts.push({ id: p.id, kind: p.kind, height: p.height, position });
      for (const o of p.objects) {
        this.#objects.set(o.id, {
          id: o.id,
          partId: p.id,
          kind: o.kind,
          x: o.x,
          y: o.y,
          w: o.w,
          h: o.h,
          z: o.z,
          readOnly: o.readOnly,
          binding: o.binding,
          content: o.content,
        });
        this.#resolved.set(o.id, {
          field: o.field,
          shape: o.shape,
          fieldId: o.fieldId,
          label: o.label,
          value: o.value,
          shapeStyle: o.shapeStyle,
        });
      }
    });
    this.#parts = parts;

    // Hydration is not undoable: drop any history, open group, and session UI.
    this.#past = [];
    this.#future = [];
    this.#pending = [];
    this.#selection.clear();
    this.#hovered = null;
    this.#hydrated = true;
  }

  // ── document read accessors ──────────────────────────────────────────────

  get layoutId(): number {
    return this.#layoutId;
  }

  get width(): number {
    return this.#width;
  }

  get hydrated(): boolean {
    return this.#hydrated;
  }

  /** The structural document record for one object (live geometry, etc.) — what
   * #46 reads to compute drag deltas. Read-only; mutate via the commands. */
  getObject(id: number): Readonly<ObjectDoc> | undefined {
    return this.#objects.get(id);
  }

  /** The structural part record for one part. */
  getPart(id: number): Readonly<PartDoc> | undefined {
    return this.#parts.find((p) => p.id === id);
  }

  /**
   * The canvas render model — the #44 `DesignModel` shape, rebuilt from document
   * state joined with the session render projection. Objects are ordered back→
   * front by `(z, id)` and parts top→bottom by `(position, id)`, mirroring the
   * engine's SQL ordering exactly, so the Svelte DOM stays byte-identical to the
   * askama golden (the parity contract, #44). Reactive: any command re-derives it.
   */
  get renderModel(): DesignModel {
    const parts: PartView[] = this.#parts
      .slice()
      .sort((a, b) => a.position - b.position || a.id - b.id)
      .map((p) => ({
        id: p.id,
        kind: p.kind,
        height: p.height,
        objects: [...this.#objects.values()]
          .filter((o) => o.partId === p.id)
          .sort((a, b) => a.z - b.z || a.id - b.id)
          .map((o) => this.#toView(o)),
      }));
    return {
      layoutId: this.#layoutId,
      rec: this.#rec,
      total: this.#total,
      width: this.#width,
      parts,
    };
  }

  // Key order MUST match the server's ObjectView serde output (main.rs), since
  // doc-check deep-equals renderModel against the committed fixture JSON.
  #toView(o: ObjectDoc): ObjectView {
    const r = this.#resolved.get(o.id) ?? EMPTY_RESOLVED;
    return {
      id: o.id,
      kind: o.kind,
      field: r.field,
      shape: r.shape,
      fieldId: r.fieldId,
      x: o.x,
      y: o.y,
      w: o.w,
      h: o.h,
      z: o.z,
      readOnly: o.readOnly,
      binding: o.binding,
      content: o.content,
      label: r.label,
      value: r.value,
      shapeStyle: r.shapeStyle,
    };
  }

  // ── document edit commands (user-sourced → recorded) ─────────────────────

  /** Move an object by a relative px delta. Convenience over `setObjectGeometry`. */
  moveObject(id: number, dx: number, dy: number): void {
    const o = this.#objects.get(id);
    if (!o) return;
    this.setObjectGeometry(id, { x: o.x + dx, y: o.y + dy });
  }

  /** Set any subset of an object's absolute geometry (x/y/w/h). Omitted sides are
   * untouched; no-op components produce no diff. Records into the open group. */
  setObjectGeometry(id: number, geom: Geometry): void {
    const diffs: Diff[] = [];
    for (const prop of GEOMETRY_PROPS) {
      const next = geom[prop];
      if (next === undefined) continue;
      const d = this.#objectDiff(id, prop, next);
      if (d) diffs.push(d);
    }
    this.#commit(diffs);
  }

  /** Resize (and optionally reposition) an object — a resize handle that drags a
   * corner changes w/h and may also shift x/y. Semantic alias of
   * `setObjectGeometry`; both flow through the same diff path. */
  resizeObject(id: number, geom: Geometry): void {
    this.setObjectGeometry(id, geom);
  }

  /** Set a single structural property (z / readOnly / binding / kind / partId).
   * Geometry has dedicated commands; this covers the rest of the #43 contract. */
  setProp(id: number, prop: ObjectProp, value: Primitive): void {
    const d = this.#objectDiff(id, prop, value);
    if (d) this.#commit([d]);
  }

  /** Set a part's band height. Clamp/never-clip semantics (#43) belong to the
   * resize interaction (#46) / engine; the store records the primitive change. */
  setPartHeight(id: number, height: number): void {
    const p = this.#parts.find((pt) => pt.id === id);
    if (!p) return;
    this.#commit([{ target: 'part', id, prop: 'height', before: p.height, after: height }]);
  }

  // ── undo history: marks, undo, redo ──────────────────────────────────────

  /**
   * Seal the open group into one atomic undo step. Many diffs committed since the
   * last mark — e.g. every pointermove of a drag — collapse into a SINGLE undo
   * step. Idempotent: marking with nothing pending is a no-op (no empty steps).
   */
  mark(): void {
    if (this.#pending.length === 0) return;
    this.#past.push(this.#pending.slice());
    this.#pending = [];
  }

  get canUndo(): boolean {
    return this.#past.length > 0 || this.#pending.length > 0;
  }

  get canRedo(): boolean {
    return this.#future.length > 0;
  }

  /** Undo the most recent step. Any open group is sealed first, so a mid-gesture
   * undo still steps cleanly. Reverts diffs in reverse order, restoring exact
   * geometry, and (session-side) selects the objects the step touched. */
  undo(): void {
    this.mark();
    const step = this.#past.pop();
    if (!step) return;
    for (let i = step.length - 1; i >= 0; i--) this.#set(step[i], step[i].before);
    this.#future.push(step);
    this.#selectTouched(step);
  }

  /** Redo the most recently undone step, re-applying its diffs in order. */
  redo(): void {
    const step = this.#future.pop();
    if (!step) return;
    for (const d of step) this.#set(d, d.after);
    this.#past.push(step);
    this.#selectTouched(step);
  }

  // ── session: selection ───────────────────────────────────────────────────

  get selection(): ReadonlySet<number> {
    return this.#selection;
  }

  isSelected(id: number): boolean {
    return this.#selection.has(id);
  }

  /** Select one object, replacing the selection unless `additive`. */
  select(id: number, additive = false): void {
    if (!additive) this.#selection.clear();
    this.#selection.add(id);
  }

  /** Replace the selection with exactly `ids`. */
  selectOnly(ids: Iterable<number>): void {
    this.#selection.clear();
    for (const id of ids) this.#selection.add(id);
  }

  /** Toggle one object's membership in the selection. */
  toggle(id: number): void {
    if (this.#selection.has(id)) this.#selection.delete(id);
    else this.#selection.add(id);
  }

  clearSelection(): void {
    this.#selection.clear();
  }

  // ── session: hover + record ──────────────────────────────────────────────

  get hovered(): number | null {
    return this.#hovered;
  }

  hover(id: number | null): void {
    this.#hovered = id;
  }

  get rec(): number {
    return this.#rec;
  }

  get total(): number {
    return this.#total;
  }

  // ── presence (multi-user seam) ───────────────────────────────────────────

  get peers(): ReadonlyMap<string, PeerPresence> {
    return this.#presence;
  }

  /** Apply a peer's ephemeral presence. Never touches the document or history. */
  applyPresence(peerId: string, presence: PeerPresence): void {
    this.#presence.set(peerId, presence);
  }

  removePeer(peerId: string): void {
    this.#presence.delete(peerId);
  }

  // ── internals ────────────────────────────────────────────────────────────

  /** Build an object diff from the object's CURRENT value, or `null` if the
   * object is gone or the value is unchanged (so callers never record no-ops). */
  #objectDiff(id: number, prop: ObjectProp, after: Primitive): Diff | null {
    const o = this.#objects.get(id);
    if (!o) return null;
    const before = o[prop] as Primitive;
    if (before === after) return null;
    return { target: 'object', id, prop, before, after };
  }

  /** Apply + record a group of user-sourced diffs. No-ops are dropped; a fresh
   * edit invalidates the redo branch; survivors apply and join the open group. */
  #commit(diffs: Diff[]): void {
    const real = diffs.filter((d) => d.before !== d.after);
    if (real.length === 0) return;
    if (this.#future.length > 0) this.#future = [];
    for (const d of real) this.#set(d, d.after);
    this.#pending.push(...real);
  }

  /** Write a single document property to `value` — the one place state mutates.
   * Used by both apply (→ after) and revert (→ before); updates are immutable
   * replacements so SvelteMap / `$state` reads stay reactive. */
  #set(d: Diff, value: Primitive): void {
    if (d.target === 'object') {
      const o = this.#objects.get(d.id);
      if (!o) return;
      this.#objects.set(d.id, { ...o, [d.prop]: value } as ObjectDoc);
    } else {
      const i = this.#parts.findIndex((p) => p.id === d.id);
      if (i < 0) return;
      const next = this.#parts.slice();
      next[i] = { ...next[i], [d.prop]: value } as PartDoc;
      this.#parts = next;
    }
  }

  /** Session-side: after an undo/redo, select the objects the step touched, so
   * the user sees what changed. Selection is session scope — this reacts to a
   * document change but is never itself part of the undo history. */
  #selectTouched(step: Step): void {
    this.#selection.clear();
    for (const d of step) if (d.target === 'object') this.#selection.add(d.id);
  }
}
