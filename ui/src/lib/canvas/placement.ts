// Placement controller (#46/#48, split in #135): the draw-to-create gesture an
// armed rail tool starts, and the field drag-and-drop pipeline from the picker
// (#79 follow-up). Owns the draw/drop previews and the `ctx.placing` in-flight
// guard for object creation (shared with the clipboard controller's clones).

import type { ToolKind } from '../doc.svelte';
import type { ObjectView } from '../model';
import { clampOrigin, snapToGrid } from '../canvas-edit';
import { defaultBox, partAtY } from '../create';
import { createObject } from '../persist';
import { FIELD_DRAG_MIME, PORTAL_COLUMN_DRAG_MIME, type PortalColumnDrag } from '../dnd';
import { llog, lerror } from '../log';
import type { CanvasContext } from './context';
import { GestureLifecycle, type GestureCancelReason } from './gesture-lifecycle';
import { objectBehavior } from './object-behavior';

type DrawTool = Exclude<ToolKind, 'pointer'>;
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

export class PlacementController {
  readonly #ctx: CanvasContext;
  readonly #lifecycle = new GestureLifecycle('draw-placement');
  /** Active draw-to-create gesture while a non-pointer tool is armed. */
  #drawing: DrawPlacement | null = null;
  #drawPreview: HTMLElement | null = null;
  /** Ghost box tracking a field drag-and-drop from the picker (see onDragOver);
   * separate from #drawPreview since a drop can land while no tool is armed and
   * no #drawing gesture is in progress. */
  #dropPreview: HTMLElement | null = null;

  constructor(ctx: CanvasContext) {
    this.#ctx = ctx;
  }

