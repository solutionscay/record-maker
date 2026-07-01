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

import type { EditorDoc, ToolKind } from './doc.svelte';
import type { ObjectView } from './model';
import { GRID, SNAP_THRESHOLD, clampOrigin, elementsToObjectIds, objectIdsInPaintOrder, snapToGrid } from './canvas-edit';
import { defaultBox, defaultProps, partAtY } from './create';
import { createObject, deleteObject, setObjectContent } from './persist';
import { llog, lerror } from './log';

type DrawTool = Exclude<ToolKind, 'pointer'>;

interface DrawPlacement {
  tool: DrawTool;
  fieldId: number | null;
  partId: number;
  partTop: number;
  partHeight: number;
  startX: number;
  startY: number;
  dragged: boolean;
  box: { x: number; y: number; w: number; h: number };
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
  /** Active draw-to-create gesture while a non-pointer tool is armed. */
  #drawing: DrawPlacement | null = null;
  #drawPreview: HTMLElement | null = null;
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

  constructor(stage: HTMLElement, doc: EditorDoc, layoutId: string) {
    this.#stage = stage;
    this.#doc = doc;
    this.#layoutId = layoutId;

    this.#moveable = new Moveable(stage, {
      target: [],
      draggable: true,
      resizable: true,
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
      // moveable's own control box (a resize handle / the drag area) → its gesture.
      if (target && this.#moveable.isMoveableElement(target)) {
        llog('drag', 'press on moveable control box → moveable owns gesture');
        e.stop();
        return;
      }
      const objEl = (target?.closest('.fm-obj') ?? null) as HTMLElement | null;
      if (!objEl) {
        llog('select', 'press on empty canvas → marquee');
        return; // empty canvas → selecto runs its marquee
      }
      const id = this.#idForElement(objEl);
      if (id === undefined) {
        llog('target', 'press on object but id UNRESOLVED', { painted: this.#paintedElements().length });
        return;
      }
      // Shift toggles selection membership without starting a drag.
      if (input.shiftKey) {
        llog('select', 'shift-toggle membership', { id });
        this.#doc.toggle(id);
        e.stop();
        this.#updateTarget();
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
      const guidelines = this.#paintedElements().filter((el) => el !== objEl);
      this.#moveable.setState({ target: objEl, elementGuidelines: guidelines }, () => {
        this.#moveable.dragStart(input, objEl);
      });
      e.stop();
    });

