//! Garde-fou de migration pour les clients legacy.
//!
//! Le protocole v1 (Socket.IO) a été remplacé par le protocole v2 (WebSocket
//! natif sur `/ws`, Phase 4). Tout client qui tente encore `/socket.io/` reçoit
//! un `426 Upgrade Required` dont le corps est le contrat exact spécifié dans
//! `docs/rust/protocol-v2.md` :
//!
//! ```json
//! {"error":"overlay_outdated","protocolVersion":2,"minOverlayVersion":"3.0.0"}
//! ```
//!
//! `minOverlayVersion` provient de la configuration (`MIN_OVERLAY_VERSION`,
//! défaut `3.0.0`) : l'overlay peut ainsi signaler à ses utilisateurs la
//! version minimale réellement attendue par le serveur déployé.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::any;
use axum::{Json, Router};
use serde::Serialize;

use crate::app::AppState;

/// Corps de la réponse 426.
#[derive(Debug, Serialize)]
struct OverlayOutdatedResponse {
    /// Code machine stable attendu par le client legacy.
    error: &'static str,
    /// Version du protocole supportée par ce serveur.
    #[serde(rename = "protocolVersion")]
    protocol_version: u8,
    /// Version minimale d'overlay configurée.
    #[serde(rename = "minOverlayVersion")]
    min_overlay_version: String,
}

/// Sous-routeur `/socket.io` (avec ou sans slash final, quelque soit la méthode).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/socket.io", any(socket_io_handler))
        .route("/socket.io/", any(socket_io_handler))
        .route("/socket.io/{*rest}", any(socket_io_handler))
}

async fn socket_io_handler(State(state): State<AppState>) -> impl IntoResponse {
    tracing::info!(
        min_overlay_version = %state.config.min_overlay_version,
        "tentative d'accès au protocole v1 (/socket.io) : 426 overlay_outdated"
    );
    (
        StatusCode::UPGRADE_REQUIRED,
        Json(OverlayOutdatedResponse {
            error: "overlay_outdated",
            protocol_version: lc_protocol::PROTOCOL_VERSION,
            min_overlay_version: state.config.min_overlay_version.clone(),
        }),
    )
}
