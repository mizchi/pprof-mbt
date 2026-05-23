//! Minimal empty-handler HTTP server: listens on 0.0.0.0:30003 and
//! responds 200 OK with an empty body for any GET /. Used as a
//! compiled-language baseline for k6 load tests against
//! moonbitlang/async's HTTP server (which listens on :30001).
//!
//! Drop-in replacement for the previous Go fixture
//! `bench-async/cmd/http_server_benchmark/server.go`. Same behavior:
//! handler does nothing, axum/hyper auto-emit `200 OK` with an empty
//! body and `Content-Length: 0`.

use axum::Router;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Catch-all fallback so any path returns 200 with an empty body,
    // matching Go's `http.HandleFunc("/", ...)` prefix semantics.
    let app = Router::new().fallback(|| async { "" });
    let listener = tokio::net::TcpListener::bind("0.0.0.0:30003").await?;
    eprintln!("http-baseline-server: listening on http://127.0.0.1:30003/");
    axum::serve(listener, app).await?;
    Ok(())
}
