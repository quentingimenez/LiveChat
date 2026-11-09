//! Assemblage de l'application axum et état partagé.
//!
//! ## Composition
//!
//! [`router`] assemble les sous-routeurs de la couche [`crate::http`] et
//! applique les middlewares transverses dans un ordre volontaire (de
//! l'extérieur vers l'intérieur) :
//!
//! 1. `SetRequestIdLayer` / `PropagateRequestIdLayer` : `x-request-id` généré ou
//!    repris, propagé au client ;
//! 2. `TraceLayer` : journalisation corrélée (sans journaliser les en-têtes, pour
//!    ne jamais logguer `Authorization` / `Cookie`) ;
//! 3. en-têtes de sécurité (CSP, `nosniff`, `frame-ancestors`…) ;
//! 4. CORS explicite ;
//! 5. compression gzip ;
//! 6. normalisation JSON des erreurs ;
//! 7. rate limit (100 requêtes / 15 min par IP) ;
//! 8. limite de taille du corps (1 Mio, parité Fastify).
//!
//! ## Extension en Phase 3b/4
//!
//! Les lanes suivantes n'ont pas besoin de modifier `main.rs` :
//!
//! - routes métier : `Router::new().merge(http::overlay::router())` etc. dans
//!   [`router`], en réutilisant [`AppState`] ;
//! - WebSocket : `axum::extract::ws` sur `/ws` et `tokio-tungstenite` restent à
//!   ajouter dans une Phase 4 (voir `docs/rust/server-core.md`) ;
//! - dépendances externes : fournir une implémentation de
//!   [`crate::http::health::HealthProbe`] (Discord, adaptateur lecture…) via
//!   [`AppState::with_discord_probe`].
//!
//! ## Démarrage sans dépendance obligatoire
//!
//! La base SQLite et Discord sont facultatifs à la construction de l'état : le
//! serveur démarre toujours (exigence d'audit), `/health` publiant l'état réel.
//! Si `DATABASE_URL` est fournie mais que l'ouverture échoue, [`open_database`]
//! renvoie l'erreur et `run()` termine en erreur (code de sortie 1) : mieux vaut
//! un échec explicite qu'un serveur qui perd silencieusement les données.

use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::Router;
use chrono::{DateTime, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{Clock, SystemClock};
use tower_http::compression::CompressionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::{Level, Span};

use crate::config::Config;
use crate::http;
use crate::http::health::{DiscordProbe, HealthProbe};
use crate::http::rate_limit::RateLimitState;
use crate::shutdown::Shutdown;

/// Limite de taille du corps des requêtes (1 Mio, défaut Fastify du legacy).
pub const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// État partagé par tous les handlers (cloneable, sans intérieur mutable
/// directement accessible).
#[derive(Clone)]
pub struct AppState {
    /// Configuration validée du serveur.
    pub config: Arc<Config>,
    /// Base SQLite ouverte, ou `None` si `DATABASE_URL` est absente.
    pub db: Option<Db>,
    /// Horloge injectable (les tests utilisent [`lc_domain::FixedClock`]).
    pub clock: Arc<dyn Clock>,
    /// Instant du démarrage (au format horloge), base de `uptimeSec`.
    pub started_at: DateTime<Utc>,
    /// Signal d'arrêt partagé avec les workers et le serveur HTTP.
    pub shutdown: Shutdown,
    /// Sonde d'état Discord (remplacée en Phase 6 par l'état réel du bot).
    pub discord: Arc<dyn HealthProbe>,
}

impl AppState {
    /// Construit l'état nominal : horloge système et sonde Discord déduite de la
    /// configuration.
    pub fn new(config: Arc<Config>, db: Option<Db>, shutdown: Shutdown) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let discord: Arc<dyn HealthProbe> = Arc::new(DiscordProbe::from_config(&config));
        let started_at = clock.now();
        Self {
            config,
            db,
            clock,
            started_at,
            shutdown,
            discord,
        }
    }

    /// Remplace l'horloge (tests déterministes ; recale `started_at`).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.started_at = clock.now();
        self.clock = clock;
        self
    }

    /// Remplace la sonde Discord (Phase 6 ou tests).
    pub fn with_discord_probe(mut self, probe: Arc<dyn HealthProbe>) -> Self {
        self.discord = probe;
        self
    }
}

