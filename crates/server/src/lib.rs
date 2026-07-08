//! Browse/Layout mode runtime + app shell. Routing is **layout-keyed**
//! (ADR-0005): `/browse/:layout` and `/design/:layout`, where `:layout` is the
//! meta_layout **id** (i64). One generic handler set serves every table by
//! reading metadata — no per-table code.
//!
//! This crate is a **library + thin bin**: the router, handlers, and state live
//! here so both the standalone CLI (`src/main.rs`) and the Tauri desktop shell
//! (#16) embed the *same* app. The public API is intentionally small — build an
//! [`AppState`], call [`app`] for the router, and [`serve`] to bind an
//! ephemeral loopback port and learn the assigned address.

use std::collections::HashSet;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use record_maker_engine::Solution;

mod format;
mod routes;
mod style;
#[cfg(test)]
mod tests;
mod validate;
mod viewmodel;

use routes::browse::{browse, design, index, layouts_page, schema_page};
use routes::design::{
    create_design_object, create_design_part, create_object_group, delete_design_object,
    delete_design_objects, delete_design_part, delete_object_group, design_model, format_sample,
    move_design_part, restore_design_objects, update_object_binding, update_object_binding_path,
    update_object_content, update_object_geometry, update_object_part, update_object_props,
    update_object_read_only, update_objects_geometry, update_objects_z, update_part_height,
    update_part_kind, update_part_props,
};
use routes::layouts::{
    create_layout, delete_layout, list_layouts, rename_layout, reorder_layouts,
    set_layout_enabled,
};
use routes::records::{create_record, delete_record, open_record, revert_record, save_record};
use routes::schema::{
    create_schema_field, create_schema_relationship, create_schema_table, create_value_list,
    delete_schema_field, delete_schema_relationship, delete_schema_table, delete_value_list,
    duplicate_value_list, rename_schema_table, reorder_schema_fields, schema_fields,
    schema_relationships, schema_tables, update_schema_field, update_schema_relationship,
    update_schema_table, update_value_list, value_list_items, value_lists,
};

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

fn not_found(what: &str, id: i64) -> axum::response::Response {
    Html(format!("<p>No such {what}: {id}</p>")).into_response()
}

/// Error half of the JSON API handlers. Carries the exact status/body pairs the
/// handlers used to build inline, so a converted handler can use `?` without
/// changing a single response byte:
/// - engine (`anyhow`) errors map to `409 CONFLICT` with the error text — the
///   blanket policy every repetitive handler already applied;
/// - a missing row maps to a bare `404 NOT_FOUND` (empty body);
/// - validation failures map to `400 BAD_REQUEST` with a plain-text message.
enum AppError {
    /// Status-only response (empty body) — the bare 404s.
    Status(StatusCode),
    /// Status + plain-text message — the `(status, msg)` tuples.
    Message(StatusCode, String),
    /// A pre-built response carried whole (the HTML "No such layout" page some
    /// design handlers return), kept byte-identical through the conversion.
    Response(axum::response::Response),
}

/// The JSON handlers' return shape: success renders as-is, [`AppError`] renders
/// the mapped status/body.
type AppResult<T> = Result<T, AppError>;

impl AppError {
    fn not_found() -> Self {
        AppError::Status(StatusCode::NOT_FOUND)
    }

    fn bad_request(msg: impl Into<String>) -> Self {
        AppError::Message(StatusCode::BAD_REQUEST, msg.into())
    }

    fn conflict(msg: impl Into<String>) -> Self {
        AppError::Message(StatusCode::CONFLICT, msg.into())
    }

    /// The HTML "No such layout" page (see [`not_found`]) as an error, for the
    /// design handlers that respond that way to an unknown layout id.
    fn no_such_layout(layout_id: i64) -> Self {
        AppError::Response(not_found("layout", layout_id))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            AppError::Status(status) => status.into_response(),
            AppError::Message(status, msg) => (status, msg).into_response(),
            AppError::Response(resp) => resp,
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::conflict(e.to_string())
    }
}

impl From<(StatusCode, String)> for AppError {
    fn from((status, msg): (StatusCode, String)) -> Self {
        AppError::Message(status, msg)
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
            "/schema/tables/:table_id/fields/:field_id",
            post(update_schema_field),
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
        .route("/value-lists", get(value_lists).post(create_value_list))
        .route("/value-lists/:id", post(update_value_list))
        .route("/value-lists/:id/duplicate", post(duplicate_value_list))
        .route("/value-lists/:id/delete", post(delete_value_list))
        .route("/value-lists/:id/items", get(value_list_items))
        .route("/schema", get(schema_page))
        .route("/layouts", get(layouts_page).post(create_layout))
        .route("/layouts/all", get(list_layouts))
        .route("/layouts/order", post(reorder_layouts))
        .route("/layouts/:id/rename", post(rename_layout))
        .route("/layouts/:id/enabled", post(set_layout_enabled))
        .route("/layouts/:id/delete", post(delete_layout))
        .route("/design/:layout", get(design))
        .route("/design/format-sample", post(format_sample))
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
        .route(
            "/design/:layout/objects/delete",
            post(delete_design_objects),
        )
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
