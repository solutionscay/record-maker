// Layout canvas interaction layer (#46) — binds moveable (drag / resize / snap +
// alignment guidelines / group transform) and selecto (marquee multi-select) to
// the editor document store (#45). The vanilla cores are consumed directly.
//
// Single source of truth is the store: pointer gestures never write element
// styles. moveable reports the new (already grid/edge-snapped) position/size and
// we push it through the store's command surface (setObjectGeometry); Svelte then
// re-renders the object from the store. moveable derives its control box from the
// gesture's cached start rect + pointer delta, so routing output back into the
// store does NOT create a feedback loop. A whole gesture commits as ONE undo step
// (mark on end) and persists to the engine via the bulk axum contract.
//
// Object identity without polluting the parity-checked canvas DOM: the Nth
// painted `.fm-obj` element maps to the Nth id in renderModel paint order (see
// canvas-edit.ts), so selections and hits resolve to ids by index — no data-*
// attributes added to the canvas.

import Moveable from 'moveable';
import Selecto from 'selecto';

import type { EditorDoc, ObjectDoc, ToolKind } from './doc.svelte';
import type { ObjectView } from './model';
import { GRID, SNAP_THRESHOLD, clampOrigin, elementsToObjectIds, objectIdsInPaintOrder, snapToGrid } from './canvas-edit';
import { defaultBox, defaultProps, partAtY } from './create';
import {
  createObject,
  deleteObject,
  setObjectContent,
  setObjectPart,
  setObjectProps as persistObjectProps,
  setObjectReadOnly,
  setObjectsZ,
} from './persist';
import type { NewObjectRequest } from './persist';
import { clipboard } from './clipboard.svelte';
import type { ClipboardObject } from './clipboard.svelte';
import { runUndo, runRedo } from './history';
import { FIELD_DRAG_MIME } from './dnd';
import { llog, lerror } from './log';
import { lineAngle, lineGeometryForAngle, lineLength, lineShapeStyle, normalizeAngle, numberProp, parseProps } from './object-props';

type DrawTool = Exclude<ToolKind, 'pointer'>;
type IdentitySnapshot = { painted: HTMLElement[]; ids: number[] };
type FieldPlacementTarget = {
  partId: number;
  partHeight: number;
  box: { x: number; y: number; w: number; h: number };
};

interface DrawPlacement {
  tool: DrawTool;
  fieldIds: number[];
  partId: number;
  partTop: number;
  partHeight: number;
  startX: number;
  startY: number;
  dragged: boolean;
  box: { x: number; y: number; w: number; h: number };
  line: { angle: number; length: number } | null;
}

export class CanvasInteraction {
  readonly #stage: HTMLElement;
  readonly #doc: EditorDoc;
  readonly #layoutId: string;
  readonly #moveable: Moveable;
  readonly #selecto: Selecto;

  /** True between a gesture's *Start and *End, so reactive re-syncs and hover
   * re-targeting don't fight the live transform moveable is driving. */
  #gesturing = false;
  /** Whether the active gesture actually moved/resized something — gates mark +
   * persist so a plain click (select, no movement) doesn't POST or push undo. */
  #moved = false;
  /** Object id currently under the cursor (drives hover pre-targeting). */
  #hoverId: number | null = null;
  #hoverOutline: HTMLElement | null = null;
  #textEditor: HTMLTextAreaElement | null = null;
  #textEditingId: number | null = null;
  /** Tears down the open inline text editor (removes its document-level
   * outside-press listener + element). Null when no editor is open. */
  #textEditorCleanup: (() => void) | null = null;
  #rectFrame: number | null = null;
  /** Object ids moveable currently targets, and a cheap key to dedupe setState. */
  #targetIds = new Set<number>();
  #targetKey = '';
  /** Canvas zoom factor (#62) — the stage is CSS-scaled by this, so client→model
   * pointer coordinates divide by it when placing a new object. */
  #zoom = 1;
  /** True while a placement POST is in flight, so a second click can't double-place. */
  #placing = false;
  /** True while selected object deletion is in flight, so repeat keys do not fan out. */
  #deleting = false;
  /** One-shot: swallow the native `click` the browser fires right after Selecto
   * commits selection. Without it, `#onClick` can run its empty-canvas deselect
   * path and wipe the marquee or modifier-click selection that just landed. A
   * bare empty-canvas click does NOT set this, so it still deselects as before. */
  #suppressNextClick = false;
  #suppressNextClickUntil = 0;
  /** Active draw-to-create gesture while a non-pointer tool is armed. */
  #drawing: DrawPlacement | null = null;
  #drawPreview: HTMLElement | null = null;
  /** Ghost box tracking a field drag-and-drop from the picker (see #onDragOver);
   * separate from #drawPreview since a drop can land while no tool is armed and
   * no #drawing gesture is in progress. */
  #dropPreview: HTMLElement | null = null;
  #resizeStarts = new Map<
    number,
    {
      x: number;
      y: number;
      w: number;
      h: number;
      direction: number[];
      clientX: number;
      clientY: number;
    }
  >();
  #rotatingLineId: number | null = null;
  #rotateStartLength = 0;
  #dirtyLineProps = new Set<number>();
  #gestureIdentity: IdentitySnapshot | null = null;