    this.#stage.addEventListener('pointermove', this.#onPointerMove);
    this.#stage.addEventListener('pointerleave', this.#onPointerLeave);
    this.#stage.addEventListener('click', this.#onClick);
    this.#stage.addEventListener('dblclick', this.#onDoubleClick);
    window.addEventListener('keydown', this.#onKeyDown);
    llog('init', 'CanvasInteraction ready', { layoutId, painted: this.#paintedElements().length });
  }

  /** Reconcile moveable's target with the store selection (called reactively when
   * selection or geometry changes — e.g. after an undo). No-op during a gesture. */
  refresh(): void {
    this.#updateTarget();
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
    const fieldId = tool === 'field' ? this.#doc.toolFieldId : null;
    if (tool === 'field' && fieldId == null) {
      llog('place', 'field tool armed but no field chosen — nothing to draw');
      this.#doc.setTool('pointer');
      return;
    }
    this.#drawing = {
      tool,
      fieldId,
      partId: where.partId,
      partTop: point.y - where.localY,
      partHeight: part.height,
      startX: point.x,
      startY: point.y,
      dragged: false,
      box: { x: point.x, y: where.localY, w: 1, h: 1 },
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
      x = Math.min(drawing.startX, endX);
      yGlobal = drawing.startY;
      w = Math.max(8, Math.abs(endX - drawing.startX));
      h = Math.max(2, Math.min(defaultBox(drawing.tool).h, Math.abs(endY - drawing.startY) || defaultBox(drawing.tool).h));
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
    this.#drawPreview.style.left = `${drawing.box.x}px`;
    this.#drawPreview.style.top = `${drawing.partTop + drawing.box.y}px`;
    this.#drawPreview.style.width = `${drawing.box.w}px`;
    this.#drawPreview.style.height = `${drawing.box.h}px`;
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
        const fieldId = drawing.fieldId;
        if (fieldId == null) {
          llog('place', 'field draw finished but no field chosen — nothing to create');
          return;
        }
        views = await createObject(this.#layoutId, {
          partId,
          kind: 'field',
          x: finalBox.x,
          y: finalBox.y,
          w: finalBox.w,
          h: finalBox.h,
          fieldId,
          createLabel: this.#doc.toolCreateLabel,
          rec: this.#doc.rec,
        });
      } else {
        views = await createObject(this.#layoutId, {
          partId,
          kind: tool,
          x: finalBox.x,
          y: finalBox.y,
          w: finalBox.w,
          h: finalBox.h,
          content: tool === 'text' ? 'Text' : null,
          props: defaultProps(tool) ?? null,
          rec: this.#doc.rec,
        });
      }
      llog('create', 'server created object(s)', {
        objects: views.map((v) => ({ id: v.id, kind: v.kind, x: v.x, y: v.y, w: v.w, h: v.h })),
      });
      for (const v of views) this.#doc.addObject(v, partId);
      this.#doc.mark();
      const placed = views.at(-1); // the field VALUE (its label sorts before it)
      if (placed) {
        this.#doc.selectOnly([placed.id]);
        // The cursor now sits over the freshly-placed object, so make it the hover
        // too: otherwise `#updateTarget` prefers a stale hover from before create.
        this.#hoverId = placed.id;
        llog('place', 'added to store + selected + hover pinned to placed', { selectedId: placed.id });
      }
    } catch (e) {
      lerror('place', 'create failed', e);
      this.#doc.setError(e instanceof Error ? e.message : String(e));
    } finally {
      this.#placing = false;
      this.#doc.setTool('pointer');
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
    this.#drawPreview?.remove();
    this.#hoverOutline?.remove();
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
    if (this.#gesturing || this.#doc.activeTool !== 'pointer') return;
    const target = e.target as Element | null;
    if (!target || this.#moveable.isMoveableElement(target)) return;
    if (target.closest('.fm-obj') || target.closest('.le-part-label, .le-part-resize')) return;

    const partEl = (target.closest('.fm-part') ?? null) as HTMLElement | null;
    if (!partEl) {
      this.#doc.clearSelection();
      return;
    }
    const id = this.#partIdForElement(partEl);
    if (id === undefined) {
      llog('target', 'click on part but id UNRESOLVED', { parts: this.#partElements().length });
      return;
    }
    this.#hoverId = null;
    this.#paintHover();
    this.#doc.selectPart(id);
    this.#updateTarget();
  };

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
    if (e.key !== 'Delete' && e.key !== 'Backspace') return;
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;
    if (this.#doc.selection.size === 0 || this.#deleting) return;
    e.preventDefault();
    void this.#deleteSelectedObjects();
  };

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

  /** Choose moveable's target from the real selection only. Hover uses a separate
   * lightweight outline, so resize handles never appear on unselected objects. */
  #updateTarget(): void {
    if (this.#gesturing) return;
    // A placement tool is armed → the canvas is a drawing surface, not a select/
    // drag surface: drop moveable's target so a press places instead of grabs.
    if (this.#doc.activeTool !== 'pointer') {
      if (this.#targetKey === '') return;
      this.#targetKey = '';
      this.#targetIds = new Set();
      this.#moveable.setState({ target: null, elementGuidelines: [] }, () => this.#moveable.forceUpdate());
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
      this.#moveable.setState({ target: null, elementGuidelines: [] }, () => this.#moveable.forceUpdate());
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
    this.#moveable.setState({ target: targets, elementGuidelines: guidelines });
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
    const top = o ? this.#partTop(o.partId) : null;
    const overlay = this.#partOverlay();
    if (!o || top === null || !overlay) return;
    this.#textEditor?.remove();
    this.#textEditingId = id;
    this.#paintHover();
    const editor = document.createElement('textarea');
    editor.className = 'le-inline-text-editor';
    editor.value = o.content;
    editor.style.left = `${o.x}px`;
    editor.style.top = `${top + o.y}px`;
    editor.style.width = `${o.w}px`;
    editor.style.height = `${o.h}px`;
    overlay.append(editor);
    this.#textEditor = editor;
    const finish = (commit: boolean) => {
      if (this.#textEditor !== editor) return;
      const next = editor.value;
      editor.remove();
      this.#textEditor = null;
      this.#textEditingId = null;
      if (commit && next !== o.content) void this.#commitTextEdit(id, next);
      this.#paintHover();
    };
    editor.addEventListener('blur', () => finish(true), { once: true });
    editor.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        finish(false);
      } else if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        finish(true);
      }
    });
    editor.focus();
    editor.select();
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
  }

  /** End a gesture: if it actually changed geometry, seal one undo step and
   * persist the moved/resized group; a no-move click does neither. Then re-target. */
  #end(kind: 'drag' | 'resize' = 'drag'): void {
    this.#gesturing = false;
    llog(kind, `${kind}End`, { moved: this.#moved, selection: [...this.#doc.selection] });
    if (this.#moved) {
      this.#doc.mark();
      void this.#persistSelection();
    }
    this.#targetKey = ''; // force a re-sync after the gesture
    this.#resizeStarts.clear();
    this.#scheduleRectUpdate();
    this.#updateTarget();
  }

  /** Make the dragged/resized single target the selection (if it wasn't already). */
  #selectFromTarget(el: HTMLElement | SVGElement): void {
    const id = this.#idForElement(el);
    if (id !== undefined && !this.#doc.isSelected(id)) this.#doc.selectOnly([id]);
  }

  #applyMove(target: HTMLElement | SVGElement, left: number, top: number): void {
    const id = this.#idForElement(target);
    if (id === undefined) {
      llog('target', 'drag: target element has NO mapped id — move is a no-op', {
        painted: this.#paintedElements().length,
      });
      return;
    }
    this.#moved = true;
    this.#doc.setObjectGeometry(id, { x: clampOrigin(left), y: clampOrigin(top) });
    this.#scheduleRectUpdate();
    llog('drag', 'apply move', { id, x: clampOrigin(left), y: clampOrigin(top) });
  }

  #captureResizeStart(target: HTMLElement | SVGElement, direction: number[], inputEvent: Event | undefined): void {
    const id = this.#idForElement(target);
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
    const id = this.#idForElement(target);
    if (id === undefined) {
      llog('target', 'resize: target element has NO mapped id — resize is a no-op', {
        painted: this.#paintedElements().length,
        paintOrderIds: objectIdsInPaintOrder(this.#doc.renderModel),
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
    this.#scheduleRectUpdate();
    llog('resize', 'apply resize', {
      id,
      w: Math.max(1, Math.round(width)),
      h: Math.max(1, Math.round(height)),
      x: clampOrigin(left),
      y: clampOrigin(top),
    });
  }

  // ── persistence (#46 bulk axum contract) ──

  async #persistSelection(): Promise<void> {
    const items = [...this.#doc.selection]
      .map((id) => this.#doc.getObject(id))
      .filter((o): o is NonNullable<typeof o> => !!o)
      .map((o) => ({ id: o.id, x: o.x, y: o.y, w: o.w, h: o.h }));
    if (items.length === 0) return;
    llog('persist', 'POST geometry', { items });
    try {
      const r = await fetch(`/design/${this.#layoutId}/geometry`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(items),
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      llog('persist', 'geometry saved', { count: items.length });
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

  #partElements(): HTMLElement[] {
    const canvas = this.#canvas();
    return canvas ? Array.from(canvas.querySelectorAll<HTMLElement>('.fm-part')) : [];
  }

  #elementsToIds(elements: Array<HTMLElement | SVGElement>): number[] {
    return elementsToObjectIds(elements, this.#paintedElements(), objectIdsInPaintOrder(this.#doc.renderModel));
  }

  #elementForId(id: number): HTMLElement | undefined {
    const ids = objectIdsInPaintOrder(this.#doc.renderModel);
    const i = ids.indexOf(id);
    return i >= 0 ? this.#paintedElements()[i] : undefined;
  }

  #idForElement(el: Element): number | undefined {
    const i = this.#paintedElements().indexOf(el as HTMLElement);
    if (i < 0) return undefined;
    return objectIdsInPaintOrder(this.#doc.renderModel)[i];
  }

  #partIdForElement(el: Element): number | undefined {
    const i = this.#partElements().indexOf(el as HTMLElement);
    if (i < 0) return undefined;
    return this.#doc.renderModel.parts[i]?.id;
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
