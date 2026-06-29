use askama::Template;
use axum::{response::Html, routing::get, Router};

/// Full page (Browse Mode shell). Extends the base layout.
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate;

/// HTMX fragment swapped into the page — proves the round-trip.
#[derive(Template)]
#[template(path = "hello.html")]
struct HelloTemplate;

async fn index() -> Html<String> {
    Html(IndexTemplate.render().expect("render index"))
}

async fn hello() -> Html<String> {
    Html(HelloTemplate.render().expect("render hello"))
}

/// Build the router. Kept as a fn so the Tauri shell (#13b) can embed the
/// same app, and so it can be reused headless for the web-publish target.
fn app() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/hello", get(hello))
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:4317";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    println!("record-maker server → http://{addr}");
    axum::serve(listener, app()).await.expect("serve");
}
