//! Normalisation des réponses d'erreur HTTP.
//!
//! ## Contrat
//!
//! Le legacy répondait toujours avec un corps JSON minimal pour les erreurs
//! applicatives : `{"error":"internal_error"}` pour un statut ≥ 500 et
//! `{"error":"request_error"}` pour un statut 4xx
//! (cf. `docs/rust/audit-dynamique.md`).
//!
//! Axum, lui, produit parfois des corps en texte brut (limite de taille du
//! corps, extracteurs, méthode non autorisée…). Le middleware
//! [`normalize_errors`] restaure le contrat : toute réponse d'erreur dont le
//! `Content-Type` n'est pas JSON est réécrite avec le corps normalisé, en
//! conservant le statut d'origine et les en-têtes porteurs de sens
//! (`x-request-id`, `retry-after`, `access-control-*`…). Les corps déjà JSON
//! (réponses applicatives comme `/socket.io/`, rate limit, `/health`) sont
//! laissés intacts.
//!
//! Le middleware journalise aussi l'erreur avec le contexte de corrélation :
//! `error!` pour les 5xx, `warn!` pour les 4xx, jamais de fuite du corps
//! d'origine dans la réponse.

use axum::http::{header, Method, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{extract::Request, Json};
use serde::Serialize;

/// Code d'erreur renvoyé pour un statut ≥ 500.
pub const INTERNAL_ERROR: &str = "internal_error";

/// Code d'erreur renvoyé pour un statut 4xx.
pub const REQUEST_ERROR: &str = "request_error";

/// Corps JSON d'erreur normalisé.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    /// Code machine stable (`internal_error` ou `request_error`).
    pub error: &'static str,
}

/// Construit une réponse JSON d'erreur normalisée.
pub fn json_error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(ErrorBody { error: code })).into_response()
}

/// Handler de repli : toute route inconnue répond 404 au format normalisé.
pub async fn fallback() -> Response {
    json_error(StatusCode::NOT_FOUND, REQUEST_ERROR)
}

/// Middleware de normalisation des erreurs (voir la documentation du module).
pub async fn normalize_errors(request: Request, next: Next) -> Response {
    let method: Method = request.method().clone();
    let uri: Uri = request.uri().clone();
    let response = next.run(request).await;
    let status = response.status();

    if !status.is_client_error() && !status.is_server_error() {
        return response;
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let already_json = content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("application/json"));
    if already_json {
        return response;
    }

    let code = if status.is_server_error() {
        tracing::error!(
            method = %method,
            uri = %uri,
            status = status.as_u16(),
            "requête en erreur serveur (réponse normalisée)"
        );
        INTERNAL_ERROR
    } else {
        tracing::warn!(
            method = %method,
            uri = %uri,
            status = status.as_u16(),
            "requête invalide (réponse normalisée)"
        );
        REQUEST_ERROR
    };

    let mut normalized = json_error(status, code);
    for (name, value) in response.headers() {
        if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
            normalized.headers_mut().insert(name.clone(), value.clone());
        }
    }
    // Trace de diagnostic : type de contenu d'origine, utile pour comprendre
    // quelle couche interne a produit le corps brut.
    if let Some(content_type) = content_type {
        tracing::debug!(uri = %uri, content_type = %content_type, "corps d'erreur brut remplacé");
    }
    normalized
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    async fn body_json(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("corps lisible");
        let value = serde_json::from_slice(&bytes).expect("corps JSON");
        (status, value)
    }

    #[tokio::test]
    async fn server_error_text_is_normalized() {
        async fn boom() -> Response {
            (StatusCode::INTERNAL_SERVER_ERROR, "boum").into_response()
        }

        let app = Router::new()
            .route("/boom", get(boom))
            .layer(middleware::from_fn(normalize_errors));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/boom")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let (status, body) = body_json(response).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, serde_json::json!({ "error": INTERNAL_ERROR }));
    }

    #[tokio::test]
    async fn client_error_text_is_normalized() {
        async fn teapot() -> Response {
            (StatusCode::IM_A_TEAPOT, "thé").into_response()
        }

        let app = Router::new()
            .route("/teapot", get(teapot))
            .layer(middleware::from_fn(normalize_errors));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/teapot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_json(response).await;
        assert_eq!(status, StatusCode::IM_A_TEAPOT);
        assert_eq!(body, serde_json::json!({ "error": REQUEST_ERROR }));
    }

    #[tokio::test]
    async fn json_error_body_is_left_untouched() {
        async fn json_error_handler() -> Response {
            json_error(StatusCode::TOO_MANY_REQUESTS, REQUEST_ERROR)
        }

        let app = Router::new()
            .route("/limite", get(json_error_handler))
            .layer(middleware::from_fn(normalize_errors));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/limite")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let (_, body) = body_json(response).await;
        assert_eq!(body, serde_json::json!({ "error": REQUEST_ERROR }));
    }

    #[tokio::test]
    async fn successful_response_is_untouched() {
        async fn ok() -> &'static str {
            "contenu brut"
        }

        let app = Router::new()
            .route("/ok", get(ok))
            .layer(middleware::from_fn(normalize_errors));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/ok")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.as_ref(), b"contenu brut");
    }
}
