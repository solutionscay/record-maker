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
    Router,
};
use record_maker_engine::{
    FieldKind, FieldMeta, LayoutMeta, NewField, ObjectKind, ObjectMeta, PartKind, PartMeta,
    Solution, TableMeta,
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

/// The three Browse views, in toggle order. The frozen `?view=` contract (#20).
const VIEWS: [&str; 3] = ["form", "list", "table"];

/// Normalise a `?view=` value to a known view, falling back to the layout's
/// stored default view (`form` for default forms) when `?view` is absent.
fn view_param(q: &HashMap<String, String>, default: &str) -> &'static str {
    match q.get("view").map(String::as_str).unwrap_or(default) {
        "form" => "form",
        "list" => "list",
        _ => "table",
    }
}

impl Chrome {
    fn build(
        sol: &Solution,
        mode: &'static str,
        current_layout: Option<i64>,
        view: Option<&str>,
    ) -> Self {
        let layouts = sol
            .layouts()
            .map(|ls| {
                ls.into_iter()
                    .map(|l| LayoutLink {
                        selected: Some(l.id) == current_layout,
                        id: l.id,
                        name: l.name,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The view toggle exists only in Browse, where a layout is open.
        let view_tabs = match (current_layout, view) {
            (Some(lid), Some(active)) => VIEWS
                .iter()
                .map(|&v| ViewTab {
                    label: match v {
                        "form" => "Form",
                        "list" => "List",
                        _ => "Table",
                    },
                    href: format!("/browse/{lid}?view={v}"),
                    active: v == active,
                })
                .collect(),
            _ => Vec::new(),
        };
        Chrome { mode, layouts, current_layout, view_tabs, nav: None, editing: false }
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
struct PartView {
    height: i64,
    objects: Vec<ObjectView>,
}

/// A positioned object. `field` objects show `label` + live `value`; other
/// kinds render `value` as plain text. `field_id` is set for bound field
/// objects so editable views can name the input `f<id>`; `None` otherwise.
/// `z` is the stacking order (CSS `z-index`); `read_only` suppresses the
/// editable input even in an editable view (per-object editability, #40/#43).
struct ObjectView {
    field: bool,
    field_id: Option<i64>,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    z: i64,
    read_only: bool,
    label: String,
    value: String,
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
}

/// Home → the first layout's Browse view.
async fn index(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.layouts().unwrap().into_iter().next() {
        Some(l) => Redirect::to(&format!("/browse/{}", l.id)).into_response(),
        None => Html("<p>No layouts yet.</p>".to_string()).into_response(),
    }
}

/// Resolve a field object's binding to its (label, value) for the current
/// record. Interim two-segment resolver: the last dot-path segment is the field
/// name, matched case-insensitively against `by_name` (lowercased field name →
/// `(display name, value)`). The full relationship resolver replaces this (#11).
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
        // non-field objects (text/…) render their binding text, if any
        _ => (false, None, String::new(), o.binding.clone().unwrap_or_default()),
    }
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
        .map(|o| {
            let (field, field_id, label, value) = resolve_object(o, by_name);
            ObjectView {
                field,
                field_id,
                x: o.x,
                y: o.y,
                w: o.w,
                h: o.h,
                z: o.z,
                read_only: o.read_only,
                label,
                value,
            }
        })
        .collect();
    PartView { height: part.height, objects }
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
    let view = view_param(&q, &lay.view);
    let mut chrome = Chrome::build(&sol, "browse", Some(layout_id), Some(view));

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

/// Layout (design) mode — placeholder until the canvas lands (#15/#24).
async fn design(State(st): State<AppState>, Path(layout_id): Path<i64>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, _table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let chrome = Chrome::build(&sol, "design", Some(layout_id), None);
    let tmpl = DesignTemplate { chrome, layout_id, layout: lay.name.clone() };
    Html(tmpl.render().unwrap()).into_response()
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
            field: true,
            field_id: Some(field_id),
            x: 0,
            y: 0,
            w: 100,
            h: 24,
            z: 0,
            read_only,
            label: format!("Field {field_id}"),
            value: value.to_string(),
        }
    }

    /// The #43 acceptance: a read-only object renders a non-editable value, while
    /// an editable object in the same (editable) Form view renders an input.
    #[test]
    fn read_only_object_renders_value_editable_object_renders_input() {
        let part = PartView {
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
                parts: vec![PartView { height: 60, objects: vec![o] }],
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
}
