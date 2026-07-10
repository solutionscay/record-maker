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
  /** Portal containment (#168/#169, Model B): the owning portal's object id when
   * this object is one of its authored columns (a child field/label positioned
   * row-relative), else absent. A portal enumerates its columns by matching this
   * against its own `id`. Omitted (serde-skipped) for every top-level object. */
  parentObjectId?: number;
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
  /** Portal (#168/#169): whether the portal resolved its bound route against a live
   * base record (Browse). The renderer keys off THIS, not the column count: true ⇒
   * render the repeating-row region (even with zero authored columns/rows), absent/
   * false ⇒ the unresolved route frame. Omitted (serde-skipped) for non-portals and
   * for the design canvas, which never resolves a portal. */
  portalResolved?: boolean;
  /** Portal (#168/#169): the header row of a resolved portal — the display name of
   * each AUTHORED column (a child field object bound route-relative to the terminal
   * table), in visual column order. Omitted for non-portals, the design canvas, and
   * a resolved portal with no authored columns yet. */
  portalColumns?: string[];
  /** Portal (#168): the repeating-row template height in px — the tallest authored
   * column field object's `h`. The header and every value row size to it, so the
   * fixed-height portal box (a clipping scroll viewport) shows `floor(body height /
   * row height)` rows and scrolls the rest; the visible-row count is geometry-driven
   * (box height + row height), never a numeric setting. Omitted for non-portals, the
   * design canvas, and a resolved portal with no authored columns yet. */
  portalRowHeight?: number;
  /** Portal inline edit (#170): the terminal field id backing each column, parallel
   * to `portalColumns`. In an editable Browse view each cell renders as an
   * `f<fieldId>` input off these ids so a per-row commit collects the right terminal
   * fields. Omitted when empty. */
  portalFieldIds?: number[];
  /** Parallel editability flags: intermediate route fields and system/read-only
   * terminal fields render as values instead of row editor inputs. */
  portalColumnEditable?: boolean[];
  /** Portal (#169): one entry per related record (after the #112 filter + declared
   * sort), each carrying the terminal record id and its cell values in column
   * order. Omitted when empty. */
  portalRows?: PortalRowView[];
  /** Portal create-new (#171): whether the trailing blank create row may render —
   * the route is create-determined AND the anchoring relationship's `allow_create`
   * is on. Omitted (serde-skipped) when false, so only a resolved, create-permitted
   * portal in Browse carries it; the design canvas never does. */
  portalCanCreate?: boolean;
  /** Portal create-new (#171): the endpoint the blank row posts to to mint a related
   * record. Present only when `portalCanCreate`. */
  portalCreateUrl?: string;
}

/** One related record rendered inside a portal (#169): its terminal-table row id
 * (stamped `data-related-id` for #170/#172) and its cell values in column order. */
export interface PortalRowView {
  id: number;
  cells: string[];
  editable?: boolean;
  /** Portal inline edit (#170): the `/related/*` endpoints this row's `.rec-edit`
   * scope posts to (open/commit/revert), precomputed server-side. Omitted on the
   * design canvas / non-editable render. */
  openUrl?: string;
  actionUrl?: string;
  revertUrl?: string;
}

/** A bindable field on the layout's primary table — the Field tool's dropdown
 * choices (#48/#62). Mirrors the server's `FieldChoice`. */
export interface FieldChoice {
  id: number;
  name: string;
  /** Logical field kind (`text`/`number`/`date`/`time`/`timestamp`/`bool`) so the
   * pickers can draw a type icon next to each name (#79). Mirrors the server. */
  kind: string;
  /** The system primary key (#156) — a field object bound to it is created
   * read-only by default. Mirrors the server. */
  system: boolean;
  /** Portal route field metadata. Present only when this choice belongs to a
   * table traversed by the selected portal route. */
  tableName?: string;
  routePath?: string;
  routeDepth?: number;
}

/** A related-data route available from the layout's base table. Routes are
 * derived from FK/reference constraints, not authored by layout/portal UI. */
export interface RelatedRouteChoice {
  relationshipId: number;
  name: string;
  direction: 'forward' | 'reverse';
  cardinality: 'toOne' | 'toMany';
  path: string;
  tableId: number;
  tableName: string;
  fromTable: number;
  fromField: number;
  toTable: number;
  toField: number;
  /** Relationship traversals in order. Direct routes have one; the determined
   * join-table route has a to-many hop followed by a to-one hop. */
  hops: RelatedRouteHopChoice[];
  /** Fields from every result table along this route, annotated with their table,
   * route prefix, and depth for grouped portal-column authoring. */
  fields: FieldChoice[];
}

export interface RelatedRouteHopChoice {
  relationshipId: number;
  name: string;
  direction: 'forward' | 'reverse';
  cardinality: 'toOne' | 'toMany';
  tableId: number;
  tableName: string;
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

/** A durable group over existing layout object ids (#75). The grouped objects
 * still render as normal children of their parts; this relationship drives
 * Layout-mode selection and group move behaviour. */
export interface ObjectGroupView {
  id: number;
  objectIds: number[];
}

/** What one object kind can do — the engine's per-kind capability record
 * (`ObjectKind::capabilities`), shipped through the design model so the editor's
 * gates ("can this kind be filled / text-formatted / bound?") read the single
 * server-side table instead of transcribing it. */
export interface ObjectCapabilities {
  /** Fill/background colour controls apply. */
  fill: boolean;
  /** Stroke/border colour + width controls apply. */
  stroke: boolean;
  /** Font/text-format controls apply. */
  textFormat: boolean;
  /** Carries static text in its own `content` slot. */
  contentSlot: boolean;
  /** Data-bound: resolves a `binding` to a live field value. */
  bindable: boolean;
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
  /** FK/reference-backed routes available for related-data tools such as portals. */
  relatedRoutes: RelatedRouteChoice[];
  /** Layout parts, rendered top→bottom in array order. */
  parts: PartView[];
  /** Durable object groups. */
  groups: ObjectGroupView[];
  /** Per-object-kind capability records, keyed by kind string. */
  capabilities: Record<string, ObjectCapabilities>;
}
