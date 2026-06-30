// The Layout Mode read model — the JSON shape returned by the engine at
// `GET /design/:layout/model?rec=N` (ADR #42 / issue #44). The Svelte canvas
// renders DOM byte-identical (after normalization) to Browse's askama band
// macro from this same model. These interfaces mirror the FROZEN contract;
// keep field names in sync with the server's serialized shape.

/** One absolutely-positioned object on a layout part (a field or static text). */
export interface ObjectView {
  /** Stable object id; used as the keyed-each key. */
  id: number;
  /** Object kind, e.g. `"field"` or `"text"`. */
  kind: string;
  /** True when the object is a bound field (renders label + value spans). */
  field: boolean;
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
  /** Data binding expression, e.g. `"Customers.Name"`. */
  binding: string;
  /** Field label (shown in the `fm-flabel` span); may be empty. */
  label: string;
  /** Display value (shown in the `fm-fvalue` / `fm-text` span). */
  value: string;
}

/** One layout part (band) and the objects it contains, ordered back→front. */
export interface PartView {
  /** Stable part id; used as the keyed-each key. */
  id: number;
  /** Part kind, e.g. `"body"`. */
  kind: string;
  /** Part height in px. */
  height: number;
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
  /** Layout parts, rendered top→bottom in array order. */
  parts: PartView[];
}
