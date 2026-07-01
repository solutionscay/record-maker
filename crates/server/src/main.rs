//! Thin CLI wrapper around the server library. Binds to an explicit port
//! (`RM_PORT`, default 4317) for standalone/dev use; the desktop shell (#16)
//! embeds the library directly via `record_maker_server::serve`.
//!
//! Configurable paths (no new dependencies): `RM_DATA_DIR` (default
//! `./.rm-data`) for the SQLite metadata directory, and `RM_UI_DIR` (default
//! `ui/dist`) for the built editor bundle the `/ui/*` route serves.

use record_maker_engine::Solution;
use record_maker_server::{app, seed, AppState, DEFAULT_UI_DIR};

#[tokio::main]
async fn main() {
    let data_dir = std::env::var("RM_DATA_DIR").unwrap_or_else(|_| "./.rm-data".to_string());
    let ui_dir = std::env::var("RM_UI_DIR").unwrap_or_else(|_| DEFAULT_UI_DIR.to_string());

    let mut sol = Solution::open(&data_dir).expect("open solution");
    seed(&mut sol).expect("seed");
    let state = AppState::new(sol).with_ui_dir(ui_dir);

    let port = std::env::var("RM_PORT").unwrap_or_else(|_| "4317".to_string());
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    println!("record-maker → http://{addr}");
    axum::serve(listener, app(state)).await.expect("serve");
}
