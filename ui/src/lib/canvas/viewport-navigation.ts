import type { ViewportCommandKind } from '../doc.svelte';
import { llog } from '../log';
import type { CanvasContext } from './context';

const FIT_GUTTER_PX = 32;

function editableTarget(target: EventTarget | null): boolean {
  const element = target instanceof Element ? target : null;
  return !!element?.closest('input, textarea, select, [contenteditable="true"], .le-inline-text-editor');
}

export class ViewportNavigationController {
  readonly #ctx: CanvasContext;
  #spaceHeld = false;
  #pan: {
    pointerId: number;
    startX: number;
    startY: number;
    scrollLeft: number;
    scrollTop: number;
    moved: boolean;
  } | null = null;
  #wheelFrame: number | null = null;
  #wheelDelta = 0;
  #wheelPoint = { x: 0, y: 0 };
  #zoomFrame: number | null = null;
  #suppressClick = false;

  constructor(ctx: CanvasContext) {
    this.#ctx = ctx;
    window.addEventListener('keydown', this.#onKeyDown, true);
    window.addEventListener('keyup', this.#onKeyUp, true);
    window.addEventListener('pointerdown', this.#onPointerDown, true);
    ctx.stage.addEventListener('wheel', this.#onWheel, { passive: false });
  }

  run(command: ViewportCommandKind): void {
    if (this.#ctx.gesturing) return;
    if (command === 'actual-size' || command === 'zoom-in' || command === 'zoom-out') {
      const rect = this.#ctx.stage.getBoundingClientRect();
      const next = command === 'actual-size'
        ? 1
        : this.#ctx.doc.zoom + (command === 'zoom-in' ? 0.1 : -0.1);
      this.#zoomAt(next, rect.left + rect.width / 2, rect.top + rect.height / 2);
      return;
    }
    const target = command === 'fit-selection' ? this.#selectionUnion() : this.#ctx.canvas()?.getBoundingClientRect();
    if (!target) return;
    const currentZoom = this.#ctx.doc.zoom || 1;
    const modelWidth = target.width / currentZoom;
    const modelHeight = target.height / currentZoom;
    const availableWidth = Math.max(1, this.#ctx.stage.clientWidth - FIT_GUTTER_PX * 2);
    const availableHeight = Math.max(1, this.#ctx.stage.clientHeight - FIT_GUTTER_PX * 2);
    const next = Math.min(availableWidth / modelWidth, availableHeight / modelHeight);
    this.#zoomAndCenter(next, command);
  }

  destroy(): void {
    this.#finishPan(false);
    window.removeEventListener('keydown', this.#onKeyDown, true);
    window.removeEventListener('keyup', this.#onKeyUp, true);
    window.removeEventListener('pointerdown', this.#onPointerDown, true);
    this.#ctx.stage.removeEventListener('wheel', this.#onWheel);
    if (this.#wheelFrame !== null) cancelAnimationFrame(this.#wheelFrame);
    if (this.#zoomFrame !== null) cancelAnimationFrame(this.#zoomFrame);
    window.removeEventListener('click', this.#onClick, true);
  }

  #onWheel = (event: WheelEvent): void => {
    if (!(event.ctrlKey || event.metaKey) || this.#ctx.gesturing || editableTarget(event.target)) return;
    event.preventDefault();
    this.#wheelDelta += event.deltaY;
    this.#wheelPoint = { x: event.clientX, y: event.clientY };
    if (this.#wheelFrame !== null) return;
    this.#wheelFrame = requestAnimationFrame(() => {
      this.#wheelFrame = null;
      const delta = this.#wheelDelta;
      this.#wheelDelta = 0;
      const next = this.#ctx.doc.zoom * Math.exp(-delta * 0.002);
      llog('zoom', 'viewport: modifier wheel', { delta, from: this.#ctx.doc.zoom, to: next });
      this.#zoomAt(next, this.#wheelPoint.x, this.#wheelPoint.y);
    });
  };

  #onKeyDown = (event: KeyboardEvent): void => {
    if (event.key === 'Escape' && this.#pan) {
      event.preventDefault();
      event.stopImmediatePropagation();
      this.#finishPan(true);
      return;
    }
    if (event.code !== 'Space' || event.repeat || editableTarget(event.target) || this.#ctx.gesturing) return;
    event.preventDefault();
    this.#spaceHeld = true;
    this.#ctx.stage.classList.add('is-pan-ready');
  };

  #onKeyUp = (event: KeyboardEvent): void => {
    if (event.code !== 'Space') return;
    this.#spaceHeld = false;
    this.#ctx.stage.classList.remove('is-pan-ready');
    if (this.#pan) this.#finishPan(true);
  };

  #onPointerDown = (event: PointerEvent): void => {
    const target = event.target instanceof Node ? event.target : null;
    if (!target || !this.#ctx.stage.contains(target) || this.#ctx.gesturing) return;
    const owns = event.button === 1 || (event.button === 0 && this.#spaceHeld);
    if (!owns || editableTarget(event.target)) return;
    const stageRect = this.#ctx.stage.getBoundingClientRect();
    const onScrollbar = event.target === this.#ctx.stage
      && (event.clientX >= stageRect.left + this.#ctx.stage.clientWidth
        || event.clientY >= stageRect.top + this.#ctx.stage.clientHeight);
    if (onScrollbar) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    this.#ctx.autoscroll.stop();
    this.#ctx.gesturing = true;
    this.#pan = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      scrollLeft: this.#ctx.stage.scrollLeft,
      scrollTop: this.#ctx.stage.scrollTop,
      moved: false,
    };
    this.#ctx.stage.classList.add('is-panning');
    try { this.#ctx.stage.setPointerCapture(event.pointerId); } catch {}
    window.addEventListener('pointermove', this.#onPanMove, true);
    window.addEventListener('pointerup', this.#onPanUp, true);
    window.addEventListener('pointercancel', this.#onPanUp, true);
    window.addEventListener('blur', this.#onBlur);
    this.#ctx.stage.addEventListener('lostpointercapture', this.#onCaptureLoss);
  };

  #onPanMove = (event: PointerEvent): void => {
    const pan = this.#pan;
    if (!pan || event.pointerId !== pan.pointerId) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const dx = event.clientX - pan.startX;
    const dy = event.clientY - pan.startY;
    pan.moved ||= Math.hypot(dx, dy) > 2;
    this.#ctx.stage.scrollTo(pan.scrollLeft - dx, pan.scrollTop - dy);
  };

  #onPanUp = (event: PointerEvent): void => {
    if (!this.#pan || event.pointerId !== this.#pan.pointerId) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    this.#finishPan(true);
  };

  #onCaptureLoss = (): void => this.#finishPan(true);

  #onBlur = (): void => {
    this.#spaceHeld = false;
    this.#ctx.stage.classList.remove('is-pan-ready');
    this.#finishPan(true);
  };

  #onClick = (event: MouseEvent): void => {
    window.removeEventListener('click', this.#onClick, true);
    if (!this.#suppressClick) return;
    this.#suppressClick = false;
    event.preventDefault();
    event.stopImmediatePropagation();
  };

  #finishPan(suppressMovedClick: boolean): void {
    const pan = this.#pan;
    if (!pan) return;
    if (suppressMovedClick && pan.moved) {
      this.#suppressClick = true;
      window.addEventListener('click', this.#onClick, true);
    }
    this.#pan = null;
    this.#ctx.gesturing = false;
    this.#ctx.stage.classList.remove('is-panning');
    window.removeEventListener('pointermove', this.#onPanMove, true);
    window.removeEventListener('pointerup', this.#onPanUp, true);
    window.removeEventListener('pointercancel', this.#onPanUp, true);
    window.removeEventListener('blur', this.#onBlur);
    this.#ctx.stage.removeEventListener('lostpointercapture', this.#onCaptureLoss);
    try {
      if (this.#ctx.stage.hasPointerCapture(pan.pointerId)) this.#ctx.stage.releasePointerCapture(pan.pointerId);
    } catch {}
  }

  #zoomAt(nextZoom: number, clientX: number, clientY: number): void {
    const workspace = this.#ctx.stage.querySelector<HTMLElement>('.le-workspace');
    if (!workspace) return;
    const oldZoom = this.#ctx.doc.zoom || 1;
    const oldRect = workspace.getBoundingClientRect();
    const modelX = (clientX - oldRect.left) / oldZoom;
    const modelY = (clientY - oldRect.top) / oldZoom;
    this.#ctx.doc.setZoom(nextZoom);
    const appliedZoom = this.#ctx.doc.zoom;
    llog('zoom', 'viewport: anchored zoom', { from: oldZoom, to: appliedZoom, clientX, clientY });
    this.#afterZoom(() => {
      const nextRect = workspace.getBoundingClientRect();
      this.#ctx.stage.scrollBy(
        nextRect.left + modelX * appliedZoom - clientX,
        nextRect.top + modelY * appliedZoom - clientY,
      );
    });
  }

  #zoomAndCenter(nextZoom: number, command: ViewportCommandKind): void {
    this.#ctx.doc.setZoom(nextZoom);
    this.#afterZoom(() => {
      const target = command === 'fit-selection' ? this.#selectionUnion() : this.#ctx.canvas()?.getBoundingClientRect();
      if (!target) return;
      const stageRect = this.#ctx.stage.getBoundingClientRect();
      const targetCenterX = target.left + target.width / 2;
      const targetCenterY = target.top + target.height / 2;
      this.#ctx.stage.scrollBy(
        targetCenterX - (stageRect.left + stageRect.width / 2),
        targetCenterY - (stageRect.top + stageRect.height / 2),
      );
    });
  }

  #afterZoom(adjust: () => void): void {
    if (this.#zoomFrame !== null) cancelAnimationFrame(this.#zoomFrame);
    this.#zoomFrame = requestAnimationFrame(() => {
      this.#zoomFrame = null;
      adjust();
      this.#ctx.transform.refresh();
    });
  }

  #selectionUnion(): DOMRect | null {
    const rects = [...this.#ctx.doc.selection]
      .map((id) => this.#ctx.elementForId(id)?.getBoundingClientRect())
      .filter((rect): rect is DOMRect => !!rect);
    if (rects.length === 0) return null;
    const left = Math.min(...rects.map((rect) => rect.left));
    const top = Math.min(...rects.map((rect) => rect.top));
    const right = Math.max(...rects.map((rect) => rect.right));
    const bottom = Math.max(...rects.map((rect) => rect.bottom));
    return new DOMRect(left, top, right - left, bottom - top);
  }
}
