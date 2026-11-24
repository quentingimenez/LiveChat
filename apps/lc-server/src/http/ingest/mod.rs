//! Routes `/ingest` — Phases 3c/5b (owner : http-core).
//!
//! Contrat hérité de `server/src/components/ingest/ingestRoutes.ts` :
//!
//! - `POST /ingest/pair/consume` : consommation d'un code d'appairage « invité »
//!   (`INVITE_READ_ONLY` côté overlay) et création d'un client d'ingestion ;
//! - `POST /ingest` : soumission d'une URL/média/texte, avec authentification
//!   par token de client (`Authorization: Bearer <token>`) ;
//! - `GET /ingest/whoami` : vérification d'un token seul, sans ingestion
//!   (diagnostic des raccourcis qui ne regardent pas le statut de `POST`).
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

// `Result<_, Response>` est le type d'erreur natif des handlers axum : le
// boxing systématique dégraderait la lisibilité sans gain sur ces chemins I/O.
// Le lint `result_large_err` est donc neutralisé à l'échelle du module, comme
// pour `http::overlay`.
#![allow(clippy::result_large_err)]

pub mod auth;
pub mod body;
pub mod file;
pub mod pair;
pub mod submit;
pub mod whoami;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;

use crate::app::AppState;
use crate::http::error::ApiError;
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
        .route("/ingest/file", post(file::upload))
        .route("/ingest/file/", post(file::upload))
        .route("/ingest/pair/consume", post(pair::consume))
        .route("/ingest/pair/consume/", post(pair::consume))
        .route("/ingest/whoami", get(whoami::whoami))
        .route("/ingest/whoami/", get(whoami::whoami))
}

/// Réponse d'erreur applicative avec un statut et un code donnés.
pub(crate) fn simple_error(status: StatusCode, code: &'static str) -> Response {
    ApiError::new(status, code).into_response()
}

/// Jeton d'ingestion inactif depuis trop longtemps : 401 distinct du 503
/// « ingestion désactivée ». Le message invite à ré-appairer depuis la board,
/// sans promettre qu'un ancien jeton puisse être réutilisé.
pub(crate) fn expired_token_error() -> Response {
    ApiError::new(StatusCode::UNAUTHORIZED, "ingest_token_expired").into_response()
}

/// `400 invalid_payload` avec une précision humaine statique optionnelle.
///
/// Le message doit être une constante française : jamais une rejection d'axum,
/// un message de parseur ni une donnée de la requête.
pub(crate) fn invalid_payload(message: Option<&'static str>) -> Response {
    let error = ApiError::invalid_payload();
    match message {
        Some(message) => error.with_message(message),
        None => error,
    }
    .into_response()
}

/// Erreur interne normalisée du legacy.
pub(crate) fn internal_error() -> Response {
    ApiError::internal().into_response()
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

/// Erreur d'ingestion média : 422 `media_ingestion_failed` avec code legacy et
/// message utilisateur du code, 500 pour une erreur inattendue, 503 si le
/// service est indisponible.
///
/// La sortie brute de l'outil (`yt-dlp`, `ffmpeg`…) reste dans les journaux :
/// elle ne doit jamais atteindre `message` (cf. [`crate::http::error`]).
pub(crate) fn media_service_error(error: MediaServiceError) -> Response {
    if let Some(media_error) = error.media_error() {
        let code = media_error.code();
        tracing::warn!(
            code = code.as_str(),
            detail = media_error.message(),
            raw = media_error.raw_message().unwrap_or(""),
            "ingestion média en échec"
        );
        return ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "media_ingestion_failed")
            .with_message(code.user_message_fr())
            .detail("code", code.as_str())
            .into_response();
    }

    match error {
        MediaServiceError::Unavailable(message) => {
            tracing::error!(detail = message, "service d'ingestion indisponible");
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

    /// Régression : le toast de l'extension affichait le stderr brut de `yt-dlp`
    /// (« Sign in to confirm your age… Use --cookies-from-browser… »). Le corps
    /// 422 doit porter la phrase utilisateur du code, jamais la sortie d'outil.
    #[tokio::test]
    async fn media_failure_never_leaks_the_raw_tool_output() {
        const RAW: &str = "ERROR: [youtube] 6GvU1WyGMok: Sign in to confirm your age. \
This video may be inappropriate for some users. Use --cookies-from-browser or --cookies \
for the authentication. See https://github.com/yt-dlp/yt-dlp/wiki/FAQ";
        let error = lc_media::MediaError::from_command_failure(
            "yt-dlp",
            "",
            RAW,
            lc_media::MediaErrorCode::DownloadFailed,
        );

        let response = media_service_error(MediaServiceError::Media(error));
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(body["error"], "media_ingestion_failed");
        assert_eq!(body["code"], "PRIVATE_OR_AUTH_REQUIRED");
        assert_eq!(
            body["message"],
            "Ce média est privé ou demande une connexion."
        );
        let message = body["message"].as_str().unwrap();
        assert!(!message.contains("ERROR:"));
        assert!(!message.contains("yt-dlp"));
        assert!(!message.contains("cookies-from-browser"));
    }
}
