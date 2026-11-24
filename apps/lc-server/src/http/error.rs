//! Contrat d'erreur HTTP unique du serveur.
//!
//! ## Forme de la réponse
//!
//! Toute réponse d'erreur produite par `lc-server` est un objet JSON :
//!
//! ```json
//! { "error": "media_ingestion_failed", "message": "Ce média est privé ou demande une connexion.", "code": "PRIVATE_OR_AUTH_REQUIRED" }
//! ```
//!
//! - `error` : code machine stable, en `snake_case`, destiné aux clients
//!   (`extension`, board, console admin). Il ne change jamais sans version.
//! - `message` : **phrase française destinée à la personne**, toujours issue de
//!   [`message_for`]. Elle ne contient jamais de sortie d'outil, de requête SQL,
//!   de chemin de fichier, de corps de réponse d'un fournisseur ni de secret.
//! - Les autres champs sont des détails structurés propres à la route
//!   (`code` legacy média, `maxBytes`, `received`…), aplatis à la racine du
//!   corps pour ne pas casser les clients existants.
//!
//! Le détail technique (stderr de `yt-dlp`, erreur SQLx, cause d'un fournisseur)
//! part dans les journaux, corrélé par `x-request-id` ; il ne doit jamais être
//! recopié dans `message` (cf. [`ApiError::with_message`]).
//!
//! ## Couverture
//!
//! [`normalize_errors`] est le filet de sécurité global : tout corps d'erreur
//! non JSON est remplacé par un corps normalisé, et tout corps JSON portant un
//! champ `error` (chaîne) mais aucun `message` est complété avec le message
//! canonique du code. Les routes doivent néanmoins construire leurs erreurs
//! avec [`ApiError`] (ou les helpers qui en dérivent) pour choisir un code et un
//! message précis.
//!
//! Le middleware journalise aussi l'erreur avec le contexte de corrélation :
//! `error!` pour les 5xx, `warn!` pour les 4xx, jamais de fuite du corps
//! d'origine dans la réponse. Seul le **chemin** est journalisé (jamais la
//! query string ni les en-têtes, cf. AUD-COR-05).

use std::borrow::Cow;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{extract::Request, Json};
use serde_json::{Map, Value};

/// Code d'erreur renvoyé pour un statut ≥ 500.
pub const INTERNAL_ERROR: &str = "internal_error";

/// Code d'erreur générique pour un statut 4xx.
pub const REQUEST_ERROR: &str = "request_error";

/// Limite de lecture du corps JSON inspecté par la normalisation.
const MAX_INSPECTED_BODY: usize = 1024 * 1024;