  constructor(stage: HTMLElement, doc: EditorDoc, layoutId: string) {
    this.#stage = stage;
    this.#doc = doc;
    this.#layoutId = layoutId;

    this.#moveable = new Moveable(stage, {
      target: [],
      draggable: true,
      resizable: true,
      rotatable: false,
      snappable: true,
      snapGridWidth: GRID,
      snapGridHeight: GRID,
      snapThreshold: SNAP_THRESHOLD,
      isDisplaySnapDigit: false,
      elementGuidelines: [],
      origin: false,
      zoom: this.#zoom,
    });

    // ── drag (single + group). Single-target start also makes it the selection. ──
    this.#moveable.on('dragStart', (e) => {
      llog('drag', 'dragStart', { id: this.#idForElement(e.target) });
      this.#begin();
      this.#selectFromTarget(e.target);
    });
    this.#moveable.on('drag', (e) => this.#applyMove(e.target, e.left, e.top));
    this.#moveable.on('dragEnd', () => this.#end('drag'));
    this.#moveable.on('dragGroupStart', () => this.#begin());
    this.#moveable.on('dragGroup', (e) => e.events.forEach((ev) => this.#applyMove(ev.target, ev.left, ev.top)));
    this.#moveable.on('dragGroupEnd', () => this.#end('drag'));

    // ── resize (single + group) — e.drag carries the new left/top for top/left handles ──
    this.#moveable.on('resizeStart', (e) => {
      llog('resize', 'resizeStart', { id: this.#idForElement(e.target) });
      this.#begin();
      this.#selectFromTarget(e.target);
      this.#captureResizeStart(e.target, e.direction, e.inputEvent);
    });
    this.#moveable.on('resize', (e) => this.#applyResize(e.target, e.width, e.height, e.drag.left, e.drag.top, e.inputEvent));
    this.#moveable.on('resizeEnd', () => this.#end('resize'));
    this.#moveable.on('resizeGroupStart', (e) => {
      this.#begin();
      e.events.forEach((ev) => this.#captureResizeStart(ev.target, ev.direction, ev.inputEvent));
    });
    this.#moveable.on('resizeGroup', (e) =>
      e.events.forEach((ev) => this.#applyResize(ev.target, ev.width, ev.height, ev.drag.left, ev.drag.top, ev.inputEvent)),
    );
    this.#moveable.on('resizeGroupEnd', () => this.#end('resize'));

    // ── rotate (line objects only) ──────────────────────────────────────────
    this.#moveable.on('rotateStart', (e) => {
      const id = this.#idForElement(e.target);
      const o = id === undefined ? undefined : this.#doc.getObject(id);
      if (id === undefined || !o || o.kind !== 'line') return false;
      this.#begin();
      this.#selectFromTarget(e.target);
      const props = this.#propsForObject(o);
      const angle = numberProp(props.angle, 0);
      this.#rotatingLineId = id;
      this.#rotateStartLength = lineLength(o, props);
      e.set(angle);
      llog('rotate', 'rotateStart', { id, angle, length: this.#rotateStartLength });
    });
    this.#moveable.on('rotate', (e) => this.#applyLineRotate(e.beforeRotation));
    this.#moveable.on('rotateEnd', () => this.#endLineRotate());

    // ── marquee multi-select ──
    this.#selecto = new Selecto({
      container: stage,
      rootContainer: stage,
      selectableTargets: ['.fm-obj'],
      selectByClick: true,
      selectFromInside: false,
      hitRate: 0,
      toggleContinueSelect: 'shift',
    });
    // A marquee over empty canvas live-updates the store selection.
    this.#selecto.on('select', (e) => this.#doc.selectOnly(this.#elementsToIds(e.selected)));
    // When the marquee ends, pin the FINAL selection and attach moveable to the
    // group SYNCHRONOUSLY. The reactive refresh (App.svelte $effect) is async, so
    // relying on it alone can leave `#targetIds` stale at the instant the user
    // presses to drag the group — the press would then re-select a single object
    // instead of grabbing the marqueed set. Running `#updateTarget()` here makes
    // the control box appear and populates `#targetIds` before the next pointer
    // stream, so a press on any selected object drags the whole group.
    this.#selecto.on('selectEnd', (e) => {
      const selectedIds = this.#elementsToIds(e.selected);
      this.#doc.selectOnly(selectedIds);
      this.#updateTarget();
      // Selecto's pointer sequence is followed by a native `click` on the
      // canvas/band. Swallow it after marquee drags, and also after object clicks
      // that produced a selection. Rotated lines can visually extend outside their
      // tiny `.fm-obj` box, so that trailing click may look like empty canvas even
      // though Selecto correctly selected the line.
      if (!e.isClick || selectedIds.length > 0) {
        this.#swallowNextClick();
      }
    });
    // Decide, at press time, who owns the gesture:
    this.#selecto.on('dragStart', (e) => {
      const input = e.inputEvent;
      // A non-pointer tool is armed → this press PLACES an object, not selects.
      if (this.#doc.activeTool !== 'pointer') {
        llog('place', 'press while tool armed', {
          tool: this.#doc.activeTool,
          clientX: input.clientX,
          clientY: input.clientY,
        });
        e.stop();
        if (!this.#pointInCanvas(input.clientX, input.clientY)) {
          llog('place', 'armed click outside canvas ignored', {
            tool: this.#doc.activeTool,
            clientX: input.clientX,
            clientY: input.clientY,
          });
          return;
        }
        this.#startDraw(input);
        return;
      }
      const target = input.target as Element | null;
      const identity = this.#identitySnapshot();
      // Moveable's group overlay can be the event target instead of the real object.
      const objEl =
        (target?.closest('.fm-obj') as HTMLElement | null) ??
        (this.#targetIds.size > 1 ? this.#objectElementAt(input.clientX, input.clientY) : null);
      const id = objEl ? this.#idForElement(objEl, identity) : undefined;
      if (objEl && id === undefined) {
        llog('target', 'press on object but id UNRESOLVED', { painted: identity.painted.length });
      }
      // Modifier toggles must run before the moveable-control-box guard because
      // group-selection presses can arrive through Moveable's transparent overlay.
      if (objEl && id !== undefined && (input.shiftKey || input.ctrlKey || input.metaKey)) {
        llog('select', 'toggle membership', { id });
        this.#doc.toggle(id);
        e.stop();
        this.#updateTarget();
        // `#updateTarget()` may detach the pressed overlay before the browser's
        // trailing native click fires, so swallow exactly that next click.
        this.#swallowNextClick();
        return;
      }
      // moveable's own control box (a resize handle / the drag area) → its gesture.
      if (target && this.#moveable.isMoveableElement(target)) {
        llog('drag', 'press on moveable control box → moveable owns gesture');
        e.stop();
        return;
      }
      if (!objEl) {
        llog('select', 'press on empty canvas → marquee');
        return; // empty canvas → selecto runs its marquee
      }
      if (id === undefined) {
        return;
      }
      // Already selected/targeted: let Moveable handle this same pointer stream.
      if (this.#targetIds.has(id)) {
        llog('drag', 'press on targeted object → moveable drags it', { id });
        e.stop();
        return;
      }
      // Select and hand the current pointer event to Moveable immediately. Hover
      // no longer pre-targets objects, so this preserves click-drag in one move
      // without showing resize handles on mere hover.
      llog('drag', 'press on un-targeted object → select + start drag', { id });
      this.#doc.selectOnly([id]);
      this.#targetKey = String(id);
      this.#targetIds = new Set([id]);
      const guidelines = identity.painted.filter((el) => el !== objEl);
      // Set the target, then start the drag SYNCHRONOUSLY on this same pointerdown.
      // moveable.dragStart() flushes the just-set target (its internal `$_timer`
      // guard forceUpdates before triggering) and only fires if `objEl` matches the
      // live target — so the press latches straight into a drag, one gesture. The
      // old code ran dragStart inside setState's async callback, a frame late and
      // past the live pointer stream, so the first press only selected and the user
      // had to press again to drag.
      this.#moveable.setState({ target: objEl, elementGuidelines: guidelines });
      this.#moveable.dragStart(input, objEl);
      e.stop();
    });

    this.#stage.addEventListener('pointermove', this.#onPointerMove);
    this.#stage.addEventListener('pointerleave', this.#onPointerLeave);
    this.#stage.addEventListener('click', this.#onClick);
    this.#stage.addEventListener('dblclick', this.#onDoubleClick);
    window.addEventListener('keydown', this.#onKeyDown);
    // Field drag-and-drop (#79 follow-up) — native HTML5 DnD, not a pointer
    // gesture, so it coexists with moveable/selecto's own pointer handling
    // above without fighting over the same events.
    this.#stage.addEventListener('dragover', this.#onDragOver);
    this.#stage.addEventListener('dragleave', this.#onDragLeave);
    this.#stage.addEventListener('drop', this.#onDrop);
    llog('init', 'CanvasInteraction ready', { layoutId, painted: this.#paintedElements().length });
  }

  /** Reconcile moveable's target with the store selection (called reactively when
   * selection or geometry changes — e.g. after an undo). No-op during a gesture. */
  refresh(): void {
    this.#updateTarget();
    // #updateTarget early-returns when the selection set is unchanged, so a
    // geometry-only change (align/distribute/resize-match, undo of same) would
    // leave moveable's control box on the old rect. Whenever a target is live,
    // re-measure it from the (now updated) DOM so the box follows the objects.
    if (!this.#gesturing && this.#targetIds.size > 0) this.#scheduleRectUpdate();
  }

  /** Tell the interaction layer the current canvas zoom (#62), so client→model
   * pointer conversion during placement divides by it. */
  setZoom(zoom: number): void {
    const z = zoom > 0 ? zoom : 1;
    if (z !== this.#zoom) llog('zoom', 'setZoom', { zoom: z });
    this.#zoom = z;
    this.#moveable.setState({ zoom: z });
  }

  /** Start a draw-to-create gesture. Release persists the final box; a very short
   * click falls back to the tool's default size, but creation still waits for
   * pointer-up so objects are not dropped on press. */
  #startDraw(input: MouseEvent | PointerEvent): void {
    const tool = this.#doc.activeTool;
    if (tool === 'pointer' || this.#placing || this.#drawing) {
      llog('place', 'draw start ignored', { tool, placing: this.#placing, drawing: !!this.#drawing });
      return;
    }
    const point = this.#canvasPoint(input.clientX, input.clientY);
    if (!point) {
      llog('error', 'draw start: no .fm-canvas in stage');
      this.#doc.setTool('pointer');
      return;
    }
    const where = partAtY(this.#doc.renderModel, point.y);
    if (!where) {
      llog('place', 'no part under draw start', { modelY: Math.round(point.y) });
      this.#doc.setTool('pointer');
      return;
    }
    const part = this.#doc.getPart(where.partId);
    if (!part) return;
    const fieldIds = tool === 'field' ? this.#doc.toolFieldIds.slice() : [];
    if (tool === 'field' && fieldIds.length === 0) {
      llog('place', 'field tool armed but no field chosen — nothing to draw');
      this.#doc.setTool('pointer');
      return;
    }
    this.#drawing = {
      tool,
      fieldIds,
      partId: where.partId,
      partTop: point.y - where.localY,
      partHeight: part.height,
      startX: point.x,
      startY: point.y,
      dragged: false,
      box: { x: point.x, y: where.localY, w: 1, h: 1 },
      line: null,
    };
    this.#drawPreview = document.createElement('div');
    this.#drawPreview.className = `le-draw-preview le-draw-${tool}`;
    this.#partOverlay()?.append(this.#drawPreview);
    this.#updateDraw(input);
    window.addEventListener('pointermove', this.#onDrawMove);
    window.addEventListener('pointerup', this.#onDrawUp, { once: true });
    window.addEventListener('mousemove', this.#onDrawMove);
    window.addEventListener('mouseup', this.#onDrawUp, { once: true });
    llog('place', 'draw start', {
      tool,
      partId: where.partId,
      startX: Math.round(point.x),
      startY: Math.round(where.localY),
      fieldCount: fieldIds.length,
    });
  }

  #onDrawMove = (e: PointerEvent | MouseEvent): void => {
    this.#updateDraw(e);
  };

  #onDrawUp = (e: PointerEvent | MouseEvent): void => {
    this.#updateDraw(e);
    void this.#finishDraw();
  };

  #updateDraw(input: MouseEvent | PointerEvent): void {
    const drawing = this.#drawing;
    const point = this.#canvasPoint(input.clientX, input.clientY);
    if (!drawing || !point) return;

    const partBottom = drawing.partTop + drawing.partHeight;
    const endX = Math.max(0, point.x);
    const endY = Math.min(partBottom, Math.max(drawing.partTop, point.y));
    const dragged = Math.abs(endX - drawing.startX) >= 4 || Math.abs(endY - drawing.startY) >= 4;
    drawing.dragged = drawing.dragged || dragged;
    let x: number;
    let yGlobal: number;
    let w: number;
    let h: number;

    if (!drawing.dragged) {
      x = Math.max(0, drawing.startX);
      yGlobal = drawing.startY;
      w = 1;
      h = 1;
    } else if (drawing.tool === 'line') {
      const sx = snapToGrid(drawing.startX);
      const sy = snapToGrid(drawing.startY);
      const ex = snapToGrid(endX);
      const ey = snapToGrid(endY);
      x = Math.min(sx, ex);
      yGlobal = Math.min(sy, ey);
      w = Math.max(1, Math.abs(ex - sx));
      h = Math.max(1, Math.abs(ey - sy));
      drawing.line = {
        angle: lineAngle(sx, sy, ex, ey),
        length: Math.max(1, Math.hypot(ex - sx, ey - sy)),
      };
    } else {
      x = Math.min(drawing.startX, endX);
      yGlobal = Math.min(drawing.startY, endY);
      w = Math.max(8, Math.abs(endX - drawing.startX));
      h = Math.max(8, Math.abs(endY - drawing.startY));
    }

    x = snapToGrid(x);
    yGlobal = snapToGrid(yGlobal);
    w = Math.max(1, snapToGrid(w));
    h = Math.max(1, snapToGrid(h));
    const y = Math.min(drawing.partHeight - 1, Math.max(0, yGlobal - drawing.partTop));
    drawing.box = {
      x: clampOrigin(x),
      y: clampOrigin(y),
      w,
      h: Math.min(h, Math.max(1, drawing.partHeight - y)),
    };
    this.#paintDrawPreview(drawing);
  }

  #paintDrawPreview(drawing: DrawPlacement): void {
    if (!this.#drawPreview) return;
    if (drawing.tool === 'line') {
      const line = drawing.line ?? { angle: 0, length: Math.max(1, drawing.box.w) };
      this.#drawPreview.style.left = `${drawing.box.x + drawing.box.w / 2 - line.length / 2}px`;
      this.#drawPreview.style.top = `${drawing.partTop + drawing.box.y + drawing.box.h / 2 - 1}px`;
      this.#drawPreview.style.width = `${line.length}px`;
      this.#drawPreview.style.height = '2px';
      this.#drawPreview.style.transform = `rotate(${line.angle}deg)`;
      return;
    }
    this.#drawPreview.style.left = `${drawing.box.x}px`;
    this.#drawPreview.style.top = `${drawing.partTop + drawing.box.y}px`;
    this.#drawPreview.style.width = `${drawing.box.w}px`;
    this.#drawPreview.style.height = `${drawing.box.h}px`;
    this.#drawPreview.style.transform = '';
  }

  /** Persist the drawn object and add the returned view(s) to the store as one
   * undoable create step. A `field` adds both its value object and spawned label. */
  async #finishDraw(): Promise<void> {
    const drawing = this.#drawing;
    if (!drawing || this.#placing) return;
    window.removeEventListener('pointermove', this.#onDrawMove);
    window.removeEventListener('pointerup', this.#onDrawUp);
    window.removeEventListener('mousemove', this.#onDrawMove);
    window.removeEventListener('mouseup', this.#onDrawUp);
    this.#drawPreview?.remove();
    this.#drawPreview = null;
    this.#drawing = null;

    const { tool, partId } = drawing;
    const finalBox = drawing.dragged ? drawing.box : this.#defaultPlacementBox(drawing);
    llog('place', 'draw finish', { tool, partId, dragged: drawing.dragged, ...finalBox });

    this.#placing = true;
    try {
      let views: ObjectView[];
      if (tool === 'field') {
        const fieldIds = drawing.fieldIds;
        if (fieldIds.length === 0) {
          llog('place', 'field draw finished but no field chosen — nothing to create');
          return;
        }
        views = await this.#createFieldObjectsAt({ partId, partHeight: drawing.partHeight, box: finalBox }, fieldIds);
      } else {
        views = await createObject(this.#layoutId, {
          partId,
          kind: tool,
          x: finalBox.x,
          y: finalBox.y,
          w: finalBox.w,
          h: finalBox.h,
          content: tool === 'text' ? 'Text' : null,
          props: this.#placementProps(tool, drawing, finalBox),
          rec: this.#doc.rec,
        });
      }
      llog('create', 'server created object(s)', {
        objects: views.map((v) => ({ id: v.id, kind: v.kind, x: v.x, y: v.y, w: v.w, h: v.h })),
      });
      const placed = views.at(-1); // the field VALUE (its label sorts before it)
      const selectedIds = tool === 'field' ? views.map((v) => v.id) : placed ? [placed.id] : [];
      const committed = this.#commitPlacedViews(partId, views, selectedIds, 'draw');
      if (committed && tool === 'text') {
        this.#startTextEdit(committed.id);
      }
    } catch (e) {
      lerror('place', 'create failed', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      this.#placing = false;
      if (this.#doc.activeTool !== 'pointer') this.#doc.setTool('pointer');
    }
  }

  #defaultPlacementBox(drawing: DrawPlacement): { x: number; y: number; w: number; h: number } {
    const size = defaultBox(drawing.tool);
    const x = clampOrigin(snapToGrid(drawing.startX));
    const y = clampOrigin(snapToGrid(drawing.startY - drawing.partTop));
    return {
      x,
      y,
      w: size.w,
      h: Math.min(size.h, Math.max(1, drawing.partHeight - y)),
    };
  }

  // ── field drag-and-drop (drag from the field picker onto the canvas) ──────
  // A second, independent way to place a field object, alongside the draw-tool
  // gesture above. Deliberately its OWN small pipeline rather than a refactor
  // of #finishDraw's field branch into a shared helper: the two have different
  // inputs (a live drag point with no start/end box vs. a completed press-drag
  // rectangle) and the draw-tool path is exercised by the existing undo/redo +
  // parity tests, so this stays additive instead of risking that path.

  /** Resolve a client point to "what would placing a field here look like" —
   * used both to paint the drag preview on every `dragover` and, identically,
   * to compute the box actually persisted on `drop`. Null when the point isn't
   * over a part (no canvas under the cursor, or past the last part's row —
   * partAtY still returns the last part then, so this is really just "no
   * `.fm-canvas` under the cursor at all"). */
  #dropTargetFor(clientX: number, clientY: number): (FieldPlacementTarget & { partTop: number }) | null {
    const point = this.#canvasPoint(clientX, clientY);
    if (!point) return null;
    const where = partAtY(this.#doc.renderModel, point.y);
    if (!where) return null;
    const part = this.#doc.getPart(where.partId);
    if (!part) return null;
    const size = defaultBox('field');
    const x = clampOrigin(snapToGrid(point.x));
    const y = clampOrigin(snapToGrid(where.localY));
    return {
      partId: where.partId,
      partTop: point.y - where.localY,
      partHeight: part.height,
      box: { x, y, w: size.w, h: Math.min(size.h, Math.max(1, part.height - y)) },
    };
  }

  #onDragOver = (e: DragEvent): void => {
    if (!e.dataTransfer?.types.includes(FIELD_DRAG_MIME)) return;
    e.preventDefault(); // required for `drop` to fire on this target at all
    e.dataTransfer.dropEffect = 'copy';
    this.#paintDropPreview(this.#dropTargetFor(e.clientX, e.clientY));
  };

  #onDragLeave = (e: DragEvent): void => {
    // `dragleave` also fires when moving between child elements WITHIN the
    // stage (e.g. crossing from the canvas onto a part label); only clear the
    // preview once the pointer has actually left the stage entirely.
    const related = e.relatedTarget as Node | null;
    if (!related || !this.#stage.contains(related)) this.#paintDropPreview(null);
  };

  #onDrop = (e: DragEvent): void => {
    this.#paintDropPreview(null);
    const raw = e.dataTransfer?.getData(FIELD_DRAG_MIME);
    if (!raw) return; // some other kind of drop (e.g. dragging in browser text) — ignore
    e.preventDefault();
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return;
    }
    if (!Array.isArray(parsed) || parsed.length === 0 || !parsed.every((v) => typeof v === 'number')) return;
    const target = this.#dropTargetFor(e.clientX, e.clientY);
    if (!target) {
      llog('place', 'field drop outside any part — ignored', { clientX: e.clientX, clientY: e.clientY });
      return;
    }
    void this.#placeFieldsAt(target, parsed);
  };

  #paintDropPreview(target: { partTop: number; box: { x: number; y: number; w: number; h: number } } | null): void {
    if (!target) {
      this.#dropPreview?.remove();
      this.#dropPreview = null;
      return;
    }
    if (!this.#dropPreview) {
      this.#dropPreview = document.createElement('div');
      this.#dropPreview.className = 'le-draw-preview le-draw-field';
      this.#partOverlay()?.append(this.#dropPreview);
    }
    this.#dropPreview.style.left = `${target.box.x}px`;
    this.#dropPreview.style.top = `${target.partTop + target.box.y}px`;
    this.#dropPreview.style.width = `${target.box.w}px`;
    this.#dropPreview.style.height = `${target.box.h}px`;
  }

  async #placeFieldsAt(target: FieldPlacementTarget, fieldIds: number[]): Promise<void> {
    if (this.#placing) return;
    this.#placing = true;
    try {
      const views = await this.#createFieldObjectsAt(target, fieldIds);
      llog('create', 'server created object(s) via drop', {
        objects: views.map((v) => ({ id: v.id, kind: v.kind, x: v.x, y: v.y, w: v.w, h: v.h })),
      });
      this.#commitPlacedViews(target.partId, views, views.map((v) => v.id), 'drop');
    } catch (e) {
      lerror('place', 'drop create failed', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      this.#placing = false;
      if (this.#doc.activeTool !== 'pointer') this.#doc.setTool('pointer');
    }
  }

  async #createFieldObjectsAt(target: FieldPlacementTarget, fieldIds: number[]): Promise<ObjectView[]> {
    const { partId, partHeight, box } = target;
    const rowStep = Math.max(32, box.h + GRID);
    const batches = await Promise.all(
      fieldIds.map((fieldId, i) => {
        const y = Math.min(partHeight - 1, box.y + i * rowStep);
        return createObject(this.#layoutId, {
          partId,
          kind: 'field',
          x: box.x,
          y,
          w: box.w,
          h: Math.min(box.h, Math.max(1, partHeight - y)),
          fieldId,
          createLabel: this.#doc.toolCreateLabel,
          rec: this.#doc.rec,
        });
      }),
    );
    return batches.flat();
  }

  #commitPlacedViews(partId: number, views: ObjectView[], selectedIds: number[], source: string): ObjectView | undefined {
    for (const v of views) this.#doc.addObject(v, partId);
    this.#doc.mark();
    this.#doc.setTool('pointer');
    const placed = views.at(-1);
    if (!placed) return undefined;
    this.#doc.selectOnly(selectedIds);
    this.#hoverId = placed.id;
    llog('place', 'added to store + selected + hover pinned to placed', {
      source,
      selectedIds,
      hoverId: placed.id,
    });
    return placed;
  }

  destroy(): void {
    this.#stage.removeEventListener('pointermove', this.#onPointerMove);
    this.#stage.removeEventListener('pointerleave', this.#onPointerLeave);
    this.#stage.removeEventListener('click', this.#onClick);
    this.#stage.removeEventListener('dblclick', this.#onDoubleClick);
    window.removeEventListener('keydown', this.#onKeyDown);
    window.removeEventListener('pointermove', this.#onDrawMove);
    window.removeEventListener('pointerup', this.#onDrawUp);
    window.removeEventListener('mousemove', this.#onDrawMove);
    window.removeEventListener('mouseup', this.#onDrawUp);
    this.#stage.removeEventListener('dragover', this.#onDragOver);
    this.#stage.removeEventListener('dragleave', this.#onDragLeave);
    this.#stage.removeEventListener('drop', this.#onDrop);
    this.#drawPreview?.remove();
    this.#dropPreview?.remove();
    this.#hoverOutline?.remove();
    this.#textEditorCleanup?.();
    this.#textEditor?.remove();
    if (this.#rectFrame !== null) cancelAnimationFrame(this.#rectFrame);
    this.#moveable.destroy();
    this.#selecto.destroy();
  }

  // ── hover indicator ──

  #onPointerMove = (e: PointerEvent): void => {
    if (this.#gesturing) return;
    const t = e.target as Element | null;
    // Over moveable's own control box → keep the current target (don't flicker).
    if (t && this.#moveable.isMoveableElement(t)) return;
    const objEl = (t?.closest('.fm-obj') ?? null) as HTMLElement | null;
    const id = objEl ? this.#idForElement(objEl) ?? null : null;
    if (id === this.#hoverId) return;
    this.#setHover(id);
  };

  #onPointerLeave = (): void => {
    if (this.#gesturing || this.#hoverId === null) return;
    this.#setHover(null);
  };

  #setHover(id: number | null): void {
    this.#hoverId = id;
    this.#doc.hover(id);
    this.#paintHover();
  }

  #paintHover(): void {
    const id = this.#hoverId;
    const o = id === null ? undefined : this.#doc.getObject(id);
    if (!o || this.#doc.isSelected(o.id) || this.#textEditingId !== null) {
      this.#hoverOutline?.remove();
      this.#hoverOutline = null;
      return;
    }
    const top = this.#partTop(o.partId);
    const overlay = this.#partOverlay();
    if (top === null || !overlay) return;
    if (!this.#hoverOutline) {
      this.#hoverOutline = document.createElement('div');
      this.#hoverOutline.className = 'le-hover-outline';
      overlay.append(this.#hoverOutline);
    }
    this.#hoverOutline.style.left = `${o.x}px`;
    this.#hoverOutline.style.top = `${top + o.y}px`;
    this.#hoverOutline.style.width = `${o.w}px`;
    this.#hoverOutline.style.height = `${o.h}px`;
  }

  #onClick = (e: MouseEvent): void => {
    // Swallow the native click that trails a Selecto commit, so a marquee or
    // modifier-click selection is not immediately cleared by the deselect path.
    if (this.#consumeSuppressedClick()) {
      return;
    }
    if (this.#gesturing || this.#doc.activeTool !== 'pointer') return;
    const target = e.target as Element | null;
    if (!target || this.#moveable.isMoveableElement(target)) return;
    if (target.closest('.fm-obj') || target.closest('.le-part-label, .le-part-resize')) return;

    // A click on band whitespace (or empty canvas) only DESELECTS. Selecting a part
    // is reserved for its label rail (`.le-part-label`, wired in App.svelte), so a
    // stray click in the body never hijacks the selection into part-edit mode.
    this.#hoverId = null;
    this.#paintHover();
    this.#doc.clearSelection();
    this.#updateTarget();
  };

  #swallowNextClick(): void {
    this.#suppressNextClick = true;
    this.#suppressNextClickUntil = performance.now() + 750;
  }

  #consumeSuppressedClick(): boolean {
    if (!this.#suppressNextClick) return false;
    this.#suppressNextClick = false;
    const shouldSuppress = performance.now() <= this.#suppressNextClickUntil;
    this.#suppressNextClickUntil = 0;
    return shouldSuppress;
  }

  #onDoubleClick = (e: MouseEvent): void => {
    if (this.#doc.activeTool !== 'pointer') return;
    const target = e.target as Element | null;
    const objEl = (target?.closest('.fm-obj') ?? null) as HTMLElement | null;
    if (!objEl || this.#moveable.isMoveableElement(objEl)) return;
    const id = this.#idForElement(objEl);
    const o = id === undefined ? undefined : this.#doc.getObject(id);
    if (id === undefined || !o || o.kind !== 'text') return;
    e.preventDefault();
    e.stopPropagation();
    this.#doc.selectOnly([id]);
    this.#updateTarget();
    this.#startTextEdit(id);
  };

  #onKeyDown = (e: KeyboardEvent): void => {
    const target = e.target as HTMLElement | null;
    const inEditable = !!target?.closest('input, textarea, select, [contenteditable="true"]');

    // Cmd/Ctrl+A selects every layout object. Native page/text select-all is
    // never useful in Layout Mode, including while focus is in the inspector.
    if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'a') {
      e.preventDefault();
      this.#selectAllObjects();
      return;
    }

    // Cut / Copy / Paste. Native clipboard wins inside editable fields, so bail on
    // inEditable BEFORE preventDefault (same ordering as undo/redo).
    if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
      const k = e.key.toLowerCase();
      if (k === 'c' || k === 'x' || k === 'v') {
        if (inEditable) return; // let the browser copy/cut/paste text
        if (this.#gesturing || this.#drawing || this.#deleting || this.#placing) return; // never mutate mid-gesture/async
        if (k === 'v') {
          if (!clipboard.hasContent) return; // nothing to paste → let event pass
          e.preventDefault();
          void this.#pasteClipboard();
        } else {
          // 'c' or 'x' need a selection
          if (this.#doc.selection.size === 0) return;
          e.preventDefault();
          if (k === 'c') this.#copySelectionToClipboard();
          else void this.#cutSelectedObjects();
        }
        return;
      }
    }

    // Undo / redo. Cmd/Ctrl+Z = undo; Cmd/Ctrl+Shift+Z or Ctrl+Y = redo.
    if ((e.metaKey || e.ctrlKey) && !e.altKey) {
      const k = e.key.toLowerCase();
      const isUndo = k === 'z' && !e.shiftKey;
      const isRedo = (k === 'z' && e.shiftKey) || k === 'y';
      if (isUndo || isRedo) {
        if (inEditable) return; // let native text-field undo win — return BEFORE preventDefault
        if (this.#gesturing || this.#drawing || this.#deleting || this.#placing) return; // never pop mid-gesture / mid-draw / mid-async
        e.preventDefault();
        if (isRedo) runRedo(this.#doc, this.#layoutId);
        else runUndo(this.#doc, this.#layoutId);
        return;
      }
    }

    // Cmd/Ctrl+D duplicates the current selection (#48). Unlike Delete/Backspace
    // this backs off in an editable field: Ctrl+D is also Cocoa's native
    // "delete forward character" text-editing binding on macOS, and browsers use
    // it for "bookmark this page" — preventDefault only once we're actually
    // going to act, so neither gets clobbered when it wouldn't do anything here.
    if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'd') {
      if (inEditable) return;
      if (this.#doc.selection.size === 0 || this.#placing) return;
      e.preventDefault();
      void this.#duplicateSelectedObjects();
      return;
    }

    if (e.key !== 'Delete' && e.key !== 'Backspace') return;
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (inEditable) return;
    if (this.#doc.selection.size === 0 || this.#deleting) return;
    e.preventDefault();
    void this.#deleteSelectedObjects();
  };

  /** Select all canvas objects (Cmd/Ctrl+A). A no-op while a placement tool is
   * armed — the canvas is a drawing surface then, not a selection surface. Syncs
   * moveable's control box immediately so the group handles appear at once. */
  #selectAllObjects(): void {
    if (this.#doc.activeTool !== 'pointer') return;
    this.#doc.selectAll();
    llog('select', 'select all (keyboard)', { count: this.#doc.selection.size });
    this.#updateTarget();
  }

  async #deleteSelectedObjects(): Promise<void> {
    const ids = [...this.#doc.selection];
    if (ids.length === 0 || this.#deleting) return;
    this.#deleting = true;
    llog('persist', 'delete selected object(s)', { ids });
    try {
      await Promise.all(ids.map((id) => deleteObject(this.#layoutId, id)));
      for (const id of ids) this.#doc.removeObject(id);
      this.#doc.mark();
      this.#setHover(null);
      this.#targetKey = '__force_empty__';
      this.#updateTarget();
    } catch (e) {
      lerror('persist', 'failed to delete selected object(s)', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      this.#deleting = false;
    }
  }

  // ── clipboard: cut / copy / paste (#85) ──

  /** Snapshot the current object selection into the session clipboard. Reads
   *  structural fields from getObject (ObjectDoc) and fieldId from #objectView
   *  (the render-model projection — there is no doc.getResolved). No store
   *  mutation, no persist, not undoable. */
  #copySelectionToClipboard(): boolean {
    const ids = [...this.#doc.selection];
    if (ids.length === 0) return false;

    const objects: ClipboardObject[] = [];
    for (const id of ids) {
      const d = this.#doc.getObject(id); // ObjectDoc: kind/partId/x/y/w/h/z/readOnly/binding/content/props
      if (!d) continue;
      const v = this.#objectView(id); // ObjectView: fieldId
      objects.push({
        kind: d.kind,
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

  /** Cut = copy the selection, then run the existing delete+persist path. One
   *  atomic undo step (the delete's single mark()). */
  async #cutSelectedObjects(): Promise<void> {
    if (this.#deleting) return;
    if (!this.#copySelectionToClipboard()) return; // capture BEFORE removal: after removeObject, fieldId is gone
    await this.#deleteSelectedObjects(); // deleteObject×N → removeObject×N → ONE mark() → detach moveable
  }

  /** Paste every clipboard object as a NEW server object into its source part,
   *  at a cascade offset, preserving relative layout / z / readOnly. All-or-nothing:
   *  either every object is created+added under one mark(), or none is and any
   *  server rows created before a failure are rolled back. */
  async #pasteClipboard(): Promise<void> {
    const payload = clipboard.payload;
    if (!payload || payload.objects.length === 0) return;
    if (this.#placing || this.#deleting || this.#gesturing || this.#drawing) return;

    this.#placing = true;
    const step = clipboard.nextPasteStep(); // 1,2,3…
    const desired = step * GRID; // n * 8px down-right, before in-band capping
    const model = this.#doc.renderModel;

    // Create in ascending source-z order so server insert order tracks original
    // stacking; a later z-normalization pass makes it exact.
    const clips = [...payload.objects].sort((a, b) => a.z - b.z);

    // Resolve each clip's target part (its source part, or the last band if that
    // part is gone), then compute ONE (dx,dy) per target part: every object in a
    // band shifts by the same delta, so relative layout is preserved exactly, and
    // the delta is capped so the group's far edge stays in-band on BOTH axes (a
    // per-object clamp would collapse offsets for objects near an edge).
    const resolved = clips.map((c) => {
      const part = this.#doc.getPart(c.partId) ?? model.parts.at(-1);
      return { c, partId: part?.id ?? c.partId, partH: part?.height ?? Number.MAX_SAFE_INTEGER };
    });
    const ext = new Map<number, { maxX: number; maxY: number; partH: number }>();
    for (const { c, partId, partH } of resolved) {
      const e = ext.get(partId) ?? { maxX: 0, maxY: 0, partH };
      e.maxX = Math.max(e.maxX, c.x + c.w);
      e.maxY = Math.max(e.maxY, c.y + c.h);
      ext.set(partId, e);
    }
    // Floor each capped delta to a whole GRID step: keeps the paste grid-aligned
    // and guarantees the group's far edge never snaps a few px past the in-band cap.
    const capToGrid = (max: number) => Math.max(0, Math.floor(Math.min(desired, max) / GRID) * GRID);
    const offset = new Map<number, { dx: number; dy: number }>();
    for (const [partId, e] of ext) {
      offset.set(partId, {
        dx: capToGrid(model.width - e.maxX),
        dy: capToGrid(e.partH - e.maxY),
      });
    }

    // Build one create request per clip, offset by its part's shared (dx,dy).
    const plans = resolved.map(({ c, partId }) => {
      const { dx, dy } = offset.get(partId) ?? { dx: 0, dy: 0 };
      const x = clampOrigin(snapToGrid(c.x + dx));
      const y = clampOrigin(snapToGrid(c.y + dy));
      const isField = c.kind === 'field';
      const req: NewObjectRequest = {
        partId,
        kind: c.kind,
        x,
        y,
        w: c.w,
        h: c.h,
        rec: this.#doc.rec,
        fieldId: isField ? c.fieldId : null,
        // The binding is what actually recreates the value object: send it so a
        // field whose fieldId is null (unresolved binding / empty table) still
        // pastes instead of 400ing, and so the copy keeps its exact binding.
        binding: isField ? c.binding : null,
        createLabel: isField ? false : undefined, // NEVER auto-spawn a caption
        content: c.kind === 'text' ? c.content : null,
        props: c.props ? parseProps(c.props) : null, // string → object for the wire
      };
      return { req, partId, clip: c };
    });

    const created: { view: ObjectView; partId: number; clip: ClipboardObject }[] = [];
    const landedIds: number[] = []; // every server row created this paste — for rollback
    let committed = false; // true once addObject + mark() fold the paste into store/undo
    try {
      // 1. Persist all (fresh ids). allSettled so a partial failure rolls back cleanly.
      const results = await Promise.allSettled(plans.map((p) => createObject(this.#layoutId, p.req)));
      for (const r of results) if (r.status === 'fulfilled') for (const v of r.value) landedIds.push(v.id);

      // 2. Any create failed → nothing enters the store/undo; the catch deletes
      //    whatever DID land so the server never keeps phantom rows.
      if (results.some((r) => r.status === 'rejected')) {
        const firstRej = results.find((r) => r.status === 'rejected') as PromiseRejectedResult | undefined;
        throw firstRej?.reason ?? new Error('paste failed');
      }

      // 3. All succeeded. Collect views (each field is length-1: createLabel:false).
      results.forEach((r, i) => {
        for (const v of (r as PromiseFulfilledResult<ObjectView[]>).value) {
          created.push({ view: v, partId: plans[i].partId, clip: plans[i].clip });
        }
      });

      // 4. z / readOnly fidelity: place the pasted group ON TOP of each target part
      //    in preserved relative order, and restore readOnly. Persist first, then
      //    mirror into the doc so store + server agree; all diffs fold into ONE step.
      //    A failure here also unwinds through the catch (rows created in step 1 are
      //    rolled back), so a network drop mid-paste never leaves phantom rows.
      await this.#applyPasteZAndReadOnly(created);

      // 5. Add every view (undoable life diffs) THEN a single mark() = one undo step.
      for (const { view, partId } of created) this.#doc.addObject(view, partId);
      for (const { view, clip } of created) {
        // mirror z/readOnly into the doc
        if (clip._targetZ !== undefined && view.z !== clip._targetZ) this.#doc.setProp(view.id, 'z', clip._targetZ);
        if (view.readOnly !== clip.readOnly) this.#doc.setProp(view.id, 'readOnly', clip.readOnly);
      }
      this.#doc.mark(); // ← ONE atomic undo step
      committed = true; // past here the paste lives in store + undo; never roll back

      // 6. Force pointer mode (a still-armed draw tool clears moveable's target), then
      //    select the pasted objects; rely on the reactive moveable sync (like #finishDraw).
      this.#doc.setTool('pointer');
      const newIds = created.map((c) => c.view.id);
      this.#doc.selectOnly(newIds);
      this.#hoverId = newIds.at(-1) ?? null; // pin hover so #updateTarget resolves right
      llog('clipboard', 'pasted objects', { count: newIds.length, step, newIds });
    } catch (e) {
      // Roll back any rows that landed before the paste committed, so store and
      // server never diverge (no phantom rows surfacing on the next reload).
      if (!committed && landedIds.length) {
        await Promise.allSettled(landedIds.map((id) => deleteObject(this.#layoutId, id)));
      }
      lerror('clipboard', 'failed to paste', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      this.#placing = false;
    }
  }

  /** Duplicate the current selection (#48), offset by one placement step so the
   * copies land visibly next to the originals rather than exactly on top of
   * them, then select the copies (not the originals) — the usual "duplicate
   * leaves you holding the new one" convention. One create call per selected
   * object, each an exact clone of its geometry/binding/content/appearance;
   * `createLabel: false` throughout so duplicating a field VALUE never conjures
   * an extra label that wasn't already there — duplicate what's selected, not
   * more. NOTE: doesn't carry over a per-object read-only flag (there's no slot
   * for it in the create request); low-priority gap since nothing in the editor
   * exposes setting that flag yet (see field-editability-in-layout-mode). */
  async #duplicateSelectedObjects(): Promise<void> {
    const ids = new Set(this.#doc.selection);
    if (ids.size === 0 || this.#placing) return;
    // ObjectView (unlike the store's own ObjectDoc) carries the resolved
    // fieldId a duplicate needs, but not which part it's in — that's implicit
    // in which PartView it's nested under in renderModel, so pair the two here.
    const found: { partId: number; view: ObjectView }[] = [];
    for (const part of this.#doc.renderModel.parts) {
      for (const view of part.objects) {
        if (ids.has(view.id)) found.push({ partId: part.id, view });
      }
    }
    if (found.length === 0) return;
    this.#placing = true;
    const offset = GRID * 2;
    try {
      const created = await Promise.all(
        found.map(async ({ partId, view }) => {
          const views = await createObject(this.#layoutId, {
            partId,
            kind: view.kind,
            x: clampOrigin(snapToGrid(view.x + offset)),
            y: clampOrigin(snapToGrid(view.y + offset)),
            w: view.w,
            h: view.h,
            rec: this.#doc.rec,
            fieldId: view.fieldId,
            // Carry the source binding so a field whose fieldId is null (empty
            // table, or an unresolved relationship path) still duplicates — the
            // server recreates the value from the binding when fieldId is absent.
            binding: view.kind === 'field' ? view.binding : null,
            createLabel: false,
            content: view.kind === 'text' ? view.content : null,
            props: view.props ? parseProps(view.props) : null,
          });
          return { partId, views };
        }),
      );
      llog('create', 'duplicated object(s)', {
        from: [...ids],
        created: created.flatMap((c) => c.views.map((v) => v.id)),
      });
      for (const { partId, views } of created) {
        for (const v of views) this.#doc.addObject(v, partId);
      }
      this.#doc.mark();
      const newIds = created.flatMap((c) => c.views.map((v) => v.id));
      this.#doc.selectOnly(newIds);
      const placed = newIds.at(-1);
      if (placed !== undefined) this.#hoverId = placed;
    } catch (e) {
      lerror('persist', 'failed to duplicate selected object(s)', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      this.#placing = false;
    }
  }

  /** Assign each pasted object a z that stacks the whole paste group on top of
   *  its target part in preserved relative order, and persist z + readOnly. Stamps
   *  the chosen z onto each clip as `_targetZ` for the doc-mirror step. */
  async #applyPasteZAndReadOnly(
    created: { view: ObjectView; partId: number; clip: ClipboardObject }[],
  ): Promise<void> {
    // Next free z per target part = 1 + max z currently in that part.
    const nextZ = new Map<number, number>();
    const model = this.#doc.renderModel;
    for (const p of model.parts) {
      const maxZ = p.objects.reduce((m, o) => Math.max(m, o.z), -1);
      nextZ.set(p.id, maxZ + 1);
    }
    const zItems: { id: number; z: number }[] = [];
    const roItems: { id: number; readOnly: boolean }[] = [];
    // created is already in ascending source-z order (creation order), so ranking
    // by iteration preserves the group's internal stacking.
    for (const c of created) {
      const z = nextZ.get(c.partId) ?? c.view.z;
      nextZ.set(c.partId, z + 1);
      c.clip._targetZ = z;
      if (z !== c.view.z) zItems.push({ id: c.view.id, z });
      if (c.clip.readOnly !== c.view.readOnly) roItems.push({ id: c.view.id, readOnly: c.clip.readOnly });
    }
    if (zItems.length) await setObjectsZ(this.#layoutId, zItems);
    for (const r of roItems) await setObjectReadOnly(this.#layoutId, r.id, r.readOnly, this.#doc.rec);
  }

  // ── public clipboard surface (for a future menu / rail; zero-refactor add) ──

  copy(): void {
    this.#copySelectionToClipboard();
  }
  cut(): void {
    void this.#cutSelectedObjects();
  }
  paste(): void {
    void this.#pasteClipboard();
  }
  canPaste(): boolean {
    return clipboard.hasContent;
  }

  /** Choose moveable's target from the real selection only. Hover uses a separate
   * lightweight outline, so resize handles never appear on unselected objects. */
  #updateTarget(): void {
    if (this.#gesturing) return;
    this.#gestureIdentity = null;
    // A placement tool is armed → the canvas is a drawing surface, not a select/
    // drag surface: drop moveable's target so a press places instead of grabs.
    if (this.#doc.activeTool !== 'pointer') {
      if (this.#targetKey === '') return;
      this.#targetKey = '';
      this.#targetIds = new Set();
      this.#moveable.setState({ target: null, elementGuidelines: [], rotatable: false }, () => this.#moveable.forceUpdate());
      llog('target', 'tool armed → moveable target cleared');
      return;
    }
    const sel = [...this.#doc.selection];
    const ids = sel.length > 0 ? sel : [];
    const key = ids.slice().sort((a, b) => a - b).join(',');
    if (key === this.#targetKey) return;
    this.#targetKey = key;
    this.#targetIds = new Set(ids);
    if (ids.length === 0) {
      this.#moveable.setState({ target: null, elementGuidelines: [], rotatable: false }, () => this.#moveable.forceUpdate());
      llog('target', 'moveable target cleared', {
        hoverId: this.#hoverId,
        selection: sel,
        paintedCount: this.#paintedElements().length,
      });
      this.#paintHover();
      return;
    }
    const targets = ids.map((id) => this.#elementForId(id)).filter((el): el is HTMLElement => !!el);
    const guidelines = this.#paintedElements().filter((el) => !targets.includes(el));
    this.#moveable.setState({ target: targets, elementGuidelines: guidelines, rotatable: this.#canRotate(ids) });
    // THE key line for "resize does nothing": if `chosenIds` has an id but
    // `resolvedEls` is fewer, moveable has no element to attach handles to — the
    // store id didn't map to a painted `.fm-obj` (stale paint order / DOM not yet
    // committed after a create).
    llog('target', 'moveable target set', {
      hoverId: this.#hoverId,
      selection: sel,
      chosenIds: ids,
      resolvedEls: targets.length,
      paintedCount: this.#paintedElements().length,
      paintOrderIds: objectIdsInPaintOrder(this.#doc.renderModel),
    });
    this.#paintHover();
  }

  #startTextEdit(id: number): void {
    const o = this.#doc.getObject(id);
    const overlay = this.#partOverlay();
    if (!o || this.#partTop(o.partId) === null || !overlay) return;
    this.#textEditor?.remove();
    this.#textEditingId = id;
    this.#paintHover();
    const editor = document.createElement('textarea');
    editor.className = 'le-inline-text-editor';
    editor.value = o.content;
    overlay.append(editor);
    this.#textEditor = editor;
    // Match the object's resolved text style (size / weight / italic / underline /
    // colour / align) so the editor LOOKS like the text it edits (#5). Kept in
    // sync live via `syncOpenTextEditor()` while the inspector is used.
    this.#applyEditorTextStyle(editor, o);

    const finish = (commit: boolean) => {
      if (this.#textEditor !== editor) return;
      const next = editor.value;
      document.removeEventListener('pointerdown', onOutsidePointerDown, true);
      editor.remove();
      this.#textEditor = null;
      this.#textEditingId = null;
      this.#textEditorCleanup = null;
      if (commit && next !== o.content) void this.#commitTextEdit(id, next);
      this.#paintHover();
    };
    // A press inside the inspector must keep the editor open even though the
    // textarea blurs. `relatedTarget` alone is unreliable: WebKit (this app's
    // Linux webview) and Safari/Firefox do NOT focus <button>s on click, so
    // toggling B/I/U would blur to `null`. So a press inside the inspector arms a
    // one-shot guard the imminent blur reads.
    let guardBlur = false;
    // A press that lands anywhere OTHER than the editor or the inspector commits
    // and closes the editor. Pressing the inspector must NOT close it, so its size
    // / style controls can be adjusted mid-edit and reflected live (#5).
    const onOutsidePointerDown = (ev: Event) => {
      const t = ev.target as Node | null;
      if (t && (editor.contains(t) || this.#inspectorEl()?.contains(t))) {
        guardBlur = true;
        setTimeout(() => {
          guardBlur = false;
        }, 0);
        return;
      }
      finish(true);
    };
    // Focus leaving the textarea commits — UNLESS it moved INTO the inspector (or a
    // just-pressed inspector control that didn't take focus), so inspector edits
    // restyle the editor live (#5).
    editor.addEventListener('blur', (e) => {
      const next = e.relatedTarget as Node | null;
      if (guardBlur || (next && this.#inspectorEl()?.contains(next))) return;
      finish(true);
    });
    editor.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        finish(false);
      } else if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        finish(true);
      }
    });
    document.addEventListener('pointerdown', onOutsidePointerDown, true);
    this.#textEditorCleanup = () => document.removeEventListener('pointerdown', onOutsidePointerDown, true);
    editor.focus();
    editor.select();
  }

  /** Re-apply the editing object's server-derived text style to the open inline
   * editor, so inspector size/style changes appear LIVE without closing it (#5).
   * No-op when no text editor is open. Called reactively from App.svelte when the
   * render model (and thus the object's `textStyle`) changes. */
  syncOpenTextEditor(): void {
    const editor = this.#textEditor;
    const id = this.#textEditingId;
    if (!editor || id === null) return;
    const o = this.#doc.getObject(id);
    if (o) this.#applyEditorTextStyle(editor, o);
  }

  /** Copy the object's resolved `textStyle` (the same CSS the server derives and
   * the canvas renders with) onto the inline editor, then re-assert the editor's
   * box. `cssText` clears prior inline styles, so left/top/width/height are set
   * AFTER it; position/border/z-index/background come from the class. */
  #applyEditorTextStyle(editor: HTMLTextAreaElement, o: Readonly<ObjectDoc>): void {
    const top = this.#partTop(o.partId) ?? 0;
    editor.style.cssText = this.#objectView(o.id)?.textStyle ?? '';
    editor.style.left = `${o.x}px`;
    editor.style.top = `${top + o.y}px`;
    editor.style.width = `${o.w}px`;
    editor.style.height = `${o.h}px`;
  }

  /** The current render-model view of one object (carries the server-derived
   * `textStyle`/styles the document `ObjectDoc` doesn't). */
  #objectView(id: number): ObjectView | undefined {
    for (const p of this.#doc.renderModel.parts) {
      const v = p.objects.find((obj) => obj.id === id);
      if (v) return v;
    }
    return undefined;
  }

  #inspectorEl(): HTMLElement | null {
    return document.getElementById('layout-inspector');
  }

  async #commitTextEdit(id: number, content: string): Promise<void> {
    llog('persist', 'inline text edit', { id });
    this.#doc.setProp(id, 'content', content);
    this.#doc.mark();
    try {
      const view = await setObjectContent(this.#layoutId, id, content);
      this.#doc.setProp(id, 'content', view.content);
    } catch (e) {
      lerror('persist', 'inline text edit failed', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    }
  }

  // ── gesture lifecycle ──

  #begin(): void {
    this.#gesturing = true;
    this.#moved = false;
    this.#resizeStarts.clear();
    this.#gestureIdentity = this.#identitySnapshot();
  }

  /** End a gesture: if it actually changed geometry, seal one undo step and
   * persist the moved/resized group; a no-move click does neither. Then re-target. */
  #end(kind: 'drag' | 'resize' = 'drag'): void {
    this.#gesturing = false;
    // A drag may have carried objects across band boundaries; settle them onto a
    // real band (reparenting) BEFORE the undo mark so it's one step. Resize never
    // crosses bands, so it skips this.
    const reparented = kind === 'drag' && this.#moved ? this.#settleBands() : new Set<number>();
    llog(kind, `${kind}End`, { moved: this.#moved, selection: [...this.#doc.selection], reparented: [...reparented] });
    if (this.#moved) {
      this.#doc.mark();
      void this.#persistSelection(reparented);
      void this.#persistDirtyLineProps();
    }
    this.#targetKey = ''; // force a re-sync after the gesture
    this.#resizeStarts.clear();
    this.#gestureIdentity = null;
    this.#scheduleRectUpdate();
    this.#updateTarget();
    // A reparent moves the object to a DIFFERENT band's keyed-each, so Svelte
    // destroys its old DOM node and creates a new one — changing paint order. The
    // id→element map is stale until that re-render commits, so the sync above can
    // target the wrong element. Re-target after the DOM flush (id-keyed dedupe
    // cleared) so moveable's handles follow the MOVED object, not its old index.
    if (reparented.size > 0) {
      requestAnimationFrame(() => {
        this.#targetKey = '';
        this.#updateTarget();
      });
    }
  }

  /** Make the dragged/resized single target the selection (if it wasn't already). */
  #selectFromTarget(el: HTMLElement | SVGElement): void {
    const id = this.#idForElement(el);
    if (id !== undefined && !this.#doc.isSelected(id)) this.#doc.selectOnly([id]);
  }

  #applyMove(target: HTMLElement | SVGElement, left: number, top: number): void {
    const identity = this.#currentIdentity();
    const id = this.#idForElement(target, identity);
    if (id === undefined) {
      llog('target', 'drag: target element has NO mapped id — move is a no-op', {
        painted: identity.painted.length,
      });
      return;
    }
    this.#moved = true;
    // y is left UNCLAMPED during a drag so the object can travel above its own band
    // (a negative part-relative y renders over the band above) — cross-band drags
    // are settled to a real band + local y on drop (#settleBands). x stays ≥ 0.
    this.#doc.setObjectGeometry(id, { x: clampOrigin(left), y: Math.round(top) });
    this.#scheduleRectUpdate();
    llog('drag', 'apply move', { id, x: clampOrigin(left), y: Math.round(top) });
  }

  /** Settle every moved object onto a real band after a drag: read its absolute
   * canvas-y (its band's top + part-relative y), find the band that y lands in, and
   * rewrite the object to that band with a clamped local y. Objects that crossed a
   * boundary are reparented (partId change); the returned set drives which ones
   * persist via the reparent endpoint vs the bulk geometry commit. */
  #settleBands(): Set<number> {
    const reparented = new Set<number>();
    const model = this.#doc.renderModel;
    const totalHeight = model.parts.reduce((sum, p) => sum + p.height, 0);
    if (totalHeight <= 0) return reparented;
    for (const id of this.#doc.selection) {
      const o = this.#doc.getObject(id);
      if (!o) continue;
      const curTop = this.#partTop(o.partId);
      if (curTop === null) continue;
      const absY = Math.min(totalHeight - 1, Math.max(0, curTop + o.y));
      const where = partAtY(model, absY);
      if (!where) continue;
      const x = clampOrigin(o.x);
      const y = clampOrigin(where.localY);
      if (where.partId !== o.partId) {
        this.#doc.setProp(id, 'partId', where.partId);
        this.#doc.setObjectGeometry(id, { x, y });
        reparented.add(id);
        llog('drag', 'settle: reparent object to band', { id, partId: where.partId, x, y });
      } else if (x !== o.x || y !== o.y) {
        this.#doc.setObjectGeometry(id, { x, y });
      }
    }
    return reparented;
  }

  #captureResizeStart(target: HTMLElement | SVGElement, direction: number[], inputEvent: Event | undefined): void {
    const identity = this.#currentIdentity();
    const id = this.#idForElement(target, identity);
    const o = id === undefined ? undefined : this.#doc.getObject(id);
    const pointer = inputEvent as PointerEvent | MouseEvent | undefined;
    if (id === undefined || !o || !pointer) return;
    this.#resizeStarts.set(id, {
      x: o.x,
      y: o.y,
      w: o.w,
      h: o.h,
      direction: direction.slice(),
      clientX: pointer.clientX,
      clientY: pointer.clientY,
    });
  }

  #applyResize(
    target: HTMLElement | SVGElement,
    width: number,
    height: number,
    left: number,
    top: number,
    inputEvent?: Event,
  ): void {
    const identity = this.#currentIdentity();
    const id = this.#idForElement(target, identity);
    if (id === undefined) {
      llog('target', 'resize: target element has NO mapped id — resize is a no-op', {
        painted: identity.painted.length,
        paintOrderIds: identity.ids,
      });
      return;
    }
    this.#moved = true;
    const pointer = inputEvent as PointerEvent | MouseEvent | undefined;
    const start = this.#resizeStarts.get(id);
    if (pointer && start) {
      const dx = (pointer.clientX - start.clientX) / (this.#zoom || 1);
      const dy = (pointer.clientY - start.clientY) / (this.#zoom || 1);
      const dirX = Math.sign(start.direction[0] ?? 1);
      const dirY = Math.sign(start.direction[1] ?? 1);
      let x = start.x;
      let y = start.y;
      let w = start.w;
      let h = start.h;
      if (dirX >= 0) {
        w = snapToGrid(start.w + dx);
      } else {
        x = snapToGrid(start.x + dx);
        w = start.w - (x - start.x);
      }
      if (dirY >= 0) {
        h = snapToGrid(start.h + dy);
      } else {
        y = snapToGrid(start.y + dy);
        h = start.h - (y - start.y);
      }
      w = Math.max(1, Math.round(w));
      h = Math.max(1, Math.round(h));
      x = clampOrigin(x);
      y = clampOrigin(y);
      this.#doc.setObjectGeometry(id, { x, y, w, h });
      this.#syncLineToBox(id);
      this.#scheduleRectUpdate();
      llog('resize', 'apply resize from pointer', { id, w, h, x, y, dx: Math.round(dx), dy: Math.round(dy) });
      return;
    }
    this.#doc.setObjectGeometry(id, {
      x: clampOrigin(left),
      y: clampOrigin(top),
      w: Math.max(1, Math.round(width)),
      h: Math.max(1, Math.round(height)),
    });
    this.#syncLineToBox(id);
    this.#scheduleRectUpdate();
    llog('resize', 'apply resize', {
      id,
      w: Math.max(1, Math.round(width)),
      h: Math.max(1, Math.round(height)),
      x: clampOrigin(left),
      y: clampOrigin(top),
    });
  }

  #applyLineRotate(angle: number): void {
    const id = this.#rotatingLineId;
    const o = id === null ? undefined : this.#doc.getObject(id);
    if (id === null || !o || o.kind !== 'line') return;
    const nextAngle = normalizeAngle(angle);
    const length = this.#rotateStartLength || lineLength(o, this.#propsForObject(o));
    const geom = lineGeometryForAngle(o, nextAngle, length);
    const props = { ...this.#propsForObject(o), angle: nextAngle, length };
    const propsJson = JSON.stringify(props);
    this.#moved = true;
    this.#doc.setObjectGeometry(id, { x: clampOrigin(geom.x), y: clampOrigin(geom.y), w: geom.w, h: geom.h });
    this.#doc.setObjectProps(id, propsJson);
    this.#setLineShapeStyle(id, props);
    this.#dirtyLineProps.add(id);
    this.#scheduleRectUpdate();
  }

  #endLineRotate(): void {
    const id = this.#rotatingLineId;
    const moved = this.#moved;
    this.#gesturing = false;
    this.#rotatingLineId = null;
    this.#rotateStartLength = 0;
    this.#gestureIdentity = null;
    if (id !== null && moved) {
      this.#doc.mark();
      void this.#persistSelection();
      void this.#persistDirtyLineProps();
    }
    this.#targetKey = '';
    this.#scheduleRectUpdate();
    this.#updateTarget();
  }

  async #persistLineProps(id: number): Promise<void> {
    const o = this.#doc.getObject(id);
    if (!o) return;
    const props = this.#propsForObject(o);
    try {
      const styles = await persistObjectProps(this.#layoutId, id, props);
      this.#doc.setObjectStyles(id, styles);
    } catch (e) {
      lerror('persist', 'failed to persist line rotation', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    }
  }

  async #persistDirtyLineProps(): Promise<void> {
    const ids = [...this.#dirtyLineProps];
    this.#dirtyLineProps.clear();
    await Promise.all(ids.map((id) => this.#persistLineProps(id)));
  }

  #placementProps(tool: DrawTool, drawing: DrawPlacement, box: { w: number }): Record<string, unknown> | null {
    const base = defaultProps(tool);
    if (tool !== 'line') return base ?? null;
    const line = drawing.dragged && drawing.line ? drawing.line : { angle: 0, length: Math.max(1, box.w) };
    return { ...(base ?? {}), angle: line.angle, length: line.length };
  }

  #canRotate(ids: number[]): boolean {
    if (ids.length !== 1) return false;
    return this.#doc.getObject(ids[0])?.kind === 'line';
  }

  #propsForObject(o: Readonly<ObjectDoc>): Record<string, unknown> {
    return parseProps(o.props);
  }

  #setLineShapeStyle(id: number, props: Record<string, unknown>): void {
    const view = this.#objectView(id);
    if (!view) return;
    this.#doc.setObjectStyles(id, {
      objectStyle: view.objectStyle,
      textStyle: view.textStyle,
      shapeStyle: lineShapeStyle(props),
    });
  }

  #syncLineToBox(id: number): void {
    const o = this.#doc.getObject(id);
    if (!o || o.kind !== 'line') return;
    const props = this.#propsForObject(o);
    const currentAngle = numberProp(props.angle, 0);
    const radians = (currentAngle * Math.PI) / 180;
    const horizontalish = currentAngle <= 5 || currentAngle >= 355 || Math.abs(currentAngle - 180) <= 5;
    const verticalish = Math.abs(currentAngle - 90) <= 5 || Math.abs(currentAngle - 270) <= 5;
    const w = Math.max(1, o.w);
    const h = horizontalish && o.h <= 2 ? 0 : Math.max(1, o.h);
    const dx = (Math.cos(radians) < 0 ? -1 : 1) * (verticalish && o.w <= 2 ? 0 : w);
    const dy = (Math.sin(radians) < 0 ? -1 : 1) * h;
    const next = {
      ...props,
      angle: lineAngle(0, 0, dx, dy),
      length: Math.max(1, Math.hypot(dx, dy)),
    };
    this.#doc.setObjectProps(id, JSON.stringify(next));
    this.#setLineShapeStyle(id, next);
    this.#dirtyLineProps.add(id);
  }

  // ── persistence (#46 bulk axum contract) ──

  async #persistSelection(reparented: Set<number> = new Set()): Promise<void> {
    const objs = [...this.#doc.selection]
      .map((id) => this.#doc.getObject(id))
      .filter((o): o is NonNullable<typeof o> => !!o);
    if (objs.length === 0) return;
    // Objects that crossed a band boundary persist their new membership (partId +
    // origin) via the reparent endpoint; the rest commit geometry in bulk as before.
    const geom = objs.filter((o) => !reparented.has(o.id)).map((o) => ({ id: o.id, x: o.x, y: o.y, w: o.w, h: o.h }));
    const moved = objs.filter((o) => reparented.has(o.id));
    llog('persist', 'POST geometry', { geometry: geom, reparent: moved.map((o) => ({ id: o.id, partId: o.partId })) });
    try {
      const posts: Promise<unknown>[] = [];
      if (geom.length > 0) {
        posts.push(
          fetch(`/design/${this.#layoutId}/geometry`, {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify(geom),
          }).then((r) => {
            if (!r.ok) throw new Error(`HTTP ${r.status}`);
          }),
        );
      }
      for (const o of moved) posts.push(setObjectPart(this.#layoutId, o.id, o.partId, o.x, o.y));
      await Promise.all(posts);
      llog('persist', 'geometry saved', { geometry: geom.length, reparented: moved.length });
    } catch (e) {
      // The store already reflects the edit; surface the persist failure rather
      // than tearing down the in-memory state (a reload would reveal divergence).
      lerror('persist', 'failed to persist object geometry', e);
    }
  }

  // ── id ↔ element mapping (paint-order index; see canvas-edit.ts) ──

  #canvas(): HTMLElement | null {
    return this.#stage.querySelector('.fm-canvas');
  }

  #partOverlay(): HTMLElement | null {
    return this.#stage.querySelector('.le-part-overlays');
  }

  #canvasPoint(clientX: number, clientY: number): { x: number; y: number } | null {
    const canvas = this.#canvas();
    if (!canvas) return null;
    const r = canvas.getBoundingClientRect();
    const z = this.#zoom || 1;
    return {
      x: Math.max(0, (clientX - r.left) / z - canvas.clientLeft),
      y: Math.max(0, (clientY - r.top) / z - canvas.clientTop),
    };
  }

  #pointInCanvas(clientX: number, clientY: number): boolean {
    const canvas = this.#canvas();
    if (!canvas) return false;
    const r = canvas.getBoundingClientRect();
    return clientX >= r.left && clientX <= r.right && clientY >= r.top && clientY <= r.bottom;
  }

  #paintedElements(): HTMLElement[] {
    const canvas = this.#canvas();
    return canvas ? Array.from(canvas.querySelectorAll<HTMLElement>('.fm-obj')) : [];
  }

  #identitySnapshot(): IdentitySnapshot {
    return {
      painted: this.#paintedElements(),
      ids: objectIdsInPaintOrder(this.#doc.renderModel),
    };
  }

  #currentIdentity(): IdentitySnapshot {
    return this.#gestureIdentity ?? this.#identitySnapshot();
  }

  /** Hit-test a client point through the FULL element stack (not just the
   * topmost element), so a `.fm-obj` underneath one of moveable's own overlay
   * proxies (e.g. the group `moveable-area` drag-proxy) can still be found. */
  #objectElementAt(clientX: number, clientY: number): HTMLElement | null {
    for (const el of document.elementsFromPoint(clientX, clientY)) {
      const objEl = el.closest('.fm-obj');
      if (objEl) return objEl as HTMLElement;
    }
    return null;
  }

  #elementsToIds(elements: Array<HTMLElement | SVGElement>): number[] {
    const identity = this.#currentIdentity();
    return elementsToObjectIds(elements, identity.painted, identity.ids);
  }

  #elementForId(id: number): HTMLElement | undefined {
    const identity = this.#currentIdentity();
    const i = identity.ids.indexOf(id);
    return i >= 0 ? identity.painted[i] : undefined;
  }

  #idForElement(el: Element, identity: IdentitySnapshot = this.#currentIdentity()): number | undefined {
    const i = identity.painted.indexOf(el as HTMLElement);
    if (i < 0) return undefined;
    return identity.ids[i];
  }

  #scheduleRectUpdate(): void {
    if (this.#rectFrame !== null) return;
    this.#rectFrame = requestAnimationFrame(() => {
      this.#rectFrame = null;
      this.#moveable.updateRect();
      this.#paintHover();
    });
  }

  #partTop(partId: number): number | null {
    let top = 0;
    for (const part of this.#doc.renderModel.parts) {
      if (part.id === partId) return top;
      top += part.height;
    }
    return null;
  }
}
