//! Browse/Layout mode runtime + app shell. Routing is **layout-keyed**
//! (ADR-0005): `/browse/:layout` and `/design/:layout`, where `:layout` is the
//! meta_layout **id** (i64). One generic handler set serves every table by
//! reading metadata — no per-table code.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Json, Router,
};
use record_maker_engine::{
    FieldKind, FieldMeta, LayoutMeta, NewField, NewObject, ObjectKind, ObjectMeta, PartKind,
    PartMeta, Solution, TableMeta,
};

#[derive(Clone)]
struct AppState {
    sol: Arc<Mutex<Solution>>,
    /// Records currently "open" for editing, keyed `(table_id, record_id)`.
    /// In-process is enough today (single-user desktop); the open→commit→release
    /// lifecycle is the point, and the registry is where multi-user lock
    /// enforcement will later hook in (#40).
    locks: Arc<Mutex<HashSet<(i64, i64)>>>,
}

impl AppState {
    fn lock_held(&self, key: (i64, i64)) -> bool {
        self.locks.lock().unwrap().contains(&key)
    }
}

/// Persistent shell context shared by every page (the chrome).
struct Chrome {
    mode: &'static str, // "browse" | "design"
    layouts: Vec<LayoutLink>,
    current_layout: Option<i64>,
    /// Form/List/Table tabs for the Browse view toggle; empty in Layout mode.
    view_tabs: Vec<ViewTab>,
    /// Record-navigation flipbook for the Browse status bar; `None` elsewhere.
    nav: Option<Flipbook>,
    /// True when the current record is open for editing (its lock is held).
    editing: bool,
}

struct LayoutLink {
    id: i64,
    name: String,
    selected: bool,
}

/// One entry in the Browse Form/List/Table view toggle.
struct ViewTab {
    label: &'static str,
    href: String,
    active: bool,
}

/// Record navigation for the Browse status sidebar: first/prev/next/last over
/// the current layout's found set (#23), plus an editable position field.
/// `current` is 1-based, `0` when empty. `layout_id`/`view` back the jump form.
struct Flipbook {
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
fn clamp_rec(q: &HashMap<String, String>, total: i64) -> i64 {
    if total <= 0 {
        return 0;
    }
    let n = q.get("rec").and_then(|s| s.parse::<i64>().ok()).unwrap_or(1);
    n.clamp(1, total)
}

/// Build the flipbook for record `current` of `total` on `layout_id`/`view`.
/// Step links preserve the current view and stay clamped to the found set.
/// `current_id` is the physical id at `current` (for the Delete action).
fn flipbook(
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
fn layout_stepper(sol: &Solution, current: &LayoutMeta) -> Option<Flipbook> {
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
fn view_param(q: &HashMap<String, String>, default: &str) -> &'static str {
    canonical_view(q.get("view").map(String::as_str).unwrap_or(default))
}

/// Normalise a stored layout `view` string to one of the three renderers. A
/// layout's view is now intrinsic — the layout id encodes the view — so Browse
/// renders by this rather than a `?view=` param (#57).
fn canonical_view(view: &str) -> &'static str {
    match view {
        "form" => "form",
        "list" => "list",
        _ => "table",
    }
}

/// Human label for a stored `view` (the toggle tabs + the Layout-mode status).
fn view_label(view: &str) -> &'static str {
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
    fn build(sol: &Solution, mode: &'static str, current: Option<&LayoutMeta>) -> Self {
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
        Chrome { mode, layouts, current_layout: current.map(|c| c.id), view_tabs, nav: None, editing: false }
    }
}

/// Resolve a layout id to its (layout, primary table). `None` if unknown.
fn layout_table(sol: &Solution, layout_id: i64) -> Option<(LayoutMeta, TableMeta)> {
    let lay = sol.layout_by_id(layout_id).ok().flatten()?;
    let tbl = sol.table_by_id(lay.table_id).ok().flatten()?;
    Some((lay, tbl))
}

fn not_found(what: &str, id: i64) -> axum::response::Response {
    Html(format!("<p>No such {what}: {id}</p>")).into_response()
}

// ---- Browse views — Table (live), Form/List placeholders until #25/#26 ----

#[derive(Template)]
#[template(path = "view_table.html")]
struct TableTemplate {
    chrome: Chrome,
    layout_id: i64,
    table: String,
    fields: Vec<FieldView>,
    records: Vec<RecordView>,
}

#[derive(Template)]
#[template(path = "view_form.html")]
struct FormTemplate {
    chrome: Chrome,
    layout: String,
    table: String,
    /// The record at the flipbook's current position; `None` when empty.
    record: Option<FormRecord>,
}

/// One record laid out per the layout's parts/objects, with live values (#25).
/// `width` is the canvas width (max object right edge + margin).
struct FormRecord {
    id: i64,
    width: i64,
    parts: Vec<PartView>,
}

/// A part band; objects are positioned **relative to it** (geometry contract).
/// Also the part half of the Layout-Mode read model (`/design/:layout/model`):
/// the Svelte canvas renders from the same fields the askama band macro uses, so
/// `id`/`kind` are carried for the editor's document store (#45) without changing
/// the rendered DOM.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PartView {
    id: i64,
    kind: &'static str,
    height: i64,
    objects: Vec<ObjectView>,
}

/// A positioned object, discriminated by `kind` (#60):
/// - `field` objects render their live `value` **only** (an input in an editable
///   view unless read-only); `field_id` names that input `f<id>`. Their caption is
///   a separate `text` object — `label` is still resolved (for the inspector) but
///   no longer rendered inline.
/// - `text` objects render their static `content`.
/// - shape objects (`shape == true`) render a styled box from `shape_style`
///   (derived server-side from `props`, so both renderers just interpolate it).
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
struct ObjectView {
    id: i64,
    kind: &'static str,
    field: bool,
    shape: bool,
    field_id: Option<i64>,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    z: i64,
    read_only: bool,
    binding: String,
    content: String,
    /// The raw appearance bag (#49) the Style zone edits. Carried alongside the
    /// server-derived `shape_style` so the canvas renders from `shape_style` while
    /// the inspector reads/writes the underlying `fill`/`stroke`/… keys. Empty
    /// string when the object has no props.
    props: String,
    label: String,
    value: String,
    shape_style: String,
}

/// A bindable field on the layout's primary table — the Field tool's dropdown
/// choices (#48/#62). Part of the Layout-Mode read model so the rail can offer
/// every field, not only the ones already placed.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldChoice {
    id: i64,
    name: String,
}

#[derive(Template)]
#[template(path = "view_list.html")]
struct ListTemplate {
    chrome: Chrome,
    layout: String,
    table: String,
    width: i64,
    /// Non-body parts (header/title/…) rendered once above the rows.
    header: Vec<PartView>,
    /// One entry per record: the Body part(s) bound to that record.
    rows: Vec<ListRow>,
    /// Footer/grand-summary parts rendered once below the rows.
    footer: Vec<PartView>,
}

/// One record's Body band(s) in List view; `current` marks the flipbook's row.
struct ListRow {
    id: i64,
    current: bool,
    parts: Vec<PartView>,
}

