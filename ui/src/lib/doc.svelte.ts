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
import type { DesignModel, FieldChoice, ObjectView, PartView } from './model';
import { llog } from './log';

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
  /** Opaque appearance bag JSON (#49) — what the Style zone edits; empty if unset. */
  props: string;
}

/** A layout part/band — the structural part contract (#43): id/kind/height plus
 * `position` (top→bottom order). Object membership is DERIVED: objects carry
 * their own `partId`, so a part record never holds an object list. */
export interface PartDoc {
  id: number;
  kind: string;
  height: number;
  position: number;
  /** Opaque appearance bag JSON (#49/Issue 7) — what the Band inspector edits
   * (today: a background `fill`); empty if unset. Document scope: undoable. */
  props: string;
  /** Server-derived inline CSS for the band's fill; empty when unstyled. A cached
   * projection of `props`, refreshed from the server after a fill edit. */
  partStyle: string;
}

// ── session types ───────────────────────────────────────────────────────────

/** The Create-zone tool palette (#62/#48). `pointer` is the select/drag tool
 * (the canvas's default behaviour); every other value ARMS the canvas so the next
 * click places that object kind. A `field` placement also needs one or more bound
 * fields — carried in [`EditorDoc.toolFieldIds`]. Part bands are added by a
 * separate rail action (they stack, not click-placed), so they are not a placement
 * tool. */
export type ToolKind = 'pointer' | 'text' | 'line' | 'rect' | 'ellipse' | 'field';

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
  objectStyle: string;
  textStyle: string;
  shapeStyle: string;
}

