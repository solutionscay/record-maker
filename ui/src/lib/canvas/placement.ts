// Placement controller (#46/#48, split in #135): the draw-to-create gesture an
// armed rail tool starts, and the field drag-and-drop pipeline from the picker
// (#79 follow-up). Owns the draw/drop previews and the `ctx.placing` in-flight
// guard for object creation (shared with the clipboard controller's clones).

import type { ToolKind } from '../doc.svelte';
import type { ObjectView } from '../model';
import { GRID, clampOrigin, snapToGrid } from '../canvas-edit';
import { defaultBox, defaultProps, partAtY } from '../create';
import { createObject } from '../persist';
import { FIELD_DRAG_MIME } from '../dnd';
import { llog, lerror } from '../log';
import { lineAngle } from '../object-props';
import type { CanvasContext } from './context';

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

  get isDrawing(): boolean {
    return this.#drawing !== null;
  }

  /** Start a draw-to-create gesture. Release persists the final box; a very short
   * click falls back to the tool's default size, but creation still waits for
   * pointer-up so objects are not dropped on press. */
  startDraw(input: MouseEvent | PointerEvent): void {
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
    this.#ctx.placeOverlay(this.#drawPreview, drawing.box, drawing.partTop);
    this.#drawPreview.style.transform = '';
  }

  /** Persist the drawn object and add the returned view(s) to the store as one
   * undoable create step. A `field` adds both its value object and spawned label. */
  async #finishDraw(): Promise<void> {
    const drawing = this.#drawing;
    if (!drawing || this.#ctx.placing) return;
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
          content: tool === 'text' ? 'Text' : null,
          // A portal binds the armed relationship route on placement (#168);
          // the server stores it in the object's `binding` slot.
          binding: tool === 'portal' ? this.#ctx.doc.toolRoute : null,
          props: this.#placementProps(tool, drawing, finalBox),
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

  #placementProps(tool: DrawTool, drawing: DrawPlacement, box: { w: number }): Record<string, unknown> | null {
    const base = defaultProps(tool);
    if (tool !== 'line') return base ?? null;
    const line = drawing.dragged && drawing.line ? drawing.line : { angle: 0, length: Math.max(1, box.w) };
    return { ...(base ?? {}), angle: line.angle, length: line.length };
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
    const x = clampOrigin(snapToGrid(point.x));
    const y = clampOrigin(snapToGrid(where.localY));
    return {
      partId: where.partId,
      partTop: point.y - where.localY,
      partHeight: part.height,
      box: { x, y, w: size.w, h: Math.min(size.h, Math.max(1, part.height - y)) },
    };
  }

  onDragOver = (e: DragEvent): void => {
    if (!e.dataTransfer?.types.includes(FIELD_DRAG_MIME)) return;
    e.preventDefault(); // required for `drop` to fire on this target at all
    e.dataTransfer.dropEffect = 'copy';
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

  async #createFieldObjectsAt(target: FieldPlacementTarget, fieldIds: number[]): Promise<ObjectView[]> {
    const { partId, partHeight, box } = target;
    const rowStep = Math.max(32, box.h + GRID);
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
    window.removeEventListener('pointermove', this.#onDrawMove);
    window.removeEventListener('pointerup', this.#onDrawUp);
    window.removeEventListener('mousemove', this.#onDrawMove);
    window.removeEventListener('mouseup', this.#onDrawUp);
    this.#drawPreview?.remove();
    this.#dropPreview?.remove();
  }
}
