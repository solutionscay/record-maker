// Transform controller (#46, split in #135): binds moveable (drag / resize /
// snap + alignment guidelines / group transform / line rotate) and selecto
// (marquee multi-select) to the editor document store (#45), and owns moveable's
// target sync with the store selection. The vanilla cores are consumed directly.
//
// Single source of truth is the store. During a drag, compositor-only transforms
// immediately paint the latest snapped pointer geometry; pointer-up then replaces
// that temporary feedback with authored left/top in one store commit.
// moveable derives its control box from the gesture's cached start rect + pointer
// delta, so routing output back into the store does NOT create a feedback loop. A
// whole gesture commits as ONE undo step (mark on end) and persists to the engine
// via the bulk axum contract.

import Moveable from 'moveable';
import Selecto from 'selecto';

import type { ObjectDoc } from '../doc.svelte';
import { SNAP_THRESHOLD, clampOrigin, objectIdsInPaintOrder, snapToGrid } from '../canvas-edit';
import { partAtY } from '../create';
import {
  setObjectPart,
  setObjectProps as persistObjectProps,
  setObjectsGeometry,
} from '../persist';
import { llog, lerror } from '../log';
import { lineGeometryForAngle, lineLength, linePropsForBox, lineShapeStyle, normalizeAngle, numberProp, parseProps } from '../object-props';
import type { CanvasContext } from './context';

type PendingObjectClick = { id: number; clientX: number; clientY: number };

export class TransformController {
  readonly #ctx: CanvasContext;
  readonly #moveable: Moveable;
  readonly #selecto: Selecto;

