//! Test helpers shared by the `/ingest` route tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

/// `GET /ingest/ingestions/{id}` with an ingest token.
pub(super) async fn get_ingestion(app: &Router, token: &str, id: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/ingest/ingestions/{id}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Polls the ingestion of a `202` answer until its background task ends.
///
/// Returns the final state (`ready` or `failed`); panics after 30 s or on any
/// status other than `200`.
pub(super) async fn wait_ingestion(app: &Router, token: &str, accepted: &Value) -> Value {
    let id = accepted["ingestionId"]
        .as_str()
        .unwrap_or_else(|| panic!("ingestionId absent de {accepted}"));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (status, body) = get_ingestion(app, token, id).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if body["status"] != "processing" {
            return body;
        }
        assert!(Instant::now() < deadline, "ingestion {id} non terminée");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
