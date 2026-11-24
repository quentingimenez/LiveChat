//! Garde-fou de migration pour les clients legacy.
//!
//! Le protocole v1 (Socket.IO) a été remplacé par le protocole v2 (WebSocket
//! natif sur `/ws`, Phase 4). Tout client qui tente encore `/socket.io/` reçoit
//! un `426 Upgrade Required` produit par le contrat d'erreur central :
//!
//! ```json
//! {"error":"overlay_outdated","message":"L'overlay doit être mis à jour.","protocolVersion":2,"minOverlayVersion":"3.0.0"}
//! ```
//!
//! `minOverlayVersion` provient de la configuration (`MIN_OVERLAY_VERSION`,
//! défaut `3.0.0`) : l'overlay peut ainsi signaler à ses utilisateurs la
//! version minimale réellement attendue par le serveur déployé.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;

use crate::app::AppState;
use crate::http::error::ApiError;

/// Sous-routeur `/socket.io` (avec ou sans slash final, quelque soit la méthode).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/socket.io", any(socket_io_handler))
        .route("/socket.io/", any(socket_io_handler))
        .route("/socket.io/{*rest}", any(socket_io_handler))
}

async fn socket_io_handler(State(state): State<AppState>) -> impl IntoResponse {
    let min_overlay_version = state.runtime().min_overlay_version();
    tracing::info!(
        min_overlay_version = %min_overlay_version,
        "tentative d'accès au protocole v1 (/socket.io) : 426 overlay_outdated"
    );
    ApiError::new(StatusCode::UPGRADE_REQUIRED, "overlay_outdated")
        .detail("protocolVersion", lc_protocol::PROTOCOL_VERSION)
        .detail("minOverlayVersion", min_overlay_version)
        .into_response()
}
