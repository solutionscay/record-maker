//! Browse/Layout mode runtime + app shell. Routing is **layout-keyed**
//! (ADR-0005): `/browse/:layout` and `/design/:layout`, where `:layout` is the
//! meta_layout **id** (i64). One generic handler set serves every table by
//! reading metadata — no per-table code.
//!
//! This crate is a **library + thin bin**: the router, handlers, and state live
//! here so both the standalone CLI (`src/main.rs`) and the Tauri desktop shell
//! (#16) embed the *same* app. The public API is intentionally small — build an
//! [`AppState`], call [`app`] for the router, [`seed`] for demo data, and
//! [`serve`] to bind an ephemeral loopback port and learn the assigned address.

use std::collections::{HashMap, HashSet};
use std::future::IntoFuture;
use std::net::SocketAddr;
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
    FieldKind, FieldMeta, LayoutMeta, NewField, NewObject, NewRelationship, ObjectGroup,
    ObjectKind, ObjectMeta, PartKind, PartMeta, RelationshipMeta, RestoreObject, RestoreResult,
    Solution, TableMeta,
};

mod format;

/// Default UI asset base directory (relative to the working directory). Used by
/// [`AppState::new`] and the thin CLI when `RM_UI_DIR` is unset.
pub const DEFAULT_UI_DIR: &str = "ui/dist";

#[derive(Clone)]
pub struct AppState {
    pub sol: Arc<Mutex<Solution>>,
    /// Records currently "open" for editing, keyed `(table_id, record_id)`.
    /// In-process is enough today (single-user desktop); the open→commit→release
    /// lifecycle is the point, and the registry is where multi-user lock
    /// enforcement will later hook in (#40).
    pub locks: Arc<Mutex<HashSet<(i64, i64)>>>,
    /// Base directory the `/ui/*` handler serves the built editor bundle from.
    /// Configurable so the desktop shell (#16) can point it at its bundled
    /// resource dir; defaults to [`DEFAULT_UI_DIR`] for CLI/dev use.
    pub ui_base_dir: String,
}

impl AppState {
    /// Build a state around an opened [`Solution`], with an empty lock registry
    /// and the default UI asset directory.
    pub fn new(sol: Solution) -> Self {
        AppState {
            sol: Arc::new(Mutex::new(sol)),
            locks: Arc::new(Mutex::new(HashSet::new())),
            ui_base_dir: DEFAULT_UI_DIR.to_string(),
        }
    }

    /// Override the base directory the `/ui/*` handler serves assets from.
    pub fn with_ui_dir(mut self, dir: impl Into<String>) -> Self {
        self.ui_base_dir = dir.into();
        self
    }

    fn lock_held(&self, key: (i64, i64)) -> bool {
        self.locks.lock().unwrap().contains(&key)
    }
}

/// Persistent shell context shared by every page (the chrome).
struct Chrome {
    mode: &'static str, // "browse" | "design" | "schema"
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
    let n = q
        .get("rec")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(1);
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
    /// Header/footer bands framing the grid, matching List/Form Browse views.
    header: Vec<PartView>,
    footer: Vec<PartView>,
    fields: Vec<FieldView>,
    records: Vec<RecordView>,
}

#[derive(Template)]
#[template(path = "view_form.html")]
struct FormTemplate {
    chrome: Chrome,
    table: String,
    /// The record at the flipbook's current position; `None` when empty.
    record: Option<FormRecord>,
}

/// One record laid out per the layout's parts/objects, with live values (#25).
struct FormRecord {
    id: i64,
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
    /// The raw appearance bag (#49/Issue 7) the Band inspector edits, carried
    /// alongside the server-derived `part_style` so the inspector reads/writes the
    /// underlying `fill` key while Browse/canvas render from `part_style`. Empty
    /// string when the band has no props.
    props: String,
    /// Server-derived inline CSS for the band's `<div class="fm-part">` (its
    /// background fill). Interpolated identically by `_band.html` and `Band.svelte`
    /// (the #44 parity contract). Empty when the band is unstyled.
    part_style: String,
    objects: Vec<ObjectView>,
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
    object_style: String,
    text_style: String,
    label: String,
    value: String,
    /// The RAW (unformatted) field value. `value` above carries the display
    /// string (value formatting #77/#78 applied); `raw` is what an editable
    /// Browse input must commit so a formatted field is never written back as its
    /// formatted text. Skipped from the design-model JSON (the canvas renders the
    /// display `value`); the askama browse band reads it directly. Equal to
    /// `value` when no format is active.
    #[serde(skip)]
    raw: String,
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
    /// Logical field kind (`FieldKind::as_str`) so the rail can draw type icons (#79).
    kind: String,
}

#[derive(Template)]
#[template(path = "view_list.html")]
struct ListTemplate {
    chrome: Chrome,
    table: String,
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
    /// RAW cell value — what the editable Table input commits.
    value: String,
    /// Display value (value formatting #77/#78 applied). Equals `value` when the
    /// column's field object carries no `format` bag.
    display: String,
    /// Inline CSS for the cell input (e.g. the value-dependent negative color);
    /// empty when unstyled.
    style: String,
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

/// The schema-builder surface (#113): a sibling to Layout Mode that manages
/// tables / fields (and, later, relationships) over the #107 `/schema/*` API.
/// App-global rather than per-layout, so it carries no current layout — the
/// Svelte island fetches the schema itself and owns the whole surface.
#[derive(Template)]
#[template(path = "schema.html")]
struct SchemaTemplate {
    chrome: Chrome,
}

/// Home → the first table's Form Browse view (the Form layout is the canonical
/// landing surface now that each view is its own layout, #57).
async fn index(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol
        .layouts()
        .unwrap()
        .into_iter()
        .find(|l| l.view == "form")
    {
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

fn parse_props(props: Option<&str>) -> Option<serde_json::Value> {
    let Some(props) = props else {
        return None;
    };
    serde_json::from_str::<serde_json::Value>(props).ok()
}

/// Derive a shape object's inline CSS from its `props` JSON. #49 owns the full
/// appearance contract; this reads the keys a rect/line/ellipse needs — `fill`,
/// `stroke`, `strokeWidth`, `radius`. The string is computed once here and carried
/// in [`ObjectView::shape_style`], so the askama band macro and the Svelte `Band`
/// both just interpolate it — there is no second derivation to keep byte-equal.
/// Empty for absent/invalid props (an unstyled shape falls back to its CSS class).
fn shape_style(kind: ObjectKind, props: Option<&str>) -> String {
    let Some(v) = parse_props(props) else {
        return String::new();
    };
    let mut s = String::new();
    // A line is a 1-D shape: `stroke` is its COLOUR and `strokeWidth` its THICKNESS
    // — rendered as a centred bar by the `.fm-line` rule, not the outer ring rects
    // use. (The ring would be clipped by `.fm-obj { overflow:hidden }` and could not
    // grow a line's weight, which is why the Border control appeared to do nothing.)
    if matches!(kind, ObjectKind::Line) {
        let stroke = v
            .get("stroke")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("#888");
        let width = v
            .get("strokeWidth")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(2)
            .max(1);
        s.push_str(&format!("background:{stroke};height:{width}px;"));
        if v.get("angle").is_some() || v.get("length").is_some() {
            let angle = v
                .get("angle")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let length = v
                .get("length")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(1.0)
                .max(1.0);
            s.push_str(&format!(
                "width:{length}px;left:50%;right:auto;transform:translate(-50%,-50%) rotate({angle}deg);transform-origin:center center;"
            ));
        }
        return s;
    }
    if let Some(fill) = v.get("fill").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("background:{fill};"));
    }
    if let Some(stroke) = v.get("stroke").and_then(serde_json::Value::as_str) {
        let width = v
            .get("strokeWidth")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(1);
        // Render the stroke OUTSIDE the box (box-shadow ring) so a thicker stroke
        // grows the object visually without eating into its stored geometry; the
        // ring also follows `border-radius`, so ellipses stay round (#44 issue 2).
        s.push_str(&format!("box-shadow:0 0 0 {width}px {stroke};"));
    }
    if let Some(radius) = v.get("radius").and_then(serde_json::Value::as_i64) {
        s.push_str(&format!("border-radius:{radius}px;"));
    }
    s
}

/// Box-level style for non-shape layout objects. Field objects use this for fill
/// and border; text objects accept the same props if present, but the first UI
/// pass exposes text formatting for text boxes rather than fill/line controls.
fn object_style(kind: ObjectKind, props: Option<&str>) -> String {
    if kind.is_shape() {
        return String::new();
    }
    let Some(v) = parse_props(props) else {
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
            .unwrap_or(1)
            .max(0);
        // Stroke grows outward (box-shadow ring) rather than inward, so geometry is
        // preserved and a thicker border makes the object visually bigger (issue 2).
        s.push_str(&format!("box-shadow:0 0 0 {width}px {stroke};"));
    }
    if let Some(radius) = v.get("radius").and_then(serde_json::Value::as_i64) {
        s.push_str(&format!("border-radius:{}px;", radius.max(0)));
    }
    s
}

/// Derive a part/band's inline CSS from its `props` JSON (#49/Issue 7), mirroring
/// [`object_style`]. The band's only appearance today is a background `fill`; the
/// derived string is computed once here and interpolated identically by the askama
/// band macro and the Svelte `Band` (the #44 parity contract). Empty for
/// absent/invalid props (an unstyled band falls back to its `.fm-part` class).
fn part_style(props: Option<&str>) -> String {
    let Some(v) = parse_props(props) else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(fill) = v.get("fill").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("background:{fill};"));
    }
    s
}

/// Text-level style for field and text objects. Alignment includes flex
/// justification because field display values are vertically-centered flex spans.
fn text_style(kind: ObjectKind, props: Option<&str>) -> String {
    if !matches!(kind, ObjectKind::Field | ObjectKind::Text) {
        return String::new();
    }
    let Some(v) = parse_props(props) else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(color) = v.get("textColor").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("color:{color};"));
    }
    if let Some(size) = v.get("fontSize").and_then(serde_json::Value::as_i64) {
        s.push_str(&format!("font-size:{}px;", size.clamp(6, 96)));
    }
    if v.get("bold")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        s.push_str("font-weight:700;");
    }
    if v.get("italic")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        s.push_str("font-style:italic;");
    }
    if v.get("underline")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        s.push_str("text-decoration:underline;");
    }
    match v.get("align").and_then(serde_json::Value::as_str) {
        Some("center") => s.push_str("text-align:center;justify-content:center;"),
        Some("right") => s.push_str("text-align:right;justify-content:flex-end;"),
        Some("left") => s.push_str("text-align:left;justify-content:flex-start;"),
        _ => {}
    }
    s
}

/// A record's field values keyed by lowercased field name → (field id, display
/// name, value) — the lookup `resolve_object` binds against.
fn by_name_map(
    fields: &[FieldMeta],
    cells: Vec<String>,
) -> HashMap<String, (i64, String, String, FieldKind)> {
    fields
        .iter()
        .zip(cells)
        .map(|(f, value)| (f.name.to_lowercase(), (f.id, f.name.clone(), value, f.kind)))
        .collect()
}