struct FieldView {
    name: String,
}

struct RecordView {
    id: i64,
    cells: Vec<CellView>,
}

/// One Table-view cell: the field id (so editable inputs can be named `f<id>`)
/// and the current value.
struct CellView {
    field_id: i64,
    value: String,
}

#[derive(Template)]
#[template(path = "design.html")]
struct DesignTemplate {
    chrome: Chrome,
    layout_id: i64,
    layout: String,
    /// Which view this layout designs (`Form`/`List`/`Table`) — shown in the
    /// status bar so the designer knows which surface they're editing (#57).
    view: &'static str,
}

/// Home → the first table's Form Browse view (the Form layout is the canonical
/// landing surface now that each view is its own layout, #57).
async fn index(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.layouts().unwrap().into_iter().find(|l| l.view == "form") {
        Some(l) => Redirect::to(&format!("/browse/{}", l.id)).into_response(),
        None => Html("<p>No layouts yet.</p>".to_string()).into_response(),
    }
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
    by_name: &HashMap<String, (i64, String, String)>,
) -> (bool, Option<i64>, String, String) {
    match (o.kind, o.binding.as_deref()) {
        (ObjectKind::Field, Some(binding)) => {
            let seg = binding.rsplit('.').next().unwrap_or(binding).to_lowercase();
            match by_name.get(&seg) {
                Some((id, label, value)) => (true, Some(*id), label.clone(), value.clone()),
                // a binding that doesn't resolve yet (e.g. a relationship path)
                None => (true, None, binding.to_string(), String::new()),
            }
        }
        _ => (false, None, String::new(), String::new()),
    }
}

/// Derive a shape object's inline CSS from its `props` JSON. #49 owns the full
/// appearance contract; this reads the keys a rect/line/ellipse needs — `fill`,
/// `stroke`, `strokeWidth`, `radius`. The string is computed once here and carried
/// in [`ObjectView::shape_style`], so the askama band macro and the Svelte `Band`
/// both just interpolate it — there is no second derivation to keep byte-equal.
/// Empty for absent/invalid props (an unstyled shape falls back to its CSS class).
fn shape_style(props: Option<&str>) -> String {
    let Some(props) = props else { return String::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(props) else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(fill) = v.get("fill").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("background:{fill};"));
    }
    if let Some(stroke) = v.get("stroke").and_then(serde_json::Value::as_str) {
        let width = v
            .get("strokeWidth")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(1);
        s.push_str(&format!("border:{width}px solid {stroke};"));
    }
    if let Some(radius) = v.get("radius").and_then(serde_json::Value::as_i64) {
        s.push_str(&format!("border-radius:{radius}px;"));
    }
    s
}

/// A record's field values keyed by lowercased field name → (field id, display
/// name, value) — the lookup `resolve_object` binds against.
fn by_name_map(fields: &[FieldMeta], cells: Vec<String>) -> HashMap<String, (i64, String, String)> {
    fields
        .iter()
        .zip(cells)
        .map(|(f, value)| (f.name.to_lowercase(), (f.id, f.name.clone(), value)))
        .collect()
}

/// Resolve one object into its `ObjectView` (#44/#60), bound against `by_name`.
/// The single per-object projection shared by [`render_part`] and the create
/// handler, so an object placed on the canvas serialises byte-identically to one
/// read back from the model — there is no second mapping to drift.
fn object_view(o: &ObjectMeta, by_name: &HashMap<String, (i64, String, String)>) -> ObjectView {
    let (field, field_id, label, value) = resolve_object(o, by_name);
    let shape = o.kind.is_shape();
    // The text slot is only meaningful for `text` objects; fields/shapes carry
    // none, so the renderer never reads a stray content.
    let content = match o.kind {
        ObjectKind::Text => o.content.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let shape_style = if shape { shape_style(o.props.as_deref()) } else { String::new() };
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
        label,
        value,
        shape_style,
    }
}

/// Render one part's objects, positioned and bound against `by_name` (an empty
/// map leaves field values blank — used for header/footer with no record).
fn render_part(
    sol: &Solution,
    part: &PartMeta,
    by_name: &HashMap<String, (i64, String, String)>,
) -> PartView {
    let objects = sol
        .objects(part.id)
        .unwrap()
        .iter()
        .map(|o| object_view(o, by_name))
        .collect();
    PartView { id: part.id, kind: part.kind.as_str(), height: part.height, objects }
}