  /** Whether the active gesture actually moved/resized something — gates mark +
   * persist so a plain click (select, no movement) doesn't POST or push undo. */
  #moved = false;
  #rectFrame: number | null = null;
  /** Latest pointer geometry awaiting the next display frame. Moveable batches
   * its own control-box paint to frames; matching that cadence prevents the
   * Svelte object from advancing one or more raw pointer events ahead (#194). */
  #moveFrame: number | null = null;
  #pendingMoves = new Map<
    number,
    { target: HTMLElement | SVGElement; next: { x: number; y: number } }
  >();
  /** Original inline compositor properties for objects receiving immediate drag
   * feedback. They remain promoted for the gesture and are restored on end. */
  #moveFeedback = new Map<
    HTMLElement | SVGElement,
    { transform: string; willChange: string }
  >();
  /** Raw pointer origin and group anchor for the compositor feedback path. */
  #dragPointer: {
    pointerId: number;
    clientX: number;
    clientY: number;
    anchorId: number;
  } | null = null;
  #pressedPointer: { pointerId: number; clientX: number; clientY: number } | null = null;
  /** Object ids moveable currently targets, and a cheap key to dedupe setState. */
  #targetIds = new Set<number>();
  #targetKey = '';
  /** One-shot: swallow the native `click` the browser fires right after Selecto
   * commits selection. Without it, `onClick` can run its empty-canvas deselect
   * path and wipe the marquee or modifier-click selection that just landed. A
   * bare empty-canvas click does NOT set this, so it still deselects as before.
   * 0 = disarmed; otherwise a `performance.now()` deadline the click must beat. */
  #suppressClickUntil = 0;
  /** Geometry at pointer-down. Group dragging snaps one anchor and applies its
   * delta to every member, preserving authored offsets between group members. */
  #dragStarts = new Map<number, { x: number; y: number }>();
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
  #pendingObjectClick: PendingObjectClick | null = null;

  constructor(ctx: CanvasContext) {
    this.#ctx = ctx;
    const stage = ctx.stage;

    // Register before Moveable so compositor feedback is authored at the entry
    // to each pointer sample, before the library's drag calculations.
    window.addEventListener('pointerdown', this.#onPointerDown, { capture: true });
    window.addEventListener('pointermove', this.#onDragPointerMove, { capture: true });
    this.#moveable = new Moveable(stage, {
      target: [],
      draggable: true,
      resizable: true,
      rotatable: false,
      snappable: true,
      snapGridWidth: ctx.doc.gridSize,
      snapGridHeight: ctx.doc.gridSize,
      snapThreshold: SNAP_THRESHOLD,
      isDisplaySnapDigit: false,
      elementGuidelines: [],
      origin: false,
      zoom: ctx.zoom,
    });

    // ── drag (single + group). Single-target start also makes it the selection. ──
    this.#moveable.on('dragStart', (e) => {
      const id = this.#ctx.idForElement(e.target);
      llog('drag', 'dragStart', { id });
      this.#begin();
      this.#selectFromTarget(e.target);
      this.#captureDragStarts();
      if (id !== undefined) this.#startDragPointerFeedback(id, e.inputEvent);
    });
    this.#moveable.on('drag', (e) => {
      if (this.#dragPointer) {
        this.#applyMove(e.target, e.left, e.top, false);
        queueMicrotask(() => this.#syncBoundsToMoveFeedback());
      }
      else this.#applyMove(e.target, e.left, e.top);
    });
    this.#moveable.on('dragEnd', () => this.#end('drag'));
    this.#moveable.on('dragGroupStart', (e) => {
      this.#begin();
      this.#captureDragStarts();
      const anchorId = this.#dragStarts.keys().next().value;
      if (anchorId !== undefined) this.#startDragPointerFeedback(anchorId, e.inputEvent);
    });
    this.#moveable.on('dragGroup', (e) => {
      if (this.#dragPointer) {
        this.#applyGroupMove(e.events, false);
        queueMicrotask(() => this.#syncBoundsToMoveFeedback());
      }
      else this.#applyGroupMove(e.events);
    });
    this.#moveable.on('dragGroupEnd', () => this.#end('drag'));

    // ── resize (single + group) — e.drag carries the new left/top for top/left handles ──
    this.#moveable.on('resizeStart', (e) => {
      llog('resize', 'resizeStart', { id: this.#ctx.idForElement(e.target) });
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
      const id = this.#ctx.idForElement(e.target);
      const o = id === undefined ? undefined : this.#ctx.doc.getObject(id);
      if (id === undefined || !o || o.kind !== 'line') return false;
      this.#begin();
      this.#selectFromTarget(e.target);
      const props = parseProps(o.props);
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
      clickBySelectEnd: true,
      selectFromInside: true,
      hitRate: 0,
      toggleContinueSelect: 'shift',
    });
    // A marquee over empty canvas live-updates the store selection.
    this.#selecto.on('select', (e) => this.#ctx.doc.selectOnly(this.#ctx.elementsToIds(e.selected)));
    // When the marquee ends, pin the FINAL selection and attach moveable to the
    // group SYNCHRONOUSLY. The reactive refresh (App.svelte $effect) is async, so
    // relying on it alone can leave `#targetIds` stale at the instant the user
    // presses to drag the group — the press would then re-select a single object
    // instead of grabbing the marqueed set. Running `updateTarget()` here makes
    // the control box appear and populates `#targetIds` before the next pointer
    // stream, so a press on any selected object drags the whole group.
    this.#selecto.on('selectEnd', (e) => {
      const selectedIds = this.#ctx.elementsToIds(e.selected);
      const clickedObjEl =
        e.isClick && e.inputEvent
          ? (((e.inputEvent.target as Element | null)?.closest('.fm-obj') as HTMLElement | null) ??
            this.#ctx.objectElementAt(e.inputEvent.clientX, e.inputEvent.clientY))
          : null;
      const clickedId = clickedObjEl ? this.#ctx.idForElement(clickedObjEl) : undefined;
      // `hitRate === 100` is set by dragStart only for a Control-drag marquee (the
      // single source of that mode); reset it now that the gesture has ended.
      const containmentMarquee = this.#selecto.hitRate === 100;
      this.#selecto.hitRate = 0;
      if (containmentMarquee && e.isClick) {
        if (clickedId !== undefined) {
          llog('select', 'control-click toggle membership', { id: clickedId });
          this.#ctx.doc.toggle(clickedId);
          this.updateTarget();
          this.#swallowNextClick();
          return;
        }
      }
      if (e.isClick && clickedId !== undefined && (e.inputEvent?.shiftKey || e.inputEvent?.metaKey)) {
        this.updateTarget();
        this.#swallowNextClick();
        return;
      }
      // A plain click can arrive with Selecto's `selected` list empty after we
      // stopped the press to hand it to Moveable. Resolve the clicked object
      // directly so persisted group membership still expands through the store.
      this.#ctx.doc.selectOnly(clickedId !== undefined ? [clickedId] : selectedIds);
      this.updateTarget();
      // Selecto's pointer sequence is followed by a native `click` on the
      // canvas/band. Swallow it after marquee drags, and also after object clicks
      // that produced a selection. Rotated lines can visually extend outside their
      // tiny `.fm-obj` box, so that trailing click may look like empty canvas even
      // though Selecto correctly selected the line.
      if (!e.isClick || selectedIds.length > 0 || clickedId !== undefined) {
        this.#swallowNextClick();
      }
    });
    // Decide, at press time, who owns the gesture:
    this.#selecto.on('dragStart', (e) => {
      const input = e.inputEvent;
      const containmentMarquee = input.ctrlKey && !input.metaKey && !input.shiftKey;
      this.#selecto.hitRate = containmentMarquee ? 100 : 0;
      // A non-pointer tool is armed → this press PLACES an object, not selects.
      if (this.#ctx.doc.activeTool !== 'pointer') {
        llog('place', 'press while tool armed', {
          tool: this.#ctx.doc.activeTool,
          clientX: input.clientX,
          clientY: input.clientY,
        });
        this.#selecto.hitRate = 0;
        e.stop();
        if (!this.#ctx.pointInCanvas(input.clientX, input.clientY)) {
          llog('place', 'armed click outside canvas ignored', {
            tool: this.#ctx.doc.activeTool,
            clientX: input.clientX,
            clientY: input.clientY,
          });
          return;
        }
        this.#ctx.placement.startDraw(input);
        return;
      }
      const target = input.target as Element | null;
      const identity = this.#ctx.identitySnapshot();
      // Moveable's group overlay can be the event target instead of the real object.
      const objEl =
        (target?.closest('.fm-obj') as HTMLElement | null) ??
        (this.#targetIds.size > 1 ? this.#ctx.objectElementAt(input.clientX, input.clientY) : null);
      const id = objEl ? this.#ctx.idForElement(objEl, identity) : undefined;
      if (objEl && id === undefined) {
        llog('target', 'press on object but id UNRESOLVED', { painted: identity.painted.length });
      }
      // Modifier toggles must run before the moveable-control-box guard because
      // group-selection presses can arrive through Moveable's transparent overlay.
      // Control-drag is the containment marquee (handled below / on selectEnd), so
      // only Shift and Meta toggle membership at press time.
      if (objEl && id !== undefined && (input.shiftKey || input.metaKey)) {
        llog('select', 'toggle membership', { id });
        this.#ctx.doc.toggle(id);
        e.stop();
        this.updateTarget();
        // `updateTarget()` may detach the pressed overlay before the browser's
        // trailing native click fires, so swallow exactly that next click.
        this.#swallowNextClick();
        return;
      }
      if (containmentMarquee) {
        llog('select', 'control-drag containment marquee');
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
      this.#armObjectClick(id, input);
      this.#ctx.doc.selectOnly([id]);
      const ids = [...this.#ctx.doc.selection];
      const persistedGroup = this.#persistedGroupIdFor(ids) !== null;
      this.#targetKey = this.#targetKeyFor(ids);
      this.#targetIds = new Set(ids);
      const targets = ids.map((objectId) => this.#ctx.elementForId(objectId)).filter((el): el is HTMLElement => !!el);
      const guidelines = identity.painted.filter((el) => !targets.includes(el));
      // Set the target, then start the drag SYNCHRONOUSLY on this same pointerdown.
      // moveable.dragStart() flushes the just-set target (its internal `$_timer`
      // guard forceUpdates before triggering) and only fires if `objEl` matches the
      // live target — so the press latches straight into a drag, one gesture. The
      // old code ran dragStart inside setState's async callback, a frame late and
      // past the live pointer stream, so the first press only selected and the user
      // had to press again to drag.
      this.#moveable.setState({
        target: targets.length > 0 ? targets : objEl,
        elementGuidelines: guidelines,
        hideChildMoveableDefaultLines: persistedGroup,
      });
      this.#moveable.dragStart(input, objEl);
      e.stop();
    });
  }

  /** Whether `el` belongs to moveable's own chrome (control box / handles). */
  isMoveableElement(el: Element): boolean {
    return this.#moveable.isMoveableElement(el);
  }

  /** Reconcile moveable's target with the store selection (called reactively when
   * selection or geometry changes — e.g. after an undo). No-op during a gesture. */
  refresh(): void {
    // The render model (and thus the canvas DOM) may have changed — drop the
    // cached paint-order snapshot so the next lookup re-reads the fresh DOM.
    this.#ctx.invalidateIdentity();
    this.updateTarget(true);
    // Geometry-only changes (align/distribute/resize-match, undo/redo of same)
    // keep the same selected ids but move the DOM boxes underneath moveable.
    // Whenever a target is live, re-measure from the updated DOM so the controls
    // follow the objects.
    if (!this.#ctx.gesturing && this.#targetIds.size > 0) this.#scheduleRectUpdate();
  }

  /** Tell the interaction layer the current canvas zoom (#62), so client→model
   * pointer conversion during placement divides by it. */
  setZoom(zoom: number): void {
    const z = zoom > 0 ? zoom : 1;
    if (z !== this.#ctx.zoom) llog('zoom', 'setZoom', { zoom: z });
    this.#ctx.zoom = z;
    this.#moveable.setState({ zoom: z });
  }

  /** Apply the current layout grid without rebuilding the interaction layer.
   * Moveable supplies live snap feedback while the absolute geometry and manual
   * resize paths below resolve against the same EditorDoc values. */
  setGrid(size: number, enabled: boolean): void {
    const grid = Math.max(1, Math.round(size || 1));
    this.#moveable.setState({
      snappable: enabled,
      snapGridWidth: grid,
      snapGridHeight: grid,
    });
  }

  /** Post-delete canvas chrome reset (the shared command layer's cleanup): force
   * moveable to re-derive its (now empty) target. */
  forceClearTarget(): void {
    this.#targetKey = '__force_empty__';
    this.updateTarget();
  }

  /** Choose moveable's target from the real selection only. Hover uses a separate
   * lightweight outline, so resize handles never appear on unselected objects. */
  updateTarget(force = false): void {
    if (this.#ctx.gesturing) return;
    this.#ctx.gestureIdentity = null;
    // A placement tool is armed → the canvas is a drawing surface, not a select/
    // drag surface: drop moveable's target so a press places instead of grabs.
    if (this.#ctx.doc.activeTool !== 'pointer') {
      if (this.#targetKey === '') return;
      this.#targetKey = '';
      this.#targetIds = new Set();
      this.#moveable.setState(
        { target: null, elementGuidelines: [], rotatable: false, hideChildMoveableDefaultLines: false },
        () => this.#moveable.forceUpdate(),
      );
      llog('target', 'tool armed → moveable target cleared');
      return;
    }
    const sel = [...this.#ctx.doc.selection];
    const ids = sel.length > 0 ? sel : [];
    const key = this.#targetKeyFor(ids);
    if (key === this.#targetKey && (!force || ids.length === 0)) return;
    this.#targetKey = key;
    this.#targetIds = new Set(ids);
    if (ids.length === 0) {
      this.#moveable.setState(
        { target: null, elementGuidelines: [], rotatable: false, hideChildMoveableDefaultLines: false },
        () => this.#moveable.forceUpdate(),
      );
      llog('target', 'moveable target cleared', {
        hoverId: this.#ctx.hover.hoverId,
        selection: sel,
        paintedCount: this.#ctx.paintedElements().length,
      });
      this.#ctx.hover.paint();
      return;
    }
    const targets = ids.map((id) => this.#ctx.elementForId(id)).filter((el): el is HTMLElement => !!el);
    const guidelines = this.#ctx.paintedElements().filter((el) => !targets.includes(el));
    const persistedGroup = this.#persistedGroupIdFor(ids) !== null;
    this.#moveable.setState({
      target: targets,
      elementGuidelines: guidelines,
      rotatable: this.#canRotate(ids),
      hideChildMoveableDefaultLines: persistedGroup,
    });
    // THE key line for "resize does nothing": if `chosenIds` has an id but
    // `resolvedEls` is fewer, moveable has no element to attach handles to — the
    // store id didn't map to a painted `.fm-obj` (stale paint order / DOM not yet
    // committed after a create).
    llog('target', 'moveable target set', {
      hoverId: this.#ctx.hover.hoverId,
      selection: sel,
      chosenIds: ids,
      persistedGroup,
      resolvedEls: targets.length,
      paintedCount: this.#ctx.paintedElements().length,
      paintOrderIds: objectIdsInPaintOrder(this.#ctx.doc.renderModel),
    });
    this.#ctx.hover.paint();
  }

  /** Select all canvas objects (Cmd/Ctrl+A). A no-op while a placement tool is
   * armed — the canvas is a drawing surface then, not a selection surface. Syncs
   * moveable's control box immediately so the group handles appear at once. */
  selectAllObjects(): void {
    if (this.#ctx.doc.activeTool !== 'pointer') return;
    this.#ctx.doc.selectAll();
    llog('select', 'select all (keyboard)', { count: this.#ctx.doc.selection.size });
    this.updateTarget();
  }

  onClick = (e: MouseEvent): void => {
    // Swallow the native click that trails a Selecto commit, so a marquee or
    // modifier-click selection is not immediately cleared by the deselect path.
    if (this.#consumeSuppressedClick()) {
      return;
    }
    if (this.#ctx.gesturing || this.#ctx.doc.activeTool !== 'pointer') return;
    const target = e.target as Element | null;
    if (!target || this.#moveable.isMoveableElement(target)) return;
    const objEl = target.closest('.fm-obj') as HTMLElement | null;
    if (objEl) {
      const id = this.#ctx.idForElement(objEl);
      if (id !== undefined) {
        if (e.shiftKey || e.metaKey) this.#ctx.doc.toggle(id);
        else this.#ctx.doc.selectOnly([id]);
        this.#targetKey = '';
        this.updateTarget(true);
      }
      return;
    }
    if (target.closest('.le-part-label, .le-part-resize')) return;

    // A click on band whitespace (or empty canvas) only DESELECTS. Selecting a part
    // is reserved for its label rail (`.le-part-label`, wired in App.svelte), so a
    // stray click in the body never hijacks the selection into part-edit mode.
    this.#ctx.hover.clearVisual();
    this.#ctx.doc.clearSelection();
    this.updateTarget();
  };

  onDoubleClick = (e: MouseEvent): void => {
    if (this.#ctx.doc.activeTool !== 'pointer') return;
    const target = e.target as Element | null;
    const objEl = (target?.closest('.fm-obj') ?? null) as HTMLElement | null;
    if (!objEl || this.#moveable.isMoveableElement(objEl)) return;
    const id = this.#ctx.idForElement(objEl);
    const o = id === undefined ? undefined : this.#ctx.doc.getObject(id);
    if (id === undefined || !o || o.kind !== 'text') return;
    e.preventDefault();
    e.stopPropagation();
    this.#ctx.doc.selectOnly([id]);
    this.updateTarget();
    this.#ctx.text.start(id);
  };

  #swallowNextClick(): void {
    this.#suppressClickUntil = performance.now() + 750;
  }

  #consumeSuppressedClick(): boolean {
    const until = this.#suppressClickUntil;
    this.#suppressClickUntil = 0;
    return until !== 0 && performance.now() <= until;
  }

  #armObjectClick(id: number, input: MouseEvent | PointerEvent): void {
    this.#pendingObjectClick = { id, clientX: input.clientX, clientY: input.clientY };
    window.removeEventListener('pointerup', this.#onObjectClickUp);
    window.addEventListener('pointerup', this.#onObjectClickUp, { once: true });
  }

  #onObjectClickUp = (e: PointerEvent): void => {
    const pending = this.#pendingObjectClick;
    this.#pendingObjectClick = null;
    if (!pending) return;
    const dx = e.clientX - pending.clientX;
    const dy = e.clientY - pending.clientY;
    if (Math.hypot(dx, dy) > 3 || this.#moved) return;

    const commit = () => {
      this.#ctx.doc.selectOnly([pending.id]);
      this.#targetKey = '';
      this.updateTarget(true);
      this.#swallowNextClick();
    };
    if (this.#ctx.gesturing) requestAnimationFrame(commit);
    else commit();
  };

  // ── gesture lifecycle ──

  #begin(): void {
    this.#clearPendingMoves();
    this.#stopDragPointerFeedback();
    this.#ctx.gesturing = true;
    this.#moved = false;
    this.#dragStarts.clear();
    this.#resizeStarts.clear();
    this.#ctx.gestureIdentity = this.#ctx.identitySnapshot();
  }

  /** End a gesture: if it actually changed geometry, seal one undo step and
   * persist the moved/resized group; a no-move click does neither. Then re-target. */
  #end(kind: 'drag' | 'resize' = 'drag'): void {
    this.#stopDragPointerFeedback();
    // Pointer-up can arrive before the last requested display frame. Commit that
    // final authored position before band settlement, undo sealing, or persist.
    this.#flushMoves();
    this.#ctx.gesturing = false;
    // A drag may have carried objects across band boundaries; settle them onto a
    // real band (reparenting) BEFORE the undo mark so it's one step. Resize never
    // crosses bands, so it skips this.
    const affectedIds = kind === 'drag' ? [...this.#dragStarts.keys()] : [...this.#ctx.doc.selection];
    const reparented = kind === 'drag' && this.#moved ? this.#settleBands(affectedIds) : new Set<number>();
    this.#clearMoveFeedback();
    this.#clearBoundsCorrection();
    llog(kind, `${kind}End`, { moved: this.#moved, selection: [...this.#ctx.doc.selection], reparented: [...reparented] });
    if (this.#moved) {
      this.#ctx.doc.mark();
      void this.#persistObjects(affectedIds, reparented);
      void this.#persistDirtyLineProps();
    }
    this.#targetKey = ''; // force a re-sync after the gesture
    this.#dragStarts.clear();
    this.#resizeStarts.clear();
    this.#ctx.gestureIdentity = null;
    this.#scheduleRectUpdate();
    this.updateTarget();
    // A reparent moves the object to a DIFFERENT band's keyed-each, so Svelte
    // destroys its old DOM node and creates a new one — changing paint order. The
    // id→element map is stale until that re-render commits, so the sync above can
    // target the wrong element. Re-target after the DOM flush (id-keyed dedupe
    // cleared) so moveable's handles follow the MOVED object, not its old index.
    if (reparented.size > 0) {
      requestAnimationFrame(() => {
        this.#targetKey = '';
        this.updateTarget();
      });
    }
  }

  /** Make the dragged/resized single target the selection (if it wasn't already). */
  #selectFromTarget(el: HTMLElement | SVGElement): void {
    const id = this.#ctx.idForElement(el);
    if (id !== undefined && !this.#ctx.doc.isSelected(id)) this.#ctx.doc.selectOnly([id]);
  }

  #captureDragStarts(): void {
    // Portal children join the effective movement set without joining the visible
    // selection. This keeps the portal's own Moveable handles while its authored
    // fields and header labels travel by the same delta (#203).
    for (const id of this.#ctx.doc.movementObjectIds()) {
      const o = this.#ctx.doc.getObject(id);
      if (o) this.#dragStarts.set(id, { x: o.x, y: o.y });
    }
  }

  /** Moveable remains responsible for gesture ownership and transform bounds,
   * but its drag callback can arrive after expensive library processing. Capture
   * the same pointer stream at window entry so the object layer receives the
   * snapped authored position before that work begins (#195). */
  #startDragPointerFeedback(anchorId: number, inputEvent?: Event): void {
    const pointer = inputEvent as PointerEvent | MouseEvent | undefined;
    const origin = pointer
      ? {
          pointerId: 'pointerId' in pointer ? pointer.pointerId : (this.#pressedPointer?.pointerId ?? -1),
          clientX: pointer.clientX,
          clientY: pointer.clientY,
        }
      : this.#pressedPointer;
    if (!origin || !this.#dragStarts.has(anchorId)) {
      return;
    }
    this.#dragPointer = {
      pointerId: origin.pointerId,
      clientX: origin.clientX,
      clientY: origin.clientY,
      anchorId,
    };
  }

  #stopDragPointerFeedback(): void {
    this.#dragPointer = null;
  }

  #onPointerDown = (event: PointerEvent): void => {
    this.#pressedPointer = { pointerId: event.pointerId, clientX: event.clientX, clientY: event.clientY };
  };

  #onDragPointerMove = (event: PointerEvent): void => {
    const pointer = this.#dragPointer;
    if (!pointer || (pointer.pointerId !== -1 && event.pointerId !== pointer.pointerId)) return;
    const anchorStart = this.#dragStarts.get(pointer.anchorId);
    if (!anchorStart) return;
    const zoom = this.#ctx.zoom || 1;
    const anchorNext = this.#snappedMovePosition(
      anchorStart.x + (event.clientX - pointer.clientX) / zoom,
      anchorStart.y + (event.clientY - pointer.clientY) / zoom,
    );
    const dx = this.#clampDragDeltaX(anchorNext.x - anchorStart.x);
    const dy = anchorNext.y - anchorStart.y;
    this.#queueDragDelta(dx, dy);
  };

  /** Constrain the common delta at the canvas origin instead of clamping each
   * member separately, which would collapse offsets within a portal/group. */
  #clampDragDeltaX(dx: number): number {
    const minX = Math.min(...[...this.#dragStarts.values()].map((start) => start.x));
    return Number.isFinite(minX) ? Math.max(dx, -minX) : dx;
  }

  #queueDragDelta(dx: number, dy: number, paintFeedback = true): void {
    for (const [id, start] of this.#dragStarts) {
      const target = this.#ctx.elementForId(id);
      if (!target) continue;
      this.#queueMove(id, target, { x: start.x + dx, y: start.y + dy }, paintFeedback);
    }
  }

  #snappedMovePosition(left: number, top: number): { x: number; y: number } {
    const grid = this.#ctx.doc.snapToGrid ? this.#ctx.doc.gridSize : 0;
    return {
      x: clampOrigin(snapToGrid(left, grid)),
      // y is deliberately not clamped until drop so objects can cross bands.
      y: snapToGrid(top, grid),
    };
  }

  #queueMove(
    id: number,
    target: HTMLElement | SVGElement,
    next: { x: number; y: number },
    paintFeedback = true,
  ): void {
    const pending = this.#pendingMoves.get(id)?.next;
    const current = this.#ctx.doc.getObject(id);
    if (
      (pending && pending.x === next.x && pending.y === next.y) ||
      (!pending && current && current.x === next.x && current.y === next.y)
    ) return;
    this.#pendingMoves.set(id, { target, next });
    if (paintFeedback) this.#paintMoveFeedback(target, next);
    this.#moved = true;
    // Keep raw-pointer movement compositor-only until pointer-up. The Moveable
    // fallback below retains the frame-coalesced store path for synthetic input.
    if (this.#dragPointer) return;
    if (this.#moveFrame !== null) return;
    this.#moveFrame = requestAnimationFrame(() => {
      this.#moveFrame = null;
      this.#flushMoves();
    });
  }

  #flushMoves(): void {
    if (this.#moveFrame !== null) {
      cancelAnimationFrame(this.#moveFrame);
      this.#moveFrame = null;
    }
    if (this.#pendingMoves.size === 0) return;
    const moved: Array<{ id: number; x: number; y: number; target: HTMLElement | SVGElement }> = [];
    for (const [id, { target, next }] of this.#pendingMoves) {
      this.#ctx.doc.setObjectGeometry(id, next);
      // Replace the temporary compositor translation with the same authored
      // left/top without changing the painted position. Svelte remains the source
      // of truth and subsequently writes these identical values.
      target.style.left = `${next.x}px`;
      target.style.top = `${next.y}px`;
      target.style.transform = this.#moveFeedback.get(target)?.transform ?? '';
      moved.push({ id, ...next, target });
    }
    this.#pendingMoves.clear();
    // Measure only after every target shares the frame's authored geometry.
    const controlBox = this.#activeControlBox();
    this.#clearBoundsCorrection();
    this.#moveable.updateRect();
    const selectedMoved = moved.filter(({ id }) => this.#ctx.doc.isSelected(id));
    const targetRects = (selectedMoved.length > 0 ? selectedMoved : moved)
      .map(({ target }) => target.getBoundingClientRect());
    const targetLeft = Math.min(...targetRects.map((rect) => rect.left));
    const targetTop = Math.min(...targetRects.map((rect) => rect.top));
    const controlRect = controlBox.getBoundingClientRect();
    const transform = new DOMMatrixReadOnly(getComputedStyle(controlBox).transform);
    const host = this.#ctx.stage.ownerDocument.documentElement;
    host.style.setProperty('--le-drag-bounds-x', `${transform.m41 + targetLeft - controlRect.left}px`);
    host.style.setProperty('--le-drag-bounds-y', `${transform.m42 + targetTop - controlRect.top}px`);
    host.classList.add('syncing-drag-bounds');
    controlBox.toggleAttribute('data-rm-drag-bounds', true);
    this.#ctx.hover.paint();
    llog('drag', 'flush move frame', { moved: moved.map(({ id, x, y }) => ({ id, x, y })) });
  }

  #clearPendingMoves(): void {
    if (this.#moveFrame !== null) cancelAnimationFrame(this.#moveFrame);
    this.#moveFrame = null;
    this.#pendingMoves.clear();
    this.#clearMoveFeedback();
    this.#clearBoundsCorrection();
  }

  /** Paint the newest authored drag position in the pointer event itself. Only a
   * transform changes here, so Chromium can advance the existing layer without
   * waiting for the next reactive-store/layout frame (#195). */
  #paintMoveFeedback(target: HTMLElement | SVGElement, next: { x: number; y: number }): void {
    let original = this.#moveFeedback.get(target);
    if (!original) {
      original = { transform: target.style.transform, willChange: target.style.willChange };
      this.#moveFeedback.set(target, original);
    }
    const left = Number.parseFloat(target.style.left) || 0;
    const top = Number.parseFloat(target.style.top) || 0;
    const translate = `translate3d(${next.x - left}px, ${next.y - top}px, 0)`;
    target.style.willChange = 'transform';
    target.style.transform = original.transform ? `${translate} ${original.transform}` : translate;
  }

  #clearMoveFeedback(): void {
    for (const [target, original] of this.#moveFeedback) {
      target.style.transform = original.transform;
      target.style.willChange = original.willChange;
    }
    this.#moveFeedback.clear();
  }

  /** Moveable advances its group wrappers after the raw object transform. Pin
   * the visible child box to the already-painted target union before this event
   * can paint, without committing layout geometry. */
  #syncBoundsToMoveFeedback(): void {
    if (this.#moveFeedback.size === 0) return;
    const controlBox = this.#activeControlBox();
    this.#clearBoundsCorrection();
    const selectedTargets = [...this.#ctx.doc.selection]
      .map((id) => this.#ctx.elementForId(id))
      .filter((target): target is HTMLElement => !!target && this.#moveFeedback.has(target));
    const targetRects = (selectedTargets.length > 0 ? selectedTargets : [...this.#moveFeedback.keys()])
      .map((target) => target.getBoundingClientRect());
    const targetLeft = Math.min(...targetRects.map((rect) => rect.left));
    const targetTop = Math.min(...targetRects.map((rect) => rect.top));
    const controlRect = controlBox.getBoundingClientRect();
    const transform = new DOMMatrixReadOnly(getComputedStyle(controlBox).transform);
    const host = this.#ctx.stage.ownerDocument.documentElement;
    host.style.setProperty('--le-drag-bounds-x', `${transform.m41 + targetLeft - controlRect.left}px`);
    host.style.setProperty('--le-drag-bounds-y', `${transform.m42 + targetTop - controlRect.top}px`);
    host.classList.add('syncing-drag-bounds');
    controlBox.toggleAttribute('data-rm-drag-bounds', true);
  }

  #clearBoundsCorrection(): void {
    const host = this.#ctx.stage.ownerDocument.documentElement;
    host.classList.remove('syncing-drag-bounds');
    host.style.removeProperty('--le-drag-bounds-x');
    host.style.removeProperty('--le-drag-bounds-y');
    this.#ctx.stage.querySelectorAll('[data-rm-drag-bounds]').forEach((box) => {
      box.removeAttribute('data-rm-drag-bounds');
    });
  }

  /** Array targets make Moveable create wrapper managers. The painted child is
   * the control box that actually owns direction lines, not always the public
   * empty wrapper returned by the top-level manager. */
  #activeControlBox(): HTMLElement {
    const boxes = [...this.#ctx.stage.querySelectorAll<HTMLElement>('.moveable-control-box')];
    return boxes.findLast((box) => box.querySelector('.moveable-line')) ?? this.#moveable.getControlBoxElement();
  }

  #applyMove(target: HTMLElement | SVGElement, left: number, top: number, paintFeedback = true): void {
    const identity = this.#ctx.currentIdentity();
    const id = this.#ctx.idForElement(target, identity);
    if (id === undefined) {
      llog('target', 'drag: target element has NO mapped id — move is a no-op', {
        painted: identity.painted.length,
      });
      return;
    }
    const next = this.#snappedMovePosition(left, top);
    // y is left UNCLAMPED during a drag so the object can travel above its own band
    // (a negative part-relative y renders over the band above) — cross-band drags
    // are settled to a real band + local y on drop (#settleBands). x stays ≥ 0.
    const start = this.#dragStarts.get(id);
    if (!start) return;
    const dx = this.#clampDragDeltaX(next.x - start.x);
    this.#queueDragDelta(dx, next.y - start.y, paintFeedback);
  }

  #applyGroupMove(
    events: Array<{ target: HTMLElement | SVGElement; left: number; top: number }>,
    paintFeedback = true,
  ): void {
    const identity = this.#ctx.currentIdentity();
    const anchorEvent = events.find((event) => {
      const id = this.#ctx.idForElement(event.target, identity);
      return id !== undefined && this.#dragStarts.has(id);
    });
    if (!anchorEvent) return;
    const anchorId = this.#ctx.idForElement(anchorEvent.target, identity);
    const anchorStart = anchorId === undefined ? undefined : this.#dragStarts.get(anchorId);
    if (!anchorStart) return;

    const anchorNext = this.#snappedMovePosition(anchorEvent.left, anchorEvent.top);
    const dx = this.#clampDragDeltaX(anchorNext.x - anchorStart.x);
    const dy = anchorNext.y - anchorStart.y;
    this.#queueDragDelta(dx, dy, paintFeedback);
    llog('drag', 'queue group move', { anchorId, dx, dy, ids: [...this.#dragStarts.keys()] });
  }

  /** Settle every moved object onto a real band after a drag: read its absolute
   * canvas-y (its band's top + part-relative y), find the band that y lands in, and
   * rewrite the object to that band with a clamped local y. Objects that crossed a
   * boundary are reparented (partId change); the returned set drives which ones
   * persist via the reparent endpoint vs the bulk geometry commit. */
  #settleBands(ids: Iterable<number>): Set<number> {
    const reparented = new Set<number>();
    const model = this.#ctx.doc.renderModel;
    const totalHeight = model.parts.reduce((sum, p) => sum + p.height, 0);
    if (totalHeight <= 0) return reparented;
    const moving = new Set(ids);
    const before = new Map<number, Readonly<ObjectDoc>>();
    for (const id of moving) {
      const o = this.#ctx.doc.getObject(id);
      if (o) before.set(id, { ...o });
    }
    const destinations = new Map<number, { partId: number; x: number; y: number }>();

    const independentDestination = (o: Readonly<ObjectDoc>) => {
      const curTop = this.#ctx.partTop(o.partId);
      if (curTop === null) return null;
      const absY = Math.min(totalHeight - 1, Math.max(0, curTop + o.y));
      const where = partAtY(model, absY);
      return where ? { partId: where.partId, x: clampOrigin(o.x), y: clampOrigin(where.localY) } : null;
    };

    // Settle selected roots first. An owned child follows its moving portal's
    // destination even when its label sits above the portal's first row, so the
    // ownership group cannot split across bands.
    for (const [id, o] of before) {
      const movingParent = o.parentObjectId !== null && moving.has(o.parentObjectId)
        ? before.get(o.parentObjectId)
        : undefined;
      if (movingParent?.kind === 'portal') continue;
      const destination = independentDestination(o);
      if (destination) destinations.set(id, destination);
    }
    for (const [id, o] of before) {
      if (destinations.has(id)) continue;
      const parent = o.parentObjectId === null ? undefined : before.get(o.parentObjectId);
      const parentDestination = parent ? destinations.get(parent.id) : undefined;
      if (parent?.kind === 'portal' && parentDestination) {
        destinations.set(id, {
          partId: parentDestination.partId,
          x: clampOrigin(parentDestination.x + o.x - parent.x),
          y: clampOrigin(parentDestination.y + o.y - parent.y),
        });
      } else {
        const destination = independentDestination(o);
        if (destination) destinations.set(id, destination);
      }
    }

    for (const [id, destination] of destinations) {
      const o = before.get(id);
      if (!o) continue;
      if (destination.partId !== o.partId) {
        this.#ctx.doc.setProp(id, 'partId', destination.partId);
        reparented.add(id);
      }
      if (destination.x !== o.x || destination.y !== o.y) {
        this.#ctx.doc.setObjectGeometry(id, { x: destination.x, y: destination.y });
      }
      if (destination.partId !== o.partId) {
        llog('drag', 'settle: reparent object to band', { id, ...destination });
      }
    }
    return reparented;
  }

  #captureResizeStart(target: HTMLElement | SVGElement, direction: number[], inputEvent: Event | undefined): void {
    const identity = this.#ctx.currentIdentity();
    const id = this.#ctx.idForElement(target, identity);
    const o = id === undefined ? undefined : this.#ctx.doc.getObject(id);
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
    const identity = this.#ctx.currentIdentity();
    const id = this.#ctx.idForElement(target, identity);
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
      const dx = (pointer.clientX - start.clientX) / (this.#ctx.zoom || 1);
      const dy = (pointer.clientY - start.clientY) / (this.#ctx.zoom || 1);
      const dirX = Math.sign(start.direction[0] ?? 1);
      const dirY = Math.sign(start.direction[1] ?? 1);
      let x = start.x;
      let y = start.y;
      let w = start.w;
      let h = start.h;
      if (dirX >= 0) {
        w = snapToGrid(start.w + dx, this.#ctx.doc.snapToGrid ? this.#ctx.doc.gridSize : 0);
      } else {
        x = snapToGrid(start.x + dx, this.#ctx.doc.snapToGrid ? this.#ctx.doc.gridSize : 0);
        w = start.w - (x - start.x);
      }
      if (dirY >= 0) {
        h = snapToGrid(start.h + dy, this.#ctx.doc.snapToGrid ? this.#ctx.doc.gridSize : 0);
      } else {
        y = snapToGrid(start.y + dy, this.#ctx.doc.snapToGrid ? this.#ctx.doc.gridSize : 0);
        h = start.h - (y - start.y);
      }
      w = Math.max(1, Math.round(w));
      h = Math.max(1, Math.round(h));
      x = clampOrigin(x);
      y = clampOrigin(y);
      this.#ctx.doc.setObjectGeometry(id, { x, y, w, h });
      this.#syncLineToBox(id);
      this.#scheduleRectUpdate();
      llog('resize', 'apply resize from pointer', { id, w, h, x, y, dx: Math.round(dx), dy: Math.round(dy) });
      return;
    }
    this.#ctx.doc.setObjectGeometry(id, {
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
    const o = id === null ? undefined : this.#ctx.doc.getObject(id);
    if (id === null || !o || o.kind !== 'line') return;
    const nextAngle = normalizeAngle(angle);
    const length = this.#rotateStartLength || lineLength(o, parseProps(o.props));
    const geom = lineGeometryForAngle(o, nextAngle, length);
    const props = { ...parseProps(o.props), angle: nextAngle, length };
    const propsJson = JSON.stringify(props);
    this.#moved = true;
    this.#ctx.doc.setObjectGeometry(id, { x: clampOrigin(geom.x), y: clampOrigin(geom.y), w: geom.w, h: geom.h });
    this.#ctx.doc.setObjectProps(id, propsJson);
    this.#setLineShapeStyle(id, props);
    this.#dirtyLineProps.add(id);
    this.#scheduleRectUpdate();
  }

  #endLineRotate(): void {
    const id = this.#rotatingLineId;
    const moved = this.#moved;
    this.#ctx.gesturing = false;
    this.#rotatingLineId = null;
    this.#rotateStartLength = 0;
    this.#ctx.gestureIdentity = null;
    if (id !== null && moved) {
      this.#ctx.doc.mark();
      void this.#persistObjects([id]);
      void this.#persistDirtyLineProps();
    }
    this.#targetKey = '';
    this.#scheduleRectUpdate();
    this.updateTarget();
  }

  async #persistLineProps(id: number): Promise<void> {
    const o = this.#ctx.doc.getObject(id);
    if (!o) return;
    const props = parseProps(o.props);
    try {
      const styles = await persistObjectProps(this.#ctx.layoutId, id, props);
      this.#ctx.doc.setObjectStyles(id, styles);
    } catch (e) {
      lerror('persist', 'failed to persist line rotation', e);
      this.#ctx.reportError(e);
    }
  }

  async #persistDirtyLineProps(): Promise<void> {
    const ids = [...this.#dirtyLineProps];
    this.#dirtyLineProps.clear();
    await Promise.all(ids.map((id) => this.#persistLineProps(id)));
  }

  #canRotate(ids: number[]): boolean {
    if (ids.length !== 1) return false;
    return this.#ctx.doc.getObject(ids[0])?.kind === 'line';
  }

  #persistedGroupIdFor(ids: number[]): number | null {
    return this.#ctx.doc.groupIdForSelection(ids);
  }

  #targetKeyFor(ids: number[]): string {
    return `${ids.slice().sort((a, b) => a - b).join(',')}|group:${this.#persistedGroupIdFor(ids) ?? ''}`;
  }

  #setLineShapeStyle(id: number, props: Record<string, unknown>): void {
    const view = this.#ctx.objectView(id);
    if (!view) return;
    this.#ctx.doc.setObjectStyles(id, {
      objectStyle: view.objectStyle,
      textStyle: view.textStyle,
      shapeStyle: lineShapeStyle(props),
    });
  }

  #syncLineToBox(id: number): void {
    const o = this.#ctx.doc.getObject(id);
    if (!o || o.kind !== 'line') return;
    const next = linePropsForBox(o, parseProps(o.props));
    this.#ctx.doc.setObjectProps(id, JSON.stringify(next));
    this.#setLineShapeStyle(id, next);
    this.#dirtyLineProps.add(id);
  }

  // ── persistence (#46 bulk axum contract) ──

  async #persistObjects(ids: Iterable<number>, reparented: Set<number> = new Set()): Promise<void> {
    const objs = [...new Set(ids)]
      .map((id) => this.#ctx.doc.getObject(id))
      .filter((o): o is NonNullable<Readonly<ObjectDoc>> => !!o);
    if (objs.length === 0) return;
    // Objects that crossed a band boundary persist their new membership (partId +
    // origin) via the reparent endpoint; the rest commit geometry in bulk as before.
    const geom = objs.filter((o) => !reparented.has(o.id)).map((o) => ({ id: o.id, x: o.x, y: o.y, w: o.w, h: o.h }));
    const moved = objs.filter((o) => reparented.has(o.id));
    llog('persist', 'POST geometry', { geometry: geom, reparent: moved.map((o) => ({ id: o.id, partId: o.partId })) });
    try {
      const posts: Promise<unknown>[] = [];
      if (geom.length > 0) posts.push(setObjectsGeometry(this.#ctx.layoutId, geom));
      for (const o of moved) posts.push(setObjectPart(this.#ctx.layoutId, o.id, o.partId, o.x, o.y));
      await Promise.all(posts);
      llog('persist', 'geometry saved', { geometry: geom.length, reparented: moved.length });
    } catch (e) {
      // The store already reflects the edit; surface the persist failure rather
      // than tearing down the in-memory state (a reload would reveal divergence).
      lerror('persist', 'failed to persist object geometry', e);
    }
  }

  #scheduleRectUpdate(): void {
    if (this.#rectFrame !== null) return;
    this.#rectFrame = requestAnimationFrame(() => {
      this.#rectFrame = null;
      this.#moveable.updateRect();
      this.#ctx.hover.paint();
    });
  }

  destroy(): void {
    window.removeEventListener('pointerup', this.#onObjectClickUp);
    window.removeEventListener('pointerdown', this.#onPointerDown, { capture: true });
    window.removeEventListener('pointermove', this.#onDragPointerMove, { capture: true });
    this.#clearPendingMoves();
    if (this.#rectFrame !== null) cancelAnimationFrame(this.#rectFrame);
    this.#moveable.destroy();
    this.#selecto.destroy();
  }
}
