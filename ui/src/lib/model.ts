// The Layout Mode read model — the JSON shape returned by the engine at
// `GET /design/:layout/model?rec=N` (ADR #42 / issue #44). The Svelte canvas
// renders DOM byte-identical (after normalization) to Browse's askama band
// macro from this same model. These interfaces mirror the FROZEN contract;
// keep field names in sync with the server's serialized shape.

/** One absolutely-positioned object on a layout part (#60): a bound `field`
 * (renders its value only), a static `text` label (renders `content`), or a
 * `shape` (`rect`/`line`/`ellipse`, renders a styled box from `shapeStyle`).
 *
 * Field ORDER mirrors the server's `ObjectView` serialization exactly; the editor
 * store's `renderModel` projection rebuilds these objects key-for-key so the
 * #44 fixture deep-equals (doc-check). Keep the two in lockstep. */
export interface ObjectView {
  /** Stable object id; used as the keyed-each key. */
  id: number;
  /** Object kind: `"field"`, `"text"`, or a shape (`"rect"`/`"line"`/`"ellipse"`). */
  kind: string;
  /** True when the object is a bound field (renders its value only). */
  field: boolean;
  /** True when the object is a drawn shape (renders a styled box from `shapeStyle`). */
  shape: boolean;
  /** Field id when `field` is true, else null. */
  fieldId: number | null;
  /** Left offset in px. */
  x: number;
  /** Top offset in px. */
  y: number;
  /** Width in px. */
  w: number;
  /** Height in px. */
  h: number;
  /** CSS stacking order (z-index). */
  z: number;
  /** Per-object read-only flag (adds `fm-readonly`). */
  readOnly: boolean;
  /** Data binding expression, e.g. `"Customers.Name"` (field objects). */
  binding: string;
  /** Static text of a `text` object (its own slot); empty for field/shape objects. */
  content: string;
  /** Raw appearance bag JSON the Style zone edits (#49); empty when unset. Carried
   * alongside the server-derived `shapeStyle` so the inspector reads/writes the
   * underlying `fill`/`stroke`/… keys while the canvas renders from `shapeStyle`. */
  props: string;
  /** Server-derived inline CSS for the object's outer box; empty when unset. */
  objectStyle: string;
  /** Server-derived inline CSS for field/text content; empty when unset. */
  textStyle: string;
  /** Resolved field label (kept for the inspector; no longer rendered inline). */
  label: string;
  /** Live field value (shown in the `fm-fvalue` span); empty for non-fields. */
  value: string;
  /** Server-derived inline CSS for a shape's appearance; empty for non-shapes. */
  shapeStyle: string;
}

/** A bindable field on the layout's primary table — the Field tool's dropdown
 * choices (#48/#62). Mirrors the server's `FieldChoice`. */
export interface FieldChoice {
  id: number;
  name: string;
}

/** One layout part (band) and the objects it contains, ordered back→front. */
export interface PartView {
  /** Stable part id; used as the keyed-each key. */
  id: number;
  /** Part kind, e.g. `"body"`. */
  kind: string;
  /** Part height in px. */
  height: number;
  /** Raw appearance bag JSON the Band inspector edits (#49/Issue 7); empty when
   * unset. Carried alongside the server-derived `partStyle` so the inspector
   * reads/writes the underlying `fill` key while the band renders from `partStyle`. */
  props: string;
  /** Server-derived inline CSS for the band's `fm-part` box (its background fill);
   * empty when the band is unstyled. */
  partStyle: string;
  /** Objects already ordered back→front (by z, then id). Render in order. */
  objects: ObjectView[];
}

/** The full design read model for one layout/record. */
export interface DesignModel {
  /** Layout id. */
  layoutId: number;
  /** Current record number (1-based). */
  rec: number;
  /** Total record count. */
  total: number;
  /** Canvas width in px. */
  width: number;
  /** The layout's Browse view (`form` | `list` | `table`); gates summary bands. */
  view: string;
  /** The primary table's fields — what the Create zone's Field tool offers. */
  fields: FieldChoice[];
  /** Layout parts, rendered top→bottom in array order. */
  parts: PartView[];
}
