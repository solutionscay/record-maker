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

import type { GestureHistoryCheckpoint, ObjectDoc } from '../doc.svelte';
import { SNAP_THRESHOLD, clampOrigin, objectIdsInPaintOrder, snapToGrid } from '../canvas-edit';
import { partAtY } from '../create';
import {
  setObjectPart,
  setObjectProps as persistObjectProps,
  setObjectsGeometry,
} from '../persist';
import { llog, lerror } from '../log';
import { parseProps } from '../object-props';
import type { CanvasContext, IdentitySnapshot } from './context';
import { GestureLifecycle, type GestureCancelReason } from './gesture-lifecycle';
import { objectBehavior } from './object-behavior';
import { classifyPress, type GestureIntent } from './press-intent';
import {
  resolveMoveGuides,
  resolveResizeGuides,
  unionGuideBoxes,
  type ActiveGuide,
  type GuideBox,
  type GuideCandidate,
} from './smart-guides';

type PendingObjectClick = {
  id: number;
  clientX: number;
  clientY: number;
  selectionBefore?: number[];
};

type ResizeProposal = {
  id: number;
  target: HTMLElement | SVGElement;
  box: GuideBox;
  partTop: number;
  direction: number[];
};

export class TransformController {
  readonly #ctx: CanvasContext;
  readonly #moveable: Moveable;
  readonly #selecto: Selecto;
  readonly #lifecycle = new GestureLifecycle('object-transform');

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
  /** Object ids moveable currently targets. Target reconciliation is cheap and
   * always derives from current selection/identity instead of sentinel keys. */
  #targetIds = new Set<number>();
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
  #rotatingObjectId: number | null = null;
  #rotateStartMeasure = 0;
  #dirtyBehaviorProps = new Set<number>();
  #pendingObjectClick: PendingObjectClick | null = null;
  #pressIntent: GestureIntent | null = null;
  #historyCheckpoint: GestureHistoryCheckpoint | null = null;
  #selectionBeforeGesture: number[] | null = null;
  #activeSelectionSnapshot: number[] = [];
  #selectionGesture = false;
  #guideCandidates: GuideCandidate[] = [];
  #dragUnionStart: GuideBox | null = null;
  #guideElements: HTMLElement[] = [];

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
      this.#begin(e.inputEvent);
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
      this.#begin(e.inputEvent);
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
      this.#begin(e.inputEvent);
      this.#selectFromTarget(e.target);
      this.#captureResizeStart(e.target, e.direction, e.inputEvent);
    });
    this.#moveable.on('resize', (e) => this.#applyResize(e.target, e.width, e.height, e.drag.left, e.drag.top, e.inputEvent));
    this.#moveable.on('resizeEnd', () => this.#end('resize'));
    this.#moveable.on('resizeGroupStart', (e) => {
      this.#begin(e.inputEvent);
      e.events.forEach((ev) => this.#captureResizeStart(ev.target, ev.direction, ev.inputEvent));
    });
    this.#moveable.on('resizeGroup', (e) => this.#applyResizeGroup(e.events));
    this.#moveable.on('resizeGroupEnd', () => this.#end('resize'));

    // ── rotate (line objects only) ──────────────────────────────────────────
    this.#moveable.on('rotateStart', (e) => {
      const id = this.#ctx.idForElement(e.target);
      const o = id === undefined ? undefined : this.#ctx.doc.getObject(id);
      const start = o ? objectBehavior(o.kind).rotationStart(o) : null;
      if (id === undefined || !o || !start) return false;
      this.#begin(e.inputEvent);
      this.#selectFromTarget(e.target);
      this.#rotatingObjectId = id;
      this.#rotateStartMeasure = start.measure;
      e.set(start.angle);
      llog('rotate', 'rotateStart', { id, angle: start.angle, measure: start.measure });
    });
    this.#moveable.on('rotate', (e) => this.#applyObjectRotate(e.beforeRotation));
    this.#moveable.on('rotateEnd', () => this.#endObjectRotate());

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
      if (this.#selectionGesture) {
        this.#selectionGesture = false;
        this.#ctx.gesturing = false;
        this.#ctx.gestureIdentity = null;
        this.#activeSelectionSnapshot = [];
        this.#lifecycle.commit();
      }
      const intent = this.#pressIntent;
      this.#pressIntent = null;
      const selectedIds = this.#ctx.elementsToIds(e.selected);
      const clickedObjEl =
        e.isClick && e.inputEvent
          ? (((e.inputEvent.target as Element | null)?.closest('.fm-obj') as HTMLElement | null) ??
            this.#ctx.objectElementAt(e.inputEvent.clientX, e.inputEvent.clientY))
          : null;
      const clickedId = clickedObjEl ? this.#ctx.idForElement(clickedObjEl) : undefined;
      // `hitRate === 100` is set by dragStart only for a Control-drag marquee (the
      // single source of that mode); reset it now that the gesture has ended.
      const containmentMarquee = intent?.kind === 'containment-marquee';
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
      if (intent?.kind === 'toggle') {
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
      const target = input.target as Element | null;
      const identity = this.#ctx.identitySnapshot();
      // Moveable's group overlay can be the event target instead of the real object.
      const objEl =
        (target?.closest('.fm-obj') as HTMLElement | null) ??
        this.#ctx.objectElementAt(input.clientX, input.clientY);
      const id = objEl ? this.#ctx.idForElement(objEl, identity) : undefined;
      const intent = classifyPress({
        activeTool: this.#ctx.doc.activeTool,
        ctrlKey: input.ctrlKey,
        metaKey: input.metaKey,
        shiftKey: input.shiftKey,
        objectId: id ?? null,
        objectIsTargeted: id !== undefined && this.#targetIds.has(id),
        moveableChrome: !!target && this.#moveable.isMoveableElement(target),
      });
      this.#pressIntent = intent;
      this.#selecto.hitRate = intent.kind === 'containment-marquee' ? 100 : 0;
      if (objEl && id === undefined) {
        llog('target', 'press on object but id UNRESOLVED', { painted: identity.painted.length });
      }

      if (intent.kind === 'place') {
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
        this.#ctx.placement.startDraw(input, this.#pressedPointer?.pointerId);
        return;
      }

      if (intent.kind === 'toggle' && objEl) {
        const selectionBefore = [...this.#ctx.doc.selection];
        this.#selectionBeforeGesture = selectionBefore;
        const dragSelection = this.#ctx.doc.isSelected(intent.id)
          ? selectionBefore
          : [...selectionBefore, intent.id];
        llog('select', 'pending modifier click or drag', {
          id: intent.id,
          selected: this.#ctx.doc.isSelected(intent.id),
          dragSelection,
        });
        this.#armObjectClick(intent.id, input, selectionBefore);
        this.#ctx.doc.selectOnly(dragSelection);
        this.#setMoveableTargets([...this.#ctx.doc.selection], identity, objEl);
        this.#moveable.dragStart(input, objEl);
        e.stop();
        return;
      }

      if (intent.kind === 'containment-marquee') {
        llog('select', 'control-drag containment marquee');
        this.#beginSelectionGesture(input);
        return;
      }

      if (intent.kind === 'drag' && intent.id === null) {
        llog('drag', 'press on moveable control box → moveable owns gesture');
        e.stop();
        return;
      }

      if (intent.kind === 'marquee') {
        llog('select', 'press on empty canvas → marquee');
        this.#beginSelectionGesture(input);
        return; // empty canvas → selecto runs its marquee
      }

      if (intent.kind !== 'drag' || intent.id === null || !objEl) return;
      if (!intent.select) {
        llog('drag', 'press on targeted object → moveable drags it', { id: intent.id });
        e.stop();
        return;
      }
      // Select and hand the current pointer event to Moveable immediately. Hover
      // no longer pre-targets objects, so this preserves click-drag in one move
      // without showing resize handles on mere hover.
      llog('drag', 'press on un-targeted object → select + start drag', { id: intent.id });
      this.#selectionBeforeGesture = [...this.#ctx.doc.selection];
      this.#armObjectClick(intent.id, input);
      this.#ctx.doc.selectOnly([intent.id]);
      const ids = [...this.#ctx.doc.selection];
      // Set the target, then start the drag SYNCHRONOUSLY on this same pointerdown.
      // moveable.dragStart() flushes the just-set target (its internal `$_timer`
      // guard forceUpdates before triggering) and only fires if `objEl` matches the
      // live target — so the press latches straight into a drag, one gesture. The
      // old code ran dragStart inside setState's async callback, a frame late and
      // past the live pointer stream, so the first press only selected and the user
      // had to press again to drag.
      this.#setMoveableTargets(ids, identity, objEl);
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
    this.updateTarget();
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
    this.updateTarget();
  }

  /** Apply one selection-derived Moveable target. The optional fallback keeps
   * same-pointer select-and-drag attached while Svelte catches up with a newly
   * selected object. */
  #setMoveableTargets(
    ids: number[],
    identity: IdentitySnapshot = this.#ctx.currentIdentity(),
    fallback?: HTMLElement,
  ): { targets: HTMLElement[]; persistedGroup: boolean } {
    this.#targetIds = new Set(ids);
    const targets = ids
      .map((id) => this.#ctx.elementForId(id, identity))
      .filter((el): el is HTMLElement => !!el);
    const persistedGroup = this.#persistedGroupIdFor(ids) !== null;
    this.#moveable.setState({
      target: targets.length > 0 ? targets : (fallback ?? []),
      // Sibling chrome is rendered only from the authoritative numeric resolver.
      elementGuidelines: [],
      rotatable: this.#canRotate(ids),
      hideChildMoveableDefaultLines: persistedGroup,
    });
    return { targets, persistedGroup };
  }

  /** Choose moveable's target from the real selection only. Hover uses a separate
   * lightweight outline, so resize handles never appear on unselected objects. */
  updateTarget(): void {
    if (this.#ctx.gesturing) return;
    this.#ctx.gestureIdentity = null;
    // A placement tool is armed → the canvas is a drawing surface, not a select/
    // drag surface: drop moveable's target so a press places instead of grabs.
    if (this.#ctx.doc.activeTool !== 'pointer') {
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
    if (ids.length === 0) {
      this.#targetIds = new Set();
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
    const { targets, persistedGroup } = this.#setMoveableTargets(ids);
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
        this.updateTarget();
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

  #armObjectClick(id: number, input: MouseEvent | PointerEvent, selectionBefore?: number[]): void {
    this.#pendingObjectClick = { id, clientX: input.clientX, clientY: input.clientY, selectionBefore };
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
      if (pending.selectionBefore) {
        this.#ctx.doc.selectOnly(pending.selectionBefore);
        this.#ctx.doc.toggle(pending.id);
      } else {
        this.#ctx.doc.selectOnly([pending.id]);
      }
      this.updateTarget();
      this.#swallowNextClick();
    };
    if (this.#ctx.gesturing) requestAnimationFrame(commit);
    else commit();
  };

  // ── gesture lifecycle ──

  #begin(inputEvent?: Event): void {
    // Moveable can emit an individual start immediately followed by the group
    // start for one physical pointer. They share one transaction/lifecycle.
    if (this.#ctx.gesturing && this.#lifecycle.active) return;
    this.#clearPendingMoves();
    this.#clearSmartGuides();
    this.#stopDragPointerFeedback();
    this.#historyCheckpoint = this.#ctx.doc.beginGestureTransaction();
    this.#activeSelectionSnapshot = this.#selectionBeforeGesture ?? [...this.#ctx.doc.selection];
    this.#selectionBeforeGesture = null;
    this.#ctx.gesturing = true;
    this.#moved = false;
    this.#dragStarts.clear();
    this.#resizeStarts.clear();
    this.#ctx.gestureIdentity = this.#ctx.identitySnapshot();
    this.#lifecycle.begin({
      inputEvent,
      pointerId: this.#pressedPointer?.pointerId,
      captureTarget: inputEvent?.target instanceof Element ? inputEvent.target : null,
      onCancel: (reason) => this.#cancelTransformGesture(reason),
    });
  }

  #beginSelectionGesture(inputEvent: Event): void {
    this.#selectionGesture = true;
    this.#activeSelectionSnapshot = [...this.#ctx.doc.selection];
    this.#ctx.gesturing = true;
    this.#ctx.gestureIdentity = this.#ctx.identitySnapshot();
    this.#lifecycle.begin({
      inputEvent,
      pointerId: this.#pressedPointer?.pointerId,
      captureTarget: this.#ctx.stage,
      onCancel: (reason) => this.#cancelSelectionGesture(reason),
    });
  }

  #cancelSelectionGesture(reason: GestureCancelReason): void {
    if (!this.#selectionGesture) return;
    this.#selectionGesture = false;
    this.#ctx.gesturing = false;
    this.#stopSelectoGesture();
    this.#ctx.doc.selectOnly(this.#activeSelectionSnapshot);
    this.#activeSelectionSnapshot = [];
    this.#pressIntent = null;
    this.#selecto.hitRate = 0;
    this.#ctx.gestureIdentity = null;
    this.updateTarget();
    llog('select', 'marquee cancelled', { reason });
  }

  /** End a gesture: if it actually changed geometry, seal one undo step and
   * persist the moved/resized group; a no-move click does neither. Then re-target. */
  #end(kind: 'drag' | 'resize' = 'drag'): void {
    if (!this.#ctx.gesturing) return;
    this.#lifecycle.commit();
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
    this.#clearSmartGuides();
    llog(kind, `${kind}End`, { moved: this.#moved, selection: [...this.#ctx.doc.selection], reparented: [...reparented] });
    if (this.#moved) {
      if (this.#historyCheckpoint) this.#ctx.doc.commitGestureTransaction(this.#historyCheckpoint);
      void this.#persistObjects(affectedIds, reparented);
      void this.#persistDirtyBehaviorProps();
    }
    this.#historyCheckpoint = null;
    this.#activeSelectionSnapshot = [];
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
        this.updateTarget();
      });
    }
  }

  #cancelTransformGesture(reason: GestureCancelReason): void {
    if (!this.#ctx.gesturing) return;
    // Make any Moveable/Selecto end event caused by stopDrag/stop a no-op.
    this.#ctx.gesturing = false;
    this.#moveable.stopDrag();
    this.#stopDragPointerFeedback();
    this.#clearPendingMoves();
    if (this.#historyCheckpoint) {
      this.#ctx.doc.cancelGestureTransaction(this.#historyCheckpoint);
      this.#historyCheckpoint = null;
    }
    for (const id of this.#dirtyBehaviorProps) {
      const object = this.#ctx.doc.getObject(id);
      if (object) this.#setBehaviorShapeStyle(id, parseProps(object.props));
    }
    this.#dirtyBehaviorProps.clear();
    for (const id of new Set([...this.#dragStarts.keys(), ...this.#resizeStarts.keys()])) {
      const object = this.#ctx.doc.getObject(id);
      const element = this.#ctx.elementForId(id);
      if (object && element) {
        element.style.left = `${object.x}px`;
        element.style.top = `${object.y}px`;
        element.style.width = `${object.w}px`;
        element.style.height = `${object.h}px`;
      }
    }
    this.#ctx.doc.selectOnly(this.#activeSelectionSnapshot);
    this.#activeSelectionSnapshot = [];
    this.#dragStarts.clear();
    this.#resizeStarts.clear();
    this.#rotatingObjectId = null;
    this.#rotateStartMeasure = 0;
    this.#moved = false;
    this.#pressIntent = null;
    this.#pendingObjectClick = null;
    window.removeEventListener('pointerup', this.#onObjectClickUp);
    this.#ctx.gestureIdentity = null;
    this.#clearBoundsCorrection();
    this.#clearSmartGuides();
    this.#scheduleRectUpdate();
    this.updateTarget();
    llog('drag', 'gesture cancelled', { reason });
  }

  #stopSelectoGesture(): void {
    // Selecto does not expose its Gesto stop method, but its documented instance
    // `destroy()` uses this same internal object. Stopping delivery here leaves
    // the reusable Selecto instance and its selection state intact.
    const selecto = this.#selecto as unknown as { gesto?: { stop(): void } };
    selecto.gesto?.stop();
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
    this.#captureGuideFrame(new Set(this.#dragStarts.keys()));
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
    const resolved = this.#resolveDragDelta(
      this.#clampDragDeltaX(anchorNext.x - anchorStart.x),
      anchorNext.y - anchorStart.y,
    );
    const dx = resolved.dx;
    const dy = resolved.dy;
    this.#queueDragDelta(dx, dy);
  };

  /** Constrain the common delta at the canvas origin instead of clamping each
   * member separately, which would collapse offsets within a portal/group. */
  #clampDragDeltaX(dx: number): number {
    const minX = Math.min(...[...this.#dragStarts.values()].map((start) => start.x));
    return Number.isFinite(minX) ? Math.max(dx, -minX) : dx;
  }

  #absoluteBox(id: number): GuideBox | null {
    const object = this.#ctx.doc.getObject(id);
    if (!object) return null;
    const partTop = this.#ctx.partTop(object.partId);
    if (partTop === null) return null;
    return { x: object.x, y: partTop + object.y, w: object.w, h: object.h };
  }

  #captureGuideFrame(excludedIds: Set<number>): void {
    const activeBoxes = [...this.#ctx.doc.selection]
      .map((id) => this.#absoluteBox(id))
      .filter((box): box is GuideBox => !!box);
    this.#dragUnionStart = unionGuideBoxes(activeBoxes);
    this.#guideCandidates = this.#ctx.currentIdentity().ids
      .filter((id) => Number.isFinite(id) && !excludedIds.has(id))
      .map((id) => {
        const box = this.#absoluteBox(id);
        return box ? { id, box } : null;
      })
      .filter((candidate): candidate is GuideCandidate => !!candidate);
  }

  #resolveDragDelta(dx: number, dy: number): { dx: number; dy: number } {
    const start = this.#dragUnionStart;
    if (!start || this.#guideCandidates.length === 0) {
      this.#clearSmartGuides();
      return { dx, dy };
    }
    const resolved = resolveMoveGuides(
      { ...start, x: start.x + dx, y: start.y + dy },
      this.#guideCandidates,
      SNAP_THRESHOLD / (this.#ctx.zoom || 1),
    );
    const next = {
      dx: this.#clampDragDeltaX(dx + resolved.box.x - (start.x + dx)),
      dy: dy + resolved.box.y - (start.y + dy),
    };
    this.#paintSmartGuides(resolved.guides);
    return next;
  }

  #paintSmartGuides(guides: ActiveGuide[]): void {
    this.#clearSmartGuides();
    const overlay = this.#ctx.partOverlay();
    if (!overlay) return;
    const height = this.#ctx.doc.renderModel.parts.reduce((sum, part) => sum + part.height, 0);
    const width = Math.max(this.#ctx.doc.renderModel.width, 760);
    for (const guide of guides) {
      const element = document.createElement('div');
      element.className = `le-smart-guide le-smart-guide-${guide.axis}`;
      if (guide.axis === 'x') {
        element.style.left = `${guide.position}px`;
        element.style.top = '0';
        element.style.height = `${height}px`;
      } else {
        element.style.left = '0';
        element.style.top = `${guide.position}px`;
        element.style.width = `${width}px`;
      }
      overlay.append(element);
      this.#guideElements.push(element);
    }
  }

  #clearSmartGuides(): void {
    for (const guide of this.#guideElements) guide.remove();
    this.#guideElements = [];
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
    const resolved = this.#resolveDragDelta(
      this.#clampDragDeltaX(next.x - start.x),
      next.y - start.y,
    );
    this.#queueDragDelta(resolved.dx, resolved.dy, paintFeedback);
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
    const resolved = this.#resolveDragDelta(
      this.#clampDragDeltaX(anchorNext.x - anchorStart.x),
      anchorNext.y - anchorStart.y,
    );
    const dx = resolved.dx;
    const dy = resolved.dy;
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
    this.#captureGuideFrame(new Set(this.#ctx.doc.selection));
  }

  #applyResize(
    target: HTMLElement | SVGElement,
    width: number,
    height: number,
    left: number,
    top: number,
    inputEvent?: Event,
  ): void {
    const proposal = this.#resizeProposal(target, width, height, left, top, inputEvent);
    if (!proposal) return;
    const resolved = resolveResizeGuides(
      proposal.box,
      proposal.direction,
      this.#guideCandidates,
      SNAP_THRESHOLD / (this.#ctx.zoom || 1),
    );
    this.#paintSmartGuides(resolved.guides);
    this.#applyResizeProposal({ ...proposal, box: resolved.box });
  }

  #applyResizeGroup(events: Array<{
    target: HTMLElement | SVGElement;
    width: number;
    height: number;
    drag: { left: number; top: number };
    inputEvent?: Event;
  }>): void {
    const proposals = events
      .map((event) => this.#resizeProposal(
        event.target,
        event.width,
        event.height,
        event.drag.left,
        event.drag.top,
        event.inputEvent,
      ))
      .filter((proposal): proposal is ResizeProposal => !!proposal);
    const union = unionGuideBoxes(proposals.map((proposal) => proposal.box));
    if (!union || proposals.length === 0) return;
    const direction = proposals[0].direction;
    const resolved = resolveResizeGuides(
      union,
      direction,
      this.#guideCandidates,
      SNAP_THRESHOLD / (this.#ctx.zoom || 1),
    );
    this.#paintSmartGuides(resolved.guides);
    const dirX = Math.sign(direction[0] ?? 0);
    const dirY = Math.sign(direction[1] ?? 0);
    const scaleX = union.w > 0 ? resolved.box.w / union.w : 1;
    const scaleY = union.h > 0 ? resolved.box.h / union.h : 1;
    for (const proposal of proposals) {
      const box = { ...proposal.box };
      if (dirX !== 0) {
        box.x = resolved.box.x + (proposal.box.x - union.x) * scaleX;
        box.w = proposal.box.w * scaleX;
      }
      if (dirY !== 0) {
        box.y = resolved.box.y + (proposal.box.y - union.y) * scaleY;
        box.h = proposal.box.h * scaleY;
      }
      this.#applyResizeProposal({ ...proposal, box });
    }
  }

  #resizeProposal(
    target: HTMLElement | SVGElement,
    width: number,
    height: number,
    left: number,
    top: number,
    inputEvent?: Event,
  ): ResizeProposal | null {
    const identity = this.#ctx.currentIdentity();
    const id = this.#ctx.idForElement(target, identity);
    if (id === undefined) {
      llog('target', 'resize: target element has NO mapped id — resize is a no-op', {
        painted: identity.painted.length,
        paintOrderIds: identity.ids,
      });
      return null;
    }
    const pointer = inputEvent as PointerEvent | MouseEvent | undefined;
    const start = this.#resizeStarts.get(id);
    const object = this.#ctx.doc.getObject(id);
    if (!object) return null;
    const partTop = this.#ctx.partTop(object.partId);
    if (partTop === null) return null;
    let x: number;
    let y: number;
    let w: number;
    let h: number;
    if (pointer && start) {
      const dx = (pointer.clientX - start.clientX) / (this.#ctx.zoom || 1);
      const dy = (pointer.clientY - start.clientY) / (this.#ctx.zoom || 1);
      const dirX = Math.sign(start.direction[0] ?? 1);
      const dirY = Math.sign(start.direction[1] ?? 1);
      x = start.x;
      y = start.y;
      w = start.w;
      h = start.h;
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
      llog('resize', 'apply resize from pointer', { id, w, h, x, y, dx: Math.round(dx), dy: Math.round(dy) });
    } else {
      x = clampOrigin(left);
      y = clampOrigin(top);
      w = Math.max(1, Math.round(width));
      h = Math.max(1, Math.round(height));
    }
    return { id, target, box: { x, y: partTop + y, w, h }, partTop, direction: start?.direction ?? [1, 1] };
  }

  #applyResizeProposal(proposal: ResizeProposal): void {
    const geometry = {
      x: clampOrigin(Math.round(proposal.box.x)),
      y: clampOrigin(Math.round(proposal.box.y - proposal.partTop)),
      w: Math.max(1, Math.round(proposal.box.w)),
      h: Math.max(1, Math.round(proposal.box.h)),
    };
    this.#moved = true;
    this.#ctx.doc.setObjectGeometry(proposal.id, geometry);
    this.#syncObjectToBox(proposal.id);
    this.#scheduleRectUpdate();
    llog('resize', 'apply resolved resize', { id: proposal.id, ...geometry });
  }

  #applyObjectRotate(angle: number): void {
    const id = this.#rotatingObjectId;
    const o = id === null ? undefined : this.#ctx.doc.getObject(id);
    const frame = o ? objectBehavior(o.kind).onRotate(o, angle, this.#rotateStartMeasure) : null;
    if (id === null || !o || !frame) return;
    this.#moved = true;
    this.#ctx.doc.setObjectGeometry(id, {
      x: clampOrigin(frame.geometry.x),
      y: clampOrigin(frame.geometry.y),
      w: frame.geometry.w,
      h: frame.geometry.h,
    });
    this.#ctx.doc.setObjectProps(id, JSON.stringify(frame.props));
    this.#setBehaviorShapeStyle(id, frame.props);
    this.#dirtyBehaviorProps.add(id);
    this.#scheduleRectUpdate();
  }

  #endObjectRotate(): void {
    if (!this.#ctx.gesturing) return;
    this.#lifecycle.commit();
    const id = this.#rotatingObjectId;
    const moved = this.#moved;
    this.#ctx.gesturing = false;
    this.#rotatingObjectId = null;
    this.#rotateStartMeasure = 0;
    this.#ctx.gestureIdentity = null;
    if (id !== null && moved) {
      if (this.#historyCheckpoint) this.#ctx.doc.commitGestureTransaction(this.#historyCheckpoint);
      void this.#persistObjects([id]);
      void this.#persistDirtyBehaviorProps();
    }
    this.#historyCheckpoint = null;
    this.#activeSelectionSnapshot = [];
    this.#scheduleRectUpdate();
    this.updateTarget();
  }

  async #persistBehaviorProps(id: number): Promise<void> {
    const o = this.#ctx.doc.getObject(id);
    if (!o) return;
    const props = parseProps(o.props);
    try {
      const styles = await persistObjectProps(this.#ctx.layoutId, id, props);
      this.#ctx.doc.setObjectStyles(id, styles);
    } catch (e) {
      lerror('persist', 'failed to persist object behavior props', e);
      this.#ctx.reportError(e);
    }
  }

  async #persistDirtyBehaviorProps(): Promise<void> {
    const ids = [...this.#dirtyBehaviorProps];
    this.#dirtyBehaviorProps.clear();
    await Promise.all(ids.map((id) => this.#persistBehaviorProps(id)));
  }

  #canRotate(ids: number[]): boolean {
    if (ids.length !== 1) return false;
    const object = this.#ctx.doc.getObject(ids[0]);
    return !!object && objectBehavior(object.kind).rotatable;
  }

  #persistedGroupIdFor(ids: number[]): number | null {
    return this.#ctx.doc.groupIdForSelection(ids);
  }

  #setBehaviorShapeStyle(id: number, props: Record<string, unknown>): void {
    const view = this.#ctx.objectView(id);
    if (!view) return;
    const shapeStyle = objectBehavior(view.kind).shapeStyle(props);
    if (shapeStyle === null) return;
    this.#ctx.doc.setObjectStyles(id, {
      objectStyle: view.objectStyle,
      textStyle: view.textStyle,
      shapeStyle,
    });
  }

  #syncObjectToBox(id: number): void {
    const o = this.#ctx.doc.getObject(id);
    if (!o) return;
    const behavior = objectBehavior(o.kind);
    const next = behavior.syncGeometry(o);
    if (!next) return;
    this.#ctx.doc.setObjectProps(id, JSON.stringify(next));
    this.#setBehaviorShapeStyle(id, next);
    if (behavior.persistAfterResize) this.#dirtyBehaviorProps.add(id);
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
    this.#lifecycle.destroy();
    this.#clearSmartGuides();
    window.removeEventListener('pointerup', this.#onObjectClickUp);
    window.removeEventListener('pointerdown', this.#onPointerDown, { capture: true });
    window.removeEventListener('pointermove', this.#onDragPointerMove, { capture: true });
    this.#clearPendingMoves();
    if (this.#rectFrame !== null) cancelAnimationFrame(this.#rectFrame);
    this.#moveable.destroy();
    this.#selecto.destroy();
  }
}