/// Ouvre la base SQLite si `DATABASE_URL` est configurée.
///
/// Absence de variable = base désactivée (pas une erreur). Variable présente
/// mais ouverture en échec = erreur fatale, remontée à `run()`.
pub async fn open_database(config: &Config) -> anyhow::Result<Option<Db>> {
    let Some(url) = config.database_url.as_deref() else {
        tracing::info!("DATABASE_URL absente : base de données désactivée (/health db=disabled)");
        return Ok(None);
    };
    let db = Db::connect(url, &DbConfig::default())
        .await
        .with_context(|| format!("ouverture de la base SQLite impossible ({url})"))?;
    tracing::info!(
        url = %url,
        schema_version = lc_db::SCHEMA_VERSION,
        "base SQLite ouverte"
    );
    Ok(Some(db))
}

/// Construit le routeur complet de l'application.
///
/// Le routeur est rendu avec son état appliqué ; il peut être servi par
/// `axum::serve` ou testé directement avec `tower::ServiceExt::oneshot`.
pub fn router(state: AppState) -> Router {
    let config = Arc::clone(&state.config);
    let rate_limit = RateLimitState::from_config(&config);
    let cors = http::cors::layer(&config);
    let trace = TraceLayer::new_for_http()
        .make_span_with(make_span)
        .on_request(DefaultOnRequest::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO))
        .on_failure(DefaultOnFailure::new().level(Level::ERROR));

    Router::new()
        .merge(http::root::router())
        .merge(http::health::router())
        .merge(http::socket_io::router())
        .merge(http::overlay::router())
        .merge(http::ingest::router())
        .merge(http::admin::router())
        .merge(crate::ws::router())
        .fallback(http::error::fallback)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            rate_limit,
            http::rate_limit::enforce,
        ))
        .layer(middleware::from_fn(http::error::normalize_errors))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(middleware::from_fn(http::security::security_headers))
        .layer(trace)
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .with_state(state)
}

