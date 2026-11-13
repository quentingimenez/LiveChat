//! Configuration CORS.
//!
//! ## Décisions vs legacy
//!
//! Le legacy activait `credentials: true` avec une liste d'origines explicites
//! en production (`server/src/server.ts`, cf. `docs/rust/audit-dynamique.md`).
//! La cible v3 corrige le point S5 du plan de migration : **aucune credential
//! CORS** n'est nécessaire, le client overlay n'utilisant ni cookie ni
//! `Authorization` cross-origin. Le prédicat reste néanmoins explicite (pas de
//! wildcard `*`) :
//!
//! - toujours autorisées : l'origine de `API_URL`, les origines de
//!   `ADDITIONAL_CORS_ORIGINS`, et les origines Tauri
//!   `http(s)://tauri.localhost` (le client desktop embarqué) ;
//! - en développement (`NODE_ENV=development`) uniquement : les origines
//!   loopback (`localhost`, `127.0.0.1`, `::1`) avec n'importe quel port, pour
//!   le serveur de dev Vite.
//!
//! Les méthodes et en-têtes autorisés reprennent la liste legacy, complétés par
//! `x-request-id` (corrélation) et par l'exposition des en-têtes de rate limit.

use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderName, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};
use url::{Host, Url};

use crate::config::Config;

/// Durée de cache des pré-vols CORS par le navigateur.
pub const PREFLIGHT_MAX_AGE: Duration = Duration::from_secs(600);

/// Construit la couche CORS à partir de la configuration validée.
pub fn layer(config: &Config) -> CorsLayer {
    let mut allowed: Vec<String> = Vec::with_capacity(config.additional_cors_origins.len() + 3);
    allowed.push(config.api_origin());
    allowed.extend(
        config
            .additional_cors_origins
            .iter()
            .map(|origin| origin.origin().ascii_serialization()),
    );
    // Origines du client Tauri (webview interne) : `tauri.localhost` en http
    // (Windows/Linux selon la config) ou https (selon `useHttpsScheme`).
    allowed.push("http://tauri.localhost".to_owned());
    allowed.push("https://tauri.localhost".to_owned());
    let allowed = Arc::new(allowed);
    let development = config.is_development();

    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin, _parts| {
            origin_is_allowed(origin, &allowed, development)
        }))
        .allow_methods([
            Method::GET,
            Method::PUT,
            Method::DELETE,
            Method::POST,
            Method::OPTIONS,
            Method::PATCH,
        ])
        .allow_headers([
            HeaderName::from_static("origin"),
            HeaderName::from_static("x-requested-with"),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("accept"),
            HeaderName::from_static("authorization"),
            HeaderName::from_static("forest-context-url"),
            HeaderName::from_static("set-cookie"),
            HeaderName::from_static("cookie"),
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("range"),
        ])
        .expose_headers([
            HeaderName::from_static("x-request-id"),
            HeaderName::from_static("x-ratelimit-limit"),
            HeaderName::from_static("x-ratelimit-remaining"),
            HeaderName::from_static("x-ratelimit-reset"),
            HeaderName::from_static("retry-after"),
        ])
        .max_age(PREFLIGHT_MAX_AGE)
}

/// Prédicat d'origine : liste explicite + loopback réservé au développement.
fn origin_is_allowed(origin: &HeaderValue, allowed: &[String], development: bool) -> bool {
    let Ok(raw) = origin.to_str() else {
        return false;
    };
    // « null » (iframe sandbox, fichier local) n'est jamais acceptable.
    if raw == "null" {
        return false;
    }
    let Ok(parsed) = Url::parse(raw) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    if development && is_loopback_origin(&parsed) {
        return true;
    }
    let normalized = parsed.origin().ascii_serialization();
    allowed.iter().any(|candidate| candidate == &normalized)
}

fn is_loopback_origin(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use axum::response::Response;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use super::*;

    fn config(values: &[(&str, &str)]) -> Config {
        Config::from_pairs(
            values
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        )
        .expect("configuration de test valide")
    }

    fn app(config: &Config) -> Router {
        Router::new()
            .route("/ping", get(|| async { "ok" }))
            .layer(layer(config))
    }

    async fn preflight(app: Router, origin: &str) -> Response {
        app.oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/ping")
                .header(header::ORIGIN, origin)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    fn allow_origin(response: &Response) -> Option<&str> {
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|value| value.to_str().ok())
    }

    #[tokio::test]
    async fn tauri_origins_are_always_allowed() {
        let config = config(&[("API_URL", "http://localhost:3000")]);
        for origin in ["http://tauri.localhost", "https://tauri.localhost"] {
            let response = preflight(app(&config), origin).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(allow_origin(&response), Some(origin));
        }
    }

    #[tokio::test]
    async fn api_url_origin_is_allowed() {
        let config = config(&[("API_URL", "https://livechat.example.com/app")]);
        let response = preflight(app(&config), "https://livechat.example.com").await;
        assert_eq!(
            allow_origin(&response),
            Some("https://livechat.example.com")
        );
    }

    #[tokio::test]
    async fn additional_origins_are_allowed() {
        let config = config(&[
            ("API_URL", "http://localhost:3000"),
            (
                "ADDITIONAL_CORS_ORIGINS",
                "https://overlay.example.com,http://127.0.0.1:5173",
            ),
        ]);
        let response = preflight(app(&config), "https://overlay.example.com").await;
        assert_eq!(allow_origin(&response), Some("https://overlay.example.com"));
    }

    #[tokio::test]
    async fn unknown_origin_is_rejected() {
        let config = config(&[("API_URL", "http://localhost:3000")]);
        let response = preflight(app(&config), "https://evil.example.com").await;
        assert_eq!(allow_origin(&response), None);
    }

    #[tokio::test]
    async fn null_origin_is_rejected() {
        let config = config(&[("API_URL", "http://localhost:3000")]);
        let response = preflight(app(&config), "null").await;
        assert_eq!(allow_origin(&response), None);
    }

    #[tokio::test]
    async fn loopback_is_allowed_only_in_development() {
        let development = config(&[
            ("API_URL", "http://localhost:3000"),
            ("NODE_ENV", "development"),
        ]);
        let response = preflight(app(&development), "http://127.0.0.1:5173").await;
        assert_eq!(
            allow_origin(&response),
            Some("http://127.0.0.1:5173"),
            "le serveur de dev Vite doit être accepté en développement"
        );

        let production = config(&[("API_URL", "http://localhost:3000")]);
        let response = preflight(app(&production), "http://127.0.0.1:5173").await;
        assert_eq!(
            allow_origin(&response),
            None,
            "le loopback ne doit pas être accepté en production"
        );
    }

    #[tokio::test]
    async fn credentials_are_never_sent() {
        let config = config(&[("API_URL", "http://localhost:3000")]);
        let response = preflight(app(&config), "http://tauri.localhost").await;
        assert!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .is_none(),
            "S5 : aucune credential CORS"
        );
    }

    #[test]
    fn loopback_detection_matches_legacy_list() {
        for url in [
            "http://localhost:5173",
            "http://127.0.0.1:3000",
            "http://[::1]:8080",
        ] {
            let parsed = Url::parse(url).unwrap();
            assert!(is_loopback_origin(&parsed), "{url} doit être loopback");
        }
        for url in ["https://livechat.example.com", "http://192.168.1.20:3000"] {
            let parsed = Url::parse(url).unwrap();
            assert!(
                !is_loopback_origin(&parsed),
                "{url} ne doit pas être loopback"
            );
        }
    }
}
