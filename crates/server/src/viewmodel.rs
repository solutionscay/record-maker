//! The render model: chrome, askama templates, the `PartView`/`ObjectView`
//! wire structs, and the projection helpers that resolve layout metadata +
//! record values into them. Shared by the Browse pages and the Layout-Mode
//! design model (#44 parity contract).

use std::collections::HashMap;

use askama::Template;
use axum::Json;
use record_maker_engine::{
    FieldKind, FieldMeta, LayoutMeta, ObjectKind, ObjectMeta, PartKind, PartMeta, Solution,
    TableMeta,
};

use crate::style::{object_style, parse_props, part_style, shape_style, text_style};
use crate::{format, AppError, AppResult};

/// Persistent shell context shared by every page (the chrome).
pub(crate) struct Chrome {
    pub(crate) mode: &'static str, // "browse" | "design" | "schema"
    pub(crate) layouts: Vec<LayoutLink>,
    pub(crate) current_layout: Option<i64>,
    /// Form/List/Table tabs for the Browse view toggle; empty in Layout mode.
    pub(crate) view_tabs: Vec<ViewTab>,
    /// Record-navigation flipbook for the Browse status bar; `None` elsewhere.
    pub(crate) nav: Option<Flipbook>,
    /// True when the current record is open for editing (its lock is held).
    pub(crate) editing: bool,
}

pub(crate) struct LayoutLink {
    id: i64,
    name: String,
    selected: bool,
}

/// One entry in the Browse Form/List/Table view toggle.
pub(crate) struct ViewTab {
    label: &'static str,
    href: String,
    active: bool,
}

/// Record navigation for the Browse status sidebar: first/prev/next/last over
/// the current layout's found set (#23), plus an editable position field.
/// `current` is 1-based, `0` when empty. `layout_id`/`view` back the jump form.
pub(crate) struct Flipbook {
    layout_id: i64,
    view: &'static str,
    current: i64,
    /// Physical id of the record at `current`; `None` when the found set is
    /// empty. Backs the toolbar's Delete action.
    current_id: Option<i64>,
    total: i64,
    first_href: String,
    prev_href: String,
    next_href: String,
    last_href: String,
    at_first: bool,
    at_last: bool,
}

/// Parse `?rec=N` (1-based) and clamp it into the found set (frozen #23):
/// `[1, total]`, defaulting to 1; `0` when there are no records.
pub(crate) fn clamp_rec(q: &HashMap<String, String>, total: i64) -> i64 {
    clamp_rec_n(q.get("rec").and_then(|s| s.parse::<i64>().ok()), total)
}

/// Clamp a client-sent record number into the found set (1-based, `0` when
/// empty) — the typed-body core [`clamp_rec`] parses `?rec=` into.
pub(crate) fn clamp_rec_n(rec: Option<i64>, total: i64) -> i64 {
    if total <= 0 {
        return 0;
    }
    rec.unwrap_or(1).clamp(1, total)
}

/// Build the flipbook for record `current` of `total` on `layout_id`/`view`.
/// Step links preserve the current view and stay clamped to the found set.
/// `current_id` is the physical id at `current` (for the Delete action).
pub(crate) fn flipbook(
    layout_id: i64,
    view: &'static str,
    current: i64,
    current_id: Option<i64>,
    total: i64,
) -> Flipbook {
    let href = |n: i64| format!("/browse/{layout_id}?view={view}&rec={n}");
    Flipbook {
        layout_id,
        view,
        current,
        current_id,
        total,
        first_href: href(1),
        prev_href: href((current - 1).max(1)),
        next_href: href((current + 1).min(total.max(1))),
        last_href: href(total.max(1)),
        at_first: current <= 1,
        at_last: current >= total,
    }
}

