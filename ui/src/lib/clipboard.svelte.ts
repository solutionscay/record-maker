// Session-scoped clipboard for Layout Mode objects (#85). A module singleton in a
// `.svelte.ts` so `hasContent` is a reactive `$state` read (lets a future Paste
// button auto-enable). Deliberately NOT on the editor doc — it must stay out of the
// undo history / hydrate surface — and NOT the OS clipboard. Survives selection
// changes and record navigation within the session; dies on full reload.

/** One clipboard entry = a structural snapshot of a single layout object,
 *  sufficient to re-create it verbatim (at a NEW id) via persist.createObject.
 *  Sourced from getObject(id) [ObjectDoc] joined with the object's ObjectView
 *  (for fieldId, which no store getter exposes). fieldId is non-null ONLY when
 *  kind === 'field'. props is the raw JSON STRING as it sits on ObjectDoc/View. */
export interface ClipboardObject {
  kind: string;            // 'field' | 'text' | 'rect' | 'line' | 'ellipse'
  partId: number;          // source band (object→part membership lives on the object)
  x: number;               // part-relative geometry at copy time
  y: number;
  w: number;
  h: number;
  z: number;               // stacking within part; relative order preserved on paste
  readOnly: boolean;
  binding: string;         // dot-path; recreates the value object on paste (server prefers it over fieldId)
  content: string;         // text slot; caption string for a label, '' otherwise
  props: string;           // opaque appearance JSON string ('' when unset)
  fieldId: number | null;  // field identity (#60); null for text/shapes
}

export interface ClipboardPayload {
  objects: ClipboardObject[];
}

class LayoutClipboard {
  #payload = $state<ClipboardPayload | null>(null);
  /** Cascade counter: 0 = fresh copy, +1 per paste; drives the n*GRID
   *  down-right offset so repeated Ctrl+V of the same clipboard stair-steps.
   *  Reset to 0 on every write(). */
  #pasteCount = $state(0);

  get hasContent(): boolean {
    return this.#payload !== null && this.#payload.objects.length > 0;
  }
  get payload(): ClipboardPayload | null {
    return this.#payload;
  }

  write(payload: ClipboardPayload): void {
    this.#payload = payload;
    this.#pasteCount = 0;
  }
  /** Advance and return the 1-based cascade step for the NEXT paste. */
  nextPasteStep(): number {
    return ++this.#pasteCount;
  }
}

export const clipboard = new LayoutClipboard();
