//! Erreurs JSON du panneau d'administration.
//!
//! Le contrat des handlers est uniforme : une erreur applicative est un corps
//! `{"error":"<code_stable>"}` avec le statut HTTP correspondant. Le détail
//! (erreur SQLite, chemin disque…) n'est jamais exposé au client : il part dans
//! les logs `tracing` via [`AdminError::db`] et le client reçoit
//! `500 {"error":"internal_error"}`, exactement comme le middleware global
//! `normalize_errors`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Erreur d'API admin : statut + code d'erreur stable du contrat legacy.
#[derive(Debug)]
pub enum AdminError {
    /// 400 avec un code métier (`invalid_guild_id`, `invalid_payload`…).
    BadRequest(&'static str),
    /// 404 avec un code métier (`overlay_client_not_found`…).
    NotFound(&'static str),
    /// 500 `internal_error` ; le détail a déjà été journalisé.
    Internal,
}

impl AdminError {
    /// Construit un 400 avec le code métier fourni.
    pub fn bad_request(code: &'static str) -> Self {
        Self::BadRequest(code)
    }

    /// Construit un 404 avec le code métier fourni.
    pub fn not_found(code: &'static str) -> Self {
        Self::NotFound(code)
    }

    /// Encapsule une erreur de base de données : journalise le détail et
    /// renvoie un 500 neutre.
    pub fn db(context: &'static str, error: impl std::fmt::Display) -> Self {
        tracing::error!(
            context = context,
            error = %error,
            "erreur de base de données dans le panneau admin"
        );
        Self::Internal
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        let (status, code): (StatusCode, &'static str) = match self {
            Self::BadRequest(code) => (StatusCode::BAD_REQUEST, code),
            Self::NotFound(code) => (StatusCode::NOT_FOUND, code),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };

        (status, Json(json!({ "error": code }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::body::to_bytes;
    use serde_json::Value;

    use super::*;

    async fn body_json(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("corps lisible");
        serde_json::from_slice(&bytes).expect("corps JSON valide")
    }

    #[tokio::test]
    async fn erreur_metier_renvoie_statut_et_code() {
        let response = AdminError::bad_request("invalid_payload").into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(response).await,
            json!({ "error": "invalid_payload" })
        );

        let response = AdminError::not_found("overlay_client_not_found").into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            body_json(response).await,
            json!({ "error": "overlay_client_not_found" })
        );
    }

    #[tokio::test]
    async fn erreur_interne_ne_fuite_aucun_detail() {
        let response = AdminError::db("test", "détail secret").into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body_json(response).await,
            json!({ "error": "internal_error" })
        );
    }
}
