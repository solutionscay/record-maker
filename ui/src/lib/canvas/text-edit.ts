// Inline text-edit controller (#46, split in #135): the floating textarea a
// double-click (or a fresh text placement) opens over a text object, styled to
// match the object's server-derived text style and kept live while the
// inspector restyles it (#5).

import type { ObjectDoc } from '../doc.svelte';
import { setObjectContent } from '../persist';
import { llog, lerror } from '../log';
import type { CanvasContext } from './context';

export class TextEditController {
  readonly #ctx: CanvasContext;
  #editor: HTMLTextAreaElement | null = null;
  #editingId: number | null = null;
  /** Tears down the open inline text editor (removes its document-level
   * outside-press listener + element). Null when no editor is open. */
  #cleanup: (() => void) | null = null;

  constructor(ctx: CanvasContext) {
    this.#ctx = ctx;
  }

  get isEditing(): boolean {
    return this.#editingId !== null;
  }

  start(id: number): void {
    const o = this.#ctx.doc.getObject(id);
    const overlay = this.#ctx.partOverlay();
    if (!o || this.#ctx.partTop(o.partId) === null || !overlay) return;
    this.#editor?.remove();
    this.#editingId = id;
    this.#ctx.hover.paint();
    const editor = document.createElement('textarea');
    editor.className = 'le-inline-text-editor';
    editor.value = o.content;
    overlay.append(editor);
    this.#editor = editor;
    // Match the object's resolved text style (size / weight / italic / underline /
    // colour / align) so the editor LOOKS like the text it edits (#5). Kept in
    // sync live via `syncOpen()` while the inspector is used.
    this.#applyEditorTextStyle(editor, o);

    const finish = (commit: boolean) => {
      if (this.#editor !== editor) return;
      const next = editor.value;
      document.removeEventListener('pointerdown', onOutsidePointerDown, true);
      editor.remove();
      this.#editor = null;
      this.#editingId = null;
      this.#cleanup = null;
      if (commit && next !== o.content) void this.#commitTextEdit(id, next);
      this.#ctx.hover.paint();
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
    this.#cleanup = () => document.removeEventListener('pointerdown', onOutsidePointerDown, true);
    editor.focus();
    editor.select();
  }

  /** Re-apply the editing object's server-derived text style to the open inline
   * editor, so inspector size/style changes appear LIVE without closing it (#5).
   * No-op when no text editor is open. Called reactively from App.svelte when the
   * render model (and thus the object's `textStyle`) changes. */
  syncOpen(): void {
    const editor = this.#editor;
    const id = this.#editingId;
    if (!editor || id === null) return;
    const o = this.#ctx.doc.getObject(id);
    if (o) this.#applyEditorTextStyle(editor, o);
  }

  /** Copy the object's resolved `textStyle` (the same CSS the server derives and
   * the canvas renders with) onto the inline editor, then re-assert the editor's
   * box. `cssText` clears prior inline styles, so left/top/width/height are set
   * AFTER it; position/border/z-index/background come from the class. */
  #applyEditorTextStyle(editor: HTMLTextAreaElement, o: Readonly<ObjectDoc>): void {
    const top = this.#ctx.partTop(o.partId) ?? 0;
    editor.style.cssText = this.#ctx.objectView(o.id)?.textStyle ?? '';
    this.#ctx.placeOverlay(editor, o, top);
  }

  #inspectorEl(): HTMLElement | null {
    return document.getElementById('layout-inspector');
  }

  async #commitTextEdit(id: number, content: string): Promise<void> {
    llog('persist', 'inline text edit', { id });
    this.#ctx.doc.setProp(id, 'content', content);
    this.#ctx.doc.mark();
    try {
      const view = await setObjectContent(this.#ctx.layoutId, id, content);
      this.#ctx.doc.setProp(id, 'content', view.content);
    } catch (e) {
      lerror('persist', 'inline text edit failed', e);
      this.#ctx.reportError(e);
    }
  }

  destroy(): void {
    this.#cleanup?.();
    this.#editor?.remove();
  }
}
