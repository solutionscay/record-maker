// Layout canvas interaction layer (#46) — the thin coordinator over the
// per-gesture controllers in ./canvas (#135): transform (moveable drag/resize/
// rotate + selecto marquee + target sync), placement (draw tool + field DnD),
// clipboard (cut/copy/paste/duplicate over one clone flow), text edit, and
// hover. The controllers share a small CanvasContext (stage, doc, layoutId,
// zoom, in-flight flags, identity cache); this class wires them together,
// owns the stage/window event subscriptions and the keyboard dispatch, and
// keeps the public API App.svelte and the context menu consume.
//
// Single source of truth is the store: pointer gestures never write element
// styles — see ./canvas/transform.ts for the moveable/selecto contract, and
// ./canvas/context.ts for how element↔id identity is resolved (the
// data-object-id both renderers stamp, #134).

import type { EditorDoc } from './doc.svelte';
import {
  canGroupSelection,
  canUngroupSelection,
  deleteSelected as deleteSelectedAction,
  groupSelected as groupSelectedAction,
  isDeleting,
  registerCanvasCleanup,
  ungroupSelected as ungroupSelectedAction,
} from './actions';
import { clipboard } from './clipboard.svelte';
import { runUndo, runRedo } from './history';
import { llog } from './log';
import { CanvasContext } from './canvas/context';
import { ClipboardController } from './canvas/clipboard-controller';
import { HoverController } from './canvas/hover';
import { PlacementController } from './canvas/placement';
import { TextEditController } from './canvas/text-edit';
import { TransformController } from './canvas/transform';

export class CanvasInteraction {
  readonly #ctx: CanvasContext;
  readonly #hover: HoverController;
  readonly #text: TextEditController;
  readonly #placement: PlacementController;
  readonly #clipboard: ClipboardController;
  readonly #transform: TransformController;
  /** Unregisters this instance's canvas-cleanup callback from the command layer. */
  #unregisterCleanup: () => void = () => {};

