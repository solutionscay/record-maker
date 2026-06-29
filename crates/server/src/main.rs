//! Browse/Layout mode runtime + app shell. Routing is **layout-keyed**
//! (ADR-0005): `/browse/:layout` and `/design/:layout`, where `:layout` is the
//! meta_layout **id** (i64). One generic handler set serves every table by
//! reading metadata — no per-table code.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use record_maker_engine::{FieldKind, LayoutMeta, NewField, Solution, TableMeta};

#[derive(Clone)]
struct AppState {
    sol: Arc<Mutex<Solution>>,
}

/// Persistent shell context shared by every page (the chrome).
struct Chrome {
    mode: &'static str, // "browse" | "design"
    tables: Vec<String>,
    layouts: Vec<LayoutLink>,
    current_layout: Option<i64>,
}

struct LayoutLink {
    id: i64,
    name: String,
    selected: bool,
}

impl Chrome {
    fn build(sol: &Solution, mode: &'static str, current_layout: Option<i64>) -> Self {
        let tables = sol
            .tables()
            .map(|ts| ts.into_iter().map(|t| t.name).collect())
            .unwrap_or_default();
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
        Chrome { mode, tables, layouts, current_layout }
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

// ---- Browse (Table view for now; Form/List arrive in #22/#25/#26) ----

#[derive(Template)]
#[template(path = "browse.html")]
struct BrowseTemplate {
    chrome: Chrome,
    layout_id: i64,
    table: String,
    fields: Vec<FieldView>,
    records: Vec<RecordView>,
}

struct FieldView {
    id: i64,
    name: String,
}

struct RecordView {
    id: i64,
    cells: Vec<String>,
}

#[derive(Template)]
#[template(path = "edit.html")]
struct EditTemplate {
    chrome: Chrome,
    layout_id: i64,
    table: String,
    id: i64,
    fields: Vec<EditFieldView>,
}

struct EditFieldView {
    id: i64,
    name: String,
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

/// Browse a layout (Table view): headers from fields, rows from data.db.
async fn browse(State(st): State<AppState>, Path(layout_id): Path<i64>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let fields = sol.fields(table.id).unwrap();
    let records = sol.list_records(&table, &fields).unwrap();
    let chrome = Chrome::build(&sol, "browse", Some(layout_id));

    let tmpl = BrowseTemplate {
        chrome,
        layout_id,
        table: table.name.clone(),
        fields: fields
            .iter()
            .map(|f| FieldView { id: f.id, name: f.name.clone() })
            .collect(),
        records: records
            .into_iter()
            .map(|r| RecordView { id: r.id, cells: r.cells })
            .collect(),
    };
    Html(tmpl.render().unwrap()).into_response()
}

/// Layout (design) mode — placeholder until the canvas lands (#15/#24).
async fn design(State(st): State<AppState>, Path(layout_id): Path<i64>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, _table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let chrome = Chrome::build(&sol, "design", Some(layout_id));
    let tmpl = DesignTemplate { chrome, layout_id, layout: lay.name.clone() };
    Html(tmpl.render().unwrap()).into_response()
}

/// Create a record from the new-record form (inputs named `f<field_id>`).
async fn create_record(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    {
        let sol = st.sol.lock().unwrap();
        if let Some((_lay, table)) = layout_table(&sol, layout_id) {
            let fields = sol.fields(table.id).unwrap();
            let values = collect_values(&fields, &form);
            sol.insert_record(&table, &values).unwrap();
        }
    }
    Redirect::to(&format!("/browse/{layout_id}"))
}

/// Show the edit form for a record, pre-filled with its current values.
async fn edit_form(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let fields = sol.fields(table.id).unwrap();
    let Some(values) = sol.get_record(&table, &fields, id).unwrap() else {
        return not_found("record", id);
    };
    let chrome = Chrome::build(&sol, "browse", Some(layout_id));
    let fv = fields
        .iter()
        .zip(values)
        .map(|(f, v)| EditFieldView { id: f.id, name: f.name.clone(), value: v })
        .collect();
    let tmpl = EditTemplate { chrome, layout_id, table: table.name.clone(), id, fields: fv };
    Html(tmpl.render().unwrap()).into_response()
}

/// Save edits to a record, then back to the list.
async fn save_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    {
        let sol = st.sol.lock().unwrap();
        if let Some((_lay, table)) = layout_table(&sol, layout_id) {
            let fields = sol.fields(table.id).unwrap();
            let values = collect_values(&fields, &form);
            sol.update_record(&table, id, &values).unwrap();
        }
    }
    Redirect::to(&format!("/browse/{layout_id}"))
}

/// Delete a record, then back to the list.
async fn delete_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    {
        let sol = st.sol.lock().unwrap();
        if let Some((_lay, table)) = layout_table(&sol, layout_id) {
            sol.delete_record(&table, id).unwrap();
        }
    }
    Redirect::to(&format!("/browse/{layout_id}"))
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
        .route("/browse/:layout/:id/edit", get(edit_form))
        .route("/browse/:layout/:id/delete", post(delete_record))
        .route("/design/:layout", get(design))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let mut sol = Solution::open("./.rm-data").expect("open solution");
    seed(&mut sol).expect("seed");
    let state = AppState { sol: Arc::new(Mutex::new(sol)) };

    let addr = "127.0.0.1:4317";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    println!("record-maker → http://{addr}");
    axum::serve(listener, app(state)).await.expect("serve");
}