/// Span de journalisation d'une requête HTTP, corrélé par `x-request-id`.
///
/// Les en-têtes ne sont volontairement pas inclus (`include_headers = false`) :
/// journaliser tous les en-têtes exposerait `Authorization`/`Cookie`.
fn make_span(request: &axum::http::Request<axum::body::Body>) -> Span {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-");
    tracing::info_span!(
        "http_request",
        method = %request.method(),
        uri = %request.uri(),
        request_id = %request_id
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::net::SocketAddr;

    use axum::body::{to_bytes, Body};
    use axum::extract::ConnectInfo;
    use axum::http::{header, HeaderMap, Method, Request as HttpRequest, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use chrono::{Duration, Utc};
    use lc_domain::FixedClock;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::http::rate_limit::RATE_LIMIT_MAX;

    fn config(values: &[(&str, &str)]) -> Config {
        Config::from_pairs(
            values
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        )
        .expect("configuration de test valide")
    }

    fn state_with(values: &[(&str, &str)]) -> AppState {
        AppState::new(Arc::new(config(values)), None, Shutdown::detached())
    }

    fn state() -> AppState {
        state_with(&[("API_URL", "http://localhost:3000")])
    }

    fn request(method: Method, path: &str) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .expect("requête de test valide")
    }

    fn from_client(mut request: HttpRequest<Body>, address: [u8; 4]) -> HttpRequest<Body> {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((address, 40_000))));
        request
    }

    async fn send(app: &Router, request: HttpRequest<Body>) -> (StatusCode, HeaderMap, Value) {
        let response = app.clone().oneshot(request).await.expect("réponse axum");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("corps lisible");
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, body)
    }

    #[tokio::test]
    async fn root_returns_legacy_service_identity() {
        let app = router(state());
        let (status, _, body) = send(&app, request(Method::GET, "/")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!({ "service": "livechat-server", "status": "ok" })
        );
    }

    #[tokio::test]
    async fn unknown_route_returns_normalized_404() {
        let app = router(state());
        let (status, _, body) = send(&app, request(Method::GET, "/inconnu")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, serde_json::json!({ "error": "request_error" }));
    }

    #[tokio::test]
    async fn method_not_allowed_is_normalized() {
        let app = router(state());
        let (status, _, body) = send(&app, request(Method::POST, "/")).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body, serde_json::json!({ "error": "request_error" }));
    }

    #[tokio::test]
    async fn health_without_dependencies_starts_and_reports_disabled() {
        let app = router(state());
        let (status, _, body) = send(&app, request(Method::GET, "/health")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["db"], "disabled");
        assert_eq!(body["discord"], "disabled");
        assert_eq!(body["protocolVersion"], lc_protocol::PROTOCOL_VERSION);
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        assert!(body["uptimeSec"].as_i64().unwrap_or(-1) >= 0);
        assert!(body["activeJobs"].is_null());
    }

    #[tokio::test]
    async fn health_reports_database_ok_and_active_jobs() {
        let db = Db::open_in_memory(&DbConfig::default())
            .await
            .expect("base mémoire");
        let state = AppState::new(
            Arc::new(config(&[("API_URL", "http://localhost:3000")])),
            Some(db),
            Shutdown::detached(),
        );
        let app = router(state);
        let (status, _, body) = send(&app, request(Method::GET, "/health")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["db"], "ok");
        assert_eq!(body["activeJobs"], 0);
    }

    #[tokio::test]
    async fn health_degrades_but_stays_http_200_when_discord_configured() {
        let state = state_with(&[
            ("API_URL", "http://localhost:3000"),
            ("DISCORD_TOKEN", "token"),
            ("DISCORD_CLIENT_ID", "123"),
        ]);
        let app = router(state);
        let (status, _, body) = send(&app, request(Method::GET, "/health")).await;
        assert_eq!(status, StatusCode::OK, "un état dégradé reste HTTP 200");
        assert_eq!(body["status"], "degraded");
        assert_eq!(body["discord"], "down");
        assert_eq!(body["db"], "disabled");
    }

    #[tokio::test]
    async fn health_uptime_follows_injected_clock() {
        let fixed = Arc::new(FixedClock::new(Utc::now()));
        let app = router(state().with_clock(fixed.clone()));

        let (_, _, body) = send(&app, request(Method::GET, "/health")).await;
        assert_eq!(body["uptimeSec"], 0);

        fixed.advance(Duration::seconds(5));
        let (_, _, body) = send(&app, request(Method::GET, "/health")).await;
        assert_eq!(body["uptimeSec"], 5);
    }

    #[tokio::test]
    async fn socket_io_returns_426_with_exact_contract() {
        let app = router(state());
        for path in [
            "/socket.io",
            "/socket.io/",
            "/socket.io/?EIO=4&transport=polling",
        ] {
            let (status, _, body) = send(&app, request(Method::GET, path)).await;
            assert_eq!(status, StatusCode::UPGRADE_REQUIRED, "route {path}");
            assert_eq!(
                body,
                serde_json::json!({
                    "error": "overlay_outdated",
                    "protocolVersion": 2,
                    "minOverlayVersion": "3.0.0",
                }),
                "route {path}"
            );
        }
        // Le legacy utilise aussi POST pour le transport polling.
        let (status, _, _) = send(&app, request(Method::POST, "/socket.io/")).await;
        assert_eq!(status, StatusCode::UPGRADE_REQUIRED);
    }

    #[tokio::test]
    async fn security_headers_are_present_on_success_and_error() {
        let app = router(state());
        for path in ["/", "/inconnu"] {
            let (_, headers, _) = send(&app, request(Method::GET, path)).await;
            assert!(
                headers.get("content-security-policy").is_some(),
                "CSP absente sur {path}"
            );
            assert_eq!(
                headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
                "nosniff"
            );
            assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
        }
    }

    #[tokio::test]
    async fn request_id_is_generated_and_propagated() {
        let app = router(state());

        let (_, headers, _) = send(&app, request(Method::GET, "/")).await;
        let generated = headers
            .get("x-request-id")
            .expect("x-request-id généré")
            .to_str()
            .unwrap()
            .to_owned();
        assert!(!generated.is_empty());

        let mut request = request(Method::GET, "/");
        request
            .headers_mut()
            .insert("x-request-id", "client-correlation-id".parse().unwrap());
        let (_, headers, _) = send(&app, request).await;
        assert_eq!(
            headers.get("x-request-id").unwrap(),
            "client-correlation-id"
        );
    }

    #[tokio::test]
    async fn cors_accepts_tauri_origin() {
        let app = router(state());
        let request = HttpRequest::builder()
            .method(Method::OPTIONS)
            .uri("/health")
            .header(header::ORIGIN, "http://tauri.localhost")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(Body::empty())
            .unwrap();
        let (status, headers, _) = send(&app, request).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "http://tauri.localhost"
        );
    }

    #[tokio::test]
    async fn rate_limit_blocks_after_100_requests_and_exempts_admin() {
        let app = router(state());
        for index in 0..RATE_LIMIT_MAX {
            let (status, _, _) = send(
                &app,
                from_client(request(Method::GET, "/health"), [203, 0, 113, 10]),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "requête {index} acceptée");
        }
        let (status, headers, body) = send(
            &app,
            from_client(request(Method::GET, "/health"), [203, 0, 113, 10]),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body, serde_json::json!({ "error": "request_error" }));
        assert_eq!(headers.get("x-ratelimit-limit").unwrap(), "100");
        assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "0");
        assert!(headers.get(header::RETRY_AFTER).is_some());

        // `/admin*` est exempté : la requête atteint le routeur (404), pas le 429.
        let (status, _, _) = send(
            &app,
            from_client(request(Method::GET, "/admin"), [203, 0, 113, 10]),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn development_mode_exempts_loopback_clients() {
        let app = router(state_with(&[
            ("API_URL", "http://localhost:3000"),
            ("NODE_ENV", "development"),
        ]));
        for _ in 0..(RATE_LIMIT_MAX + 5) {
            let (status, _, _) = send(
                &app,
                from_client(request(Method::GET, "/health"), [127, 0, 0, 1]),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn body_limit_rejects_oversized_payload_with_normalized_error() {
        async fn consume(_body: axum::body::Bytes) -> &'static str {
            "ok"
        }
        let app = Router::new()
            .route("/echo", post(consume))
            .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
            .layer(middleware::from_fn(http::error::normalize_errors));

        let payload = vec![0_u8; MAX_REQUEST_BODY_BYTES + 1];
        let request = HttpRequest::builder()
            .method(Method::POST)
            .uri("/echo")
            .body(Body::from(payload))
            .unwrap();
        let (status, _, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body, serde_json::json!({ "error": "request_error" }));
    }

    #[tokio::test]
    async fn open_database_is_optional() {
        let config = config(&[("API_URL", "http://localhost:3000")]);
        assert!(open_database(&config).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn open_database_connects_when_configured() {
        // `file::memory:` ne convient pas avec un pool : chaque connexion SQLite
        // aurait sa propre base (migrations invisibles pour les autres). On
        // utilise donc une base fichier temporaire, comme en production.
        let directory = tempfile::tempdir().expect("dossier temporaire");
        let database_url = format!("file:{}", directory.path().join("livechat.db").display());
        let values = [
            ("API_URL", "http://localhost:3000"),
            ("DATABASE_URL", database_url.as_str()),
        ];
        let config = config(&values);
        let db = open_database(&config)
            .await
            .expect("ouverture possible")
            .expect("base présente");
        db.close().await;
    }
}