/// Build the Layout-mode stepper: prev/next steps through the **logical layouts**
/// (one per table, in picker order) while holding the current view, so the
/// designer flips between layouts the way the record stepper flips records (#57).
/// In Layout mode the pagination control navigates layouts, not records.
pub(crate) fn layout_stepper(sol: &Solution, current: &LayoutMeta) -> Option<Flipbook> {
    let view = canonical_view(&current.view);
    // Each table (its Form layout is the canonical handle) → that table's layout
    // for the CURRENT view, so stepping holds the view axis steady.
    let steps: Vec<i64> = sol
        .layouts()
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.view == "form")
        .filter_map(|l| {
            sol.layouts_for_table(l.table_id)
                .ok()?
                .into_iter()
                .find(|s| s.view == view)
                .map(|s| s.id)
        })
        .collect();
    let idx = steps.iter().position(|&id| id == current.id)?;
    let href = |i: usize| format!("/design/{}", steps[i]);
    Some(Flipbook {
        layout_id: current.id,
        view,
        current: idx as i64 + 1,
        current_id: None,
        total: steps.len() as i64,
        first_href: href(0),
        prev_href: href(idx.saturating_sub(1)),
        next_href: href((idx + 1).min(steps.len() - 1)),
        last_href: href(steps.len() - 1),
        at_first: idx == 0,
        at_last: idx + 1 >= steps.len(),
    })
}

/// The three Browse views, in toggle order. The frozen `?view=` contract (#20).
const VIEWS: [&str; 3] = ["form", "list", "table"];

/// Normalise a `?view=` value to a known view, falling back to the layout's
/// stored view when `?view` is absent. Retained for the record-action handlers'
/// redirects; Browse itself now renders by the layout's own view (see
/// [`canonical_view`]), since each view is its own layout (#57).
pub(crate) fn view_param(q: &HashMap<String, String>, default: &str) -> &'static str {
    canonical_view(q.get("view").map(String::as_str).unwrap_or(default))
}

/// Normalise a stored layout `view` string to one of the three renderers. A
/// layout's view is now intrinsic — the layout id encodes the view — so Browse
/// renders by this rather than a `?view=` param (#57).
pub(crate) fn canonical_view(view: &str) -> &'static str {
    match view {
        "form" => "form",
        "list" => "list",
        _ => "table",
    }
}

/// Human label for a stored `view` (the toggle tabs + the Layout-mode status).
pub(crate) fn view_label(view: &str) -> &'static str {
    match view {
        "form" => "Form",
        "list" => "List",
        _ => "Table",
    }
}