/// Canvas width for a layout: the rightmost object edge + a margin. Geometry is
/// record-independent, so this is the same for every record (Form and List).
fn layout_canvas_width(sol: &Solution, layout_id: i64) -> i64 {
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
fn build_form_record(
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
    Some(FormRecord { id, width: layout_canvas_width(sol, layout_id), parts })
}

/// Build the List-view render: header/footer parts once, the Body part(s)
/// repeated per record bound to its values. `current_rec` (1-based) marks the
/// flipbook's row. Returns `(header, rows, footer)`.
fn build_list(
    sol: &Solution,
    layout_id: i64,
    table: &TableMeta,
    fields: &[FieldMeta],
    ids: &[i64],
    current_rec: i64,
) -> (Vec<PartView>, Vec<ListRow>, Vec<PartView>) {
    let no_record = HashMap::new();
    let (mut header, mut footer, mut body_parts) = (Vec::new(), Vec::new(), Vec::new());
    for p in sol.parts(layout_id).unwrap() {
        match p.kind {
            PartKind::Body => body_parts.push(p),
            PartKind::Footer | PartKind::GrandSummary => {
                footer.push(render_part(sol, &p, &no_record))
            }
            // header / sub-summary render once above the rows.
            PartKind::Header | PartKind::SubSummary => {
                header.push(render_part(sol, &p, &no_record))
            }
        }
    }

    let mut rows = Vec::new();
    for (i, &id) in ids.iter().enumerate() {
        let Some(cells) = sol.get_record(table, fields, id).unwrap() else {
            continue;
        };
        let by_name = by_name_map(fields, cells);
        let parts = body_parts.iter().map(|p| render_part(sol, p, &by_name)).collect();
        rows.push(ListRow { id, current: (i as i64) + 1 == current_rec, parts });
    }
    (header, rows, footer)
}

/// Browse a layout. `?view=table|form|list` (frozen #20) picks the renderer;
/// Table is the field-derived grid, Form/List render the layout's objects.
async fn browse(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    // Each layout renders in its own intrinsic view; the layout id (not `?view=`)
    // selects the surface, so Form/List are independent designs (#57).
    let view = canonical_view(&lay.view);
    let mut chrome = Chrome::build(&sol, "browse", Some(&lay));

    // Found set + flipbook position drive record navigation across all views.
    let ids = sol.record_ids(&table).unwrap();
    let total = ids.len() as i64;
    let rec = clamp_rec(&q, total);
    let current_id = if rec >= 1 { ids.get((rec - 1) as usize).copied() } else { None };
    chrome.nav = Some(flipbook(layout_id, view, rec, current_id, total));
    chrome.editing = current_id.is_some_and(|cid| st.lock_held((table.id, cid)));

    match view {
        "form" => {
            let fields = sol.fields(table.id).unwrap();
            let record = build_form_record(&sol, layout_id, &table, &fields, &ids, rec);
            Html(
                FormTemplate {
                    chrome,
                    layout: lay.name.clone(),
                    table: table.name.clone(),
                    record,
                }
                .render()
                .unwrap(),
            )
            .into_response()
        }
        "list" => {
            let fields = sol.fields(table.id).unwrap();
            let (header, rows, footer) =
                build_list(&sol, layout_id, &table, &fields, &ids, rec);
            Html(
                ListTemplate {
                    chrome,
                    layout: lay.name.clone(),
                    table: table.name.clone(),
                    width: layout_canvas_width(&sol, layout_id),
                    header,
                    rows,
                    footer,
                }
                .render()
                .unwrap(),
            )
            .into_response()
        }
        _ => {
            let fields = sol.fields(table.id).unwrap();
            let records = sol.list_records(&table, &fields).unwrap();
            let tmpl = TableTemplate {
                chrome,
                layout_id,
                table: table.name.clone(),
                fields: fields
                    .iter()
                    .map(|f| FieldView { name: f.name.clone() })
                    .collect(),
                records: records
                    .into_iter()
                    .map(|r| RecordView {
                        id: r.id,
                        cells: fields
                            .iter()
                            .zip(r.cells)
                            .map(|(f, value)| CellView { field_id: f.id, value })
                            .collect(),
                    })
                    .collect(),
            };
            Html(tmpl.render().unwrap()).into_response()
        }
    }
}

/// Layout (design) mode shell. Renders the chrome + the Svelte editor mount node;
/// the canvas itself is drawn client-side by the editor, which fetches geometry
/// from [`design_model`] (#44) and renders objects from the same fields the
/// askama band macro uses, so Browse and Layout stay pixel-identical.
async fn design(State(st): State<AppState>, Path(layout_id): Path<i64>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, _table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let mut chrome = Chrome::build(&sol, "design", Some(&lay));
    // Keep the pagination control in Layout mode — repurposed to step layouts.
    chrome.nav = layout_stepper(&sol, &lay);
    let tmpl = DesignTemplate { chrome, layout_id, layout: lay.name.clone(), view: view_label(&lay.view) };
    Html(tmpl.render().unwrap()).into_response()
}

/// The Layout-Mode read model (#44): the layout's parts/objects with resolved
/// labels + live values for record `?rec=N` (1-based; defaults to the first
/// record, blank values when the table is empty — geometry is record-independent,
/// so an empty table still has a designable canvas). The Svelte canvas renders
/// from this over the same axum contract Browse uses (ADR #42). `render_part` is
/// the single server-side resolver shared with Browse, so values/bindings can
/// never diverge between the two surfaces; only the DOM emission is mirrored
/// client-side (and guarded by a parity test).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DesignModel {
    layout_id: i64,
    rec: i64,
    total: i64,
    width: i64,
    /// The primary table's fields — what the Create zone's Field tool offers
    /// (#48/#62). Geometry-independent, so the same list rides every record.
    fields: Vec<FieldChoice>,
    parts: Vec<PartView>,
}

async fn design_model(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let ids = sol.record_ids(&table).unwrap();
    let total = ids.len() as i64;
    let rec = clamp_rec(&q, total);
    let fields = sol.fields(table.id).unwrap();
    // Bind to the record at `rec` when present; otherwise render geometry blank.
    let by_name = if rec >= 1 {
        match sol.get_record(&table, &fields, ids[(rec - 1) as usize]).unwrap() {
            Some(cells) => by_name_map(&fields, cells),
            None => HashMap::new(),
        }
    } else {
        HashMap::new()
    };
    let parts = sol
        .parts(layout_id)
        .unwrap()
        .iter()
        .map(|p| render_part(&sol, p, &by_name))
        .collect();
    let field_choices = fields
        .iter()
        .map(|f| FieldChoice { id: f.id, name: f.name.clone() })
        .collect();
    let model = DesignModel {
        layout_id,
        rec,
        total,
        width: layout_canvas_width(&sol, layout_id),
        fields: field_choices,
        parts,
    };
    axum::Json(model).into_response()
}