  constructor(stage: HTMLElement, doc: EditorDoc, layoutId: string) {
    const ctx = new CanvasContext(stage, doc, layoutId);
    this.#ctx = ctx;
    // Controllers only reach their peers inside event handlers, so constructing
    // them before the cross-references are wired is safe.
    this.#hover = new HoverController(ctx);
    this.#text = new TextEditController(ctx);
    this.#placement = new PlacementController(ctx);
    this.#clipboard = new ClipboardController(ctx);
    this.#transform = new TransformController(ctx);
    ctx.hover = this.#hover;
    ctx.text = this.#text;
    ctx.placement = this.#placement;
    ctx.clipboard = this.#clipboard;
    ctx.transform = this.#transform;

    stage.addEventListener('pointermove', this.#hover.onPointerMove);
    stage.addEventListener('pointerleave', this.#hover.onPointerLeave);
    stage.addEventListener('click', this.#transform.onClick);
    stage.addEventListener('dblclick', this.#transform.onDoubleClick);
    window.addEventListener('keydown', this.#onKeyDown);
    // Field drag-and-drop (#79 follow-up) — native HTML5 DnD, not a pointer
    // gesture, so it coexists with moveable/selecto's own pointer handling
    // without fighting over the same events.
    stage.addEventListener('dragover', this.#placement.onDragOver);
    stage.addEventListener('dragleave', this.#placement.onDragLeave);
    stage.addEventListener('drop', this.#placement.onDrop);
    // The shared command layer (./actions) runs this after a delete — whichever
    // surface issued it — so hover + moveable chrome never outlive the objects.
    this.#unregisterCleanup = registerCanvasCleanup(() => {
      this.#hover.set(null);
      this.#transform.forceClearTarget();
    });
    llog('init', 'CanvasInteraction ready', { layoutId, painted: ctx.paintedElements().length });
  }

  /** Reconcile moveable's target with the store selection (called reactively when
   * selection or geometry changes — e.g. after an undo). No-op during a gesture. */
  refresh(): void {
    this.#transform.refresh();
  }

  /** Tell the interaction layer the current canvas zoom (#62), so client→model
   * pointer conversion during placement divides by it. */
  setZoom(zoom: number): void {
    this.#transform.setZoom(zoom);
  }

  /** Re-apply the editing object's server-derived text style to the open inline
   * editor, so inspector size/style changes appear LIVE without closing it (#5). */
  syncOpenTextEditor(): void {
    this.#text.syncOpen();
  }

  destroy(): void {
    const stage = this.#ctx.stage;
    stage.removeEventListener('pointermove', this.#hover.onPointerMove);
    stage.removeEventListener('pointerleave', this.#hover.onPointerLeave);
    stage.removeEventListener('click', this.#transform.onClick);
    stage.removeEventListener('dblclick', this.#transform.onDoubleClick);
    window.removeEventListener('keydown', this.#onKeyDown);
    stage.removeEventListener('dragover', this.#placement.onDragOver);
    stage.removeEventListener('dragleave', this.#placement.onDragLeave);
    stage.removeEventListener('drop', this.#placement.onDrop);
    this.#placement.destroy();
    this.#hover.destroy();
    this.#text.destroy();
    this.#unregisterCleanup();
    this.#transform.destroy();
  }

  // ── keyboard dispatch (cross-controller, so it lives on the coordinator) ──

  #onKeyDown = (e: KeyboardEvent): void => {
    const target = e.target as HTMLElement | null;
    const inEditable = !!target?.closest('input, textarea, select, [contenteditable="true"]');
    const doc = this.#ctx.doc;

    // Cmd/Ctrl+A selects every layout object. Native page/text select-all is
    // never useful in Layout Mode, including while focus is in the inspector.
    if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'a') {
      e.preventDefault();
      this.#transform.selectAllObjects();
      return;
    }

    // Cut / Copy / Paste. Native clipboard wins inside editable fields, so bail on
    // inEditable BEFORE preventDefault (same ordering as undo/redo).
    if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
      const k = e.key.toLowerCase();
      if (k === 'c' || k === 'x' || k === 'v') {
        if (inEditable) return; // let the browser copy/cut/paste text
        if (this.#ctx.gesturing || this.#placement.isDrawing || isDeleting() || this.#ctx.placing) return; // never mutate mid-gesture/async
        if (k === 'v') {
          if (!clipboard.hasContent) return; // nothing to paste → let event pass
          e.preventDefault();
          void this.#clipboard.paste();
        } else {
          // 'c' or 'x' need a selection
          if (doc.selection.size === 0) return;
          e.preventDefault();
          if (k === 'c') this.#clipboard.copySelection();
          else void this.#clipboard.cutSelected();
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
        if (this.#ctx.gesturing || this.#placement.isDrawing || isDeleting() || this.#ctx.placing) return; // never pop mid-gesture / mid-draw / mid-async
        e.preventDefault();
        if (isRedo) runRedo(doc, this.#ctx.layoutId);
        else runUndo(doc, this.#ctx.layoutId);
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
      if (doc.selection.size === 0 || this.#ctx.placing) return;
      e.preventDefault();
      void this.#clipboard.duplicateSelected();
      return;
    }

    if (e.key !== 'Delete' && e.key !== 'Backspace') return;
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (inEditable) return;
    if (doc.selection.size === 0 || isDeleting()) return;
    e.preventDefault();
    void deleteSelectedAction(doc, this.#ctx.layoutId);
  };

  // ── public clipboard surface (context menu / a future menu rail) ──

  copy(): void {
    this.#clipboard.copySelection();
  }
  cut(): void {
    void this.#clipboard.cutSelected();
  }
  paste(): void {
    void this.#clipboard.paste();
  }
  canPaste(): boolean {
    return this.#clipboard.canPaste();
  }
  duplicate(): void {
    void this.#clipboard.duplicateSelected();
  }
  // Delete / group / ungroup run through the shared command layer (./actions),
  // the same implementation the Inspector buttons invoke.
  deleteSelected(): void {
    void deleteSelectedAction(this.#ctx.doc, this.#ctx.layoutId);
  }
  canGroup(): boolean {
    return canGroupSelection(this.#ctx.doc);
  }
  canUngroup(): boolean {
    return canUngroupSelection(this.#ctx.doc);
  }
  group(): void {
    void groupSelectedAction(this.#ctx.doc, this.#ctx.layoutId);
  }
  ungroup(): void {
    void ungroupSelectedAction(this.#ctx.doc, this.#ctx.layoutId);
  }
}