impl Chrome {
    /// Build the shared chrome. `current` is the layout in focus (its view + table
    /// drive the toggle and picker). Per #57 a table has one layout **per view**,
    /// so the view toggle switches among sibling layout ids and the picker lists
    /// one entry per table (its Form layout is the canonical handle).
    pub(crate) fn build(sol: &Solution, mode: &'static str, current: Option<&LayoutMeta>) -> Self {
        let current_table = current.map(|c| c.table_id);
        let layouts = sol
            .layouts()
            .map(|ls| {
                ls.into_iter()
                    .filter(|l| l.view == "form")
                    .map(|l| LayoutLink {
                        selected: current_table == Some(l.table_id),
                        id: l.id,
                        name: l.name,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The view toggle switches among the current table's per-view sibling
        // layouts — each view is its own layout id now. It stays in the current
        // mode, so Layout mode can design each view (Browse browses each).
        let view_tabs = match current {
            Some(cur) => {
                let siblings = sol.layouts_for_table(cur.table_id).unwrap_or_default();
                VIEWS
                    .iter()
                    .filter_map(|&v| {
                        siblings.iter().find(|l| l.view == v).map(|l| ViewTab {
                            label: view_label(v),
                            href: format!("/{mode}/{}", l.id),
                            active: cur.view == v,
                        })
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        Chrome {
            mode,
            layouts,
            current_layout: current.map(|c| c.id),
            view_tabs,
            nav: None,
            editing: false,
        }
    }
}

/// Resolve a layout id to its (layout, primary table). `None` if unknown.
pub(crate) fn layout_table(sol: &Solution, layout_id: i64) -> Option<(LayoutMeta, TableMeta)> {
    let lay = sol.layout_by_id(layout_id).ok().flatten()?;
    let tbl = sol.table_by_id(lay.table_id).ok().flatten()?;
    Some((lay, tbl))
}

// ---- Browse views — Table (live), Form/List placeholders until #25/#26 ----

#[derive(Template)]
#[template(path = "view_table.html")]
pub(crate) struct TableTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) layout_id: i64,
    pub(crate) table: String,
    /// Header/footer bands framing the grid, matching List/Form Browse views.
    pub(crate) header: Vec<PartView>,
    pub(crate) footer: Vec<PartView>,
    pub(crate) fields: Vec<FieldView>,
    pub(crate) records: Vec<RecordView>,
}

#[derive(Template)]
#[template(path = "view_form.html")]
pub(crate) struct FormTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) table: String,
    /// The record at the flipbook's current position; `None` when empty.
    pub(crate) record: Option<FormRecord>,
}

/// One record laid out per the layout's parts/objects, with live values (#25).
pub(crate) struct FormRecord {
    pub(crate) id: i64,
    pub(crate) parts: Vec<PartView>,
}

/// A part band; objects are positioned **relative to it** (geometry contract).
/// Also the part half of the Layout-Mode read model (`/design/:layout/model`):
/// the Svelte canvas renders from the same fields the askama band macro uses, so
/// `id`/`kind` are carried for the editor's document store (#45) without changing
/// the rendered DOM.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartView {
    pub(crate) id: i64,
    pub(crate) kind: &'static str,
    pub(crate) height: i64,
    /// The raw appearance bag (#49/Issue 7) the Band inspector edits, carried
    /// alongside the server-derived `part_style` so the inspector reads/writes the
    /// underlying `fill` key while Browse/canvas render from `part_style`. Empty
    /// string when the band has no props.
    pub(crate) props: String,
    /// Server-derived inline CSS for the band's `<div class="fm-part">` (its
    /// background fill). Interpolated identically by `_band.html` and `Band.svelte`
    /// (the #44 parity contract). Empty when the band is unstyled.
    pub(crate) part_style: String,
    pub(crate) objects: Vec<ObjectView>,
}

/// A positioned object, discriminated by `kind` (#60):
/// - `field` objects render their live `value` **only** (an input in an editable
///   view unless read-only); `field_id` names that input `f<id>`. Their caption is
///   a separate `text` object — `label` is still resolved (for the inspector) but
///   no longer rendered inline.
/// - `text` objects render their static `content`.
/// - shape objects (`shape == true`) render a styled box from `shape_style`.
/// - field/text objects render box/text styles derived from `props`.
///
/// `z` is the stacking order (CSS `z-index`); `read_only` suppresses the editable
/// input even in an editable view (per-object editability, #40/#43).
///
/// Also the object half of the Layout-Mode read model: the canvas hydrates its
/// document store from these fields. The rendered DOM (askama macro and the
/// mirroring Svelte `Band` component) uses only the visual/geometry fields, so
/// Browse and Layout stay byte-identical (#44). **Field order is the wire
/// contract** — the editor store's `renderModel` projection mirrors it key-for-key
/// (doc.svelte.ts `#toView`), so keep the two in lockstep.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectView {
    pub(crate) id: i64,
    pub(crate) kind: &'static str,
    pub(crate) field: bool,
    pub(crate) shape: bool,
    pub(crate) field_id: Option<i64>,
    pub(crate) x: i64,
    pub(crate) y: i64,
    pub(crate) w: i64,
    pub(crate) h: i64,
    pub(crate) z: i64,
    pub(crate) read_only: bool,
    pub(crate) binding: String,
    pub(crate) content: String,
    /// The raw appearance bag (#49) the Style zone edits. Carried alongside the
    /// server-derived `shape_style` so the canvas renders from `shape_style` while
    /// the inspector reads/writes the underlying `fill`/`stroke`/… keys. Empty
    /// string when the object has no props.
    pub(crate) props: String,
    pub(crate) object_style: String,
    pub(crate) text_style: String,
    pub(crate) label: String,
    pub(crate) value: String,
    /// The RAW (unformatted) field value. `value` above carries the display
    /// string (value formatting #77/#78 applied); `raw` is what an editable
    /// Browse input must commit so a formatted field is never written back as its
    /// formatted text. Skipped from the design-model JSON (the canvas renders the
    /// display `value`); the askama browse band reads it directly. Equal to
    /// `value` when no format is active.
    #[serde(skip)]
    pub(crate) raw: String,
    pub(crate) shape_style: String,
}

/// A bindable field on the layout's primary table — the Field tool's dropdown
/// choices (#48/#62). Part of the Layout-Mode read model so the rail can offer
/// every field, not only the ones already placed.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FieldChoice {
    pub(crate) id: i64,
    pub(crate) name: String,
    /// Logical field kind (`FieldKind::as_str`) so the rail can draw type icons (#79).
    pub(crate) kind: String,
}

/// A relationship route the layout can choose for related data. These are
/// derived from declared FK constraints, not authored by portal/layout UI.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelatedRouteChoice {
    relationship_id: i64,
    name: String,
    direction: &'static str,
    cardinality: &'static str,
    path: String,
    table_id: i64,
    table_name: String,
    from_table: i64,
    from_field: i64,
    to_table: i64,
    to_field: i64,
}

