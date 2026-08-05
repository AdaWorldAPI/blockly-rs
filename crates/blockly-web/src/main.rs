//! The showcase server — one page, one endpoint, one binary.
//!
//! Real Blockly (loaded from a pinned CDN build, per the standing rule:
//! *"JavaScript keeps dragging puzzle pieces; Rust owns semantics"* — a block
//! renderer is never ported) drags blocks in the browser; every change POSTs
//! the workspace save to `/api/cast`, which runs the shipped pipeline —
//! `blockly-shim` → `lower_program` → stored-node bytes — and returns what
//! the panels show, refusals included.
//!
//! Binds `0.0.0.0:$PORT` (Railway injects `PORT`; 8080 is only the local
//! fallback — never hardcoded as the deploy port).

mod cast;

use askama::Template;
use axum::extract::Query;
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate;

#[derive(Deserialize)]
struct CastQuery {
    /// `pairs` (default) / `triples` / `quads` — anything else falls back to
    /// `pairs`, the same forgiving-field rule the workspace's other demo
    /// selectors use.
    shape: Option<String>,
}

async fn index() -> Html<String> {
    Html(IndexTemplate.render().expect("static template renders"))
}

async fn api_cast(Query(q): Query<CastQuery>, body: String) -> Json<cast::CastOut> {
    Json(cast::cast_workspace(
        &body,
        q.shape.as_deref().unwrap_or("pairs"),
    ))
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let app = Router::new()
        .route("/", get(index))
        .route("/api/cast", post(api_cast));
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    println!("blockly-web listening on http://{addr}");
    axum::serve(listener, app).await.expect("server runs");
}
