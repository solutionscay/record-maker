//! Browse Mode runtime + app shell. One generic set of handlers serves every
//! table by reading metadata — no per-table code. The shell (top bar with mode
//! toggle + layout selector, left rail, status bar) is server-rendered and
//! shared by every view (#19, ADR-0005).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{Html, IntoResponse, Redirect},
    routing::{get, post},
    Router,
};
use record_maker_engine::{FieldKind, NewField, Solution};

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
    /// Build the chrome from the solution for a given mode + current layout.
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

#[derive(Template)]
#[template(path = "browse.html")]
struct BrowseTemplate {
    chrome: Chrome,
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
    table: String,
    id: i64,
    fields: Vec<EditFieldView>,
}

struct EditFieldView {
    id: i64,
    name: String,
    value: String,
}

/// Home → first table's Browse view, or a hint if there are none.
async fn index(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.tables().unwrap().into_iter().next() {
        Some(t) => Redirect::to(&format!("/t/{}", t.name)).into_response(),
        None => Html("<p>No tables yet.</p>".to_string()).into_response(),
    }
}

/// Browse a table (Table view): headers from fields, rows from data.db.
async fn browse(State(st): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some(table) = sol.table_by_name(&name).unwrap() else {
        return Html(format!("<p>No such table: {name}</p>")).into_response();
    };
    let fields = sol.fields(table.id).unwrap();
    let records = sol.list_records(&table, &fields).unwrap();
    let chrome = Chrome::build(&sol, "browse", None);

    let tmpl = BrowseTemplate {
        chrome,
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

/// Create a record from the new-record form (inputs named `f<field_id>`).
async fn create_record(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    {
        let sol = st.sol.lock().unwrap();
        if let Some(table) = sol.table_by_name(&name).unwrap() {
            let fields = sol.fields(table.id).unwrap();
            let values = fields
                .iter()
                .filter_map(|f| form.get(&format!("f{}", f.id)).map(|v| (f, v.clone())))
                .collect::<Vec<_>>();
            sol.insert_record(&table, &values).unwrap();
        }
    }
    Redirect::to(&format!("/t/{name}"))
}

/// Delete a record by physical id.
async fn delete_record(
    State(st): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
) -> impl IntoResponse {
    {
        let sol = st.sol.lock().unwrap();
        if let Some(table) = sol.table_by_name(&name).unwrap() {
            sol.delete_record(&table, id).unwrap();
        }
    }
    Redirect::to(&format!("/t/{name}"))
}

/// Show the edit form for a record, pre-filled with its current values.
async fn edit_form(
    State(st): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some(table) = sol.table_by_name(&name).unwrap() else {
        return Html(format!("<p>No such table: {name}</p>")).into_response();
    };
    let fields = sol.fields(table.id).unwrap();
    let Some(values) = sol.get_record(&table, &fields, id).unwrap() else {
        return Html(format!("<p>No such record: {id}</p>")).into_response();
    };
    let chrome = Chrome::build(&sol, "browse", None);
    let fv = fields
        .iter()
        .zip(values)
        .map(|(f, v)| EditFieldView { id: f.id, name: f.name.clone(), value: v })
        .collect();
    let tmpl = EditTemplate { chrome, table: table.name.clone(), id, fields: fv };
    Html(tmpl.render().unwrap()).into_response()
}

/// Save edits to a record (inputs named `f<field_id>`), then back to the list.
async fn save_record(
    State(st): State<AppState>,
    Path((name, id)): Path<(String, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    {
        let sol = st.sol.lock().unwrap();
        if let Some(table) = sol.table_by_name(&name).unwrap() {
            let fields = sol.fields(table.id).unwrap();
            let values = fields
                .iter()
                .filter_map(|f| form.get(&format!("f{}", f.id)).map(|v| (f, v.clone())))
                .collect::<Vec<_>>();
            sol.update_record(&table, id, &values).unwrap();
        }
    }
    Redirect::to(&format!("/t/{name}"))
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
        .route("/t/:name", get(browse).post(create_record))
        .route("/t/:name/:id", post(save_record))
        .route("/t/:name/:id/edit", get(edit_form))
        .route("/t/:name/:id/delete", post(delete_record))
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