/// The geometry a Layout-canvas drag/resize commits for one object (#15) —
/// part-relative px integers mirroring the #43 geometry contract.
#[derive(serde::Deserialize)]
struct GeometryUpdate {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

/// Persist one object's new geometry from the Layout canvas (#15): the canvas
/// POSTs `{x,y,w,h}` after a drag and this writes it to `meta_object`, scoped to
/// the layout. Coordinates clamp to the canvas origin (no negative part-relative
/// geometry) and to a 1px minimum size, so a stray value can't push an object off
/// the top-left or collapse it. 200 on success; 404 when no such object belongs to
/// the layout. The geometry is authoritative, so Browse shows it on the next read.
async fn update_object_geometry(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(geom): Json<GeometryUpdate>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let updated = sol
        .set_object_geometry(
            layout_id,
            object_id,
            geom.x.max(0),
            geom.y.max(0),
            geom.w.max(1),
            geom.h.max(1),
        )
        .unwrap();
    if updated == 0 {
        return StatusCode::NOT_FOUND.into_response();
    }
    StatusCode::OK.into_response()
}

/// One object's geometry in a bulk commit (#46): the object id plus its new box.
#[derive(serde::Deserialize)]
struct ObjectGeometry {
    id: i64,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

/// Persist a whole group's geometry from the Layout canvas (#46): the canvas
/// POSTs `[{id,x,y,w,h}, …]` after a multi-select drag/resize and this writes
/// them in one transaction, each scoped to the layout and clamped like the
/// single-object commit. Always 200 (unknown ids are simply skipped); the body
/// is the count actually updated, so the client can detect a stale selection.
async fn update_objects_geometry(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(items): Json<Vec<ObjectGeometry>>,
) -> impl IntoResponse {
    let clamped: Vec<(i64, i64, i64, i64, i64)> = items
        .iter()
        .map(|g| (g.id, g.x.max(0), g.y.max(0), g.w.max(1), g.h.max(1)))
        .collect();
    let mut sol = st.sol.lock().unwrap();
    let updated = sol.set_objects_geometry(layout_id, &clamped).unwrap();
    (StatusCode::OK, updated.to_string()).into_response()
}

/// Clamp a client-sent record number into the found set (1-based, `0` when
/// empty) — the create handler's equivalent of [`clamp_rec`] for a typed body.
fn clamp_rec_n(rec: Option<i64>, total: i64) -> i64 {
    if total <= 0 {
        return 0;
    }
    rec.unwrap_or(1).clamp(1, total)
}

/// One object the Create zone places (#48). `kind` is the [`ObjectKind`] string;
/// for a `field` the `field_id` names which field to bind (the server builds the
/// `Table.Field` binding + spawns the caption label per #60). `rec` is the record
/// the canvas is showing, so the returned object resolves its live value to match.
/// `props` is the optional appearance bag for a shape.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateObjectBody {
    part_id: i64,
    kind: String,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    rec: Option<i64>,
    field_id: Option<i64>,
    content: Option<String>,
    props: Option<serde_json::Value>,
}

/// Create an object on a layout part from the Create zone (#48). Resolves the
/// requested record so the returned object(s) carry the same live value/label the
/// model would, and returns them as `ObjectView`s for the store to add WITHOUT a
/// re-hydrate (so the canvas's undo history survives a placement). A `field`
/// returns BOTH its value object and its spawned caption label (#60); other kinds
/// return one. 404 when the part isn't in the layout; 400 on a bad kind/field.
async fn create_design_object(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<CreateObjectBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let Some(kind) = ObjectKind::parse(&body.kind) else {
        return (StatusCode::BAD_REQUEST, "bad object kind").into_response();
    };
    let fields = sol.fields(table.id).unwrap();
    let ids = sol.record_ids(&table).unwrap();
    let rec = clamp_rec_n(body.rec, ids.len() as i64);
    let by_name = if rec >= 1 {
        match sol.get_record(&table, &fields, ids[(rec - 1) as usize]).unwrap() {
            Some(cells) => by_name_map(&fields, cells),
            None => HashMap::new(),
        }
    } else {
        HashMap::new()
    };

    let created_ids: Vec<i64> = if kind == ObjectKind::Field {
        let Some(fid) = body.field_id else {
            return (StatusCode::BAD_REQUEST, "field tool needs a fieldId").into_response();
        };
        let Some(f) = fields.iter().find(|f| f.id == fid) else {
            return (StatusCode::BAD_REQUEST, "no such field").into_response();
        };
        let binding = format!("{}.{}", table.name, f.name);
        let label = f.name.clone();
        match sol
            .create_field_object(layout_id, body.part_id, &binding, &label, body.x, body.y, body.w, body.h)
            .unwrap()
        {
            Some((label_id, field_id)) => vec![label_id, field_id],
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        let content = match kind {
            ObjectKind::Text => Some(body.content.clone().unwrap_or_default()),
            _ => None,
        };
        let props = body.props.as_ref().map(|v| v.to_string());
        let new = NewObject {
            part_id: body.part_id,
            kind,
            x: body.x,
            y: body.y,
            w: body.w,
            h: body.h,
            binding: None,
            content,
            props,
        };
        match sol.create_object(layout_id, &new).unwrap() {
            Some(id) => vec![id],
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };

    // Re-read the freshly inserted rows and project them exactly as the model
    // would, so the store's added object is byte-identical to a model fetch.
    let part_objs = sol.objects(body.part_id).unwrap();
    let views: Vec<ObjectView> = created_ids
        .iter()
        .filter_map(|id| part_objs.iter().find(|o| o.id == *id))
        .map(|o| object_view(o, &by_name))
        .collect();
    axum::Json(views).into_response()
}

/// A band the Create zone adds (#48): the [`PartKind`] string and an optional
/// height (defaults to a workable band height).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePartBody {
    kind: String,
    height: Option<i64>,
}

/// Append a band to a layout (#48) and return its `PartView` (no objects yet) so
/// the store can add it without a re-hydrate. 404 unknown layout; 400 bad kind.
async fn create_design_part(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<CreatePartBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    if layout_table(&sol, layout_id).is_none() {
        return not_found("layout", layout_id);
    }
    let Some(kind) = PartKind::parse(&body.kind) else {
        return (StatusCode::BAD_REQUEST, "bad part kind").into_response();
    };
    let height = body.height.unwrap_or(80).max(1);
    let id = sol.create_part(layout_id, kind, height).unwrap();
    axum::Json(PartView { id, kind: kind.as_str(), height, objects: Vec::new() }).into_response()
}

/// Delete an object from a layout (#48) — the Create zone's delete and the undo
/// of a create. 200 when removed, 404 when no such object belongs to the layout.
async fn delete_design_object(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.delete_object(layout_id, object_id).unwrap() {
        0 => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::OK.into_response(),
    }
}

/// The appearance bag the Style zone commits (#49) — an opaque JSON object the
/// server stores verbatim and re-derives the shape style from on the next read.
#[derive(serde::Deserialize)]
struct PropsBody {
    props: serde_json::Value,
}

/// The canvas-facing result of a props commit (#49): the freshly **server-derived**
/// shape style, so the canvas updates without a client-side re-derivation (the
/// single-source rule, [[layout-object-types]]). Empty for a non-shape object.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PropsResult {
    shape_style: String,
}

/// Persist an object's `props` from the Style zone (#49), layout-scoped, and echo
/// back the re-derived shape style for the canvas. 200 on success, 404 when no
/// such object belongs to the layout.
async fn update_object_props(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<PropsBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let props = body.props.to_string();
    if sol.set_object_props(layout_id, object_id, &props).unwrap() == 0 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let shape_style = match sol.object_by_id(layout_id, object_id).unwrap() {
        Some(o) if o.kind.is_shape() => shape_style(o.props.as_deref()),
        _ => String::new(),
    };
    axum::Json(PropsResult { shape_style }).into_response()
}

/// Directory holding the built Layout Mode editor bundle (Svelte 5 + Vite static
/// output), relative to the server's working directory. Empty until the frontend
/// is built (`cd ui && npm install && npm run build`).
const UI_DIST: &str = "ui/dist";

/// Map a file extension to a content type for the editor bundle. Only the few
/// kinds Vite emits are listed; anything else falls back to octet-stream.
fn ui_content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Serve the built Layout Mode editor bundle from `ui/dist` under the stable
/// `/ui/...` prefix (ADR #42: the island is a static bundle served by axum). Vite
/// emits predictable, non-hashed names (`layout-editor.js` / `layout-editor.css`),
/// so the design page references them by a fixed path. Requests 404 until the
/// frontend is built. A small `tokio::fs` handler keeps this dependency-free
/// rather than pulling in a static-file crate.
async fn ui_asset(Path(path): Path<String>) -> impl IntoResponse {
    // Reject path traversal and empty segments before touching the filesystem.
    if path
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let full = std::path::Path::new(UI_DIST).join(&path);
    match tokio::fs::read(&full).await {
        Ok(bytes) => {
            ([(axum::http::header::CONTENT_TYPE, ui_content_type(&path))], bytes).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Create a record from the new-record form (inputs named `f<field_id>`).
async fn create_record(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let mut target = format!("/browse/{layout_id}");
    {
        let sol = st.sol.lock().unwrap();
        if let Some((lay, table)) = layout_table(&sol, layout_id) {
            let fields = sol.fields(table.id).unwrap();
            let values = collect_values(&fields, &form);
            sol.insert_record(&table, &values).unwrap();
            // Land on the new record: it sorts last by id (record_ids is ORDER BY id).
            let total = sol.record_ids(&table).unwrap().len();
            let view = view_param(&form, &lay.view);
            target = format!("/browse/{layout_id}?view={view}&rec={total}");
        }
    }
    Redirect::to(&target)
}

/// Commit a record: write the buffered field values, release the edit lock, and
/// stay on the record. The form carries `view`/`rec` so the redirect lands back
/// on the same record in the same view (the "commit on exit" half of #40).
async fn save_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let mut target = format!("/browse/{layout_id}");
    {
        let sol = st.sol.lock().unwrap();
        if let Some((lay, table)) = layout_table(&sol, layout_id) {
            let fields = sol.fields(table.id).unwrap();
            let values = collect_values(&fields, &form);
            sol.update_record(&table, id, &values).unwrap();
            st.locks.lock().unwrap().remove(&(table.id, id));
            let view = view_param(&form, &lay.view);
            let rec = clamp_rec(&form, sol.record_ids(&table).unwrap().len() as i64);
            target = format!("/browse/{layout_id}?view={view}&rec={rec}");
        }
    }
    Redirect::to(&target)
}

/// Open a record for editing: acquire its in-process lock. 200 once held (the
/// single session may re-open its own lock); 409 if held elsewhere (multi-user,
/// not reachable yet); 404 for an unknown layout. The "open on focus" half of #40.
async fn open_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let table_id = {
        let sol = st.sol.lock().unwrap();
        match layout_table(&sol, layout_id) {
            Some((_lay, table)) => table.id,
            None => return (StatusCode::NOT_FOUND, "no such layout"),
        }
    };
    st.locks.lock().unwrap().insert((table_id, id));
    (StatusCode::OK, "open")
}

/// Revert: release the edit lock without writing (the "Escape" path of #40). The
/// client discards its buffer and reloads to the committed values.
async fn revert_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if let Some(table_id) = {
        let sol = st.sol.lock().unwrap();
        layout_table(&sol, layout_id).map(|(_lay, table)| table.id)
    } {
        st.locks.lock().unwrap().remove(&(table_id, id));
    }
    (StatusCode::OK, "reverted")
}

/// Delete a record, then back to the same view near where you were. The form
/// carries the current `view` and `rec` so the redirect can preserve both.
async fn delete_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let mut target = format!("/browse/{layout_id}");
    {
        let sol = st.sol.lock().unwrap();
        if let Some((lay, table)) = layout_table(&sol, layout_id) {
            sol.delete_record(&table, id).unwrap();
            let total = sol.record_ids(&table).unwrap().len() as i64;
            let view = view_param(&form, &lay.view);
            target = if total > 0 {
                // Stay put if possible; clamp into the now-shorter found set.
                let rec = clamp_rec(&form, total);
                format!("/browse/{layout_id}?view={view}&rec={rec}")
            } else {
                format!("/browse/{layout_id}?view={view}")
            };
        }
    }
    Redirect::to(&target)
}