/// Message canonique d'un code machine.
///
/// `None` signifie que le code est inconnu : [`ApiError`] retombe alors sur un
/// message générique dépendant du statut. Cette table est l'unique source des
/// messages par défaut : ajouter un code d'erreur ici, jamais un message brut
/// dans une route.
pub fn message_for(code: &str) -> Option<&'static str> {
    let message = match code {
        // Générique / transport.
        INTERNAL_ERROR => "Une erreur interne est survenue. Réessayez dans un instant.",
        REQUEST_ERROR => "La requête n'a pas pu être traitée.",
        "request_timeout" => "Le serveur n'a pas répondu à temps.",
        "too_many_requests" => "Trop de requêtes. Réessayez dans un instant.",
        "invalid_payload" => "Les données envoyées sont invalides.",
        "payload_too_large" => "Le corps de la requête est trop volumineux.",
        "part_too_large" => "Un fragment envoyé dépasse la taille maximale.",
        "invalid_origin" => "Origine de requête non autorisée.",
        "websocket_required" => "Cette route exige une connexion WebSocket.",

        // Authentification / autorisation.
        "unauthorized" => "Authentification requise.",
        "forbidden" => "Action non autorisée.",
        "forbidden_role" => "Votre rôle ne permet pas cette action.",
        "invalid_secret" => "Secret incorrect.",
        "method_disabled" => "Cette méthode d'authentification est désactivée.",
        "invalid_code" => "Le code est invalide.",
        "read_only_session" => "Cette session est en lecture seule.",
        "member_required" => "Un membre de la guilde doit être rattaché à cet overlay.",
        "member_banned" => "Ce membre est banni de la guilde.",
        "display_name_required" => "Un pseudo est requis pour rejoindre la guilde.",

        // Administration.
        "admin_local_only" =>
            "La console d'administration est accessible uniquement depuis le réseau local.",
        "admin_ui_disabled" => "La console d'administration est désactivée sur ce serveur.",
        "invalid_guild_id" => "Identifiant de guilde invalide.",
        "invalid_client_id" => "Identifiant de client invalide.",
        "invalid_author_discord_user_id" => "L'identifiant Discord de l'auteur est invalide.",
        "invalid_before" => "Le paramètre « before » est invalide.",
        "invalid_limit" => "La limite demandée est invalide.",
        "invalid_name" => "Le nom est invalide.",
        "invalid_label" => "Le libellé est invalide.",
        "invalid_right" => "Droit inconnu.",
        "fixed_right" => "Ce droit ne peut pas être modifié.",
        "author_not_found" => "Auteur introuvable.",
        "pairing_code_not_found" => "Code d'appairage introuvable.",
        "confirm_guild_id_mismatch" =>
            "L'identifiant de confirmation ne correspond pas à la guilde.",
        "guild_not_found" => "Guilde introuvable.",
        "native_guild_unlink" => "Une guilde liée nativement ne peut pas être dissociée.",
        "not_linked" => "Cette guilde n'est pas liée.",
        "ingest_not_found" => "Client d'ingestion introuvable.",
        "overlay_not_found" => "Overlay introuvable.",
        "database_unavailable" => "La base de données est indisponible sur ce serveur.",

        // Ingestion.
        "ingest_api_disabled" => "L'envoi depuis l'extension est désactivé sur ce serveur.",
        "ingest_unavailable" =>
            "Le service d'ingestion est momentanément indisponible. Réessayez dans un instant.",
        "ingest_token_expired" => "Ce jeton d'ingestion a expiré après 180 jours sans activité. Ré-appairez l'appareil depuis la board pour obtenir un nouveau jeton ; l'ancien jeton n'est plus valable sur l'ancien appareil.",
        "media_ingestion_failed" => "Le média n'a pas pu être ingéré.",
        "unsupported_media" => "Ce média n'est pas pris en charge par cette route.",
        "media_url_invalid" => "L'adresse du média est invalide.",
        "media_url_expired" => "Le lien du média a expiré.",
        "media_not_found" => "Média introuvable.",
        "media_not_found_on_disk" => "Le fichier du média est introuvable sur le serveur.",
        "text_too_long" => "Le texte est trop long.",
        "speech_failed" => "La synthèse vocale a échoué.",
        "too_many_ingestions" => "Trop d'ingestions simultanées. Réessayez dans un instant.",
        "too_many_uploads" => "Trop d'envois simultanés. Réessayez dans un instant.",

        // Envoi de fichiers par fragments.
        "upload_not_found" => "Envoi introuvable ou expiré.",
        "upload_offset_mismatch" => "La position du fragment envoyé est incohérente.",

        // Appairage / overlays.
        "pairing_code_invalid_or_expired" => "Ce code d'appairage est invalide ou expiré.",
        "pairing_code_scope_mismatch" =>
            "Ce code d'appairage ne peut pas être utilisé par ce client.",
        "overlay_client_conflict" => "Un autre overlay utilise déjà cette session. Réessayez.",
        "overlay_client_not_found" => "Client d'overlay introuvable.",
        "overlay_outdated" => "L'overlay doit être mis à jour.",
        "guild_mismatch" => "La guilde demandée ne correspond pas au jeton d'ingestion.",

        _ => return None,
    };
    Some(message)
}

/// Code machine déduit du statut pour une réponse sans corps structuré.
///
/// Le repli est [`REQUEST_ERROR`] (4xx) ou [`INTERNAL_ERROR`] (5xx) : les
/// statuts qui ont un code métier stable en reçoivent un, pour que le client
/// puisse réagir sans analyser le message.
fn code_for_status(status: StatusCode) -> &'static str {
    match status {
        StatusCode::REQUEST_TIMEOUT => "request_timeout",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "invalid_payload",
        StatusCode::TOO_MANY_REQUESTS => "too_many_requests",
        status if status.is_server_error() => INTERNAL_ERROR,
        _ => REQUEST_ERROR,
    }
}

/// Message générique de repli pour un code inconnu, selon le statut.
fn fallback_message(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "La requête est invalide.",
        StatusCode::UNAUTHORIZED => "Authentification requise.",
        StatusCode::FORBIDDEN => "Action non autorisée.",
        StatusCode::NOT_FOUND => "Ressource introuvable.",
        StatusCode::METHOD_NOT_ALLOWED => "Méthode HTTP non autorisée.",
        StatusCode::REQUEST_TIMEOUT => "Le serveur n'a pas répondu à temps.",
        StatusCode::CONFLICT => "La requête entre en conflit avec l'état actuel.",
        StatusCode::PAYLOAD_TOO_LARGE => "Le corps de la requête est trop volumineux.",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "Type de contenu non pris en charge.",
        StatusCode::UNPROCESSABLE_ENTITY => "Les données envoyées sont invalides.",
        StatusCode::TOO_MANY_REQUESTS => "Trop de requêtes. Réessayez dans un instant.",
        status if status.is_server_error() => {
            "Une erreur interne est survenue. Réessayez dans un instant."
        }
        _ => "La requête n'a pas pu être traitée.",
    }
}

