//! AthenaDB standalone HTTP service (Axum) — scaffold.
//!
//! Mode 2 of the dual-mode design: an independent microservice other systems
//! can query. Routes are stubbed to 501 until Phase 5 commits to the
//! multi-service deployment (per the plan, build-or-delete decided then).
//!
//! Run with: `cargo run --bin server --features server`

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;

const ADDR: &str = "127.0.0.1:8001";

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/add", post(not_implemented))
        .route("/search", post(not_implemented))
        .route("/snapshot", post(not_implemented));

    let listener = tokio::net::TcpListener::bind(ADDR)
        .await
        .expect("failed to bind AthenaDB server address");
    println!("AthenaDB HTTP service (scaffold) listening on http://{ADDR}");
    axum::serve(listener, app)
        .await
        .expect("AthenaDB server error");
}

async fn not_implemented() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        "AthenaDB HTTP API is scaffolded; endpoints arrive in Phase 5.",
    )
}