#[derive(Template)]
#[template(path = "view_list.html")]
pub(crate) struct ListTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) table: String,
    /// Non-body parts (header/title/…) rendered once above the rows.
    pub(crate) header: Vec<PartView>,
    /// One entry per record: the Body part(s) bound to that record.
    pub(crate) rows: Vec<ListRow>,
    /// Footer/grand-summary parts rendered once below the rows.
    pub(crate) footer: Vec<PartView>,
}

/// One record's Body band(s) in List view; `current` marks the flipbook's row.
pub(crate) struct ListRow {
    id: i64,
    current: bool,
    parts: Vec<PartView>,
}

pub(crate) struct FieldView {
    pub(crate) name: String,
}

pub(crate) struct RecordView {
    pub(crate) id: i64,
    pub(crate) cells: Vec<CellView>,
}

/// One Table-view cell: the field id (so editable inputs can be named `f<id>`)
/// and the current value.
pub(crate) struct CellView {
    pub(crate) field_id: i64,
    /// RAW cell value — what the editable Table input commits.
    pub(crate) value: String,
    /// Display value (value formatting #77/#78 applied). Equals `value` when the
    /// column's field object carries no `format` bag.
    pub(crate) display: String,
    /// Inline CSS for the cell input (e.g. the value-dependent negative color);
    /// empty when unstyled.
    pub(crate) style: String,
}

#[derive(Template)]
#[template(path = "design.html")]
pub(crate) struct DesignTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) layout_id: i64,
    pub(crate) layout: String,
    /// Which view this layout designs (`Form`/`List`/`Table`) — shown in the
    /// status bar so the designer knows which surface they're editing (#57).
    pub(crate) view: &'static str,
}

/// The schema-builder surface (#113): a sibling to Layout Mode that manages
/// tables / fields (and, later, relationships) over the #107 `/schema/*` API.
/// App-global rather than per-layout, so it carries no current layout — the
/// Svelte island fetches the schema itself and owns the whole surface.
#[derive(Template)]
#[template(path = "schema.html")]
pub(crate) struct SchemaTemplate {
    pub(crate) chrome: Chrome,
}

/// Resolve a field object's binding to its (field, field_id, label, value) for the
/// current record. Interim two-segment resolver: the last dot-path segment is the
/// field name, matched case-insensitively against `by_name` (lowercased field name
/// → `(display name, value)`). The full relationship resolver replaces this (#11).
///
/// Non-field objects (text / shapes) resolve to no live value — text renders from
/// its own `content` slot and shapes from `props`, neither of which is
/// record-dependent (#60). Only the bound value/label come from the record here.
fn resolve_object(
    o: &ObjectMeta,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
) -> (bool, Option<i64>, String, String, Option<FieldKind>) {
    match (o.kind, o.binding.as_deref()) {
        (ObjectKind::Field, Some(binding)) => {
            let seg = binding.rsplit('.').next().unwrap_or(binding).to_lowercase();
            match by_name.get(&seg) {
                Some((id, label, value, kind)) => {
                    (true, Some(*id), label.clone(), value.clone(), Some(*kind))
                }
                // A binding that doesn't resolve yet (e.g. a relationship path)
                // still renders a useful placeholder instead of a blank object.
                None => (true, None, binding.to_string(), binding.to_string(), None),
            }
        }
        _ => (false, None, String::new(), String::new(), None),
    }
}

