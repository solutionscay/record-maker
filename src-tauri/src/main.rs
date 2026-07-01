//! Tauri 2 desktop shell for record-maker (#16).
//!
//! The desktop app does **not** rewrite the UI as native/IPC. Instead it embeds
//! the exact same axum server the CLI runs (`record_maker_server`) in-process,
//! binds it to an ephemeral loopback port (`127.0.0.1:0`), learns the assigned
//! port via `local_addr()`, and points a `WebviewWindow` at
//! `http://127.0.0.1:<port>`. Zero IPC commands, zero route rewrites.
//!
//! Configurable paths (no bundled-asset crate needed):
//!  * DB dir  → Tauri `app_data_dir()`  (also exported as `RM_DATA_DIR`)
//!  * UI dir  → Tauri `resource_dir()/ui/dist`  (also exported as `RM_UI_DIR`)

// On release Windows builds, hide the extra console window. Harmless elsewhere.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use record_maker_engine::Solution;
use record_maker_server::{seed, serve, AppState};

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // --- Resolve per-app, writable/bundled paths from Tauri. ---
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("resolve app data dir");
            std::fs::create_dir_all(&data_dir).ok();

            // Resolve the built editor bundle (`/ui/*` assets), most-specific
            // first: (1) an explicit `RM_UI_DIR` (the dev run script sets this to
            // the repo's `ui/dist`); (2) the bundled Tauri resource for a packaged
            // app (`<resources>/ui/dist`, see `bundle.resources`); (3) a dev
            // fallback to this crate's `../ui/dist` in the source tree. Each
            // candidate must exist to win, so the packaged and dev paths both work.
            let ui_dir = std::env::var("RM_UI_DIR")
                .map(std::path::PathBuf::from)
                .ok()
                .filter(|p| p.exists())
                .or_else(|| {
                    app.path()
                        .resource_dir()
                        .ok()
                        .map(|r| r.join("ui").join("dist"))
                        .filter(|p| p.exists())
                })
                .unwrap_or_else(|| {
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("..")
                        .join("ui")
                        .join("dist")
                });

            // Export for anything downstream that reads the env directly (the
            // server lib reads its config from AppState, but the CLI defaults
            // and any child process honor these).
            std::env::set_var("RM_DATA_DIR", &data_dir);
            std::env::set_var("RM_UI_DIR", &ui_dir);

            // --- Open the solution + build the shared app state. ---
            let mut sol = Solution::open(&data_dir).expect("open solution");
            seed(&mut sol).expect("seed solution");
            let state = AppState::new(sol).with_ui_dir(ui_dir.to_string_lossy().to_string());

            // --- Bind the ephemeral loopback port NOW (fast, deterministic),
            // then hand the long-running server future to the async runtime. ---
            let (addr, server) = tauri::async_runtime::block_on(serve(state, None))
                .expect("bind embedded server");
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server.await {
                    eprintln!("embedded server exited: {e}");
                }
            });

            // --- Point the WebView at the now-known port. ---
            let url = format!("http://127.0.0.1:{}", addr.port());
            eprintln!("record-maker desktop → {url}");
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(url.parse().expect("parse server url")),
            )
            .title("Record Maker")
            .inner_size(1280.0, 960.0)
            .min_inner_size(400.0, 300.0)
            .resizable(true)
            .center()
            .build()
            .expect("create main window");

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
