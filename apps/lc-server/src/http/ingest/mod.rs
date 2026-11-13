//! Routes `/ingest` — Phases 3c/5b (owner : http-core).
//!
//! Contrat hérité de `server/src/components/ingest/ingestRoutes.ts` :
//!
//! - `POST /ingest/pair/consume` : consommation d'un code d'appairage « invité »
//!   (`INVITE_READ_ONLY` côté overlay) et création d'un client d'ingestion ;
//! - `POST /ingest` : soumission d'une URL/média/texte, avec authentification
//!   par token de client (`Authorization: Bearer <token>`).
//!
//! Sous-routeur monté par [`crate::app::router`] ; il porte lui-même le préfixe
//! `/ingest` (les sous-routeurs sont fusionnés sans préfixe dans l'application).
//!
//! ## Décision d'authentification
//!
//! Le legacy n'utilise **jamais** `INGEST_API_TOKEN` pour authentifier `/ingest`
//! (variable seulement affichée dans l'admin) : seuls les clients d'ingestion
//! créés par appairage, identifiés par le hash SHA-256 de leur token, sont
//! acceptés. Le token global n'est conservé que pour parité de configuration.

pub mod auth;
pub mod body;
pub mod pair;
pub mod submit;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::media::errors::MediaServiceError;

/// Nom d'auteur par défaut des clients d'ingestion (legacy).
pub const DEFAULT_INGEST_AUTHOR_NAME: &str = "LiveChat Extension";

/// Préfixe des noms d'appareil générés (legacy).
pub const DEFAULT_INGEST_DEVICE_PREFIX: &str = "Extension";

/// Sous-routeur `/ingest`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ingest", post(submit::submit))
        .route("/ingest/", post(submit::submit))
        .route("/ingest/pair/consume", post(pair::consume))
        .route("/ingest/pair/consume/", post(pair::consume))
}

/// Réponse JSON d'erreur applicative avec un statut donné.
pub(crate) fn error_response(status: StatusCode, payload: Value) -> Response {
    (status, Json(payload)).into_response()
}

/// `{"error": code}` sans message libre.
pub(crate) fn simple_error(status: StatusCode, code: &str) -> Response {
    error_response(status, json!({ "error": code }))
}

/// `{"error":"invalid_payload","message":...}` (message optionnel).
pub(crate) fn invalid_payload(message: Option<&str>) -> Response {
    let payload = match message {
        Some(message) => json!({ "error": "invalid_payload", "message": message }),
        None => json!({ "error": "invalid_payload" }),
    };
    error_response(StatusCode::BAD_REQUEST, payload)
}

/// Erreur interne normalisée du legacy.
pub(crate) fn internal_error() -> Response {
    simple_error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}

/// Conflit d'unicité quel que soit le chemin de conversion de l'erreur.
///
/// `lc-db` convertit parfois `SQLITE_CONSTRAINT` en `DomainError::Conflict`,
/// parfois laisse le `DbError::Sqlx` brut : les deux formes doivent être
/// reconnues pour que les branches de récupération (régénération de token,
/// relecture d'un item créé en concurrence) ne soient pas mortes (AUD-A2-010).
pub(crate) fn is_conflict(db_error: &lc_db::DbError) -> bool {
    db_error.is_unique_violation()
        || matches!(
            db_error,
            lc_db::DbError::Domain(lc_domain::DomainError::Conflict(_))
        )
}

/// Erreur d'ingestion média : 422 `media_ingestion_failed` avec code legacy,
/// 500 pour une erreur inattendue, 503 si le service est indisponible.
pub(crate) fn media_service_error(error: MediaServiceError) -> Response {
    if let Some(media_error) = error.media_error() {
        tracing::warn!(
            code = media_error.code().as_str(),
            message = media_error.message(),
            "ingestion média en échec"
        );
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({
                "error": "media_ingestion_failed",
                "code": media_error.code().as_str(),
                "message": media_error.message(),
            }),
        );
    }

    match error {
        MediaServiceError::Unavailable(message) => {
            tracing::error!(message, "service d'ingestion indisponible");
            simple_error(StatusCode::SERVICE_UNAVAILABLE, "ingest_unavailable")
        }
        other => {
            tracing::error!(error = %other, "erreur interne du service d'ingestion");
            internal_error()
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::config::Config;
    use crate::shutdown::Shutdown;
    use std::sync::Arc;

    fn state() -> AppState {
        let config =
            Config::from_pairs([("API_URL".to_owned(), "http://localhost:3000".to_owned())])
                .expect("configuration de test valide");
        AppState::new(Arc::new(config), None, Shutdown::detached())
    }

    async fn send(app: Router, uri: &str) -> (StatusCode, Value) {
        let request = HttpRequest::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn routes_are_mounted_on_both_slash_variants() {
        let app = router().with_state(state());
        for uri in ["/ingest/pair/consume", "/ingest/pair/consume/"] {
            let (status, body) = send(app.clone(), uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(body["error"], "invalid_payload", "{uri}");
        }
    }
}