/// A record's field values keyed by lowercased field name → (field id, display
/// name, value) — the lookup `resolve_object` binds against.
pub(crate) fn by_name_map(
    fields: &[FieldMeta],
    cells: Vec<String>,
) -> HashMap<String, (i64, String, String, FieldKind)> {
    fields
        .iter()
        .zip(cells)
        .map(|(f, value)| (f.name.to_lowercase(), (f.id, f.name.clone(), value, f.kind)))
        .collect()
}

pub(crate) fn by_name_for_rec(
    sol: &Solution,
    table: &TableMeta,
    fields: &[FieldMeta],
    rec: Option<i64>,
) -> HashMap<String, (i64, String, String, FieldKind)> {
    let ids = sol.record_ids(table).unwrap();
    let rec = clamp_rec_n(rec, ids.len() as i64);
    if rec < 1 {
        return HashMap::new();
    }
    match sol
        .get_record(table, fields, ids[(rec - 1) as usize])
        .unwrap()
    {
        Some(cells) => by_name_map(fields, cells),
        None => HashMap::new(),
    }
}

/// Resolve one object into its `ObjectView` (#44/#60), bound against `by_name`.
/// The single per-object projection shared by [`render_part`] and the create
/// handler, so an object placed on the canvas serialises byte-identically to one
/// read back from the model — there is no second mapping to drift.
pub(crate) fn object_view(
    o: &ObjectMeta,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
) -> ObjectView {
    let (field, field_id, label, raw_value, field_kind) = resolve_object(o, by_name);
    let shape = o.kind.is_shape();
    // The text slot is only meaningful for `text` objects; fields/shapes carry
    // none, so the renderer never reads a stray content.
    let content = match o.kind {
        ObjectKind::Text => o.content.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let shape_style = if shape {
        shape_style(o.kind, o.props.as_deref())
    } else {
        String::new()
    };
    let object_style = object_style(o.kind, o.props.as_deref());
    let mut text_style = text_style(o.kind, o.props.as_deref());
    // Value formatting (#77/#78) is display-only: applied to the resolved value
    // for BOTH Browse and the design canvas, driven by the `format` sub-bag of
    // the object's props and the bound field's kind. A negative-number color is
    // value-dependent, so it rides `text_style` here (appended last, so it wins
    // over any static textColor) rather than the static props CSS. An unresolved
    // binding (`field_kind == None`) leaves the placeholder untouched.
    let value = match field_kind {
        Some(kind) => {
            let props = parse_props(o.props.as_deref());
            let spec = props.as_ref().and_then(|v| v.get("format"));
            let formatted = format::format_value(&raw_value, spec, kind);
            if let Some(color) = formatted.color {
                text_style.push_str(&format!("color:{color};"));
            }
            formatted.text
        }
        None => raw_value.clone(),
    };
    ObjectView {
        id: o.id,
        kind: o.kind.as_str(),
        field,
        shape,
        field_id,
        x: o.x,
        y: o.y,
        w: o.w,
        h: o.h,
        z: o.z,
        read_only: o.read_only,
        binding: o.binding.clone().unwrap_or_default(),
        content,
        props: o.props.clone().unwrap_or_default(),
        object_style,
        text_style,
        label,
        value,
        raw: raw_value,
        shape_style,
    }
}

pub(crate) fn object_view_for_rec(
    sol: &Solution,
    layout_id: i64,
    object_id: i64,
    rec: Option<i64>,
) -> Option<ObjectView> {
    let (_lay, table) = layout_table(sol, layout_id)?;
    let fields = sol.fields(table.id).ok()?;
    let by_name = by_name_for_rec(sol, &table, &fields, rec);
    let object = sol.object_by_id(layout_id, object_id).ok()??;
    Some(object_view(&object, &by_name))
}

/// Shared tail of the single-object mutation handlers (binding / binding-path /
/// content / read-only): 404 when the write matched no row, otherwise re-project
/// the object against `rec` exactly as a model fetch would.
pub(crate) fn updated_object_view(
    sol: &Solution,
    layout_id: i64,
    object_id: i64,
    rec: Option<i64>,
    updated: usize,
) -> AppResult<Json<ObjectView>> {
    if updated == 0 {
        return Err(AppError::not_found());
    }
    object_view_for_rec(sol, layout_id, object_id, rec)
        .map(Json)
        .ok_or_else(AppError::not_found)
}

pub(crate) fn related_route_choices(sol: &Solution, table: &TableMeta) -> Vec<RelatedRouteChoice> {
    sol.relationships()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rel| {
            if rel.from_table == table.id {
                let target = sol.table_by_id(rel.to_table).ok().flatten()?;
                Some(RelatedRouteChoice {
                    relationship_id: rel.id,
                    name: rel.name.clone(),
                    direction: "forward",
                    cardinality: "toOne",
                    path: rel.name.clone(),
                    table_id: target.id,
                    table_name: target.name,
                    from_table: rel.from_table,
                    from_field: rel.from_field,
                    to_table: rel.to_table,
                    to_field: rel.to_field,
                })
            } else if rel.to_table == table.id {
                let target = sol.table_by_id(rel.from_table).ok().flatten()?;
                Some(RelatedRouteChoice {
                    relationship_id: rel.id,
                    name: rel.name.clone(),
                    direction: "reverse",
                    cardinality: "toMany",
                    path: rel.name.clone(),
                    table_id: target.id,
                    table_name: target.name,
                    from_table: rel.from_table,
                    from_field: rel.from_field,
                    to_table: rel.to_table,
                    to_field: rel.to_field,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Render one part's objects, positioned and bound against `by_name` (an empty
/// map leaves field values blank — used for header/footer with no record).
pub(crate) fn render_part(
    sol: &Solution,
    part: &PartMeta,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
) -> PartView {
    let objects = sol
        .objects(part.id)
        .unwrap()
        .iter()
        .map(|o| object_view(o, by_name))
        .collect();
    PartView {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects,
    }
}

/// Project a part into the objects-free `PartView` the part-mutation handlers
/// echo (create / height / kind / props) — one literal instead of four.
pub(crate) fn part_view(part: &PartMeta) -> PartView {
    PartView {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects: Vec::new(),
    }
}

/// Shared tail of the part-mutation handlers (height / kind / props): 404 when
/// the write matched no row, otherwise re-read the part and echo its view.
pub(crate) fn updated_part_view(
    sol: &Solution,
    layout_id: i64,
    part_id: i64,
    updated: usize,
) -> AppResult<Json<PartView>> {
    if updated == 0 {
        return Err(AppError::not_found());
    }
    let part = sol
        .part_by_id(layout_id, part_id)
        .unwrap()
        .ok_or_else(AppError::not_found)?;
    Ok(Json(part_view(&part)))
}

/// Canvas width for a layout: the rightmost object edge + a margin. Geometry is
/// record-independent, so this is the same for every record (Form and List).
pub(crate) fn layout_canvas_width(sol: &Solution, layout_id: i64) -> i64 {
    let mut w = 0i64;
    for p in sol.parts(layout_id).unwrap() {
        for o in sol.objects(p.id).unwrap() {
            w = w.max(o.x + o.w);
        }
    }
    w + 24
}

/// Build the Form-view render of the record at flipbook position `rec`: the
/// layout's parts, each with its objects positioned and bound to live values.
/// `None` when the found set is empty (`rec == 0`) or the row vanished.
pub(crate) fn build_form_record(
    sol: &Solution,
    layout_id: i64,
    table: &TableMeta,
    fields: &[FieldMeta],
    ids: &[i64],
    rec: i64,
) -> Option<FormRecord> {
    if rec <= 0 {
        return None;
    }
    let id = ids[(rec - 1) as usize];
    let cells = sol.get_record(table, fields, id).unwrap()?;
    let by_name = by_name_map(fields, cells);
    let parts = sol
        .parts(layout_id)
        .unwrap()
        .iter()
        .map(|p| render_part(sol, p, &by_name))
        .collect();
    Some(FormRecord { id, parts })
}

/// Build the List-view render: header/footer parts once, the Body part(s)
/// repeated per record bound to its values. `current_rec` (1-based) marks the
/// flipbook's row. Returns `(header, rows, footer)`.
/// The header and footer bands of a layout, rendered once with no record bound.
/// Shared by List and Table Browse views so both frame their rows with the same
/// bands: header / sub-summary render above, footer / grand-summary below.
pub(crate) fn build_bands(sol: &Solution, layout_id: i64) -> (Vec<PartView>, Vec<PartView>) {
    let no_record = HashMap::new();
    let (mut header, mut footer) = (Vec::new(), Vec::new());
    for p in sol.parts(layout_id).unwrap() {
        match p.kind {
            PartKind::Footer | PartKind::GrandSummary => {
                footer.push(render_part(sol, &p, &no_record))
            }
            PartKind::Header | PartKind::SubSummary => {
                header.push(render_part(sol, &p, &no_record))
            }
            PartKind::Body => {}
        }
    }
    (header, footer)
}

pub(crate) fn build_list(
    sol: &Solution,
    layout_id: i64,
    table: &TableMeta,
    fields: &[FieldMeta],
    ids: &[i64],
    current_rec: i64,
) -> (Vec<PartView>, Vec<ListRow>, Vec<PartView>) {
    let (header, footer) = build_bands(sol, layout_id);
    let body_parts: Vec<_> = sol
        .parts(layout_id)
        .unwrap()
        .into_iter()
        .filter(|p| p.kind == PartKind::Body)
        .collect();

    let mut rows = Vec::new();
    for (i, &id) in ids.iter().enumerate() {
        let Some(cells) = sol.get_record(table, fields, id).unwrap() else {
            continue;
        };
        let by_name = by_name_map(fields, cells);
        let parts = body_parts
            .iter()
            .map(|p| render_part(sol, p, &by_name))
            .collect();
        rows.push(ListRow {
            id,
            current: (i as i64) + 1 == current_rec,
            parts,
        });
    }
    (header, rows, footer)
}

/// Map each table column to its value-format bag (the `format` sub-object of a
/// field object's props) drawn from `layout_id`'s objects. Table columns are
/// field-derived, so a column formats iff the layout holds a field object bound
/// to it that carries a `format` bag; later objects win on a duplicate binding.
/// Form/List format per-object via [`object_view`]; this brings Table to parity.
pub(crate) fn layout_field_formats(
    sol: &Solution,
    layout_id: i64,
    fields: &[FieldMeta],
) -> HashMap<i64, serde_json::Value> {
    let by_name: HashMap<String, i64> = fields
        .iter()
        .map(|f| (f.name.to_lowercase(), f.id))
        .collect();
    let mut map = HashMap::new();
    let Ok(parts) = sol.parts(layout_id) else {
        return map;
    };
    for p in parts {
        let Ok(objects) = sol.objects(p.id) else {
            continue;
        };
        for o in objects {
            let Some(binding) = o.binding.as_deref() else {
                continue;
            };
            let seg = binding.rsplit('.').next().unwrap_or(binding).to_lowercase();
            let Some(&fid) = by_name.get(&seg) else {
                continue;
            };
            if let Some(fmt) =
                parse_props(o.props.as_deref()).and_then(|v| v.get("format").cloned())
            {
                map.insert(fid, fmt);
            }
        }
    }
    map
}
