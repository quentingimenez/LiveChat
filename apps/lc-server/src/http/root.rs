//! Route d'identité du service (`GET /`).
//!
//! Le legacy répondait exactement
//! `{"service":"livechat-server","status":"ok"}` ; ce contrat est conservé pour
//! les sondes de déploiement et les tests de fumée existants
//! (cf. `docs/rust/audit-dynamique.md`).

use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::app::AppState;

/// Corps de `GET /`.
#[derive(Debug, Serialize)]
pub struct RootResponse {
    /// Nom du service, identique au legacy.
    pub service: &'static str,
    /// État brut (« ok » : la route ne dépend d'aucune ressource externe).
    pub status: &'static str,
}

/// Sous-routeur de la racine.
pub fn router() -> Router<AppState> {
    Router::new().route("/", get(root_handler))
}

async fn root_handler() -> impl IntoResponse {
    Json(RootResponse {
        service: "livechat-server",
        status: "ok",
    })
}