/// Erreur applicative normalisée.
///
/// Construite avec un statut et un code machine ; le message par défaut vient de
/// [`message_for`]. Une route peut le préciser via [`ApiError::with_message`],
/// mais uniquement avec une phrase statique en français : jamais du texte
/// provenant d'une entrée, d'un outil ou d'une base.
#[derive(Clone, Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: Cow<'static, str>,
    details: Map<String, Value>,
}

impl ApiError {
    /// Erreur avec le message canonique du code (ou le repli du statut).
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        let message = message_for(code)
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Borrowed(fallback_message(status)));
        Self {
            status,
            code,
            message,
            details: Map::new(),
        }
    }

    /// Erreur interne générique (`500 internal_error`).
    pub fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR)
    }

    /// `401 unauthorized`.
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }

    /// `400 invalid_payload`.
    pub fn invalid_payload() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_payload")
    }

    /// Remplace le message par une phrase humaine statique.
    ///
    /// Le message doit être une constante de la route (ou un littéral), jamais
    /// une donnée d'entrée ni un `Display` d'erreur technique.
    pub fn with_message(mut self, message: &'static str) -> Self {
        self.message = Cow::Borrowed(message);
        self
    }

    /// Variante possédée, réservée aux validateurs typés qui construisent une
    /// phrase humaine (bornes numériques, choix invalides) et jamais au texte
    /// d'une entrée, d'un outil ou d'une base.
    pub(crate) fn with_owned_message(mut self, message: String) -> Self {
        self.message = Cow::Owned(message);
        self
    }

    /// Ajoute un détail structuré, aplati dans le corps JSON.
    pub fn detail(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        if key != "error" && key != "message" {
            self.details.insert(key.to_owned(), value.into());
        }
        self
    }

    /// Statut HTTP de l'erreur.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Code machine de l'erreur.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Corps JSON complet.
    pub fn body(&self) -> Value {
        let mut body = Map::new();
        body.insert("error".to_owned(), Value::String(self.code.to_owned()));
        body.insert(
            "message".to_owned(),
            Value::String(self.message.as_ref().to_owned()),
        );
        for (key, value) in &self.details {
            body.insert(key.clone(), value.clone());
        }
        Value::Object(body)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.status.is_server_error() {
            tracing::error!(
                code = self.code,
                status = self.status.as_u16(),
                "réponse d'erreur serveur"
            );
        }
        (self.status, Json(self.body())).into_response()
    }
}

/// Construit une réponse JSON d'erreur normalisée.
pub fn json_error(status: StatusCode, code: &'static str) -> Response {
    ApiError::new(status, code).into_response()
}

/// Handler de repli : toute route inconnue répond 404 au format normalisé.
pub async fn fallback() -> Response {
    ApiError::new(StatusCode::NOT_FOUND, REQUEST_ERROR)
        .with_message("Ressource introuvable.")
        .into_response()
}

/// Middleware de normalisation des erreurs (voir la documentation du module).
pub async fn normalize_errors(request: Request, next: Next) -> Response {
    let method: Method = request.method().clone();
    // Chemin seul : la query string peut porter un secret (`?token=`), elle ne
    // doit jamais apparaître dans les journaux (AUD-COR-05).
    let path = request.uri().path().to_owned();
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
        return complete_json_error(response, status, &method, &path).await;
    }

    let code = if status.is_server_error() {
        tracing::error!(
            method = %method,
            path = %path,
            status = status.as_u16(),
            "requête en erreur serveur (réponse normalisée)"
        );
        code_for_status(status)
    } else {
        tracing::warn!(
            method = %method,
            path = %path,
            status = status.as_u16(),
            "requête invalide (réponse normalisée)"
        );
        code_for_status(status)
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
        tracing::debug!(path = %path, content_type = %content_type, "corps d'erreur brut remplacé");
    }
    normalized
}

