export const AUTOSCROLL_EDGE_PX = 56;
const MIN_SPEED_PX_S = 120;
const MAX_SPEED_PX_S = 900;

export type AutoscrollOwner = 'transform' | 'marquee' | 'draw' | 'drop' | 'band-resize';
export type ScrollDelta = { x: number; y: number };

export function edgeVelocity(
  point: number,
  start: number,
  end: number,
  edge = AUTOSCROLL_EDGE_PX,
): number {
  const signedDepth = point < start + edge
    ? -(start + edge - point) / edge
    : point > end - edge
      ? (point - (end - edge)) / edge
      : 0;
  if (signedDepth === 0) return 0;
  const direction = Math.sign(signedDepth);
  const depth = Math.min(1, Math.abs(signedDepth));
  return direction * (MIN_SPEED_PX_S + (MAX_SPEED_PX_S - MIN_SPEED_PX_S) * depth * depth);
}

/** One frame-driven viewport scroller shared by every Layout gesture. It emits
 * actual (clamped) scroll deltas so gesture geometry can preserve its grab point
 * at either boundary without guessing how far the browser moved the viewport. */
export class EdgeAutoscroll {
  readonly #stage: HTMLElement;
  #owner: AutoscrollOwner | null = null;
  #pointer = { x: 0, y: 0 };
  #onScroll: ((delta: ScrollDelta) => void) | null = null;
  #frame: number | null = null;
  #lastFrameAt = 0;

  constructor(stage: HTMLElement) {
    this.#stage = stage;
    window.addEventListener('blur', this.#onWindowLoss);
    window.addEventListener('dragend', this.#onWindowLoss);
    document.addEventListener('visibilitychange', this.#onVisibilityChange);
  }

  get active(): boolean {
    return this.#owner !== null;
  }

  start(owner: AutoscrollOwner, clientX: number, clientY: number, onScroll: (delta: ScrollDelta) => void): void {
    if (this.#owner !== owner) this.stop();
    this.#owner = owner;
    this.#onScroll = onScroll;
    this.update(owner, clientX, clientY);
  }

  update(owner: AutoscrollOwner, clientX: number, clientY: number): void {
    if (this.#owner !== owner) return;
    this.#pointer = { x: clientX, y: clientY };
    const velocity = this.#velocity();
    if (velocity.x === 0 && velocity.y === 0) {
      this.#cancelFrame();
      return;
    }
    this.#ensureFrame();
  }

  stop(owner?: AutoscrollOwner): void {
    if (owner && this.#owner !== owner) return;
    this.#owner = null;
    this.#onScroll = null;
    this.#lastFrameAt = 0;
    this.#cancelFrame();
  }

  destroy(): void {
    this.stop();
    window.removeEventListener('blur', this.#onWindowLoss);
    window.removeEventListener('dragend', this.#onWindowLoss);
    document.removeEventListener('visibilitychange', this.#onVisibilityChange);
  }

  #onWindowLoss = (): void => this.stop();

  #onVisibilityChange = (): void => {
    if (document.visibilityState === 'hidden') this.stop();
  };

  #velocity(): ScrollDelta {
    const rect = this.#stage.getBoundingClientRect();
    return {
      x: edgeVelocity(this.#pointer.x, rect.left, rect.right),
      y: edgeVelocity(this.#pointer.y, rect.top, rect.bottom),
    };
  }

  #ensureFrame(): void {
    if (this.#frame !== null) return;
    this.#frame = requestAnimationFrame(this.#tick);
  }

  #cancelFrame(): void {
    if (this.#frame !== null) cancelAnimationFrame(this.#frame);
    this.#frame = null;
  }

  #tick = (now: number): void => {
    this.#frame = null;
    if (!this.#owner) return;
    const velocity = this.#velocity();
    if (velocity.x === 0 && velocity.y === 0) return;
    const seconds = this.#lastFrameAt === 0 ? 1 / 60 : Math.min(0.034, Math.max(0, (now - this.#lastFrameAt) / 1_000));
    this.#lastFrameAt = now;
    const beforeX = this.#stage.scrollLeft;
    const beforeY = this.#stage.scrollTop;
    this.#stage.scrollBy(velocity.x * seconds, velocity.y * seconds);
    const delta = { x: this.#stage.scrollLeft - beforeX, y: this.#stage.scrollTop - beforeY };
    if (delta.x === 0 && delta.y === 0) {
      this.#lastFrameAt = 0;
      return;
    }
    this.#onScroll?.(delta);
    this.#ensureFrame();
  };
}