const EMPTY_RESOLVED: ObjectResolved = {
  field: false,
  shape: false,
  fieldId: null,
  label: '',
  value: '',
  objectStyle: '',
  textStyle: '',
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
  | 'props'
  | 'kind'
  | 'partId';

/** The part properties a part diff may target. */
export type PartProp = 'height' | 'position' | 'kind' | 'props';

/** A whole object captured for an insert/delete diff — its document record plus
 * the session render projection, so undo of a delete (or redo of a create) brings
 * the object back EXACTLY, value/style and all, without a re-hydrate. */
export interface ObjectSnapshot {
  doc: ObjectDoc;
  resolved: ObjectResolved;
}

/** One reversible change to document state. A `prop` diff carries the EXACT prior
 * and next primitive values (so a revert restores geometry/props byte-for-byte —
 * the "undo restores exact geometry" guarantee, #45). A `life` diff is a whole
 * object appearing/disappearing: `before`/`after` are the full snapshot or `null`
 * (insert = null→snapshot, delete = snapshot→null), so create/delete are atomic
 * undo steps (#48). */
export type Diff =
  | { target: 'object'; id: number; prop: ObjectProp; before: Primitive; after: Primitive }
  | { target: 'part'; id: number; prop: PartProp; before: Primitive; after: Primitive }
  | { target: 'life'; id: number; before: ObjectSnapshot | null; after: ObjectSnapshot | null };

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
  /** The layout's Browse view (`form` | `list` | `table`). Gates summary part
   * kinds (a form allows only header/body/footer, Issue 3). Hydrated, never edited. */
  #view = $state('');
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
  #selectedPartId = $state<number | null>(null);
  #hovered = $state<number | null>(null);
  #rec = $state(0);
  #total = $state(0);
  #hydrated = $state(false);
  /** The primary table's fields, for the Create zone's Field tool (#62). Refreshed
   * on hydrate; UI-only, never undoable. */
  #fields = $state<FieldChoice[]>([]);
  /** Active Create-zone tool (#62). `pointer` is select/drag; any other value arms
   * the canvas to place that kind on the next click. */
  #activeTool = $state<ToolKind>('pointer');
  /** The primary field a `field`-tool placement binds (legacy single-select path). */
  #toolFieldId = $state<number | null>(null);
  /** All fields the rail's `field` tool should place as a batch. */
  #toolFieldIds = $state<number[]>([]);
  /** Whether a field placement should also create a static label object. */
  #toolCreateLabel = $state(true);
  /** Canvas zoom factor (#62 Zoom zone): 1 = 100%. A viewport concern — applied as
   * a CSS scale on the stage, never persisted, never undoable. */
  #zoom = $state(1);
  /** Last hydration/load error, surfaced in the editor chrome. */
  #error = $state<string | null>(null);

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
    this.#view = model.view ?? '';
    this.#rec = model.rec;
    this.#total = model.total;
    this.#fields = model.fields.slice();

    this.#objects.clear();
    this.#resolved.clear();
    const parts: PartDoc[] = [];
    model.parts.forEach((p, position) => {
      // `position` is the part's top→bottom order; the wire model carries it as
      // array order (server: ORDER BY position, id), so the index is authoritative.
      parts.push({
        id: p.id,
        kind: p.kind,
        height: p.height,
        position,
        props: p.props,
        partStyle: p.partStyle,
      });
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
          props: o.props,
        });
        this.#resolved.set(o.id, {
          field: o.field,
          shape: o.shape,
          fieldId: o.fieldId,
          label: o.label,
          value: o.value,
          objectStyle: o.objectStyle,
          textStyle: o.textStyle,
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
    this.#selectedPartId = null;
    this.#hovered = null;
    this.#error = null;
    this.#hydrated = true;
    llog('store', 'hydrated', {
      parts: this.#parts.length,
      objects: this.#objects.size,
      fields: this.#fields.length,
    });
  }

  // ── document read accessors ──────────────────────────────────────────────

  get layoutId(): number {
    return this.#layoutId;
  }

  get width(): number {
    return this.#width;
  }

  /** The layout's Browse view (`form` | `list` | `table`) — the UI gates summary
   * part kinds on it (Issue 3). Empty until hydrated. */
  get view(): string {
    return this.#view;
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

  get parts(): readonly Readonly<PartDoc>[] {
    return this.#parts;
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
        props: p.props,
        partStyle: p.partStyle,
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
      view: this.#view,
      fields: this.#fields,
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
      props: o.props,
      objectStyle: r.objectStyle,
      textStyle: r.textStyle,
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

  /** Set a part's semantic kind. */
  setPartKind(id: number, kind: string): void {
    const p = this.#parts.find((pt) => pt.id === id);
    if (!p) return;
    this.#commit([{ target: 'part', id, prop: 'kind', before: p.kind, after: kind }]);
  }

  /** Set a part's appearance bag (#49/Issue 7 / Band inspector) — the opaque
   * `props` JSON string, an undoable document change mirroring `setObjectProps`.
   * The band's `partStyle` is server-derived (single source); after persisting,
   * the UI calls [`EditorDoc.setPartStyle`] with the server's value to refresh it. */
  setPartProps(id: number, props: string): void {
    const p = this.#parts.find((pt) => pt.id === id);
    if (!p) return;
    this.#commit([{ target: 'part', id, prop: 'props', before: p.props, after: props }]);
  }

  /** Update a part's derived fill style (session cache) from a fresh server
   * derivation — the response to a Band-inspector fill commit. Keeps the single
   * source of style derivation on the server; a plain mutation, not undo-recorded
   * (mirrors `setObjectStyles` / `applyPartPositions`). */
  setPartStyle(id: number, partStyle: string): void {
    const i = this.#parts.findIndex((p) => p.id === id);
    if (i < 0) return;
    const next = this.#parts.slice();
    next[i] = { ...next[i], partStyle };
    this.#parts = next;
  }

  /** Set an object's appearance bag (#49 / Style zone) — the opaque `props` JSON
   * string, an undoable document change. The canvas's `shapeStyle` is server-
   * derived (single source, [[layout-object-types]]); after persisting, the UI
   * calls [`EditorDoc.refreshResolved`] with the server's view to update it. */
  setObjectProps(id: number, props: string): void {
    this.setProp(id, 'props', props);
  }

  /** Add an object the server just created (#48) as one undoable insert step. The
   * caller supplies the object's `ObjectView` (resolved value/style and all) plus
   * its owning `partId` (the view carries geometry, not membership). Records a
   * `life` diff so undo deletes it and redo restores it exactly. */
  addObject(view: ObjectView, partId: number): void {
    if (this.#objects.has(view.id)) {
      llog('store', 'addObject SKIPPED (id already present)', { id: view.id });
      return;
    }
    this.#commitLife(view.id, null, this.#snapshotFromView(view, partId));
    llog('store', 'addObject', { id: view.id, partId, kind: view.kind, x: view.x, y: view.y, w: view.w, h: view.h });
  }

  /** Append a band the server just created (#48). A plain document mutation —
   * part lifecycle is not (yet) in the undo model the way objects are; a band-add
   * is rare and structural. The server assigns the slot (summaries land between
   * body and footer, shifting trailing parts down); `positions` carries the whole
   * layout's `[{id, position}]` after the insert, which we resync so the band
   * never renders below the footer. Falls back to bottom-most only when the caller
   * has no server positions. */
  addPart(view: PartView, positions?: { id: number; position: number }[]): void {
    if (this.#parts.some((p) => p.id === view.id)) return;
    // Provisional slot; the authoritative `positions` (below) overwrites it.
    const position =
      positions?.find((p) => p.id === view.id)?.position ??
      this.#parts.reduce((m, p) => Math.max(m, p.position), -1) + 1;
    this.#parts = [
      ...this.#parts,
      {
        id: view.id,
        kind: view.kind,
        height: view.height,
        position,
        props: view.props,
        partStyle: view.partStyle,
      },
    ];
    if (positions && positions.length) this.applyPartPositions(positions);
    this.selectPart(view.id);
    llog('store', 'addPart', { id: view.id, kind: view.kind, position });
  }

  /** Resync part positions after a server reorder (#Issue 4 `move_part`). Takes the
   * server's `[{id, position}]` and rewrites each matching `PartDoc.position`; the
   * part array is replaced immutably so reads stay reactive, and `renderModel`'s
   * position sort reflows the canvas. A plain structural mutation like
   * `addPart`/`removePart` — NOT undo-recorded. Unknown ids are ignored. */
  applyPartPositions(list: { id: number; position: number }[]): void {
    if (list.length === 0) return;
    const byId = new Map(list.map((p) => [p.id, p.position]));
    this.#parts = this.#parts.map((p) =>
      byId.has(p.id) ? { ...p, position: byId.get(p.id)! } : p,
    );
    llog('store', 'applyPartPositions', { list });
  }

  /** Remove a part and all objects it owns from the local document. Part lifecycle
   * is structural today, matching `addPart`: immediate, not undo-recorded. */
  removePart(id: number): void {
    if (!this.#parts.some((p) => p.id === id)) return;
    this.#parts = this.#parts.filter((p) => p.id !== id);
    for (const [objectId, o] of [...this.#objects.entries()]) {
      if (o.partId === id) {
        this.#objects.delete(objectId);
        this.#resolved.delete(objectId);
        this.#selection.delete(objectId);
      }
    }
    if (this.#selectedPartId === id) this.#selectedPartId = null;
    llog('store', 'removePart', { id });
  }

  /** Update an object's derived styles (session) from a fresh server
   * derivation — the response to a Style-zone props commit. Keeps the single
   * source of style derivation on the server ([[layout-object-types]]). */
  setObjectStyles(id: number, styles: Pick<ObjectView, 'objectStyle' | 'textStyle' | 'shapeStyle'>): void {
    const r = this.#resolved.get(id);
    if (!r) return;
    this.#resolved.set(id, {
      ...r,
      objectStyle: styles.objectStyle,
      textStyle: styles.textStyle,
      shapeStyle: styles.shapeStyle,
    });
  }

  /** Remove an object (#48 delete / undo of a create) as one undoable delete step.
   * No-op if the object is unknown. */
  removeObject(id: number): void {
    const o = this.#objects.get(id);
    if (!o) return;
    const snap: ObjectSnapshot = {
      doc: { ...o },
      resolved: { ...(this.#resolved.get(id) ?? EMPTY_RESOLVED) },
    };
    this.#commitLife(id, snap, null);
    llog('store', 'removeObject', { id });
  }

  /** Refresh an object's SESSION render projection (label/value/shape/shapeStyle)
   * from a fresh server view — e.g. after a props edit re-derives the shape style
   * server-side. Document state and undo history are untouched (session scope). */
  refreshResolved(view: ObjectView): void {
    if (!this.#objects.has(view.id)) return;
    this.#resolved.set(view.id, {
      field: view.field,
      shape: view.shape,
      fieldId: view.fieldId,
      label: view.label,
      value: view.value,
      objectStyle: view.objectStyle,
      textStyle: view.textStyle,
      shapeStyle: view.shapeStyle,
    });
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

  get selectedPartId(): number | null {
    return this.#selectedPartId;
  }

  isSelected(id: number): boolean {
    return this.#selection.has(id);
  }

  /** Select one object, replacing the selection unless `additive`. */
  select(id: number, additive = false): void {
    this.#selectedPartId = null;
    if (!additive) this.#selection.clear();
    this.#selection.add(id);
  }

  /** Replace the selection with exactly `ids`. */
  selectOnly(ids: Iterable<number>): void {
    this.#selectedPartId = null;
    this.#selection.clear();
    for (const id of ids) this.#selection.add(id);
    llog('select', 'selectOnly', { ids: [...this.#selection] });
  }

  /** Select every object in the document (Cmd/Ctrl+A). Order-insensitive — the
   * selection is a set; a placement-vs-select policy is the canvas's concern, not
   * the store's. Clears any part selection, mirroring `selectOnly`. */
  selectAll(): void {
    this.#selectedPartId = null;
    this.#selection.clear();
    for (const id of this.#objects.keys()) this.#selection.add(id);
    llog('select', 'selectAll', { count: this.#selection.size });
  }

  /** Toggle one object's membership in the selection. */
  toggle(id: number): void {
    this.#selectedPartId = null;
    if (this.#selection.has(id)) this.#selection.delete(id);
    else this.#selection.add(id);
  }

  clearSelection(): void {
    this.#selection.clear();
    this.#selectedPartId = null;
  }

  selectPart(id: number | null): void {
    this.#selection.clear();
    this.#selectedPartId = id === null || this.#parts.some((p) => p.id === id) ? id : null;
    llog('select', 'selectPart', { id: this.#selectedPartId });
  }

  /** Lowest legal part height: never clip objects already in the band. */
  minPartHeight(id: number): number {
    let min = 1;
    for (const o of this.#objects.values()) {
      if (o.partId === id) min = Math.max(min, o.y + o.h);
    }
    return min;
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

  // ── session: create-tool palette + field choices (#62 Create zone) ───────

  get fields(): readonly FieldChoice[] {
    return this.#fields;
  }

  get activeTool(): ToolKind {
    return this.#activeTool;
  }

  get toolFieldId(): number | null {
    return this.#toolFieldId;
  }

  get toolFieldIds(): readonly number[] {
    return this.#toolFieldIds;
  }

  get toolCreateLabel(): boolean {
    return this.#toolCreateLabel;
  }

  /** Arm a Create-zone tool. `pointer` returns the canvas to select/drag; for the
   * `field` tool, `fieldIds` are the fields a placement binds (ignored otherwise). */
  setTool(tool: ToolKind, fieldIds: number | number[] | null = null, createLabel = true): void {
    this.#activeTool = tool;
    const ids = tool === 'field' ? (Array.isArray(fieldIds) ? fieldIds.slice() : fieldIds === null ? [] : [fieldIds]) : [];
    this.#toolFieldIds = ids;
    this.#toolFieldId = ids[0] ?? null;
    this.#toolCreateLabel = tool === 'field' ? createLabel : true;
    llog('tool', 'setTool', { tool, fieldIds: this.#toolFieldIds, createLabel: this.#toolCreateLabel });
  }

  // ── session: canvas zoom (#62 Zoom zone) ─────────────────────────────────

  get zoom(): number {
    return this.#zoom;
  }

  /** Set the canvas zoom factor, clamped to 25%–400% and rounded to whole percents
   * so the readout and the CSS scale never drift. */
  setZoom(z: number): void {
    this.#zoom = Math.min(4, Math.max(0.25, Math.round(z * 100) / 100));
  }

  // ── session: lifecycle error ─────────────────────────────────────────────

  get error(): string | null {
    return this.#error;
  }

  setError(message: string | null): void {
    this.#error = message;
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

  /** Apply + record a single `life` (insert/delete) diff. Mirrors `#commit`: a
   * fresh edit clears the redo branch, then the snapshot is applied and the diff
   * joins the open group so a create/delete is one atomic undo step (#48). */
  #commitLife(id: number, before: ObjectSnapshot | null, after: ObjectSnapshot | null): void {
    if (this.#future.length > 0) this.#future = [];
    const d: Diff = { target: 'life', id, before, after };
    this.#set(d, after);
    this.#pending.push(d);
  }

  /** Write a single document change to `value` — the one place state mutates. Used
   * by both apply (→ after) and revert (→ before); updates are immutable
   * replacements so SvelteMap / `$state` reads stay reactive. A `life` value is a
   * whole-object snapshot (insert) or `null` (delete). */
  #set(d: Diff, value: Primitive | ObjectSnapshot | null): void {
    if (d.target === 'life') {
      this.#applyLife(d.id, value as ObjectSnapshot | null);
      return;
    }
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

  /** Apply a `life` diff: a `null` snapshot deletes the object (and drops its
   * resolved projection + selection); a snapshot (re)creates it exactly. */
  #applyLife(id: number, snap: ObjectSnapshot | null): void {
    if (snap === null) {
      this.#objects.delete(id);
      this.#resolved.delete(id);
      this.#selection.delete(id);
    } else {
      this.#objects.set(id, { ...snap.doc });
      this.#resolved.set(id, { ...snap.resolved });
    }
  }

  /** Build an insert snapshot from a server `ObjectView` (resolved value/style and
   * all) plus its owning `partId` (the view carries geometry, not membership). */
  #snapshotFromView(view: ObjectView, partId: number): ObjectSnapshot {
    return {
      doc: {
        id: view.id,
        partId,
        kind: view.kind,
        x: view.x,
        y: view.y,
        w: view.w,
        h: view.h,
        z: view.z,
        readOnly: view.readOnly,
        binding: view.binding,
        content: view.content,
        props: view.props,
      },
      resolved: {
        field: view.field,
        shape: view.shape,
        fieldId: view.fieldId,
        label: view.label,
        value: view.value,
        objectStyle: view.objectStyle,
        textStyle: view.textStyle,
        shapeStyle: view.shapeStyle,
      },
    };
  }

  /** Session-side: after an undo/redo, select the objects the step touched that
   * still exist (an undone insert leaves nothing to select). Selection is session
   * scope — this reacts to a document change but is never part of undo history. */
  #selectTouched(step: Step): void {
    this.#selection.clear();
    this.#selectedPartId = null;
    for (const d of step) {
      if ((d.target === 'object' || d.target === 'life') && this.#objects.has(d.id)) {
        this.#selection.add(d.id);
      } else if (d.target === 'part' && this.#parts.some((p) => p.id === d.id)) {
        this.#selectedPartId = d.id;
      }
    }
  }
}