/// Complète un corps JSON d'erreur qui aurait oublié son `message`.
///
/// Le corps est laissé strictement intact s'il porte déjà un `message` non vide
/// ou s'il ne s'agit pas d'un objet `{"error": "<chaîne>"}`. C'est le filet de
/// sécurité pour les routes qui répondent encore avec un `json!` local.
async fn complete_json_error(
    response: Response,
    status: StatusCode,
    method: &Method,
    path: &str,
) -> Response {
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_INSPECTED_BODY).await else {
        // Corps trop volumineux pour être inspecté : on le remplace, mais les
        // en-têtes de corrélation (x-request-id, retry-after, CORS…) restent.
        let mut normalized = json_error(status, code_for_status(status));
        for (name, value) in &parts.headers {
            if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
                normalized.headers_mut().insert(name.clone(), value.clone());
            }
        }
        return normalized;
    };
    let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(&bytes) else {
        return rebuild_json_error(status, parts.headers, bytes.to_vec());
    };

    let Some(Value::String(code)) = object.get("error").cloned() else {
        return rebuild_json_error(status, parts.headers, bytes.to_vec());
    };
    let has_message = object
        .get("message")
        .and_then(Value::as_str)
        .is_some_and(|message| !message.trim().is_empty());
    if has_message {
        return rebuild_json_error(status, parts.headers, bytes.to_vec());
    }

    let message = message_for(&code)
        .unwrap_or_else(|| fallback_message(status))
        .to_owned();
    object.insert("message".to_owned(), Value::String(message));
    tracing::debug!(
        method = %method,
        path = %path,
        code = %code,
        status = status.as_u16(),
        "message d'erreur JSON complété par le contrat"
    );
    let encoded = serde_json::to_vec(&Value::Object(object)).unwrap_or_default();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    rebuild_json_error(parts.status, parts.headers, encoded)
}

/// Réémet un corps JSON déjà valide sans le modifier.
fn rebuild_json_error(
    status: StatusCode,
    mut headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
) -> Response {
    headers.remove(header::CONTENT_LENGTH);
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
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

    #[test]
    fn every_known_code_has_a_french_message() {
        for code in [
            INTERNAL_ERROR,
            REQUEST_ERROR,
            "media_ingestion_failed",
            "invalid_payload",
            "unauthorized",
            "overlay_client_not_found",
            "ingest_token_expired",
        ] {
            let message = message_for(code).unwrap_or_else(|| panic!("message absent pour {code}"));
            assert!(!message.trim().is_empty(), "{code}");
            assert!(!message.contains('\n'), "{code}");
        }
    }

    #[test]
    fn api_error_serializes_code_message_and_details() {
        let error = ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
            .detail("maxBytes", 1024_u64);
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            error.body(),
            serde_json::json!({
                "error": "payload_too_large",
                "message": "Le corps de la requête est trop volumineux.",
                "maxBytes": 1024,
            })
        );
    }

    #[test]
    fn unknown_code_falls_back_on_status_without_leaking() {
        let error = ApiError::new(StatusCode::CONFLICT, "custom_conflict");
        assert_eq!(
            error.body()["message"],
            "La requête entre en conflit avec l'état actuel."
        );
        assert_eq!(error.body()["error"], "custom_conflict");
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
        assert_eq!(body["error"], INTERNAL_ERROR);
        assert_eq!(
            body["message"],
            "Une erreur interne est survenue. Réessayez dans un instant."
        );
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
        assert_eq!(body["error"], REQUEST_ERROR);
        assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
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
        assert_eq!(body["error"], REQUEST_ERROR);
        assert_eq!(body["message"], "La requête n'a pas pu être traitée.");
    }

    #[tokio::test]
    async fn json_error_without_message_is_completed() {
        async fn legacy() -> Response {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid_payload" })),
            )
                .into_response()
        }

        let app = Router::new()
            .route("/legacy", get(legacy))
            .layer(middleware::from_fn(normalize_errors));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/legacy")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (status, body) = body_json(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_payload");
        assert_eq!(body["message"], "Les données envoyées sont invalides.");
    }

    #[tokio::test]
    async fn json_error_message_is_never_overwritten() {
        async fn precise() -> Response {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_payload",
                    "message": "Le corps de la requête est vide.",
                })),
            )
                .into_response()
        }

        let app = Router::new()
            .route("/precis", get(precise))
            .layer(middleware::from_fn(normalize_errors));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/precis")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (_, body) = body_json(response).await;
        assert_eq!(body["message"], "Le corps de la requête est vide.");
    }

    #[tokio::test]
    async fn oversized_json_error_is_replaced_but_keeps_its_headers() {
        async fn huge() -> Response {
            let body = format!(
                r#"{{"error":"internal_error","detail":"{}"}}"#,
                "x".repeat(MAX_INSPECTED_BODY)
            );
            let mut response = Response::new(Body::from(body));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/json"),
            );
            response.headers_mut().insert(
                axum::http::HeaderName::from_static("x-request-id"),
                axum::http::HeaderValue::from_static("req-42"),
            );
            response
        }

        let app = Router::new()
            .route("/huge", get(huge))
            .layer(middleware::from_fn(normalize_errors));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/huge")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("req-42")
        );
        let (status, body) = body_json(response).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], INTERNAL_ERROR);
        assert_eq!(
            body["message"],
            "Une erreur interne est survenue. Réessayez dans un instant."
        );
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
