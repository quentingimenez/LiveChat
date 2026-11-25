//! `GET /ingest/ingestions/{id}` : state of a media accepted with `202`.
//!
//! Same token as the submission (`Authorization: Bearer <token>`); an
//! ingestion is only visible to the client that submitted it, any other id is
//! `404 ingestion_not_found`. The body is the registry entry:
//!
//! - `{"ingestionId", "status": "processing"}` while the task runs;
//! - `{"status": "ready", "jobId"}` for a play, or
//!   `{"status": "ready", "created", "itemId"}` for a board add;
//! - `{"status": "failed", "error": {"code", "message"?, "mediaCode"?}}`.
//!
//! A finished entry stays readable for
//! [`crate::ingestions::FINISHED_RETENTION`].

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::app::AppState;

use super::{auth, simple_error};

/// Handler of `GET /ingest/ingestions/{id}`.
pub async fn ingestion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(ingestion_id): Path<String>,
) -> Response {
    let client = match auth::require_client(&state, &headers).await {
        Ok(client) => client,
        Err(response) => return response,
    };
    match state
        .ingestions
        .get(client.id.as_uuid(), &ingestion_id, state.clock.now())
    {
        Some(entry) => (StatusCode::OK, Json(entry.payload())).into_response(),
        None => simple_error(StatusCode::NOT_FOUND, "ingestion_not_found"),
    }
}
