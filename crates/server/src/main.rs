//! Browse Mode runtime (#14): wires the embedded axum server to the engine so
//! layouts/records are served live from data.db. One generic set of handlers
//! serves every table by reading metadata — no per-table code.

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

#[derive(Template)]
#[template(path = "browse.html")]
struct BrowseTemplate {
    tables: Vec<String>,
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

/// Home → first table's Browse view, or a hint if there are none.
async fn index(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    match sol.tables().unwrap().into_iter().next() {
        Some(t) => Redirect::to(&format!("/t/{}", t.name)).into_response(),
        None => Html("<p>No tables yet.</p>".to_string()).into_response(),
    }
}

/// Browse a table: column headers from fields, rows from data.db.
async fn browse(State(st): State<AppState>, Path(name): Path<String>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let tables = sol
        .tables()
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect::<Vec<_>>();
    let Some(table) = sol.table_by_name(&name).unwrap() else {
        return Html(format!("<p>No such table: {name}</p>")).into_response();
    };
    let fields = sol.fields(table.id).unwrap();
    let records = sol.list_records(&table, &fields).unwrap();

    let tmpl = BrowseTemplate {
        tables,
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

/// Build the router. A fn so the Tauri shell (#13b) embeds the same app.
fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/t/:name", get(browse).post(create_record))
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