/// Pull `f<field_id>` form values into engine `(field, value)` pairs.
fn collect_values<'a>(
    fields: &'a [record_maker_engine::FieldMeta],
    form: &HashMap<String, String>,
) -> Vec<(&'a record_maker_engine::FieldMeta, String)> {
    fields
        .iter()
        .filter_map(|f| form.get(&format!("f{}", f.id)).map(|v| (f, v.clone())))
        .collect()
}

/// Seed a demo "Customers" table on first run so there's something to browse.
fn seed(sol: &mut Solution) -> anyhow::Result<()> {
    if sol.tables()?.is_empty() {
        sol.create_table(
            "Customers",
            &[
                NewField { name: "Name".into(), kind: FieldKind::Text },
                NewField { name: "Email".into(), kind: FieldKind::Text },
                NewField { name: "Age".into(), kind: FieldKind::Number },
            ],
        )?;
    }
    Ok(())
}

/// Build the router. A fn so the Tauri shell (#16) embeds the same app.
fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/browse/:layout", get(browse).post(create_record))
        .route("/browse/:layout/:id", post(save_record))
        .route("/browse/:layout/:id/open", post(open_record))
        .route("/browse/:layout/:id/revert", post(revert_record))
        .route("/browse/:layout/:id/delete", post(delete_record))
        .route("/design/:layout", get(design))
        .route("/design/:layout/model", get(design_model))
        .route("/design/:layout/object", post(create_design_object))
        .route("/design/:layout/part", post(create_design_part))
        .route("/design/:layout/object/:id/geometry", post(update_object_geometry))
        .route("/design/:layout/object/:id/props", post(update_object_props))
        .route("/design/:layout/object/:id/delete", post(delete_design_object))
        .route("/design/:layout/geometry", post(update_objects_geometry))
        .route("/ui/*path", get(ui_asset))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let mut sol = Solution::open("./.rm-data").expect("open solution");
    seed(&mut sol).expect("seed");
    let state = AppState {
        sol: Arc::new(Mutex::new(sol)),
        locks: Arc::new(Mutex::new(HashSet::new())),
    };

    let addr = "127.0.0.1:4317";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    println!("record-maker → http://{addr}");
    axum::serve(listener, app(state)).await.expect("serve");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare Form chrome with a flipbook present (the band only renders inside
    /// the `<form>`, which requires `chrome.nav` to be `Some`).
    fn form_chrome() -> Chrome {
        Chrome {
            mode: "browse",
            layouts: Vec::new(),
            current_layout: Some(1),
            view_tabs: Vec::new(),
            nav: Some(flipbook(1, "form", 1, Some(1), 1)),
            editing: false,
        }
    }

    fn field_obj(field_id: i64, value: &str, read_only: bool) -> ObjectView {
        ObjectView {
            id: field_id,
            kind: "field",
            field: true,
            shape: false,
            field_id: Some(field_id),
            x: 0,
            y: 0,
            w: 100,
            h: 24,
            z: 0,
            read_only,
            binding: format!("T.Field{field_id}"),
            content: String::new(),
            props: String::new(),
            label: format!("Field {field_id}"),
            value: value.to_string(),
            shape_style: String::new(),
        }
    }

    /// The #43 acceptance: a read-only object renders a non-editable value, while
    /// an editable object in the same (editable) Form view renders an input.
    #[test]
    fn read_only_object_renders_value_editable_object_renders_input() {
        let part = PartView {
            id: 1,
            kind: "body",
            height: 60,
            objects: vec![
                field_obj(1, "EDITABLE_VAL", false),
                field_obj(2, "READONLY_VAL", true),
            ],
        };
        let tmpl = FormTemplate {
            chrome: form_chrome(),
            layout: "L".into(),
            table: "T".into(),
            record: Some(FormRecord { id: 1, width: 200, parts: vec![part] }),
        };
        let html = tmpl.render().unwrap();

        // Editable object → an input bound to f1 carrying its value.
        assert!(
            html.contains(r#"name="f1""#) && html.contains(r#"value="EDITABLE_VAL""#),
            "editable object should render an input"
        );
        // Read-only object → no input for f2; its value shows in a read-only span.
        assert!(
            !html.contains(r#"name="f2""#),
            "read-only object must not render an editable input"
        );
        assert!(
            html.contains("fm-readonly") && html.contains("READONLY_VAL"),
            "read-only object should render its value as a non-editable span"
        );
    }

    /// z-order reaches the DOM as an explicit CSS `z-index` so overlap is
    /// deterministic regardless of source order.
    #[test]
    fn object_z_order_renders_as_css_z_index() {
        let mut o = field_obj(1, "v", false);
        o.z = 7;
        let tmpl = FormTemplate {
            chrome: form_chrome(),
            layout: "L".into(),
            table: "T".into(),
            record: Some(FormRecord {
                id: 1,
                width: 200,
                parts: vec![PartView { id: 1, kind: "body", height: 60, objects: vec![o] }],
            }),
        };
        assert!(tmpl.render().unwrap().contains("z-index:7"));
    }

    /// End-to-end through the real route: a default form is all-editable, but
    /// once a field object is flagged read-only the Browse Form view stops
    /// rendering an input for it (and keeps the input for editable fields) — the
    /// #43 read-only flag honored by Browse, wired engine → handler → template.
    #[tokio::test]
    async fn browse_form_honors_per_object_read_only_end_to_end() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt; // for `oneshot`

        let mut sol = Solution::open_in_memory().unwrap();
        let tid = sol
            .create_table(
                "Customers",
                &[
                    NewField { name: "Name".into(), kind: FieldKind::Text },
                    NewField { name: "Email".into(), kind: FieldKind::Text },
                ],
            )
            .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(tid).unwrap();
        let (name_fid, email_fid) = (fields[0].id, fields[1].id);
        sol.insert_record(
            &table,
            &[(&fields[0], "Ada".into()), (&fields[1], "ada@x.com".into())],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        // Flag the Name object read-only (what the Layout canvas will do, #47).
        sol.app
            .execute(
                "UPDATE meta_object SET read_only=1 WHERE binding='Customers.Name'",
                [],
            )
            .unwrap();

        let state = AppState {
            sol: Arc::new(Mutex::new(sol)),
            locks: Arc::new(Mutex::new(HashSet::new())),
        };
        let req = Request::builder()
            .uri(format!("/browse/{layout_id}?view=form"))
            .body(Body::empty())
            .unwrap();
        let resp = app(state).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();

        // Read-only Name: value shown, but no editable input bound to it.
        assert!(html.contains("Ada"), "read-only value still rendered");
        assert!(
            !html.contains(&format!(r#"name="f{name_fid}""#)),
            "read-only field must not render an input"
        );
        assert!(html.contains("fm-readonly"), "read-only object marked in markup");
        // Editable Email: input present.
        assert!(
            html.contains(&format!(r#"name="f{email_fid}""#))
                && html.contains(r#"value="ada@x.com""#),
            "editable field still renders an input"
        );
    }

    // ---- #44 shared-renderer parity oracle --------------------------------
    //
    // The Layout canvas (Svelte) renders objects from the same fields the askama
    // band macro uses. These tests pin BOTH ends of that to committed goldens:
    //   - `canvas.parity.html`  — the canonical band DOM (this macro is the spec).
    //   - `canvas.fixture.json` — the exact `/design/:layout/model` response.
    // The Svelte side (ui/) renders `LayoutPreview` from the SAME fixture JSON and
    // asserts it normalizes to the SAME canvas golden, so neither renderer can
    // drift. `normalize_html` is the shared contract — keep it byte-equal to the
    // JS copy in `ui/scripts/parity-check.mjs`.
    //
    // Run `REGEN=1 cargo test -p record-maker-server` to (re)generate the goldens
    // from the live macro/endpoint output after an intentional DOM change.

    /// Strip HTML comments, collapse whitespace runs to one space, then drop
    /// spaces adjacent to tag boundaries. This absorbs (1) Svelte 5 SSR hydration
    /// markers like `<!--[-->`/`<!---->` (the macro emits none, so stripping is a
    /// no-op on the Browse side) and (2) harmless indentation/newline differences,
    /// while preserving text content and attribute strings. The JS copy in
    /// `ui/scripts/parity-check.mjs` MUST stay byte-equivalent to this.
    fn normalize_html(s: &str) -> String {
        // 1. remove `<!-- ... -->` comments.
        let mut decommented = String::with_capacity(s.len());
        let mut rest = s;
        loop {
            match rest.find("<!--") {
                None => {
                    decommented.push_str(rest);
                    break;
                }
                Some(i) => {
                    decommented.push_str(&rest[..i]);
                    match rest[i..].find("-->") {
                        Some(j) => rest = &rest[i + j + 3..],
                        None => break,
                    }
                }
            }
        }
        // 2. collapse whitespace runs to a single space.
        let mut collapsed = String::with_capacity(decommented.len());
        let mut prev_ws = false;
        for c in decommented.chars() {
            if c.is_whitespace() {
                if !prev_ws {
                    collapsed.push(' ');
                }
                prev_ws = true;
            } else {
                collapsed.push(c);
                prev_ws = false;
            }
        }
        // 3. drop spaces adjacent to tag boundaries.
        collapsed.replace("> ", ">").replace(" <", "<").trim().to_string()
    }

    fn golden_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/tests")
            .join(name)
    }

    /// Assert `actual` equals the committed golden, or (re)write it under `REGEN`.
    fn assert_or_regen(name: &str, actual: &str) {
        let path = golden_path(name);
        if std::env::var("REGEN").is_ok() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, actual).unwrap();
            eprintln!("[REGEN] wrote {}", path.display());
            return;
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing golden {name}; run `REGEN=1 cargo test`"));
        assert_eq!(actual.trim(), expected.trim(), "golden {name} drifted");
    }

    /// A deterministic layout for parity: the default Customers form (per field a
    /// label `text` object + a value `field` object, #60), both field objects made
    /// read-only (so Browse renders the display/non-editing state #44 compares),
    /// Email lifted to z=5, plus a free static `text` object and a `rect` shape with
    /// appearance props — covering fm-field / fm-readonly / z-index / fm-text /
    /// fm-shape and the server-derived shape_style in one fixture.
    fn parity_fixture() -> (Solution, i64) {
        let mut sol = Solution::open_in_memory().unwrap();
        let tid = sol
            .create_table(
                "Customers",
                &[
                    NewField { name: "Name".into(), kind: FieldKind::Text },
                    NewField { name: "Email".into(), kind: FieldKind::Text },
                ],
            )
            .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(tid).unwrap();
        sol.insert_record(
            &table,
            &[(&fields[0], "Ada".into()), (&fields[1], "ada@example.com".into())],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        sol.app
            .execute("UPDATE meta_object SET read_only=1 WHERE binding='Customers.Name'", [])
            .unwrap();
        sol.app
            .execute(
                "UPDATE meta_object SET read_only=1, z=5 WHERE binding='Customers.Email'",
                [],
            )
            .unwrap();
        let part_id: i64 = sol
            .app
            .query_row(
                "SELECT id FROM meta_part WHERE layout_id=?1 AND kind='body'",
                [layout_id],
                |r| r.get(0),
            )
            .unwrap();
        sol.app
            .execute(
                "INSERT INTO meta_object(part_id, kind, x, y, w, h, z, content) \
                 VALUES (?1, 'text', 16, 80, 200, 24, 0, 'Note')",
                [part_id],
            )
            .unwrap();
        // A rect shape with appearance props — drives the shape kind + the
        // server-derived shape_style through the byte-equal parity gate.
        sol.app
            .execute(
                "INSERT INTO meta_object(part_id, kind, x, y, w, h, z, props) \
                 VALUES (?1, 'rect', 230, 16, 64, 64, 0, \
                 '{\"fill\":\"#eef\",\"stroke\":\"#88a\",\"strokeWidth\":1,\"radius\":4}')",
                [part_id],
            )
            .unwrap();
        (sol, layout_id)
    }

    async fn get_body(state: AppState, uri: &str) -> (StatusCode, String) {
        use axum::http::Request;
        use tower::ServiceExt;
        let resp = app(state)
            .oneshot(Request::builder().uri(uri).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn state_for(sol: Solution) -> AppState {
        AppState {
            sol: Arc::new(Mutex::new(sol)),
            locks: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    async fn post_json(state: AppState, uri: &str, body: &str) -> StatusCode {
        post_json_body(state, uri, body).await.0
    }

    /// POST JSON and return both the status and the response body (for endpoints
    /// that echo the created object/part back to the canvas).
    async fn post_json_body(state: AppState, uri: &str, body: &str) -> (StatusCode, String) {
        use axum::http::Request;
        use tower::ServiceExt;
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    /// #57: a table carries independent per-view layouts. The Browse view toggle
    /// links to sibling layout ids (not one layout re-rendered via `?view=`), and
    /// each layout renders in its own view.
    #[tokio::test]
    async fn browse_view_tabs_link_to_sibling_layouts_and_render_by_view() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(table.id).unwrap();
        sol.insert_record(&table, &[(&fields[0], "Ada".into())]).unwrap();
        let layouts = sol.layouts_for_table(table.id).unwrap();
        let form = layouts.iter().find(|l| l.view == "form").unwrap().id;
        let list = layouts.iter().find(|l| l.view == "list").unwrap().id;
        let table_l = layouts.iter().find(|l| l.view == "table").unwrap().id;
        assert!(form != list && list != table_l && form != table_l, "distinct per-view ids");
        let state = state_for(sol);

        // The Form layout renders the canvas and offers tabs to the SIBLING ids.
        let (status, html) = get_body(state.clone(), &format!("/browse/{form}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"<div class="fm-canvas""#), "form renders the canvas");
        assert!(html.contains(&format!(r#"href="/browse/{list}""#)), "List tab → list layout");
        assert!(html.contains(&format!(r#"href="/browse/{table_l}""#)), "Table tab → table layout");

        // The List layout renders the list surface by its own view, not the canvas.
        let (status, html) = get_body(state, &format!("/browse/{list}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"class="fm-list""#), "list renders the list surface");
        assert!(!html.contains(r#"<div class="fm-canvas""#), "list view is not the form canvas");
    }

    /// #57 Layout-mode chrome: the view toggle stays (switching which view you
    /// DESIGN, via /design/ siblings) and the pagination control is repurposed to
    /// step layouts; record actions are Browse-only.
    #[tokio::test]
    async fn design_mode_keeps_view_toggle_and_layout_stepper() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let layouts = sol.layouts_for_table(table.id).unwrap();
        let form = layouts.iter().find(|l| l.view == "form").unwrap().id;
        let list = layouts.iter().find(|l| l.view == "list").unwrap().id;
        let (status, html) = get_body(state_for(sol), &format!("/design/{form}")).await;
        assert_eq!(status, StatusCode::OK);
        // View toggle present, switching which view you DESIGN (links into /design/).
        assert!(html.contains(&format!(r#"href="/design/{list}""#)), "view toggle → design the List layout");
        // Pagination control repurposed to layout navigation.
        assert!(html.contains("Layout navigation"), "stepper navigates layouts in design mode");
        // Record actions don't belong in Layout mode.
        assert!(html.contains(r#"title="Records are managed in Browse mode""#), "no record actions in layout mode");
    }

    /// #46 group commit: a bulk POST persists every object's geometry in one
    /// request (scoped + clamped), returns the updated count, and skips unknown ids.
    #[tokio::test]
    async fn design_bulk_geometry_persists_group() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[
                NewField { name: "Name".into(), kind: FieldKind::Text },
                NewField { name: "Email".into(), kind: FieldKind::Text },
            ],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part = sol.parts(layout_id).unwrap()[0].clone();
        let objs = sol.objects(part.id).unwrap();
        let (a, b) = (objs[0].id, objs[1].id);
        let state = state_for(sol);

        let resp = {
            use axum::http::Request;
            use tower::ServiceExt;
            let body = format!(
                r#"[{{"id":{a},"x":10,"y":20,"w":100,"h":24}},{{"id":{b},"x":-5,"y":40,"w":100,"h":24}},{{"id":999999,"x":0,"y":0,"w":1,"h":1}}]"#
            );
            app(state.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/design/{layout_id}/geometry"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
        };
        assert_eq!(resp.status(), StatusCode::OK);
        let count = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(String::from_utf8(count.to_vec()).unwrap(), "2", "only real ids count");

        let sol = state.sol.lock().unwrap();
        let after = sol.objects(part.id).unwrap();
        assert_eq!((after[0].x, after[0].y), (10, 20));
        assert_eq!((after[1].x, after[1].y), (0, 40), "negative x clamped to origin");
    }

    /// #62 two-mount rail: the design page renders the `#layout-tools` mount node
    /// in the sidebar (where the Svelte Create/Style/Zoom zones mount, sharing the
    /// canvas store); Browse mode does not.
    #[tokio::test]
    async fn design_page_renders_tool_rail_mount_node() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let form = sol.layouts().unwrap().into_iter().find(|l| l.view == "form").unwrap().id;
        let state = state_for(sol);

        let (status, html) = get_body(state.clone(), &format!("/design/{form}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"id="layout-tools""#), "design page mounts the tool rail");

        let (_, browse) = get_body(state, &format!("/browse/{form}")).await;
        assert!(!browse.contains(r#"id="layout-tools""#), "browse has no tool rail");
    }

    /// #48 create: placing a shape POSTs `{partId,kind,x,y,w,h,props}`, persists a
    /// `meta_object`, and echoes back its `ObjectView` (with the server-derived
    /// shape_style) so the store can add it without a re-hydrate.
    #[tokio::test]
    async fn design_create_shape_object_persists_and_returns_view() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = sol.parts(layout_id).unwrap()[0].id;
        let before = sol.objects(part_id).unwrap().len();
        let state = state_for(sol);

        let body = format!(
            r##"{{"partId":{part_id},"kind":"rect","x":20,"y":12,"w":64,"h":48,"props":{{"fill":"#eef","stroke":"#88a","strokeWidth":1}}}}"##
        );
        let (status, resp) = post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(r#""kind":"rect""#) && resp.contains(r#""shape":true"#));
        assert!(resp.contains(r#""shapeStyle":"background:#eef;border:1px solid #88a;""#), "derived style echoed\n{resp}");
        assert!(resp.contains("strokeWidth"), "raw props echoed for the inspector\n{resp}");

        let sol = state.sol.lock().unwrap();
        let objs = sol.objects(part_id).unwrap();
        assert_eq!(objs.len(), before + 1, "one row inserted");
        assert!(objs.iter().any(|o| o.kind == ObjectKind::Rect && (o.x, o.y) == (20, 12)));
    }

    /// #48/#60 create: the Field tool POSTs `{kind:"field",fieldId,…}` and gets
    /// back TWO views — the value field (live value resolved for the record) and
    /// its spawned caption label.
    #[tokio::test]
    async fn design_create_field_object_spawns_label_and_returns_both() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(table.id).unwrap();
        let name_fid = fields[0].id;
        sol.insert_record(&table, &[(&fields[0], "Ada".into())]).unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = sol.parts(layout_id).unwrap()[0].id;
        let before = sol.objects(part_id).unwrap().len();
        let state = state_for(sol);

        let body = format!(
            r#"{{"partId":{part_id},"kind":"field","x":120,"y":40,"w":200,"h":24,"fieldId":{name_fid},"rec":1}}"#
        );
        let (status, resp) = post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
        assert_eq!(status, StatusCode::OK);
        // The value field resolves "Ada" and binds Customers.Name; the label
        // carries the caption "Name".
        assert!(resp.contains(r#""kind":"field""#) && resp.contains(r#""value":"Ada""#));
        assert!(resp.contains(r#""binding":"Customers.Name""#));
        assert!(resp.contains(r#""kind":"text""#) && resp.contains(r#""content":"Name""#), "label spawned\n{resp}");

        let sol = state.sol.lock().unwrap();
        assert_eq!(sol.objects(part_id).unwrap().len(), before + 2, "value + label inserted");
    }

    /// #48 create-part: POSTing a kind appends a band and echoes its `PartView`.
    #[tokio::test]
    async fn design_create_part_appends_band_and_returns_view() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let before = sol.parts(layout_id).unwrap().len();
        let state = state_for(sol);

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/part"),
            r#"{"kind":"footer","height":40}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(r#""kind":"footer""#) && resp.contains(r#""height":40"#));
        assert_eq!(state.sol.lock().unwrap().parts(layout_id).unwrap().len(), before + 1);
    }

    /// #48 delete + #49 props: a placed object can have its props set (shape style
    /// re-derives on the next read) and can be deleted; both are layout-scoped.
    #[tokio::test]
    async fn design_object_props_then_delete_round_trip() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = sol.parts(layout_id).unwrap()[0].id;
        let state = state_for(sol);

        // Create a rect to operate on.
        let (status, _) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object"),
            &format!(r#"{{"partId":{part_id},"kind":"rect","x":0,"y":0,"w":40,"h":40}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rect_id = {
            let sol = state.sol.lock().unwrap();
            sol.objects(part_id).unwrap().iter().find(|o| o.kind == ObjectKind::Rect).unwrap().id
        };

        // Set props → the model now derives a shape_style from them.
        let status = post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/{rect_id}/props"),
            r##"{"props":{"fill":"#102030","radius":6}}"##,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
        assert!(model.contains("background:#102030;border-radius:6px;"), "props drive shape_style\n{model}");

        // Delete it (scoped): a foreign layout is a no-op 404, the real one 200.
        assert_eq!(
            post_json(state.clone(), &format!("/design/{}/object/{rect_id}/delete", layout_id + 999), "").await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            post_json(state.clone(), &format!("/design/{layout_id}/object/{rect_id}/delete"), "").await,
            StatusCode::OK
        );
        assert!(!state.sol.lock().unwrap().objects(part_id).unwrap().iter().any(|o| o.id == rect_id));
    }

    /// #15 round-trip: POSTing new geometry persists to `meta_object` (scoped to
    /// the layout) and is visible on the next read; bad ids 404 and change nothing;
    /// negative coordinates clamp to the canvas origin.
    #[tokio::test]
    async fn design_object_geometry_persists_clamps_and_is_scoped() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part = sol.parts(layout_id).unwrap()[0].clone();
        let obj_id = sol.objects(part.id).unwrap()[0].id;
        let state = state_for(sol);

        let geom = |state: &AppState| {
            let sol = state.sol.lock().unwrap();
            let o = &sol.objects(part.id).unwrap()[0];
            (o.x, o.y, o.w, o.h)
        };

        // A drag commit persists and round-trips.
        let status = post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/{obj_id}/geometry"),
            r#"{"x":33,"y":44,"w":120,"h":30}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(geom(&state), (33, 44, 120, 30));

        // Negative coordinates clamp to the origin (and size to a 1px floor).
        let status = post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/{obj_id}/geometry"),
            r#"{"x":-50,"y":-9,"w":0,"h":-3}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(geom(&state), (0, 0, 1, 1));

        // Unknown object ⇒ 404.
        let status = post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/999999/geometry"),
            r#"{"x":1,"y":1,"w":1,"h":1}"#,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Foreign layout id ⇒ 404 (scoped); geometry unchanged.
        let status = post_json(
            state.clone(),
            &format!("/design/{}/object/{obj_id}/geometry", layout_id + 999),
            r#"{"x":5,"y":5,"w":5,"h":5}"#,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(geom(&state), (0, 0, 1, 1));
    }

    /// The `/design/:layout/model` JSON is the read contract the canvas hydrates
    /// from; pin it to a committed fixture so the Svelte side renders the same
    /// model. Also sanity-checks the shape inline.
    #[tokio::test]
    async fn design_model_endpoint_matches_committed_fixture() {
        let (sol, layout_id) = parity_fixture();
        let (status, body) = get_body(state_for(sol), &format!("/design/{layout_id}/model?rec=1")).await;
        assert_eq!(status, StatusCode::OK);
        // Shape sanity (independent of the golden), so a contract change is loud.
        for needle in [
            r#""width":320"#,
            r#""kind":"field""#,
            r#""kind":"text""#,
            r#""kind":"rect""#,
            r#""readOnly":true"#,
            r#""binding":"Customers.Name""#,
            r#""value":"Ada""#,
            r#""content":"Name""#,
            r#""content":"Note""#,
            r#""shape":true"#,
            r#""shapeStyle":"background:#eef;border:1px solid #88a;border-radius:4px;""#,
            r#""z":5"#,
        ] {
            assert!(body.contains(needle), "model JSON missing {needle}\n{body}");
        }
        assert_or_regen("canvas.fixture.json", &body);
    }

    /// Browse renders the parity fixture's canvas; this is the canonical band DOM
    /// the Svelte `LayoutPreview` must reproduce (the macro is the spec).
    #[tokio::test]
    async fn browse_canvas_matches_parity_golden() {
        let (sol, layout_id) = parity_fixture();
        let (status, html) = get_body(state_for(sol), &format!("/browse/{layout_id}?view=form")).await;
        assert_eq!(status, StatusCode::OK);
        // The form holds exactly one `.fm-canvas`; slice it out up to `</form>`.
        let start = html.find(r#"<div class="fm-canvas""#).expect("canvas present");
        let end = start + html[start..].find("</form>").expect("form closes");
        let canvas = normalize_html(&html[start..end]);
        assert!(canvas.starts_with(r#"<div class="fm-canvas""#) && canvas.ends_with("</div>"));
        assert_or_regen("canvas.parity.html", &canvas);
    }
}