  #snap(value: number): number {
    const doc = this.#ctx.doc;
    return snapToGrid(value, doc.snapToGrid ? doc.gridSize : 0);
  }

  #step(): number {
    return this.#ctx.doc.snapToGrid ? this.#ctx.doc.gridSize : 1;
  }

  get isDrawing(): boolean {
    return this.#drawing !== null;
  }

  /** Start a draw-to-create gesture. Release persists the final box; a very short
   * click falls back to the tool's default size, but creation still waits for
   * pointer-up so objects are not dropped on press. */
  startDraw(input: MouseEvent | PointerEvent, pointerId?: number): void {
    const doc = this.#ctx.doc;
    const tool = doc.activeTool;
    if (tool === 'pointer' || this.#ctx.placing || this.#drawing) {
      llog('place', 'draw start ignored', { tool, placing: this.#ctx.placing, drawing: !!this.#drawing });
      return;
    }
    const point = this.#ctx.canvasPoint(input.clientX, input.clientY);
    if (!point) {
      llog('error', 'draw start: no .fm-canvas in stage');
      doc.setTool('pointer');
      return;
    }
    const where = partAtY(doc.renderModel, point.y);
    if (!where) {
      llog('place', 'no part under draw start', { modelY: Math.round(point.y) });
      doc.setTool('pointer');
      return;
    }
    const part = doc.getPart(where.partId);
    if (!part) return;
    const fieldIds = tool === 'field' ? doc.toolFieldIds.slice() : [];
    if (tool === 'field' && fieldIds.length === 0) {
      llog('place', 'field tool armed but no field chosen — nothing to draw');
      doc.setTool('pointer');
      return;
    }
    if (tool === 'portal' && !doc.toolRoute) {
      llog('place', 'portal tool armed but no route chosen — nothing to draw');
      doc.setTool('pointer');
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
    this.#ctx.partOverlay()?.append(this.#drawPreview);
    this.#updateDraw(input);
    this.#ctx.gesturing = true;
    this.#lifecycle.begin({
      inputEvent: input,
      pointerId,
      captureTarget: this.#ctx.stage,
      onCancel: (reason) => this.#cancelDraw(reason),
    });
    window.addEventListener('pointermove', this.#onDrawMove);
    window.addEventListener('pointerup', this.#onDrawUp, { once: true });
    llog('place', 'draw start', {
      tool,
      partId: where.partId,
      startX: Math.round(point.x),
      startY: Math.round(where.localY),
      fieldCount: fieldIds.length,
    });
  }

  #onDrawMove = (e: PointerEvent | MouseEvent): void => {
    if (e instanceof PointerEvent && !this.#lifecycle.owns(e)) return;
    this.#updateDraw(e);
  };

  #onDrawUp = (e: PointerEvent | MouseEvent): void => {
    if (e instanceof PointerEvent && !this.#lifecycle.owns(e)) return;
    this.#updateDraw(e);
    this.#lifecycle.commit();
    void this.#finishDraw();
  };

  #updateDraw(input: MouseEvent | PointerEvent): void {
    const drawing = this.#drawing;
    const point = this.#ctx.canvasPoint(input.clientX, input.clientY);
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
    } else {
      const geometry = objectBehavior(drawing.tool).drawGeometry({
        startX: drawing.startX,
        startY: drawing.startY,
        endX,
        endY,
        snap: (value) => this.#snap(value),
      });
      ({ x, yGlobal, w, h } = geometry);
      drawing.line = geometry.line;
    }

    x = this.#snap(x);
    yGlobal = this.#snap(yGlobal);
    w = Math.max(1, this.#snap(w));
    h = Math.max(1, this.#snap(h));
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
    const frame = { dragged: drawing.dragged, box: drawing.box, partTop: drawing.partTop, line: drawing.line };
    const style = objectBehavior(drawing.tool).previewStyle(frame);
    if (style) {
      this.#drawPreview.style.left = `${style.left}px`;
      this.#drawPreview.style.top = `${style.top}px`;
      this.#drawPreview.style.width = `${style.width}px`;
      this.#drawPreview.style.height = `${style.height}px`;
      this.#drawPreview.style.transform = style.transform;
      return;
    }
    this.#ctx.placeOverlay(this.#drawPreview, drawing.box, drawing.partTop);
    this.#drawPreview.style.transform = '';
  }

  /** Persist the drawn object and add the returned view(s) to the store as one
   * undoable create step. A `field` adds both its value object and spawned label. */
  async #finishDraw(): Promise<void> {
    const drawing = this.#drawing;
    if (!drawing || this.#ctx.placing) return;
    this.#clearDrawGesture();

    const { tool, partId } = drawing;
    const finalBox = drawing.dragged ? drawing.box : this.#defaultPlacementBox(drawing);
    llog('place', 'draw finish', { tool, partId, dragged: drawing.dragged, ...finalBox });

    this.#ctx.placing = true;
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
        views = await createObject(this.#ctx.layoutId, {
          partId,
          kind: tool,
          x: finalBox.x,
          y: finalBox.y,
          w: finalBox.w,
          h: finalBox.h,
          content: objectBehavior(tool).defaultContent,
          // A portal binds the armed relationship route on placement (#168);
          // the server stores it in the object's `binding` slot.
          binding: tool === 'portal' ? this.#ctx.doc.toolRoute : null,
          props: objectBehavior(tool).placementProps({
            dragged: drawing.dragged,
            box: finalBox,
            partTop: drawing.partTop,
            line: drawing.line,
          }),
          rec: this.#ctx.doc.rec,
        });
      }
      llog('create', 'server created object(s)', {
        objects: views.map((v) => ({ id: v.id, kind: v.kind, x: v.x, y: v.y, w: v.w, h: v.h })),
      });
      const placed = views.at(-1); // the field VALUE (its label sorts before it)
      const selectedIds = tool === 'field' ? views.map((v) => v.id) : placed ? [placed.id] : [];
      const committed = this.#commitPlacedViews(partId, views, selectedIds, 'draw');
      if (committed && tool === 'text') {
        this.#ctx.text.start(committed.id);
      }
    } catch (e) {
      lerror('place', 'create failed', e);
      this.#ctx.reportError(e);
    } finally {
      this.#ctx.placing = false;
      if (this.#ctx.doc.activeTool !== 'pointer') this.#ctx.doc.setTool('pointer');
    }
  }

  #cancelDraw(reason: GestureCancelReason): void {
    if (!this.#drawing) return;
    this.#clearDrawGesture();
    llog('place', 'draw cancelled', { reason });
  }

  #clearDrawGesture(): void {
    window.removeEventListener('pointermove', this.#onDrawMove);
    window.removeEventListener('pointerup', this.#onDrawUp);
    this.#drawPreview?.remove();
    this.#drawPreview = null;
    this.#drawing = null;
    this.#ctx.gesturing = false;
  }

  #defaultPlacementBox(drawing: DrawPlacement): { x: number; y: number; w: number; h: number } {
    const size = defaultBox(drawing.tool);
    const x = clampOrigin(this.#snap(drawing.startX));
    const y = clampOrigin(this.#snap(drawing.startY - drawing.partTop));
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
    const point = this.#ctx.canvasPoint(clientX, clientY);
    if (!point) return null;
    const where = partAtY(this.#ctx.doc.renderModel, point.y);
    if (!where) return null;
    const part = this.#ctx.doc.getPart(where.partId);
    if (!part) return null;
    const size = defaultBox('field');
    const x = clampOrigin(this.#snap(point.x));
    const y = clampOrigin(this.#snap(where.localY));
    return {
      partId: where.partId,
      partTop: point.y - where.localY,
      partHeight: part.height,
      box: { x, y, w: size.w, h: Math.min(size.h, Math.max(1, part.height - y)) },
    };
  }

  onDragOver = (e: DragEvent): void => {
    // Both a base-field drag (rail "Field to place") and a portal-column drag
    // (portal inspector Columns picker, #168) paint the same drop preview; they
    // diverge only at drop, keyed on which MIME the payload carried.
    const types = e.dataTransfer?.types;
    if (!types || (!types.includes(FIELD_DRAG_MIME) && !types.includes(PORTAL_COLUMN_DRAG_MIME))) return;
    e.preventDefault(); // required for `drop` to fire on this target at all
    e.dataTransfer!.dropEffect = 'copy';
    this.#paintDropPreview(this.#dropTargetFor(e.clientX, e.clientY));
  };

  onDragLeave = (e: DragEvent): void => {
    // `dragleave` also fires when moving between child elements WITHIN the
    // stage (e.g. crossing from the canvas onto a part label); only clear the
    // preview once the pointer has actually left the stage entirely.
    const related = e.relatedTarget as Node | null;
    if (!related || !this.#ctx.stage.contains(related)) this.#paintDropPreview(null);
  };

  onDrop = (e: DragEvent): void => {
    this.#paintDropPreview(null);
    // A portal-column drag (#168) takes priority: the same picker gesture, but the
    // dragged related fields become COLUMNS of the payload's portal (parent-aware
    // create), not top-level base-field objects.
    const portalRaw = e.dataTransfer?.getData(PORTAL_COLUMN_DRAG_MIME);
    if (portalRaw) {
      e.preventDefault();
      const payload = this.#parsePortalColumnDrag(portalRaw);
      if (payload) void this.#placePortalColumnsAt(payload, e.clientX, e.clientY);
      return;
    }
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

  #parsePortalColumnDrag(raw: string): PortalColumnDrag | null {
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return null;
    }
    if (typeof parsed !== 'object' || parsed === null) return null;
    const { portalId, route, fieldIds } = parsed as Record<string, unknown>;
    if (typeof portalId !== 'number' || typeof route !== 'string') return null;
    if (!Array.isArray(fieldIds) || fieldIds.length === 0 || !fieldIds.every((v) => typeof v === 'number')) return null;
    return { portalId, route, fieldIds: fieldIds as number[] };
  }

  #paintDropPreview(target: { partTop: number; box: { x: number; y: number; w: number; h: number } } | null): void {
    if (!target) {
      this.#dropPreview?.remove();
      this.#dropPreview = null;
      return;
    }
    if (!this.#dropPreview) {
      this.#dropPreview = document.createElement('div');
      this.#dropPreview.className = 'le-draw-preview le-draw-field';
      this.#ctx.partOverlay()?.append(this.#dropPreview);
    }
    this.#ctx.placeOverlay(this.#dropPreview, target.box, target.partTop);
  }

  async #placeFieldsAt(target: FieldPlacementTarget, fieldIds: number[]): Promise<void> {
    if (this.#ctx.placing) return;
    this.#ctx.placing = true;
    try {
      const views = await this.#createFieldObjectsAt(target, fieldIds);
      llog('create', 'server created object(s) via drop', {
        objects: views.map((v) => ({ id: v.id, kind: v.kind, x: v.x, y: v.y, w: v.w, h: v.h })),
      });
      this.#commitPlacedViews(target.partId, views, views.map((v) => v.id), 'drop');
    } catch (e) {
      lerror('place', 'drop create failed', e);
      this.#ctx.reportError(e);
    } finally {
      this.#ctx.placing = false;
      if (this.#ctx.doc.activeTool !== 'pointer') this.#ctx.doc.setTool('pointer');
    }
  }

  /** Create the dragged related fields as COLUMNS of the payload's portal (#168) —
   * the drop-side of the portal inspector's Columns picker. Same parent-aware
   * create the click-append used (parentObjectId = portal, so the server builds
   * the route-relative `<route>.<field>` binding and spawns each column's top
   * header label), but positioned from the DROP point: the drop x maps to the
   * column's x — and since columns render left→right by x, that x is the column
   * ORDER. Multiple fields dropped at once step to the right so they land as
   * successive columns. Columns share the portal's header row (y = portal.y),
   * matching the click-append geometry. The portal stays selected afterwards so
   * its Columns list/card stays open for the next add. */
  async #placePortalColumnsAt(payload: PortalColumnDrag, clientX: number, clientY: number): Promise<void> {
    if (this.#ctx.placing) return;
    const doc = this.#ctx.doc;
    const portal = doc.getObject(payload.portalId);
    if (!portal || portal.kind !== 'portal') {
      llog('place', 'portal-column drop: portal missing or not a portal — ignored', { portalId: payload.portalId });
      return;
    }
    const point = this.#ctx.canvasPoint(clientX, clientY);
    const size = defaultBox('field');
    const baseX = point ? clampOrigin(this.#snap(point.x)) : portal.x;
    const y = portal.y;
    this.#ctx.placing = true;
    try {
      const views = (
        await Promise.all(
          payload.fieldIds.map((fieldId, i) =>
            createObject(this.#ctx.layoutId, {
              partId: portal.partId,
              kind: 'field',
              x: baseX + i * size.w,
              y,
              w: size.w,
              h: size.h,
              fieldId,
              createLabel: true,
              parentObjectId: portal.id,
              rec: doc.rec,
            }),
          ),
        )
      ).flat();
      llog('create', 'server created portal column(s) via drop', {
        portal: portal.id,
        objects: views.map((v) => ({ id: v.id, kind: v.kind, x: v.x, y: v.y })),
      });
      for (const v of views) doc.addObject(v, portal.partId);
      doc.mark();
      doc.setTool('pointer');
      // Keep the PORTAL selected (not the new columns) so its inspector Columns
      // card stays open for the next drag — the inspector-authoring counterpart of
      // how base-field placement re-selects what it just placed.
      doc.selectOnly([portal.id]);
    } catch (e) {
      lerror('place', 'portal-column drop create failed', e);
      this.#ctx.reportError(e);
    } finally {
      this.#ctx.placing = false;
      if (this.#ctx.doc.activeTool !== 'pointer') this.#ctx.doc.setTool('pointer');
    }
  }

  async #createFieldObjectsAt(target: FieldPlacementTarget, fieldIds: number[]): Promise<ObjectView[]> {
    const { partId, partHeight, box } = target;
    const rowStep = Math.max(32, box.h + this.#step());
    // The Field tool always places PRIMARY/base-table fields as top-level objects.
    // Portal columns are authored from the portal inspector's Columns picker (#168),
    // which POSTs the create route with `parentObjectId` directly — the canvas
    // placement pipeline never creates portal-column children.
    const batches = await Promise.all(
      fieldIds.map((fieldId, i) => {
        const y = Math.min(partHeight - 1, box.y + i * rowStep);
        return createObject(this.#ctx.layoutId, {
          partId,
          kind: 'field',
          x: box.x,
          y,
          w: box.w,
          h: Math.min(box.h, Math.max(1, partHeight - y)),
          fieldId,
          createLabel: this.#ctx.doc.toolCreateLabel,
          rec: this.#ctx.doc.rec,
        });
      }),
    );
    return batches.flat();
  }

  #commitPlacedViews(partId: number, views: ObjectView[], selectedIds: number[], source: string): ObjectView | undefined {
    const doc = this.#ctx.doc;
    for (const v of views) doc.addObject(v, partId);
    doc.mark();
    doc.setTool('pointer');
    const placed = views.at(-1);
    if (!placed) return undefined;
    doc.selectOnly(selectedIds);
    this.#ctx.hover.pin(placed.id);
    llog('place', 'added to store + selected + hover pinned to placed', {
      source,
      selectedIds,
      hoverId: placed.id,
    });
    return placed;
  }

  destroy(): void {
    this.#lifecycle.destroy();
    this.#clearDrawGesture();
    this.#dropPreview?.remove();
  }
}
