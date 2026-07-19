export type GestureState = 'idle' | 'pending' | 'active' | 'committing' | 'cancelling';
export type GestureCancelReason =
  | 'escape'
  | 'pointercancel'
  | 'lostpointercapture'
  | 'blur'
  | 'hidden'
  | 'teardown';

export type GestureStart = {
  inputEvent?: Event;
  pointerId?: number;
  captureTarget?: Element | null;
  onCancel(reason: GestureCancelReason): void;
};

/** Shared direct-manipulation lifecycle. It owns cancellation/recovery events
 * and pointer capture; controllers still own their gesture-specific preview,
 * document rollback, commit, and persistence work. */
export class GestureLifecycle {
  readonly owner: string;
  #state: GestureState = 'idle';
  #pointerId: number | null = null;
  #captureTarget: Element | null = null;
  #onCancel: ((reason: GestureCancelReason) => void) | null = null;

  constructor(owner: string) {
    this.owner = owner;
  }

  get state(): GestureState {
    return this.#state;
  }

  get active(): boolean {
    return this.#state === 'pending' || this.#state === 'active';
  }

  begin(start: GestureStart): void {
    if (this.active) this.cancel('teardown');
    this.#state = 'pending';
    this.#onCancel = start.onCancel;
    const pointer = start.inputEvent instanceof PointerEvent ? start.inputEvent : null;
    this.#pointerId = start.pointerId ?? pointer?.pointerId ?? null;
    this.#captureTarget = start.captureTarget ?? (pointer?.target instanceof Element ? pointer.target : null);
    this.#attach();
    if (this.#pointerId !== null && this.#captureTarget instanceof HTMLElement) {
      try {
        this.#captureTarget.setPointerCapture(this.#pointerId);
      } catch {
        // Synthetic pointer input and detached targets cannot be captured. The
        // global recovery listeners still guarantee cleanup.
      }
    }
    this.#state = 'active';
  }

  owns(event: PointerEvent): boolean {
    return this.#pointerId === null || event.pointerId === this.#pointerId;
  }

  commit(): void {
    if (!this.active) return;
    this.#state = 'committing';
    this.#finish();
  }

  cancel(reason: GestureCancelReason): void {
    if (!this.active) return;
    this.#state = 'cancelling';
    const cancel = this.#onCancel;
    this.#onCancel = null;
    try {
      cancel?.(reason);
    } finally {
      this.#finish();
    }
  }

  destroy(): void {
    if (this.active) this.cancel('teardown');
    else this.#finish();
  }

  #finish(): void {
    this.#detach();
    const target = this.#captureTarget;
    const pointerId = this.#pointerId;
    this.#captureTarget = null;
    this.#pointerId = null;
    this.#onCancel = null;
    if (pointerId !== null && target instanceof HTMLElement) {
      try {
        if (target.hasPointerCapture(pointerId)) target.releasePointerCapture(pointerId);
      } catch {
        // Target teardown can make releasePointerCapture throw.
      }
    }
    this.#state = 'idle';
  }

  #attach(): void {
    window.addEventListener('keydown', this.#onKeyDown, true);
    window.addEventListener('pointercancel', this.#onPointerCancel, true);
    window.addEventListener('blur', this.#onBlur);
    document.addEventListener('visibilitychange', this.#onVisibilityChange);
    this.#captureTarget?.addEventListener('lostpointercapture', this.#onLostPointerCapture);
  }

  #detach(): void {
    window.removeEventListener('keydown', this.#onKeyDown, true);
    window.removeEventListener('pointercancel', this.#onPointerCancel, true);
    window.removeEventListener('blur', this.#onBlur);
    document.removeEventListener('visibilitychange', this.#onVisibilityChange);
    this.#captureTarget?.removeEventListener('lostpointercapture', this.#onLostPointerCapture);
  }

  #onKeyDown = (event: KeyboardEvent): void => {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    this.cancel('escape');
  };

  #onPointerCancel = (event: PointerEvent): void => {
    if (this.owns(event)) this.cancel('pointercancel');
  };

  #onLostPointerCapture = (): void => {
    // Browser dispatches this synchronously after our deliberate release too;
    // committing/cancelling are not active, so only unexpected loss cancels.
    this.cancel('lostpointercapture');
  };

  #onBlur = (): void => this.cancel('blur');

  #onVisibilityChange = (): void => {
    if (document.visibilityState === 'hidden') this.cancel('hidden');
  };
}