fn by_name_for_rec(
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
fn object_view(
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

fn object_view_for_rec(
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

/// Render one part's objects, positioned and bound against `by_name` (an empty
/// map leaves field values blank — used for header/footer with no record).
fn render_part(
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
    Some(FormRecord { id, parts })
}

/// Build the List-view render: header/footer parts once, the Body part(s)
/// repeated per record bound to its values. `current_rec` (1-based) marks the
/// flipbook's row. Returns `(header, rows, footer)`.
/// The header and footer bands of a layout, rendered once with no record bound.
/// Shared by List and Table Browse views so both frame their rows with the same
/// bands: header / sub-summary render above, footer / grand-summary below.
fn build_bands(sol: &Solution, layout_id: i64) -> (Vec<PartView>, Vec<PartView>) {
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

fn build_list(
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
fn layout_field_formats(
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
    let current_id = if rec >= 1 {
        ids.get((rec - 1) as usize).copied()
    } else {
        None
    };
    chrome.nav = Some(flipbook(layout_id, view, rec, current_id, total));
    chrome.editing = current_id.is_some_and(|cid| st.lock_held((table.id, cid)));

    match view {
        "form" => {
            let fields = sol.fields(table.id).unwrap();
            let record = build_form_record(&sol, layout_id, &table, &fields, &ids, rec);
            Html(
                FormTemplate {
                    chrome,
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
            let (header, rows, footer) = build_list(&sol, layout_id, &table, &fields, &ids, rec);
            Html(
                ListTemplate {
                    chrome,
                    table: table.name.clone(),
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
            let formats = layout_field_formats(&sol, layout_id, &fields);
            let (header, footer) = build_bands(&sol, layout_id);
            let tmpl = TableTemplate {
                chrome,
                layout_id,
                table: table.name.clone(),
                header,
                footer,
                fields: fields
                    .iter()
                    .map(|f| FieldView {
                        name: f.name.clone(),
                    })
                    .collect(),
                records: records
                    .into_iter()
                    .map(|r| RecordView {
                        id: r.id,
                        cells: fields
                            .iter()
                            .zip(r.cells)
                            .map(|(f, value)| {
                                // Format the DISPLAY value only; the input still
                                // commits the raw `value` (see _band controller).
                                let (display, style) = match formats.get(&f.id) {
                                    Some(spec) => {
                                        let fmt = format::format_value(&value, Some(spec), f.kind);
                                        let style = fmt
                                            .color
                                            .map(|c| format!("color:{c};"))
                                            .unwrap_or_default();
                                        (fmt.text, style)
                                    }
                                    None => (value.clone(), String::new()),
                                };
                                CellView {
                                    field_id: f.id,
                                    value,
                                    display,
                                    style,
                                }
                            })
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
    let tmpl = DesignTemplate {
        chrome,
        layout_id,
        layout: lay.name.clone(),
        view: view_label(&lay.view),
    };
    Html(tmpl.render().unwrap()).into_response()
}

/// The schema-builder page (#113). Renders the shell in `schema` mode with a
/// single mount node; the Svelte island drives everything over `/schema/*`.
async fn schema_page(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let chrome = Chrome::build(&sol, "schema", None);
    Html(SchemaTemplate { chrome }.render().unwrap()).into_response()
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
    /// The layout's Browse view (`form` | `list` | `table`) — the client gates the
    /// summary part-kinds on it (a form allows only header/body/footer, Issue 3).
    view: String,
    /// The primary table's fields — what the Create zone's Field tool offers
    /// (#48/#62). Geometry-independent, so the same list rides every record.
    fields: Vec<FieldChoice>,
    parts: Vec<PartView>,
    /// Durable object groups (#75). Objects remain rendered under their parts;
    /// these ids only drive Layout-mode selection/move behaviour.
    groups: Vec<ObjectGroupView>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ObjectGroupView {
    id: i64,
    object_ids: Vec<i64>,
}

fn object_group_view(g: ObjectGroup) -> ObjectGroupView {
    ObjectGroupView {
        id: g.id,
        object_ids: g.object_ids,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TableSchemaView {
    id: i64,
    name: String,
    notes: String,
    phys: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldSchemaView {
    id: i64,
    name: String,
    notes: String,
    phys: String,
    kind: String,
    position: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationshipSchemaView {
    id: i64,
    name: String,
    from_table: i64,
    to_table: i64,
    from_field: i64,
    to_field: i64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTableBody {
    name: String,
    notes: Option<String>,
    fields: Option<Vec<FieldBody>>,
}

#[derive(serde::Deserialize)]
struct RenameBody {
    name: String,
}

#[derive(serde::Deserialize)]
struct FieldBody {
    name: String,
    kind: String,
    notes: Option<String>,
}

#[derive(serde::Deserialize)]
struct UpdateTableBody {
    name: String,
    notes: Option<String>,
}

#[derive(serde::Deserialize)]
struct UpdateFieldBody {
    name: String,
    kind: String,
    notes: Option<String>,
}

#[derive(serde::Deserialize)]
struct FieldKindBody {
    kind: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldOrderBody {
    field_ids: Vec<i64>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelationshipBody {
    name: String,
    from_table: i64,
    to_table: i64,
    from_field: i64,
    to_field: i64,
}

fn table_schema_view(t: TableMeta) -> TableSchemaView {
    TableSchemaView {
        id: t.id,
        name: t.name,
        notes: t.notes,
        phys: t.phys,
    }
}

fn field_schema_view(f: FieldMeta) -> FieldSchemaView {
    FieldSchemaView {
        id: f.id,
        name: f.name,
        notes: f.notes,
        phys: f.phys,
        kind: f.kind.as_str().to_string(),
        position: f.position,
    }
}

fn relationship_schema_view(r: RelationshipMeta) -> RelationshipSchemaView {
    RelationshipSchemaView {
        id: r.id,
        name: r.name,
        from_table: r.from_table,
        to_table: r.to_table,
        from_field: r.from_field,
        to_field: r.to_field,
    }
}

fn parse_new_field(f: FieldBody) -> Result<NewField, &'static str> {
    let Some(kind) = FieldKind::parse(&f.kind) else {
        return Err("bad field kind");
    };
    Ok(NewField { name: f.name, kind })
}

fn relationship_body(body: RelationshipBody) -> NewRelationship {
    NewRelationship {
        name: body.name,
        from_table: body.from_table,
        to_table: body.to_table,
        from_field: body.from_field,
        to_field: body.to_field,
    }
}

async fn schema_tables(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let views: Vec<TableSchemaView> = sol
        .tables()
        .unwrap()
        .into_iter()
        .map(table_schema_view)
        .collect();
    Json(views)
}

async fn create_schema_table(
    State(st): State<AppState>,
    Json(body): Json<CreateTableBody>,
) -> impl IntoResponse {
    let notes = body.notes.unwrap_or_default();
    let fields = match body
        .fields
        .unwrap_or_default()
        .into_iter()
        .map(parse_new_field)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(fields) => fields,
        Err(msg) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };
    let mut sol = st.sol.lock().unwrap();
    let table_id = match sol.create_table(&body.name, &fields) {
        Ok(id) => id,
        Err(e) => return (StatusCode::CONFLICT, e.to_string()).into_response(),
    };
    let table = if notes.is_empty() {
        sol.table_by_id(table_id).unwrap().unwrap()
    } else {
        match sol.update_table(table_id, &body.name, &notes) {
            Ok(Some(table)) => table,
            Ok(None) => return StatusCode::NOT_FOUND.into_response(),
            Err(e) => return (StatusCode::CONFLICT, e.to_string()).into_response(),
        }
    };
    Json(table_schema_view(table)).into_response()
}

async fn update_schema_table(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<UpdateTableBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    match sol.update_table(table_id, &body.name, body.notes.as_deref().unwrap_or("")) {
        Ok(Some(table)) => Json(table_schema_view(table)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn rename_schema_table(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    match sol.rename_table(table_id, &body.name) {
        Ok(Some(table)) => Json(table_schema_view(table)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn delete_schema_table(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    match sol.delete_table(table_id) {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn schema_fields(State(st): State<AppState>, Path(table_id): Path<i64>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    if sol.table_by_id(table_id).unwrap().is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let views: Vec<FieldSchemaView> = sol
        .fields(table_id)
        .unwrap()
        .into_iter()
        .map(field_schema_view)
        .collect();
    Json(views).into_response()
}

async fn create_schema_field(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<FieldBody>,
) -> impl IntoResponse {
    let notes = body.notes.clone().unwrap_or_default();
    let field = match parse_new_field(body) {
        Ok(field) => field,
        Err(msg) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };
    let mut sol = st.sol.lock().unwrap();
    match sol.add_field(table_id, &field) {
        Ok(field) => {
            let field = if notes.is_empty() {
                field
            } else {
                match sol.update_field(table_id, field.id, &field.name, field.kind, &notes) {
                    Ok(Some(field)) => field,
                    Ok(None) => return StatusCode::NOT_FOUND.into_response(),
                    Err(e) => return (StatusCode::CONFLICT, e.to_string()).into_response(),
                }
            };
            Json(field_schema_view(field)).into_response()
        }
        Err(e) if e.to_string().contains("no table") => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn update_schema_field(
    State(st): State<AppState>,
    Path((table_id, field_id)): Path<(i64, i64)>,
    Json(body): Json<UpdateFieldBody>,
) -> impl IntoResponse {
    let Some(kind) = FieldKind::parse(&body.kind) else {
        return (StatusCode::BAD_REQUEST, "bad field kind").into_response();
    };
    let mut sol = st.sol.lock().unwrap();
    match sol.update_field(
        table_id,
        field_id,
        &body.name,
        kind,
        body.notes.as_deref().unwrap_or(""),
    ) {
        Ok(Some(field)) => Json(field_schema_view(field)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn rename_schema_field(
    State(st): State<AppState>,
    Path((table_id, field_id)): Path<(i64, i64)>,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    match sol.rename_field(table_id, field_id, &body.name) {
        Ok(Some(field)) => Json(field_schema_view(field)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn retype_schema_field(
    State(st): State<AppState>,
    Path((table_id, field_id)): Path<(i64, i64)>,
    Json(body): Json<FieldKindBody>,
) -> impl IntoResponse {
    let Some(kind) = FieldKind::parse(&body.kind) else {
        return (StatusCode::BAD_REQUEST, "bad field kind").into_response();
    };
    let mut sol = st.sol.lock().unwrap();
    match sol.retype_field(table_id, field_id, kind) {
        Ok(Some(field)) => Json(field_schema_view(field)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn reorder_schema_fields(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<FieldOrderBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    if sol.table_by_id(table_id).unwrap().is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    match sol.reorder_fields(table_id, &body.field_ids) {
        Ok(fields) => Json(
            fields
                .into_iter()
                .map(field_schema_view)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn delete_schema_field(
    State(st): State<AppState>,
    Path((table_id, field_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    match sol.delete_field(table_id, field_id) {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn schema_relationships(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let views: Vec<RelationshipSchemaView> = sol
        .relationships()
        .unwrap()
        .into_iter()
        .map(relationship_schema_view)
        .collect();
    Json(views)
}

async fn create_schema_relationship(
    State(st): State<AppState>,
    Json(body): Json<RelationshipBody>,
) -> impl IntoResponse {
    let rel = relationship_body(body);
    let mut sol = st.sol.lock().unwrap();
    match sol.create_relationship(&rel) {
        Ok(Some(rel)) => Json(relationship_schema_view(rel)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn update_schema_relationship(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<RelationshipBody>,
) -> impl IntoResponse {
    let rel = relationship_body(body);
    let mut sol = st.sol.lock().unwrap();
    match sol.update_relationship(id, &rel) {
        Ok(Some(rel)) => Json(relationship_schema_view(rel)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn delete_schema_relationship(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.delete_relationship(id) {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn design_model(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let ids = sol.record_ids(&table).unwrap();
    let total = ids.len() as i64;
    let rec = clamp_rec(&q, total);
    let fields = sol.fields(table.id).unwrap();
    // Bind to the record at `rec` when present; otherwise render geometry blank.
    let by_name = if rec >= 1 {
        match sol
            .get_record(&table, &fields, ids[(rec - 1) as usize])
            .unwrap()
        {
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
        .map(|f| FieldChoice {
            id: f.id,
            name: f.name.clone(),
            kind: f.kind.as_str().to_string(),
        })
        .collect();
    let model = DesignModel {
        layout_id,
        rec,
        total,
        width: layout_canvas_width(&sol, layout_id),
        view: lay.view.clone(),
        fields: field_choices,
        parts,
        groups: sol
            .object_groups(layout_id)
            .unwrap()
            .into_iter()
            .map(object_group_view)
            .collect(),
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

/// A cross-band move from the Layout canvas (#46): the object's new owning part
/// and its part-relative origin. `x`/`y` clamp to the canvas origin server-side.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObjectPartUpdate {
    part_id: i64,
    x: i64,
    y: i64,
}

/// Persist an object's new band membership from the Layout canvas (#46): a drag
/// that crosses a band boundary POSTs `{partId,x,y}` and this reparents the object
/// to that part, scoped to the layout and clamped to the canvas origin like the
/// geometry commit. 200 on success; 404 when the object or target part isn't in
/// the layout. Authoritative, so Browse reflects the new band on the next read.
async fn update_object_part(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<ObjectPartUpdate>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let updated = sol
        .set_object_part(
            layout_id,
            object_id,
            body.part_id,
            body.x.max(0),
            body.y.max(0),
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

/// One object's stacking order in a bulk commit (#83): the object id plus its new `z`.
#[derive(serde::Deserialize)]
struct ObjectZ {
    id: i64,
    z: i64,
}

/// Persist a group's stacking order from the Arrange panel (#83): the panel
/// re-densifies a part's `z` after a Bring-to-Front / Send-to-Back / step command
/// and POSTs `[{id,z}, …]`; this writes them in one transaction, each scoped to
/// the layout. Always 200 (unknown ids are simply skipped); the body is the count
/// actually updated, mirroring [`update_objects_geometry`].
async fn update_objects_z(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(items): Json<Vec<ObjectZ>>,
) -> impl IntoResponse {
    let pairs: Vec<(i64, i64)> = items.iter().map(|z| (z.id, z.z)).collect();
    let mut sol = st.sol.lock().unwrap();
    let updated = sol.set_objects_z(layout_id, &pairs).unwrap();
    (StatusCode::OK, updated.to_string()).into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateObjectGroupBody {
    id: Option<i64>,
    object_ids: Vec<i64>,
}

/// Create a durable group over selected layout objects (#75). This is a metadata
/// relationship only: no child geometry/style/z values change. Re-grouping
/// objects already in groups replaces their old memberships.
async fn create_object_group(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<CreateObjectGroupBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    match sol
        .create_object_group(layout_id, &body.object_ids, body.id)
        .unwrap()
    {
        Some(group) => axum::Json(object_group_view(group)).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            "group needs at least two objects in the layout",
        )
            .into_response(),
    }
}

/// Ungroup without touching member geometry/styles (#75).
async fn delete_object_group(
    State(st): State<AppState>,
    Path((layout_id, group_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.delete_object_group(layout_id, group_id).unwrap() {
        0 => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::OK.into_response(),
    }
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
    create_label: Option<bool>,
    content: Option<String>,
    props: Option<serde_json::Value>,
    /// The source object's binding, carried verbatim by a value-only field copy
    /// (duplicate/paste, #48/#85). Lets the copy round-trip even when the binding
    /// doesn't resolve to a live `field_id` — an empty table or an unresolved
    /// relationship path renders the object with `field_id: null`, and re-deriving
    /// the binding from `field_id` would 400. Ignored when `create_label` is true
    /// (Field-tool placement resolves the binding from `field_id` instead).
    binding: Option<String>,
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
        match sol
            .get_record(&table, &fields, ids[(rec - 1) as usize])
            .unwrap()
        {
            Some(cells) => by_name_map(&fields, cells),
            None => HashMap::new(),
        }
    } else {
        HashMap::new()
    };

    let created_ids: Vec<i64> = if kind == ObjectKind::Field {
        if body.create_label.unwrap_or(true) {
            // Field-tool placement: resolve the chosen field to build its binding
            // and spawn the caption label atomically (#60).
            let Some(fid) = body.field_id else {
                return (StatusCode::BAD_REQUEST, "field tool needs a fieldId").into_response();
            };
            let Some(f) = fields.iter().find(|f| f.id == fid) else {
                return (StatusCode::BAD_REQUEST, "no such field").into_response();
            };
            let binding = format!("{}.{}", table.name, f.name);
            let label = f.name.clone();
            match sol
                .create_field_object(
                    layout_id,
                    body.part_id,
                    &binding,
                    &label,
                    body.x,
                    body.y,
                    body.w,
                    body.h,
                )
                .unwrap()
            {
                Some((label_id, field_id)) => vec![label_id, field_id],
                None => return StatusCode::NOT_FOUND.into_response(),
            }
        } else {
            // Value-only field copy (duplicate/paste, #48/#85). Prefer the source
            // object's `binding` verbatim so the copy round-trips even when the
            // binding doesn't resolve to a live field_id (empty table, or an
            // unresolved relationship path) — those render with `field_id: null`,
            // and re-deriving the binding from `field_id` would 400. Fall back to
            // the field_id→binding derivation only when no binding is supplied.
            let binding = match body.binding.clone() {
                Some(b) => b,
                None => {
                    let Some(fid) = body.field_id else {
                        return (StatusCode::BAD_REQUEST, "field tool needs a fieldId")
                            .into_response();
                    };
                    let Some(f) = fields.iter().find(|f| f.id == fid) else {
                        return (StatusCode::BAD_REQUEST, "no such field").into_response();
                    };
                    format!("{}.{}", table.name, f.name)
                }
            };
            let new = NewObject {
                part_id: body.part_id,
                kind,
                x: body.x,
                y: body.y,
                w: body.w,
                h: body.h,
                binding: Some(binding),
                content: None,
                // Honor props on a value-only field create (paste, #85) so a pasted
                // field keeps its fill/border/format bag; the label-spawning branch
                // above is normal placement and has no props to carry yet.
                props: body.props.as_ref().map(|v| v.to_string()),
            };
            match sol.create_object(layout_id, &new).unwrap() {
                Some(id) => vec![id],
                None => return StatusCode::NOT_FOUND.into_response(),
            }
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

/// One object restored at its ORIGINAL id (#84). The client sends the store's
/// full `ObjectDoc` for each object it recreated on undo-of-delete / redo-of-
/// create, so the server re-inserts it byte-identically at the same id.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreObjectBody {
    id: i64,
    part_id: i64,
    kind: String,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    z: i64,
    read_only: bool,
    binding: Option<String>,
    content: Option<String>,
    props: Option<String>,
}

#[derive(serde::Deserialize)]
struct RestoreObjectsBody {
    objects: Vec<RestoreObjectBody>,
    rec: Option<i64>,
}

/// Restore deleted objects at their ORIGINAL ids (#84 undo/redo replay) and return
/// each one's `ObjectView` resolved against `rec` — byte-identical to a model
/// fetch, so the store's already-recreated objects match the server without a
/// re-hydrate. 400 on a bad kind; 404 if a part isn't in the layout; 409 if an id
/// is already occupied (reused by an intervening create). The batch is atomic.
async fn restore_design_objects(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<RestoreObjectsBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    if layout_table(&sol, layout_id).is_none() {
        return not_found("layout", layout_id);
    }
    let mut restores = Vec::with_capacity(body.objects.len());
    for o in &body.objects {
        let Some(kind) = ObjectKind::parse(&o.kind) else {
            return (StatusCode::BAD_REQUEST, "bad object kind").into_response();
        };
        restores.push(RestoreObject {
            id: o.id,
            part_id: o.part_id,
            kind,
            x: o.x,
            y: o.y,
            w: o.w,
            h: o.h,
            z: o.z,
            read_only: o.read_only,
            binding: o.binding.clone(),
            content: o.content.clone(),
            props: o.props.clone(),
        });
    }
    match sol.restore_objects(layout_id, &restores).unwrap() {
        RestoreResult::Restored => {}
        RestoreResult::PartNotFound => return StatusCode::NOT_FOUND.into_response(),
        RestoreResult::IdInUse => return (StatusCode::CONFLICT, "id in use").into_response(),
    }
    let rec = body.rec;
    let mut views = Vec::with_capacity(restores.len());
    for o in &restores {
        match object_view_for_rec(&sol, layout_id, o.id, rec) {
            Some(v) => views.push(v),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    }
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

/// The result of appending a band (#48): the new `part` plus the layout's full
/// `[{id, position}]` ordering *after* the insert. `create_part` places summary
/// bands between the body and footer and shifts the trailing parts down, so the
/// client can't guess the slot — it must resync every part's `position` from
/// `positions` (mirrors the move endpoint) or the new band renders below the
/// footer.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatePartResult {
    part: PartView,
    positions: Vec<PartPosition>,
}

/// Append a band to a layout (#48) and return the new `PartView` plus the layout's
/// post-insert `[{id, position}]` ordering so the store places the band in its
/// server-assigned slot (summaries land above the footer). 404 unknown layout;
/// 400 bad kind.
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
    let id = match sol.create_part(layout_id, kind, height) {
        Ok(id) => id,
        Err(e) => return (StatusCode::CONFLICT, e.to_string()).into_response(),
    };
    let positions: Vec<PartPosition> = sol
        .parts(layout_id)
        .unwrap()
        .into_iter()
        .map(|p| PartPosition {
            id: p.id,
            position: p.position,
        })
        .collect();
    axum::Json(CreatePartResult {
        part: PartView {
            id,
            kind: kind.as_str(),
            height,
            props: String::new(),
            part_style: String::new(),
            objects: Vec::new(),
        },
        positions,
    })
    .into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartHeightBody {
    height: i64,
}

/// Resize a band by setting its stored height. 200 echoes the updated `PartView`;
/// 404 when no such part belongs to the layout.
async fn update_part_height(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PartHeightBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let height = body.height.max(1);
    if sol.set_part_height(layout_id, part_id, height).unwrap() == 0 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(part) = sol.part_by_id(layout_id, part_id).unwrap() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    axum::Json(PartView {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects: Vec::new(),
    })
    .into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartKindBody {
    kind: String,
}

/// Change a band's kind. 400 for an unknown kind; 404 for a foreign/unknown part.
async fn update_part_kind(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PartKindBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some(kind) = PartKind::parse(&body.kind) else {
        return (StatusCode::BAD_REQUEST, "bad part kind").into_response();
    };
    match sol.set_part_kind(layout_id, part_id, kind) {
        Ok(0) => return StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {}
        Err(e) => return (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
    let Some(part) = sol.part_by_id(layout_id, part_id).unwrap() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    axum::Json(PartView {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects: Vec::new(),
    })
    .into_response()
}

/// Persist a band's `props` from the Band inspector (#49/Issue 7), layout-scoped,
/// and echo back the updated `PartView` (with the re-derived `part_style`) so the
/// canvas updates without a client-side re-derivation. 200 on success, 404 when no
/// such part belongs to the layout. Mirrors [`update_object_props`].
async fn update_part_props(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PropsBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let props = body.props.to_string();
    if sol.set_part_props(layout_id, part_id, &props).unwrap() == 0 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(part) = sol.part_by_id(layout_id, part_id).unwrap() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    axum::Json(PartView {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects: Vec::new(),
    })
    .into_response()
}

/// The direction a summary band moves within its layout (Issue 4).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartMoveBody {
    up: bool,
}

/// A part's id + resolved position after a reorder — the lightweight shape the
/// move endpoint returns so the client can resync `PartDoc.position` (Issue 4).
#[derive(serde::Serialize)]
struct PartPosition {
    id: i64,
    position: i64,
}

/// Move a summary band up/down within its layout, staying between the header and
/// footer (Issue 4). 200 returns the layout's parts as `[{id, position}]` (after
/// the move) so the client resyncs positions; 404 when the move was a no-op (no
/// such movable part / clamped at a boundary).
async fn move_design_part(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PartMoveBody>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    match sol.move_part(layout_id, part_id, body.up) {
        Ok(0) => return StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {}
        Err(e) => return (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
    let positions: Vec<PartPosition> = sol
        .parts(layout_id)
        .unwrap()
        .into_iter()
        .map(|p| PartPosition {
            id: p.id,
            position: p.position,
        })
        .collect();
    axum::Json(positions).into_response()
}

/// Delete a band from a layout. Child objects are removed with it.
async fn delete_design_part(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.delete_part(layout_id, part_id) {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
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

/// The canvas-facing result of a props commit (#49): freshly server-derived
/// styles, so the canvas updates without a client-side re-derivation.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StyleResult {
    object_style: String,
    text_style: String,
    shape_style: String,
}

/// Persist an object's `props` from the Style zone (#49), layout-scoped, and echo
/// back re-derived styles for the canvas. 200 on success, 404 when no
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
    let Some(o) = sol.object_by_id(layout_id, object_id).unwrap() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    axum::Json(StyleResult {
        object_style: object_style(o.kind, o.props.as_deref()),
        text_style: text_style(o.kind, o.props.as_deref()),
        shape_style: if o.kind.is_shape() {
            shape_style(o.kind, o.props.as_deref())
        } else {
            String::new()
        },
    })
    .into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BindingBody {
    field_id: i64,
    rec: Option<i64>,
}

/// Rebind a selected field object to another field on the layout's primary table.
/// The client supplies a field id rather than a raw binding so the server remains
/// the single source for the stored dot-path.
async fn update_object_binding(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<BindingBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let fields = sol.fields(table.id).unwrap();
    let Some(field) = fields.iter().find(|f| f.id == body.field_id) else {
        return (StatusCode::BAD_REQUEST, "no such field").into_response();
    };
    let binding = format!("{}.{}", table.name, field.name);
    if sol
        .set_object_binding(layout_id, object_id, &binding)
        .unwrap()
        == 0
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match object_view_for_rec(&sol, layout_id, object_id, body.rec) {
        Some(view) => axum::Json(view).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize)]
struct BindingPathBody {
    binding: String,
    rec: Option<i64>,
}

/// Set an object's binding dot-path VERBATIM (history replay of a binding diff,
/// #84). Unlike [`update_object_binding`] (keyed by `fieldId` for live field-
/// picking) this writes the already-resolved path the undo diff carries, so a
/// binding undo/redo round-trips without re-deriving from a field id.
async fn update_object_binding_path(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<BindingPathBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    if layout_table(&sol, layout_id).is_none() {
        return not_found("layout", layout_id);
    }
    if sol
        .set_object_binding(layout_id, object_id, &body.binding)
        .unwrap()
        == 0
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match object_view_for_rec(&sol, layout_id, object_id, body.rec) {
        Some(view) => axum::Json(view).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ContentBody {
    content: String,
}

/// Update the static content for a selected text object.
async fn update_object_content(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<ContentBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    if sol
        .set_object_content(layout_id, object_id, &body.content)
        .unwrap()
        == 0
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match object_view_for_rec(&sol, layout_id, object_id, None) {
        Some(view) => axum::Json(view).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadOnlyBody {
    read_only: bool,
    rec: Option<i64>,
}

/// Toggle whether a selected object renders as editable in Browse mode.
async fn update_object_read_only(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<ReadOnlyBody>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    if sol
        .set_object_read_only(layout_id, object_id, body.read_only)
        .unwrap()
        == 0
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match object_view_for_rec(&sol, layout_id, object_id, body.rec) {
        Some(view) => axum::Json(view).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

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

/// Serve the built Layout Mode editor bundle from the configured base directory
/// (`AppState::ui_base_dir`, default `ui/dist`) under the stable `/ui/...` prefix
/// (ADR #42: the island is a static bundle served by axum). Vite emits
/// predictable, non-hashed names (`layout-editor.js` / `layout-editor.css`), so
/// the design page references them by a fixed path. Requests 404 until the
/// frontend is built. A small `tokio::fs` handler keeps this dependency-free
/// rather than pulling in a static-file crate. The base dir is configurable so
/// the desktop shell (#16) can serve from its bundled resource directory.
async fn ui_asset(State(st): State<AppState>, Path(path): Path<String>) -> impl IntoResponse {
    // Reject path traversal and empty segments before touching the filesystem.
    if path
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let full = std::path::Path::new(&st.ui_base_dir).join(&path);
    match tokio::fs::read(&full).await {
        Ok(bytes) => (
            [(axum::http::header::CONTENT_TYPE, ui_content_type(&path))],
            bytes,
        )
            .into_response(),
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
pub fn seed(sol: &mut Solution) -> anyhow::Result<()> {
    let customer_fields = demo_customer_fields();
    if sol.tables()?.is_empty() {
        sol.create_table("Customers", &customer_fields)?;
    } else if let Some(table) = sol.table_by_name("Customers")? {
        let existing: HashSet<String> = sol.fields(table.id)?.into_iter().map(|f| f.name).collect();
        for f in customer_fields {
            if !existing.contains(&f.name) {
                sol.add_field(table.id, &f)?;
            }
        }
    }
    Ok(())
}

fn demo_customer_fields() -> Vec<NewField> {
    vec![
        NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Email".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Age".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "DOB".into(),
            kind: FieldKind::Date,
        },
        NewField {
            name: "Phone".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Street".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "City".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "State".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "ZIP".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Balance".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "CreditLimit".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "LoyaltyPoints".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "DiscountPct".into(),
            kind: FieldKind::Number,
        },
    ]
}

/// Build the router. A fn so the Tauri shell (#16) embeds the same app.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/browse/:layout", get(browse).post(create_record))
        .route("/browse/:layout/:id", post(save_record))
        .route("/browse/:layout/:id/open", post(open_record))
        .route("/browse/:layout/:id/revert", post(revert_record))
        .route("/browse/:layout/:id/delete", post(delete_record))
        .route(
            "/schema/tables",
            get(schema_tables).post(create_schema_table),
        )
        .route("/schema/tables/:table_id", post(update_schema_table))
        .route("/schema/tables/:table_id/rename", post(rename_schema_table))
        .route("/schema/tables/:table_id/delete", post(delete_schema_table))
        .route(
            "/schema/tables/:table_id/fields",
            get(schema_fields).post(create_schema_field),
        )
        .route(
            "/schema/tables/:table_id/fields/order",
            post(reorder_schema_fields),
        )
        .route(
            "/schema/tables/:table_id/fields/:field_id/rename",
            post(rename_schema_field),
        )
        .route(
            "/schema/tables/:table_id/fields/:field_id",
            post(update_schema_field),
        )
        .route(
            "/schema/tables/:table_id/fields/:field_id/retype",
            post(retype_schema_field),
        )
        .route(
            "/schema/tables/:table_id/fields/:field_id/delete",
            post(delete_schema_field),
        )
        .route(
            "/schema/relationships",
            get(schema_relationships).post(create_schema_relationship),
        )
        .route(
            "/schema/relationships/:id",
            post(update_schema_relationship),
        )
        .route(
            "/schema/relationships/:id/delete",
            post(delete_schema_relationship),
        )
        .route("/schema", get(schema_page))
        .route("/design/:layout", get(design))
        .route("/design/:layout/model", get(design_model))
        .route("/design/:layout/object", post(create_design_object))
        .route(
            "/design/:layout/object/restore",
            post(restore_design_objects),
        )
        .route("/design/:layout/part", post(create_design_part))
        .route("/design/:layout/part/:id/height", post(update_part_height))
        .route("/design/:layout/part/:id/kind", post(update_part_kind))
        .route("/design/:layout/part/:id/props", post(update_part_props))
        .route("/design/:layout/part/:id/move", post(move_design_part))
        .route("/design/:layout/part/:id/delete", post(delete_design_part))
        .route(
            "/design/:layout/object/:id/geometry",
            post(update_object_geometry),
        )
        .route(
            "/design/:layout/object/:id/props",
            post(update_object_props),
        )
        .route(
            "/design/:layout/object/:id/binding",
            post(update_object_binding),
        )
        .route(
            "/design/:layout/object/:id/binding-path",
            post(update_object_binding_path),
        )
        .route(
            "/design/:layout/object/:id/content",
            post(update_object_content),
        )
        .route(
            "/design/:layout/object/:id/read-only",
            post(update_object_read_only),
        )
        .route(
            "/design/:layout/object/:id/delete",
            post(delete_design_object),
        )
        .route("/design/:layout/object/:id/part", post(update_object_part))
        .route("/design/:layout/geometry", post(update_objects_geometry))
        .route("/design/:layout/z", post(update_objects_z))
        .route("/design/:layout/group", post(create_object_group))
        .route(
            "/design/:layout/group/:id/delete",
            post(delete_object_group),
        )
        .route("/ui/*path", get(ui_asset))
        .with_state(state)
}

/// Bind the app to a loopback port and hand back the resolved address plus the
/// running server future. Pass `port = None` (or `Some(0)`) to let the OS assign
/// an ephemeral port — the desktop shell (#16) reads the returned
/// [`SocketAddr`]'s port and points the WebView at it. The caller drives the
/// returned future (spawn it on a task or `.await` it) to actually serve.
///
/// ```no_run
/// # async fn example() {
/// use record_maker_server::{serve, AppState};
/// use record_maker_engine::Solution;
/// let sol = Solution::open("./.rm-data").unwrap();
/// let (addr, server) = serve(AppState::new(sol), None).await.unwrap();
/// println!("listening on http://{addr}");
/// server.await.unwrap();
/// # }
/// ```
pub async fn serve(
    state: AppState,
    port: Option<u16>,
) -> std::io::Result<(
    SocketAddr,
    impl std::future::Future<Output = std::io::Result<()>>,
)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    let server = axum::serve(listener, app(state)).into_future();
    Ok((bound, server))
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
            object_style: String::new(),
            text_style: String::new(),
            label: format!("Field {field_id}"),
            value: value.to_string(),
            raw: value.to_string(),
            shape_style: String::new(),
        }
    }

    fn body_part(sol: &Solution, layout_id: i64) -> PartMeta {
        sol.parts(layout_id)
            .unwrap()
            .into_iter()
            .find(|p| p.kind == PartKind::Body)
            .expect("body part")
    }

    #[test]
    fn unresolved_field_binding_renders_binding_fallback() {
        let object = ObjectMeta {
            id: 1,
            part_id: 1,
            kind: ObjectKind::Field,
            x: 0,
            y: 0,
            w: 100,
            h: 24,
            z: 0,
            read_only: true,
            binding: Some("Customers.Missing".into()),
            content: None,
            props: None,
        };
        let view = object_view(&object, &HashMap::new());
        assert_eq!(view.label, "Customers.Missing");
        assert_eq!(view.value, "Customers.Missing");
    }

    /// The #43 acceptance: a read-only object renders a non-editable value, while
    /// an editable object in the same (editable) Form view renders an input.
    #[test]
    fn read_only_object_renders_value_editable_object_renders_input() {
        let part = PartView {
            id: 1,
            kind: "body",
            height: 60,
            props: String::new(),
            part_style: String::new(),
            objects: vec![
                field_obj(1, "EDITABLE_VAL", false),
                field_obj(2, "READONLY_VAL", true),
            ],
        };
        let tmpl = FormTemplate {
            chrome: form_chrome(),
            table: "T".into(),
            record: Some(FormRecord {
                id: 1,
                parts: vec![part],
            }),
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
            table: "T".into(),
            record: Some(FormRecord {
                id: 1,
                parts: vec![PartView {
                    id: 1,
                    kind: "body",
                    height: 60,
                    props: String::new(),
                    part_style: String::new(),
                    objects: vec![o],
                }],
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
                    NewField {
                        name: "Name".into(),
                        kind: FieldKind::Text,
                    },
                    NewField {
                        name: "Email".into(),
                        kind: FieldKind::Text,
                    },
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

        let state = AppState::new(sol);
        let req = Request::builder()
            .uri(format!("/browse/{layout_id}?view=form"))
            .body(Body::empty())
            .unwrap();
        let resp = app(state).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();

        // Read-only Name: value shown, but no editable input bound to it.
        assert!(html.contains("Ada"), "read-only value still rendered");
        assert!(
            !html.contains(&format!(r#"name="f{name_fid}""#)),
            "read-only field must not render an input"
        );
        assert!(
            html.contains("fm-readonly"),
            "read-only object marked in markup"
        );
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
        collapsed
            .replace("> ", ">")
            .replace(" <", "<")
            .trim()
            .to_string()
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
                    NewField {
                        name: "Name".into(),
                        kind: FieldKind::Text,
                    },
                    NewField {
                        name: "Email".into(),
                        kind: FieldKind::Text,
                    },
                ],
            )
            .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(tid).unwrap();
        sol.insert_record(
            &table,
            &[
                (&fields[0], "Ada".into()),
                (&fields[1], "ada@example.com".into()),
            ],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        sol.app
            .execute(
                "UPDATE meta_object SET read_only=1 WHERE binding='Customers.Name'",
                [],
            )
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
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn state_for(sol: Solution) -> AppState {
        AppState::new(sol)
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
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn schema_table_and_field_routes_manage_metadata_and_physical_table() {
        let state = state_for(Solution::open_in_memory().unwrap());
        let body = serde_json::json!({
            "name": "Invoices",
            "notes": "Billing data",
            "fields": [
                {"name": "Number", "kind": "text"},
                {"name": "Total", "kind": "number"}
            ]
        });
        let (status, resp) =
            post_json_body(state.clone(), "/schema/tables", &body.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        let table: serde_json::Value = serde_json::from_str(&resp).unwrap();
        let table_id = table["id"].as_i64().unwrap();
        let table_phys = table["phys"].as_str().unwrap().to_string();
        assert_eq!(table["notes"].as_str(), Some("Billing data"));

        let (status, fields_body) =
            get_body(state.clone(), &format!("/schema/tables/{table_id}/fields")).await;
        assert_eq!(status, StatusCode::OK, "{fields_body}");
        let fields: serde_json::Value = serde_json::from_str(&fields_body).unwrap();
        let number_id = fields[0]["id"].as_i64().unwrap();
        let total_id = fields[1]["id"].as_i64().unwrap();

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/tables/{table_id}/rename"),
            r#"{"name":"Bills"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        assert!(resp.contains(r#""name":"Bills""#));
        assert!(resp.contains(r#""notes":"Billing data""#));

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/tables/{table_id}"),
            r#"{"name":"Bills","notes":"Paid and open invoices"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        assert!(resp.contains(r#""notes":"Paid and open invoices""#));

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/tables/{table_id}/fields/{number_id}/rename"),
            r#"{"name":"Invoice Number"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/tables/{table_id}/fields/{number_id}"),
            r#"{"name":"Invoice Number","kind":"text","notes":"Shown on customer forms"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        assert!(resp.contains(r#""notes":"Shown on customer forms""#));

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/tables/{table_id}/fields/{total_id}/retype"),
            r#"{"kind":"text"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        assert!(resp.contains(r#""kind":"text""#));

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/tables/{table_id}/fields/order"),
            &serde_json::json!({"fieldIds": [total_id, number_id]}).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        let ordered: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(ordered[0]["id"].as_i64(), Some(total_id));

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/tables/{table_id}/fields/{number_id}/delete"),
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");

        let sol = state.sol.lock().unwrap();
        let fields = sol.fields(table_id).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].id, total_id);
        let columns: Vec<String> = {
            let mut stmt = sol
                .data
                .prepare(&format!("PRAGMA table_info({table_phys})"))
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(columns, vec!["id".to_string(), fields[0].phys.clone()]);
    }

    #[tokio::test]
    async fn schema_relationship_routes_crud_and_validate_field_ownership() {
        let mut sol = Solution::open_in_memory().unwrap();
        let customers = sol
            .create_table(
                "Customers",
                &[NewField {
                    name: "Id".into(),
                    kind: FieldKind::Number,
                }],
            )
            .unwrap();
        let invoices = sol
            .create_table(
                "Invoices",
                &[NewField {
                    name: "Customer Id".into(),
                    kind: FieldKind::Number,
                }],
            )
            .unwrap();
        let customer_id = sol.fields(customers).unwrap()[0].id;
        let invoice_customer_id = sol.fields(invoices).unwrap()[0].id;
        let state = state_for(sol);

        let bad = serde_json::json!({
            "name": "bad",
            "fromTable": invoices,
            "toTable": customers,
            "fromField": customer_id,
            "toField": invoice_customer_id
        });
        let (status, _) =
            post_json_body(state.clone(), "/schema/relationships", &bad.to_string()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let create = serde_json::json!({
            "name": "customer",
            "fromTable": invoices,
            "toTable": customers,
            "fromField": invoice_customer_id,
            "toField": customer_id
        });
        let (status, resp) =
            post_json_body(state.clone(), "/schema/relationships", &create.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        let rel: serde_json::Value = serde_json::from_str(&resp).unwrap();
        let rel_id = rel["id"].as_i64().unwrap();

        let update = serde_json::json!({
            "name": "bill_to",
            "fromTable": invoices,
            "toTable": customers,
            "fromField": invoice_customer_id,
            "toField": customer_id
        });
        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/schema/relationships/{rel_id}"),
            &update.to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        assert!(resp.contains(r#""name":"bill_to""#));

        let (status, resp) = get_body(state.clone(), "/schema/relationships").await;
        assert_eq!(status, StatusCode::OK, "{resp}");
        assert!(resp.contains(r#""fromTable":"#));

        let (status, resp) = post_json_body(
            state,
            &format!("/schema/relationships/{rel_id}/delete"),
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resp}");
    }

    /// #57: a table carries independent per-view layouts. The Browse view toggle
    /// links to sibling layout ids (not one layout re-rendered via `?view=`), and
    /// each layout renders in its own view.
    #[tokio::test]
    async fn browse_view_tabs_link_to_sibling_layouts_and_render_by_view() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(table.id).unwrap();
        sol.insert_record(&table, &[(&fields[0], "Ada".into())])
            .unwrap();
        let layouts = sol.layouts_for_table(table.id).unwrap();
        let form = layouts.iter().find(|l| l.view == "form").unwrap().id;
        let list = layouts.iter().find(|l| l.view == "list").unwrap().id;
        let table_l = layouts.iter().find(|l| l.view == "table").unwrap().id;
        assert!(
            form != list && list != table_l && form != table_l,
            "distinct per-view ids"
        );
        let state = state_for(sol);

        // The Form layout renders the canvas and offers tabs to the SIBLING ids.
        let (status, html) = get_body(state.clone(), &format!("/browse/{form}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            html.contains(r#"<div class="fm-canvas""#),
            "form renders the canvas"
        );
        assert!(
            html.contains(&format!(r#"href="/browse/{list}""#)),
            "List tab → list layout"
        );
        assert!(
            html.contains(&format!(r#"href="/browse/{table_l}""#)),
            "Table tab → table layout"
        );

        // The List layout renders the list surface by its own view, not the canvas.
        let (status, html) = get_body(state, &format!("/browse/{list}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            html.contains(r#"class="fm-list""#),
            "list renders the list surface"
        );
        assert!(
            !html.contains(r#"<div class="fm-canvas""#),
            "list view is not the form canvas"
        );
    }

    /// Table Browse frames its field-grid with the layout's header/footer bands,
    /// the same as Form/List — so all three views share the fixed-band shape.
    #[tokio::test]
    async fn table_view_renders_header_and_footer_bands() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        sol.insert_record(&table, &[]).unwrap();
        let table_l = sol
            .layouts_for_table(table.id)
            .unwrap()
            .into_iter()
            .find(|l| l.view == "table")
            .unwrap()
            .id;

        let (status, html) = get_body(state_for(sol), &format!("/browse/{table_l}")).await;
        assert_eq!(status, StatusCode::OK);
        // Still the field-derived grid…
        assert!(html.contains(r#"class="fm-tableview""#) && html.contains("<thead>"));
        // …now wrapped by header/footer band regions.
        assert!(
            html.contains(r#"<div class="fm-bands-head">"#),
            "table view renders the header band region"
        );
        assert!(
            html.contains(r#"<div class="fm-bands-foot">"#),
            "table view renders the footer band region"
        );
        // The layout's header + footer parts both render as bands (the grid body
        // is field-derived, so these are the only .fm-part divs in Table view).
        assert!(
            html.matches(r#"class="fm-part""#).count() >= 2,
            "both header and footer bands render their parts"
        );
    }

    /// #57 Layout-mode chrome: the view toggle stays (switching which view you
    /// DESIGN, via /design/ siblings) and the pagination control is repurposed to
    /// step layouts; record actions are Browse-only.
    #[tokio::test]
    async fn design_mode_keeps_view_toggle_and_layout_stepper() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let layouts = sol.layouts_for_table(table.id).unwrap();
        let form = layouts.iter().find(|l| l.view == "form").unwrap().id;
        let list = layouts.iter().find(|l| l.view == "list").unwrap().id;
        let (status, html) = get_body(state_for(sol), &format!("/design/{form}")).await;
        assert_eq!(status, StatusCode::OK);
        // View toggle present, switching which view you DESIGN (links into /design/).
        assert!(
            html.contains(&format!(r#"href="/design/{list}""#)),
            "view toggle → design the List layout"
        );
        // Pagination control repurposed to layout navigation.
        assert!(
            html.contains("Layout navigation"),
            "stepper navigates layouts in design mode"
        );
        // Record actions don't belong in Layout mode.
        assert!(
            html.contains(r#"title="Records are managed in Browse mode""#),
            "no record actions in layout mode"
        );
    }

    /// #46 group commit: a bulk POST persists every object's geometry in one
    /// request (scoped + clamped), returns the updated count, and skips unknown ids.
    #[tokio::test]
    async fn design_bulk_geometry_persists_group() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part = body_part(&sol, layout_id);
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
        let count = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8(count.to_vec()).unwrap(),
            "2",
            "only real ids count"
        );

        let sol = state.sol.lock().unwrap();
        let after = sol.objects(part.id).unwrap();
        assert_eq!((after[0].x, after[0].y), (10, 20));
        assert_eq!(
            (after[1].x, after[1].y),
            (0, 40),
            "negative x clamped to origin"
        );
    }

    /// #83 z-order: a bulk POST to `/z` persists every object's stacking order in
    /// one request (scoped), returns the updated count, and skips unknown ids.
    #[tokio::test]
    async fn design_bulk_z_persists_group() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part = body_part(&sol, layout_id);
        let objs = sol.objects(part.id).unwrap();
        let (a, b) = (objs[0].id, objs[1].id);
        let state = state_for(sol);

        let resp = {
            use axum::http::Request;
            use tower::ServiceExt;
            let body = format!(r#"[{{"id":{a},"z":3}},{{"id":{b},"z":7}},{{"id":999999,"z":1}}]"#);
            app(state.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(format!("/design/{layout_id}/z"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
        };
        assert_eq!(resp.status(), StatusCode::OK);
        let count = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8(count.to_vec()).unwrap(),
            "2",
            "only real ids count"
        );

        let sol = state.sol.lock().unwrap();
        let after = sol.objects(part.id).unwrap();
        // `objects()` sorts by (z, id), so read back by id rather than position.
        assert_eq!(after.iter().find(|o| o.id == a).unwrap().z, 3);
        assert_eq!(after.iter().find(|o| o.id == b).unwrap().z, 7);
    }

    /// #75 durable groups: the group relationship persists in the design model,
    /// and Ungroup removes only the relationship, not child geometry/styles.
    #[tokio::test]
    async fn design_object_group_persists_and_ungroups() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part = body_part(&sol, layout_id);
        let objs = sol.objects(part.id).unwrap();
        let (a, b) = (objs[0].id, objs[1].id);
        let state = state_for(sol);

        let (status, body) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/group"),
            &format!(r#"{{"objectIds":[{a},{b}]}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains(&format!(r#""objectIds":[{a},{b}]"#)),
            "{body}"
        );

        let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
        assert!(
            model.contains(&format!(r#""groups":[{{"id":1,"objectIds":[{a},{b}]}}]"#)),
            "model includes durable group\n{model}"
        );

        let status = post_json(
            state.clone(),
            &format!("/design/{layout_id}/group/1/delete"),
            "{}",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
        assert!(model.contains(r#""groups":[]"#), "group removed\n{model}");
        assert!(
            model.contains(&format!(r#""id":{a}"#)) && model.contains(&format!(r#""id":{b}"#)),
            "ungroup leaves child objects in place\n{model}"
        );

        let (status, body) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/group"),
            &format!(r#"{{"id":42,"objectIds":[{a},{b}]}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.contains(&format!(r#""id":42,"objectIds":[{a},{b}]"#)),
            "explicit-id group restore echoes the restored id\n{body}"
        );
        let (_, model) = get_body(state, &format!("/design/{layout_id}/model")).await;
        assert!(
            model.contains(&format!(r#""groups":[{{"id":42,"objectIds":[{a},{b}]}}]"#)),
            "model preserves restored group id\n{model}"
        );
    }

    /// #62 two-mount rail: the design page renders the `#layout-tools` mount node
    /// in the sidebar (where the Svelte Create/Style/Zoom zones mount, sharing the
    /// canvas store); Browse mode does not.
    #[tokio::test]
    async fn design_page_renders_tool_rail_mount_node() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let form = sol
            .layouts()
            .unwrap()
            .into_iter()
            .find(|l| l.view == "form")
            .unwrap()
            .id;
        let state = state_for(sol);

        let (status, html) = get_body(state.clone(), &format!("/design/{form}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            html.contains(r#"id="layout-tools""#),
            "design page mounts the tool rail"
        );

        let (_, browse) = get_body(state, &format!("/browse/{form}")).await;
        assert!(
            !browse.contains(r#"id="layout-tools""#),
            "browse has no tool rail"
        );
    }

    /// #113: the schema-builder surface renders in `schema` mode with the single
    /// island mount node and the global Schema nav marked active. It's app-global,
    /// so it renders even with no tables/layouts.
    #[tokio::test]
    async fn schema_page_renders_builder_mount_node() {
        let sol = Solution::open_in_memory().unwrap();
        let state = state_for(sol);

        let (status, html) = get_body(state.clone(), "/schema").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            html.contains(r#"id="schema-root""#),
            "schema page mounts the builder island"
        );
        assert!(
            html.contains(r#"src="/ui/schema-builder.js""#),
            "schema page loads the schema-builder bundle"
        );

        // The builder node never appears on other surfaces.
        let (_, browse) = get_body(state, "/").await;
        assert!(
            !browse.contains(r#"id="schema-root""#),
            "the schema island is scoped to /schema"
        );
    }

    /// #48 create: placing a shape POSTs `{partId,kind,x,y,w,h,props}`, persists a
    /// `meta_object`, and echoes back its `ObjectView` (with the server-derived
    /// shape_style) so the store can add it without a re-hydrate.
    #[tokio::test]
    async fn design_create_shape_object_persists_and_returns_view() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let before = sol.objects(part_id).unwrap().len();
        let state = state_for(sol);

        let body = format!(
            r##"{{"partId":{part_id},"kind":"rect","x":20,"y":12,"w":64,"h":48,"props":{{"fill":"#eef","stroke":"#88a","strokeWidth":1}}}}"##
        );
        let (status, resp) =
            post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(r#""kind":"rect""#) && resp.contains(r#""shape":true"#));
        assert!(
            resp.contains(r#""shapeStyle":"background:#eef;box-shadow:0 0 0 1px #88a;""#),
            "derived style echoed\n{resp}"
        );
        assert!(
            resp.contains("strokeWidth"),
            "raw props echoed for the inspector\n{resp}"
        );

        let sol = state.sol.lock().unwrap();
        let objs = sol.objects(part_id).unwrap();
        assert_eq!(objs.len(), before + 1, "one row inserted");
        assert!(objs
            .iter()
            .any(|o| o.kind == ObjectKind::Rect && (o.x, o.y) == (20, 12)));
    }

    /// #48/#60 create: the Field tool POSTs `{kind:"field",fieldId,…}` and gets
    /// back TWO views — the value field (live value resolved for the record) and
    /// its spawned caption label.
    #[tokio::test]
    async fn design_create_field_object_spawns_label_and_returns_both() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(table.id).unwrap();
        let name_fid = fields[0].id;
        sol.insert_record(&table, &[(&fields[0], "Ada".into())])
            .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let before = sol.objects(part_id).unwrap().len();
        let state = state_for(sol);

        let body = format!(
            r#"{{"partId":{part_id},"kind":"field","x":120,"y":40,"w":200,"h":24,"fieldId":{name_fid},"rec":1}}"#
        );
        let (status, resp) =
            post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
        assert_eq!(status, StatusCode::OK);
        // The value field resolves "Ada" and binds Customers.Name; the label
        // carries the caption "Name".
        assert!(resp.contains(r#""kind":"field""#) && resp.contains(r#""value":"Ada""#));
        assert!(resp.contains(r#""binding":"Customers.Name""#));
        assert!(
            resp.contains(r#""kind":"text""#) && resp.contains(r#""content":"Name""#),
            "label spawned\n{resp}"
        );

        let sol = state.sol.lock().unwrap();
        assert_eq!(
            sol.objects(part_id).unwrap().len(),
            before + 2,
            "value + label inserted"
        );
        drop(sol);

        let body = format!(
            r#"{{"partId":{part_id},"kind":"field","x":120,"y":80,"w":200,"h":24,"fieldId":{name_fid},"createLabel":false,"rec":1}}"#
        );
        let (status, resp) =
            post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(r#""kind":"field""#), "field created\n{resp}");
        assert!(
            !resp.contains(r#""kind":"text""#),
            "label suppressed\n{resp}"
        );

        let sol = state.sol.lock().unwrap();
        assert_eq!(
            sol.objects(part_id).unwrap().len(),
            before + 3,
            "second placement inserted value only"
        );
    }

    /// #85 paste: a value-only field create (createLabel:false) honors `props` so a
    /// pasted field keeps its appearance. Regression for the value-only branch
    /// silently dropping props — the derived shape style must round-trip + persist.
    #[tokio::test]
    async fn design_field_paste_create_honors_props() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let name_fid = sol.fields(table.id).unwrap()[0].id;
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let state = state_for(sol);

        let body = format!(
            r##"{{"partId":{part_id},"kind":"field","x":10,"y":10,"w":120,"h":24,"fieldId":{name_fid},"createLabel":false,"rec":1,"props":{{"fill":"#ffeecc","stroke":"#335577","strokeWidth":3}}}}"##
        );
        let (status, resp) =
            post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            resp.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#),
            "pasted field keeps props-derived style\n{resp}"
        );
        let (_, model) = get_body(state, &format!("/design/{layout_id}/model")).await;
        assert!(
            model.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#),
            "pasted field props persist in the model\n{model}"
        );
    }

    /// #48 duplicate: a value-only field copy (createLabel:false) carries the
    /// source object's `binding` verbatim, so Ctrl/Cmd+D round-trips even when the
    /// binding doesn't resolve to a live field_id — an empty table (no records)
    /// renders every field object with `field_id: null`, exactly the state that
    /// used to 400 "field tool needs a fieldId".
    #[tokio::test]
    async fn design_duplicate_field_by_binding_without_field_id() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        // No records inserted: with an empty table the read model resolves no
        // value, so a field object's field_id is null — the crashing scenario.
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let before = sol.objects(part_id).unwrap().len();
        let state = state_for(sol);

        // Exactly what the canvas POSTs on Ctrl/Cmd+D of a field whose field_id is
        // null: no fieldId, but the binding fully determines the copy.
        let body = format!(
            r#"{{"partId":{part_id},"kind":"field","x":40,"y":40,"w":120,"h":24,"fieldId":null,"createLabel":false,"binding":"Customers.Name"}}"#
        );
        let (status, resp) =
            post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
        assert_eq!(status, StatusCode::OK, "duplicate by binding\n{resp}");
        assert!(resp.contains(r#""kind":"field""#), "field created\n{resp}");
        assert!(
            !resp.contains(r#""kind":"text""#),
            "no caption spawned for a value-only copy\n{resp}"
        );

        let sol = state.sol.lock().unwrap();
        let objs = sol.objects(part_id).unwrap();
        assert_eq!(objs.len(), before + 1, "one value-only row inserted");
        let created = objs.iter().find(|o| (o.x, o.y) == (40, 40)).unwrap();
        assert_eq!(created.kind, ObjectKind::Field);
        assert_eq!(
            created.binding.as_deref(),
            Some("Customers.Name"),
            "source binding preserved verbatim"
        );
    }

    #[tokio::test]
    async fn design_selected_object_inspector_updates_field_text_and_read_only() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let table = sol.table_by_name("Customers").unwrap().unwrap();
        let fields = sol.fields(table.id).unwrap();
        sol.insert_record(
            &table,
            &[
                (&fields[0], "Ada".into()),
                (&fields[1], "ada@example.test".into()),
            ],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let objects = sol.objects(part_id).unwrap();
        let label_id = objects
            .iter()
            .find(|o| o.kind == ObjectKind::Text)
            .unwrap()
            .id;
        let field_id = objects
            .iter()
            .find(|o| o.kind == ObjectKind::Field)
            .unwrap()
            .id;
        let email_fid = fields[1].id;
        let state = state_for(sol);

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object/{field_id}/binding"),
            &format!(r#"{{"fieldId":{email_fid},"rec":1}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            resp.contains(r#""binding":"Customers.Email""#),
            "binding response\n{resp}"
        );
        assert!(
            resp.contains(r#""fieldId":"#) && resp.contains(r#""value":"ada@example.test""#),
            "field projection\n{resp}"
        );

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object/{label_id}/content"),
            r#"{"content":"Primary email"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            resp.contains(r#""content":"Primary email""#),
            "content response\n{resp}"
        );

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object/{field_id}/read-only"),
            r#"{"readOnly":true,"rec":1}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            resp.contains(r#""readOnly":true"#),
            "read-only response\n{resp}"
        );

        let sol = state.sol.lock().unwrap();
        let updated = sol.objects(part_id).unwrap();
        let label = updated.iter().find(|o| o.id == label_id).unwrap();
        let field = updated.iter().find(|o| o.id == field_id).unwrap();
        assert_eq!(label.content.as_deref(), Some("Primary email"));
        assert_eq!(field.binding.as_deref(), Some("Customers.Email"));
        assert!(field.read_only);
    }

    #[tokio::test]
    async fn design_object_props_style_field_and_text_objects() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let objects = sol.objects(part_id).unwrap();
        let label_id = objects
            .iter()
            .find(|o| o.kind == ObjectKind::Text)
            .unwrap()
            .id;
        let field_id = objects
            .iter()
            .find(|o| o.kind == ObjectKind::Field)
            .unwrap()
            .id;
        let state = state_for(sol);

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object/{field_id}/props"),
            r##"{"props":{"fill":"#ffeecc","stroke":"#335577","strokeWidth":3,"textColor":"#112233","fontSize":18,"bold":true,"italic":true,"underline":true,"align":"right"}}"##,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            resp.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#),
            "field box style\n{resp}"
        );
        assert!(
            resp.contains("color:#112233;font-size:18px;font-weight:700;font-style:italic;text-decoration:underline;text-align:right;justify-content:flex-end;"),
            "field text style\n{resp}"
        );

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object/{label_id}/props"),
            r##"{"props":{"textColor":"#445566","fontSize":16,"align":"center"}}"##,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            resp.contains(r#""textStyle":"color:#445566;font-size:16px;text-align:center;justify-content:center;""#),
            "text formatting style\n{resp}"
        );

        let (_, model) = get_body(state, &format!("/design/{layout_id}/model")).await;
        assert!(
            model.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#)
                && model.contains(r#""textStyle":"color:#445566;font-size:16px;text-align:center;justify-content:center;""#),
            "styles persist in design model\n{model}"
        );
    }

    /// #48 create-part: POSTing a kind appends a band and echoes its `PartView`.
    #[tokio::test]
    async fn design_create_part_appends_band_and_returns_view() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        // Summaries are a List/Table feature (Issue 3): design on the List view.
        let layout_id = sol
            .layouts()
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap()
            .id;
        let before = sol.parts(layout_id).unwrap().len();
        let state = state_for(sol);

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/part"),
            r#"{"kind":"subsummary","height":40}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(r#""kind":"subsummary""#) && resp.contains(r#""height":40"#));
        // The response carries the post-insert ordering so the client resyncs
        // positions instead of guessing bottom-most.
        assert!(
            resp.contains(r#""positions""#),
            "create echoes positions\n{resp}"
        );
        let parts = state.sol.lock().unwrap().parts(layout_id).unwrap();
        assert_eq!(parts.len(), before + 1);
        // The new summary must sit ABOVE the footer — never below it.
        let sub = parts
            .iter()
            .find(|p| p.kind == PartKind::SubSummary)
            .unwrap();
        let footer = parts.iter().find(|p| p.kind == PartKind::Footer).unwrap();
        assert!(
            sub.position < footer.position,
            "sub-summary must land above the footer (sub {} vs footer {})",
            sub.position,
            footer.position
        );
    }

    /// Part editing: height/kind/delete round-trip through layout-scoped design
    /// endpoints, and deleting a band removes its child objects.
    #[tokio::test]
    async fn design_part_editing_round_trip() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        // Summaries are a List/Table feature (Issue 3): design on the List view.
        let layout_id = sol
            .layouts()
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap()
            .id;
        let part_id = sol
            .create_part(layout_id, PartKind::SubSummary, 80)
            .unwrap();
        let state = state_for(sol);

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/part/{part_id}/height"),
            r#"{"height":164}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(r#""height":164"#));
        assert_eq!(
            state
                .sol
                .lock()
                .unwrap()
                .part_by_id(layout_id, part_id)
                .unwrap()
                .unwrap()
                .height,
            164
        );

        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/part/{part_id}/kind"),
            r#"{"kind":"grandsummary"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(r#""kind":"grandsummary""#));
        assert_eq!(
            state
                .sol
                .lock()
                .unwrap()
                .part_by_id(layout_id, part_id)
                .unwrap()
                .unwrap()
                .kind,
            PartKind::GrandSummary
        );
        let body_id = state
            .sol
            .lock()
            .unwrap()
            .parts(layout_id)
            .unwrap()
            .into_iter()
            .find(|p| p.kind == PartKind::Body)
            .unwrap()
            .id;
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/part/{body_id}/kind"),
                r#"{"kind":"header"}"#
            )
            .await,
            StatusCode::CONFLICT
        );

        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{}/part/{part_id}/height", layout_id + 999),
                r#"{"height":1}"#
            )
            .await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/part/{part_id}/kind"),
                r#"{"kind":"bad"}"#
            )
            .await,
            StatusCode::BAD_REQUEST
        );

        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/part/{body_id}/delete"),
                ""
            )
            .await,
            StatusCode::CONFLICT
        );
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/part/{part_id}/delete"),
                ""
            )
            .await,
            StatusCode::OK
        );
        let sol = state.sol.lock().unwrap();
        assert!(sol.part_by_id(layout_id, part_id).unwrap().is_none());
        assert!(
            sol.objects(part_id).unwrap().is_empty(),
            "objects deleted with the band"
        );
    }

    /// Issue 7: setting a band's fill persists its `props`, echoes the re-derived
    /// `part_style`, and surfaces on the next model/Browse read; a foreign layout
    /// id is a scoped no-op (404).
    #[tokio::test]
    async fn design_part_props_sets_band_fill_and_is_scoped() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let state = state_for(sol);

        // A fill commit echoes the raw props AND the server-derived part_style.
        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/part/{part_id}/props"),
            r##"{"props":{"fill":"#334455"}}"##,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            resp.contains(r#""partStyle":"background:#334455;""#),
            "derived band style echoed\n{resp}"
        );
        assert_eq!(
            state
                .sol
                .lock()
                .unwrap()
                .part_by_id(layout_id, part_id)
                .unwrap()
                .unwrap()
                .props
                .as_deref(),
            Some(r##"{"fill":"#334455"}"##)
        );

        // The design model carries the derived style so the canvas renders it live.
        let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
        assert!(
            model.contains(r#""partStyle":"background:#334455;""#),
            "band fill persists in design model\n{model}"
        );

        // A foreign layout id is a scoped no-op ⇒ 404.
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{}/part/{part_id}/props", layout_id + 999),
                r##"{"props":{"fill":"#000000"}}"##,
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }

    /// Issue 4: the move endpoint reorders a summary band and returns the layout's
    /// `[{id, position}]` after the move; a clamped move (past the footer) is 404.
    #[tokio::test]
    async fn design_move_part_reorders_and_returns_positions() {
        let mut sol = Solution::open_in_memory().unwrap();
        let tid = sol
            .create_table(
                "Customers",
                &[NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                }],
            )
            .unwrap();
        // Summaries live on List/Table (Issue 3).
        let layout_id = sol
            .layouts_for_table(tid)
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap()
            .id;
        // header, body, sub, grand, footer.
        let sub = sol
            .create_part(layout_id, PartKind::SubSummary, 40)
            .unwrap();
        let grand = sol
            .create_part(layout_id, PartKind::GrandSummary, 40)
            .unwrap();
        let state = state_for(sol);

        // Move the grand summary up: it swaps with the sub summary; response lists
        // every part's post-move position.
        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/part/{grand}/move"),
            r#"{"up":true}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp.contains(&format!(r#"{{"id":{grand},"position":2}}"#)));
        assert!(resp.contains(&format!(r#"{{"id":{sub},"position":3}}"#)));

        // The sub summary can't move below the footer — clamped ⇒ 404.
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/part/{sub}/move"),
                r#"{"up":false}"#
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }

    /// #48 delete + #49 props: a placed object can have its props set (shape style
    /// re-derives on the next read) and can be deleted; both are layout-scoped.
    #[tokio::test]
    async fn design_object_props_then_delete_round_trip() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
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
            sol.objects(part_id)
                .unwrap()
                .iter()
                .find(|o| o.kind == ObjectKind::Rect)
                .unwrap()
                .id
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
        assert!(
            model.contains("background:#102030;border-radius:6px;"),
            "props drive shape_style\n{model}"
        );

        // Delete it (scoped): a foreign layout is a no-op 404, the real one 200.
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{}/object/{rect_id}/delete", layout_id + 999),
                ""
            )
            .await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/object/{rect_id}/delete"),
                ""
            )
            .await,
            StatusCode::OK
        );
        assert!(!state
            .sol
            .lock()
            .unwrap()
            .objects(part_id)
            .unwrap()
            .iter()
            .any(|o| o.id == rect_id));
    }

    /// #84 restore: helper that creates a rect and returns (state, layout_id,
    /// part_id, rect_id) ready for a delete→restore round-trip.
    async fn seeded_rect() -> (AppState, i64, i64, i64) {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part_id = body_part(&sol, layout_id).id;
        let state = state_for(sol);
        let (status, _) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object"),
            &format!(r#"{{"partId":{part_id},"kind":"rect","x":7,"y":9,"w":40,"h":40}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rect_id = {
            let sol = state.sol.lock().unwrap();
            sol.objects(part_id)
                .unwrap()
                .iter()
                .find(|o| o.kind == ObjectKind::Rect)
                .unwrap()
                .id
        };
        (state, layout_id, part_id, rect_id)
    }

    fn object_ids(state: &AppState, part_id: i64) -> Vec<i64> {
        state
            .sol
            .lock()
            .unwrap()
            .objects(part_id)
            .unwrap()
            .iter()
            .map(|o| o.id)
            .collect()
    }

    /// #84 undo-of-delete: restore re-inserts a deleted object at its EXACT
    /// original id (identity preserved so bindings/labels survive), with its
    /// geometry and props intact and visible on the next model read.
    #[tokio::test]
    async fn design_object_restore_preserves_identity() {
        let (state, layout_id, part_id, rect_id) = seeded_rect().await;
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/object/{rect_id}/delete"),
                ""
            )
            .await,
            StatusCode::OK
        );
        assert!(!object_ids(&state, part_id).contains(&rect_id));

        let body = format!(
            r##"{{"objects":[{{"id":{rect_id},"partId":{part_id},"kind":"rect","x":7,"y":9,"w":40,"h":40,"z":0,"readOnly":false,"binding":null,"content":null,"props":"{{\"fill\":\"#102030\",\"radius\":6}}"}}],"rec":null}}"##
        );
        let (status, resp) = post_json_body(
            state.clone(),
            &format!("/design/{layout_id}/object/restore"),
            &body,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "restore 200\n{resp}");

        // Same id, back in the part.
        assert!(object_ids(&state, part_id).contains(&rect_id));
        // Geometry + props survived: the model re-derives the shape_style.
        let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
        assert!(
            model.contains("background:#102030;border-radius:6px;"),
            "restored props drive shape_style\n{model}"
        );
    }

    /// #84 restore rejects an id already in use (reused by an intervening create):
    /// 409 and the live row is untouched — never clobbered.
    #[tokio::test]
    async fn design_object_restore_rejects_id_in_use() {
        let (state, layout_id, part_id, rect_id) = seeded_rect().await;
        let before = object_ids(&state, part_id);
        let body = format!(
            r##"{{"objects":[{{"id":{rect_id},"partId":{part_id},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}}],"rec":null}}"##
        );
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/object/restore"),
                &body
            )
            .await,
            StatusCode::CONFLICT
        );
        assert_eq!(object_ids(&state, part_id), before, "live row untouched");
    }

    /// #84 restore rejects a part not in the layout: 404, nothing written.
    #[tokio::test]
    async fn design_object_restore_rejects_unknown_part() {
        let (state, layout_id, part_id, rect_id) = seeded_rect().await;
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/{rect_id}/delete"),
            "",
        )
        .await;
        let bogus_part = part_id + 9999;
        let body = format!(
            r##"{{"objects":[{{"id":{rect_id},"partId":{bogus_part},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}}],"rec":null}}"##
        );
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/object/restore"),
                &body
            )
            .await,
            StatusCode::NOT_FOUND
        );
        assert!(
            !object_ids(&state, part_id).contains(&rect_id),
            "nothing written"
        );
    }

    /// #84 restore is atomic: a valid object followed by one referencing a bad part
    /// rolls the whole batch back — the field+label pair never half-restores.
    #[tokio::test]
    async fn design_object_restore_is_atomic() {
        let (state, layout_id, part_id, rect_id) = seeded_rect().await;
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/{rect_id}/delete"),
            "",
        )
        .await;
        let free_id = rect_id + 1000; // unused rowid for the second (doomed) object
        let bogus_part = part_id + 9999;
        let body = format!(
            r##"{{"objects":[{{"id":{rect_id},"partId":{part_id},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}},{{"id":{free_id},"partId":{bogus_part},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}}],"rec":null}}"##
        );
        assert_eq!(
            post_json(
                state.clone(),
                &format!("/design/{layout_id}/object/restore"),
                &body
            )
            .await,
            StatusCode::NOT_FOUND
        );
        let ids = object_ids(&state, part_id);
        assert!(
            !ids.contains(&rect_id),
            "first object rolled back with the batch"
        );
        assert!(!ids.contains(&free_id));
    }

    /// #15 round-trip: POSTing new geometry persists to `meta_object` (scoped to
    /// the layout) and is visible on the next read; bad ids 404 and change nothing;
    /// negative coordinates clamp to the canvas origin.
    #[tokio::test]
    async fn design_object_geometry_persists_clamps_and_is_scoped() {
        let mut sol = Solution::open_in_memory().unwrap();
        sol.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let layout_id = sol.layouts().unwrap()[0].id;
        let part = body_part(&sol, layout_id);
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
        let (status, body) =
            get_body(state_for(sol), &format!("/design/{layout_id}/model?rec=1")).await;
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
            r#""shapeStyle":"background:#eef;box-shadow:0 0 0 1px #88a;border-radius:4px;""#,
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
        let (status, html) =
            get_body(state_for(sol), &format!("/browse/{layout_id}?view=form")).await;
        assert_eq!(status, StatusCode::OK);
        // The form holds exactly one `.fm-canvas`; slice it out up to `</form>`.
        let start = html
            .find(r#"<div class="fm-canvas""#)
            .expect("canvas present");
        let end = start + html[start..].find("</form>").expect("form closes");
        let canvas = normalize_html(&html[start..end]);
        assert!(canvas.starts_with(r#"<div class="fm-canvas""#) && canvas.ends_with("</div>"));
        assert_or_regen("canvas.parity.html", &canvas);
    }

    /// Value formatting (#77/#78) must reach ALL Browse views — including Table,
    /// which renders a field-derived grid that used to bypass the formatter. The
    /// editable input DISPLAYS the formatted value but carries the RAW value in
    /// data-raw so committing never writes the formatted string back (#80 guard).
    #[tokio::test]
    async fn browse_applies_value_format_in_form_list_and_table() {
        let mut sol = Solution::open_in_memory().unwrap();
        let tid = sol
            .create_table(
                "Invoices",
                &[
                    NewField {
                        name: "Total".into(),
                        kind: FieldKind::Number,
                    },
                    NewField {
                        name: "Due".into(),
                        kind: FieldKind::Date,
                    },
                ],
            )
            .unwrap();
        let table = sol.table_by_name("Invoices").unwrap().unwrap();
        let fields = sol.fields(tid).unwrap();
        sol.insert_record(
            &table,
            &[
                (&fields[0], "1234.5".into()),
                (&fields[1], "12/25/2003".into()),
            ],
        )
        .unwrap();
        // Set formats on every layout's field objects.
        sol.app
            .execute(
                "UPDATE meta_object SET props=?1 WHERE binding='Invoices.Total'",
                [r#"{"format":{"mode":"decimal","fixedDecimals":true,"decimalDigits":2,"thousandsSeparator":","}}"#],
            )
            .unwrap();
        sol.app
            .execute(
                "UPDATE meta_object SET props=?1 WHERE binding='Invoices.Due'",
                [r#"{"format":{"mode":"predefined","predefined":"yyyy-mm-dd"}}"#],
            )
            .unwrap();
        let layouts = sol.layouts().unwrap();
        let by_view = |v: &str| {
            layouts
                .iter()
                .find(|l| canonical_view(&l.view) == v)
                .map(|l| l.id)
        };
        let (form, list, table_l) = (
            by_view("form").unwrap(),
            by_view("list").unwrap(),
            by_view("table").unwrap(),
        );
        let state = state_for(sol);

        for (label, lid) in [("form", form), ("list", list), ("table", table_l)] {
            let (status, html) = get_body(state.clone(), &format!("/browse/{lid}")).await;
            assert_eq!(status, StatusCode::OK, "{label} renders");
            assert!(
                html.contains("1,234.50"),
                "{label} shows the formatted value"
            );
            assert!(
                html.contains("2003-12-25"),
                "{label} shows the formatted date"
            );
            // The raw value must ride data-raw (so the editable input commits raw),
            // not be the visible/committed default.
            assert!(
                html.contains(r#"data-raw="1234.5""#),
                "{label} keeps the raw value in data-raw for safe commit"
            );
            assert!(
                html.contains(r#"data-raw="12/25/2003""#),
                "{label} keeps the raw date in data-raw for safe commit"
            );
        }
    }
}
