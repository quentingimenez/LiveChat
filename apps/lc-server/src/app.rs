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
//!    ne jamais logguer `Authorization` / `Cookie`, ni la query string : seul le
//!    chemin est journalisé, cf. AUD-COR-05) ;
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
use std::time::Duration;

use anyhow::Context as _;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, Extensions, HeaderMap, Method, StatusCode, Version};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use lc_db::{Db, DbConfig};
use lc_domain::{Clock, SystemClock};
use time::UtcDateTime;
use tower_http::compression::predicate::{DefaultPredicate, Predicate};
use tower_http::compression::CompressionLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::{Level, Span};

use crate::config::Config;
use crate::discord::guild_leave::{DisabledLeaver, DiscordGuildLeaver};
use crate::discord::profiles::{DisabledProfiles, DiscordProfileSource};
use crate::http;
use crate::http::admin::hub::{AdminHub, NoopAdminHub};
use crate::http::admin::{
    AdminEvents, AdminOAuth, BinaryProbe, DisabledOAuth, DiscordOAuth, SystemBinaryProbe,
};
use crate::http::error::ApiError;
use crate::http::health::{DiscordProbe, HealthProbe};
use crate::http::rate_limit::RateLimitState;
use crate::ingestions::IngestionRegistry;
use crate::rights::RightsStore;
use crate::runtime::Runtime;
use crate::shutdown::Shutdown;

/// Limite de taille du corps des requêtes (1 Mio, défaut Fastify du legacy).
pub const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Timeout par requête des routes non-streaming (AUD-COR-06).
///
/// Les routes suivantes en sont exclues car leur durée dépasse volontairement
/// un budget court de requête :
///
/// - `/ws` : WebSocket de longue durée (liveness bornée par le hub) ;
/// - `/overlay/media/` : streaming média `Range`/`206` ;
/// - `/overlay/media-board/media/` : réception d'un upload brut et aperçus ;
/// - `PUT /overlay/media-board/uploads/<id>/parts` : un fragment d'upload.
///
/// Les routes `/ingest` ne sont **pas** exemptées : elles reçoivent leur propre
/// budget, dérivé de `MEDIA_INGEST_TIMEOUT_MS` (voir [`TimeoutPolicy`]).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Marge ajoutée au budget média pour borner une requête `/ingest` complète.
///
/// `MEDIA_INGEST_TIMEOUT_MS` (240 s par défaut) borne le téléchargement et le
/// transcodage ; la marge couvre ce qui les entoure et n'a pas de budget
/// propre : résolution du lien court (8 s), validation de la source, cache
/// d'assets, quotas, création du job et notification de lecture.
///
/// Avant cette borne, `/ingest` n'était soumise à aucun timeout : une étape
/// lente ou bloquée (résolution DNS, hook de lecture) laissait la requête
/// ouverte indéfiniment, sans réponse et donc sans ligne « finished processing
/// request » dans les journaux.
pub const INGEST_TIMEOUT_GRACE: Duration = Duration::from_secs(60);

/// Portée d'une requête pour le choix du budget de temps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeoutScope {
    /// Route JSON ordinaire : [`REQUEST_TIMEOUT`].
    Request,
    /// Route d'ingestion synchrone : [`TimeoutPolicy::ingest`].
    Ingest,
}

impl TimeoutScope {
    /// Erreur renvoyée quand le budget de cette portée est dépassé.
    fn error(self) -> ApiError {
        match self {
            // `408` historique des routes JSON.
            Self::Request => ApiError::new(StatusCode::REQUEST_TIMEOUT, "request_timeout"),
            // `504` : le serveur a abandonné sa propre ingestion (média trop
            // lent, source muette), distinct d'un refus d'authentification.
            Self::Ingest => ApiError::new(StatusCode::GATEWAY_TIMEOUT, "ingest_timeout"),
        }
    }
}

/// Budgets de temps appliqués par [`enforce_timeout`].
#[derive(Clone, Copy, Debug)]
pub struct TimeoutPolicy {
    /// Budget des routes ordinaires, lecture du corps comprise.
    pub request: Duration,
    /// Budget d'une requête d'ingestion complète (`/ingest`, `/ingest/*`).
    pub ingest: Duration,
}

impl TimeoutPolicy {
    /// Budgets nominaux : [`REQUEST_TIMEOUT`] pour les routes ordinaires,
    /// `MEDIA_INGEST_TIMEOUT_MS` + [`INGEST_TIMEOUT_GRACE`] pour l'ingestion.
    pub fn from_config(config: &Config) -> Self {
        Self {
            request: REQUEST_TIMEOUT,
            ingest: Duration::from_millis(config.media_ingest_timeout_ms) + INGEST_TIMEOUT_GRACE,
        }
    }
}

/// Portée de timeout d'une requête ; `None` quand aucune borne ne s'applique.
///
/// Les exemptions restent resserrées au strict nécessaire (AUD-COR-06) : les
/// autres routes de l'API admin / board JSON / config / pairing / health
/// restent soumises à [`REQUEST_TIMEOUT`].
fn timeout_scope(method: &Method, path: &str) -> Option<TimeoutScope> {
    // WebSocket de longue durée.
    if path == "/ws" {
        return None;
    }
    // Streaming média signé (`Range`/`206`) et aperçus du board.
    if path.starts_with("/overlay/media/") || path.starts_with("/overlay/media-board/media/") {
        return None;
    }
    // A board upload part streams up to 90 MiB before answering. Opening and
    // completing an upload, or adding a link, keep the ordinary timeout.
    if method == Method::PUT
        && path
            .strip_prefix("/overlay/media-board/uploads/")
            .is_some_and(|rest| rest.ends_with("/parts"))
    {
        return None;
    }
    // Ingestion média synchrone : son propre budget, jamais l'infini. Le test
    // du segment évite de traiter `/ingestion-log` comme une ingestion.
    if path == "/ingest" || path.starts_with("/ingest/") {
        return Some(TimeoutScope::Ingest);
    }
    Some(TimeoutScope::Request)
}

/// Middleware de timeout de requête (jamais appliqué aux routes exemptées).
///
/// Le timeout couvre aussi la lecture du corps : les extracteurs s'exécutent
/// dans le futur du handler, donc un client lent (slowloris applicatif) est
/// interrompu au bout du budget de sa portée.
async fn enforce_timeout(
    State(policy): State<TimeoutPolicy>,
    request: Request,
    next: Next,
) -> Response {
    let Some(scope) = timeout_scope(request.method(), request.uri().path()) else {
        return next.run(request).await;
    };
    let budget = match scope {
        TimeoutScope::Request => policy.request,
        TimeoutScope::Ingest => policy.ingest,
    };
    match tokio::time::timeout(budget, next.run(request)).await {
        Ok(response) => response,
        Err(_) => scope.error().into_response(),
    }
}

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
    pub started_at: UtcDateTime,
    /// Signal d'arrêt partagé avec les workers et le serveur HTTP.
    pub shutdown: Shutdown,
    /// Sonde d'état Discord (remplacée en Phase 6 par l'état réel du bot).
    pub discord: Arc<dyn HealthProbe>,
    /// Hub WebSocket exposé aux handlers admin (AUD-COR-14).
    pub admin_hub: Arc<dyn AdminHub>,
    /// Client capable de faire quitter le bot d'un serveur Discord (panneau
    /// admin). `DisabledLeaver` par défaut quand Discord n'est pas configuré.
    pub discord_leaver: Arc<dyn DiscordGuildLeaver>,
    /// Reads the Discord profile of linked members. `DisabledProfiles` when
    /// Discord is not configured.
    pub discord_profiles: Arc<dyn DiscordProfileSource>,
    /// Discord OAuth2 client used by the admin login (`DisabledOAuth` when the
    /// Discord login is not configured; injectable in tests).
    pub admin_oauth: Arc<dyn AdminOAuth>,
    /// Registre partagé des ingestions média asynchrones.
    pub ingestions: Arc<IngestionRegistry>,
    /// Board uploads being received in parts.
    pub uploads: Arc<crate::http::overlay::UploadSessions>,
    /// Configurable rights cache (`role_rights`), reloaded after `PUT
    /// /admin/api/rights`.
    pub rights: RightsStore,
    /// Serializes the read-check-write of `PATCH /overlay/admin/settings`.
    /// The settings version makes a stale save detectable, but only a lock
    /// makes the check and the write atomic when two boards save at once.
    pub settings_write: Arc<tokio::sync::Mutex<()>>,
    /// Version probe used by the admin health route. Tests inject a fake so no
    /// real binary is ever spawned.
    pub(crate) binary_probe: Arc<dyn BinaryProbe>,
    /// Hot settings loaded from `admin_runtime.json` and updated by the admin
    /// `PATCH /admin/api/runtime-settings`.
    runtime: Arc<Runtime>,
    /// Broadcast channel of the live admin event stream.
    pub events: AdminEvents,
}

impl AppState {
    /// Construit l'état nominal : horloge système et sonde Discord déduite de la
    /// configuration.
    pub fn new(config: Arc<Config>, db: Option<Db>, shutdown: Shutdown) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let discord: Arc<dyn HealthProbe> = Arc::new(DiscordProbe::from_config(&config));
        let admin_oauth: Arc<dyn AdminOAuth> = if config.admin_discord_login_enabled() {
            Arc::new(DiscordOAuth::new(
                config
                    .discord_client_id
                    .as_deref()
                    .unwrap_or_default()
                    .trim(),
                config.discord_client_secret.clone().unwrap_or_default(),
            ))
        } else {
            Arc::new(DisabledOAuth)
        };
        let started_at = clock.now();
        let runtime = Arc::new(Runtime::new(Arc::clone(&config)));
        let events = AdminEvents::new();
        Self {
            config,
            db,
            clock,
            started_at,
            shutdown,
            discord,
            admin_hub: Arc::new(NoopAdminHub),
            discord_leaver: Arc::new(DisabledLeaver),
            discord_profiles: Arc::new(DisabledProfiles),
            admin_oauth,
            ingestions: Arc::new(IngestionRegistry::with_events(events.clone())),
            uploads: Arc::default(),
            rights: RightsStore::new(),
            settings_write: Arc::new(tokio::sync::Mutex::new(())),
            binary_probe: Arc::new(SystemBinaryProbe::new()),
            runtime,
            events,
        }
    }

    /// Hot settings shared by the handlers (`state.runtime().default_duration()`).
    #[must_use]
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
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

    /// Injecte le hub WebSocket réel (remplace le `NoopAdminHub` par défaut).
    #[must_use]
    pub fn with_admin_hub(mut self, hub: Arc<dyn AdminHub>) -> Self {
        self.admin_hub = hub;
        self
    }

    /// Remplace le client de départ Discord (Phase 7c ou tests).
    #[must_use]
    pub fn with_discord_leaver(mut self, leaver: Arc<dyn DiscordGuildLeaver>) -> Self {
        self.discord_leaver = leaver;
        self
    }

    /// Replaces the Discord profile reader (real client or tests).
    #[must_use]
    pub fn with_discord_profiles(mut self, profiles: Arc<dyn DiscordProfileSource>) -> Self {
        self.discord_profiles = profiles;
        self
    }

    /// Replaces the Discord OAuth2 client used by the admin login (tests).
    #[must_use]
    pub fn with_admin_oauth(mut self, oauth: Arc<dyn AdminOAuth>) -> Self {
        self.admin_oauth = oauth;
        self
    }

    /// Replaces the configurable rights cache (tests or a preloaded store).
    #[must_use]
    pub fn with_rights_store(mut self, rights: RightsStore) -> Self {
        self.rights = rights;
        self
    }

    /// Replaces the binary version probe used by the admin health route (tests).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_binary_probe(mut self, probe: Arc<dyn BinaryProbe>) -> Self {
        self.binary_probe = probe;
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

/// Prédicat de compression : jamais de média binaire ni de `206 Partial
/// Content`.
///
/// Le [[CompressionLayer]] par défaut (`DefaultPredicate`) n'exclut pas
/// `video/*`/`audio/*` : sans ce garde-fou, un `GET /overlay/media/{id}` 200
/// complet serait gzippé, supprimant `Content-Length`/`Accept-Ranges` et cassant
/// le streaming côté WebView (cf. docs/rust/server-overlay.md §6.4). La couche
/// reste appliquée aux réponses JSON/HTML.
fn compression_predicate(
    status: StatusCode,
    _version: Version,
    headers: &HeaderMap,
    _extensions: &Extensions,
) -> bool {
    if status == StatusCode::PARTIAL_CONTENT {
        return false;
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase());
    match content_type {
        Some(value) => {
            // `text/event-stream` must never be compressed: gzip buffers the
            // fragments and breaks the live admin stream.
            !(value.starts_with("text/event-stream")
                || value.starts_with("video/")
                || value.starts_with("audio/")
                || value.starts_with("application/octet-stream")
                || value.starts_with("multipart/"))
        }
        None => true,
    }
}

/// Construit le routeur complet de l'application.
///
/// Le routeur est rendu avec son état appliqué ; il peut être servi par
/// `axum::serve` ou testé directement avec `tower::ServiceExt::oneshot`.
///
/// Le hub WebSocket est créé localement : pour le partager avec le câblage
/// Discord, le hook média et l'admin (`install_admin_hub`), utiliser
/// [`router_with_hub`].
pub fn router(state: AppState) -> Router {
    router_with_hub(state, Arc::new(crate::ws::Hub::new()))
}

/// Construit le routeur complet autour d'un hub WebSocket existant.
///
/// Utilisé par `serve()` : le même [`crate::ws::Hub`] alimente le routeur
/// (`/ws`), l'adaptateur `AdminHub` et le hook d'ingestion média.
pub fn router_with_hub(state: AppState, hub: Arc<crate::ws::Hub>) -> Router {
    let config = Arc::clone(&state.config);
    if config.trusted_proxies.is_empty() {
        // Fail closed: the flags alone no longer make the headers trusted.
        if config.trust_proxy_headers {
            tracing::warn!(
                "TRUST_PROXY_HEADERS=true sans TRUSTED_PROXIES : en-têtes de proxy ignorés"
            );
        }
        if config.admin_trust_forwarded_for {
            tracing::warn!(
                "ADMIN_TRUST_FORWARDED_FOR=true sans TRUSTED_PROXIES : en-têtes de proxy ignorés"
            );
        }
    }
    let rate_limit = RateLimitState::from_config(&config);
    let cors = http::cors::layer(&config);
    // The WebSocket hub signals overlay connections and board/queue changes to
    // the admin SSE stream, and reads the hot default duration.
    hub.install_admin_events(state.events.clone());
    hub.install_runtime(Arc::clone(&state.runtime));
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
        .merge(crate::ws::router_with_hub(hub))
        .fallback(http::error::fallback)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            rate_limit,
            http::rate_limit::enforce,
        ))
        .layer(middleware::from_fn_with_state(
            TimeoutPolicy::from_config(&config),
            enforce_timeout,
        ))
        .layer(middleware::from_fn(http::error::normalize_errors))
        .layer(
            CompressionLayer::new()
                .compress_when(DefaultPredicate::new().and(compression_predicate)),
        )
        .layer(cors)
        .layer(middleware::from_fn(http::security::security_headers))
        .layer(trace)
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        // The admin audit middleware reads the shared state from the request
        // extensions (it cannot take `State` because the admin router is built
        // before the application state is applied).
        .layer(axum::Extension(state.clone()))
        .with_state(state)
}

/// Span de journalisation d'une requête HTTP, corrélé par `x-request-id`.
///
/// Les en-têtes ne sont volontairement pas inclus (`include_headers = false`) :
/// journaliser tous les en-têtes exposerait `Authorization`/`Cookie`.
///
/// Seul le **chemin** est journalisé : `request.uri()` contiendrait la query
/// string (`?token=...`, codes d'appairage…) qui finirait en clair dans les
/// journaux (AUD-COR-05).
fn make_span(request: &axum::http::Request<axum::body::Body>) -> Span {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-");
    tracing::info_span!(
        "http_request",
        method = %request.method(),
        path = %request_path(request.uri()),
        request_id = %request_id
    )
}

/// Chemin seul d'une URI (sans query string ni fragment), pour journalisation.
fn request_path(uri: &axum::http::Uri) -> &str {
    uri.path()
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
    use lc_domain::FixedClock;
    use serde_json::Value;
    use time::{Duration, UtcDateTime};
    use tower::ServiceExt;

    use super::*;
    use crate::http::rate_limit::{
        AUTH_RATE_LIMIT_MAX, LEGACY_RATE_LIMIT_MAX, MEDIA_RATE_LIMIT_MAX, RATE_LIMIT_MAX,
    };

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
        assert_eq!(
            body,
            serde_json::json!({ "error": "request_error", "message": "Ressource introuvable." })
        );
    }

    #[tokio::test]
    async fn method_not_allowed_is_normalized() {
        let app = router(state());
        let (status, _, body) = send(&app, request(Method::POST, "/")).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            body,
            serde_json::json!({
                "error": "request_error",
                "message": "La requête n'a pas pu être traitée.",
            })
        );
    }

    #[tokio::test]
    async fn health_without_dependencies_starts_and_reports_disabled() {
        let app = router(state());
        let (status, _, body) = send(&app, request(Method::GET, "/health")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["db"], "disabled");
        assert_eq!(body["discord"], "disabled");
        assert_eq!(body["engine"], "disabled");
        assert!(body.get("engineFailures").is_none());
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
        let fixed = Arc::new(FixedClock::new(UtcDateTime::now()));
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
                    "message": "L'overlay doit être mis à jour.",
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
    async fn rate_limit_applies_the_strict_budget_to_admin_logins_only() {
        let app = router(state());
        let client = [203, 0, 113, 10];
        // The secret login checks a guessable secret: strict budget.
        for index in 0..AUTH_RATE_LIMIT_MAX {
            let (status, _, _) = send(
                &app,
                from_client(request(Method::POST, "/admin/auth/secret"), client),
            )
            .await;
            assert_ne!(
                status,
                StatusCode::TOO_MANY_REQUESTS,
                "request {index} accepted"
            );
        }
        let (status, headers, body) = send(
            &app,
            from_client(request(Method::POST, "/admin/auth/secret"), client),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body,
            serde_json::json!({
                "error": "too_many_requests",
                "message": "Trop de requêtes. Réessayez dans un instant.",
            })
        );
        assert_eq!(
            *headers.get("x-ratelimit-limit").unwrap(),
            AUTH_RATE_LIMIT_MAX.to_string()
        );
        assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "0");
        assert!(headers.get(header::RETRY_AFTER).is_some());

        // The Discord login and the pairing codes share that budget.
        for path in ["/admin/auth/discord", "/admin/auth/discord/callback"] {
            let (status, _, _) = send(&app, from_client(request(Method::GET, path), client)).await;
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{path}");
        }
        let (status, _, _) = send(
            &app,
            from_client(request(Method::POST, "/overlay/pair/consume"), client),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

        // The console behind the login (page, API, session probe) and the
        // overlay API keep the general budget.
        for path in [
            "/admin/",
            "/admin/api/overview",
            "/admin/auth/session",
            "/health",
        ] {
            let (status, headers, _) =
                send(&app, from_client(request(Method::GET, path), client)).await;
            assert_ne!(status, StatusCode::TOO_MANY_REQUESTS, "{path}");
            assert_eq!(
                *headers.get("x-ratelimit-limit").unwrap(),
                RATE_LIMIT_MAX.to_string(),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn spoofed_proxy_headers_do_not_reset_the_rate_limit() {
        // Trust is enabled but no proxy is declared: the headers must be
        // ignored, so rotating `X-Real-IP` cannot buy a fresh budget.
        let app = router(state_with(&[
            ("API_URL", "http://localhost:3000"),
            ("TRUST_PROXY_HEADERS", "true"),
        ]));
        let client = [203, 0, 113, 60];
        for index in 0..AUTH_RATE_LIMIT_MAX {
            let mut request = from_client(request(Method::POST, "/admin/auth/secret"), client);
            request.headers_mut().insert(
                "x-real-ip",
                header::HeaderValue::from_str(&format!("198.51.100.{index}"))
                    .expect("adresse de test valide"),
            );
            let (status, _, _) = send(&app, request).await;
            assert_ne!(
                status,
                StatusCode::TOO_MANY_REQUESTS,
                "request {index} accepted"
            );
        }

        let mut forged = from_client(request(Method::POST, "/admin/auth/secret"), client);
        forged.headers_mut().insert(
            "x-real-ip",
            header::HeaderValue::from_static("198.51.100.250"),
        );
        let (status, _, _) = send(&app, forged).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn declared_trusted_proxy_headers_are_used_for_the_rate_limit() {
        let app = router(state_with(&[
            ("API_URL", "http://localhost:3000"),
            ("TRUST_PROXY_HEADERS", "true"),
            ("TRUSTED_PROXIES", "127.0.0.1"),
        ]));
        // The proxy is local; each request announces a different client, so no
        // budget is consumed twice.
        for index in 0..=AUTH_RATE_LIMIT_MAX {
            let mut request =
                from_client(request(Method::POST, "/admin/auth/secret"), [127, 0, 0, 1]);
            request.headers_mut().insert(
                "x-real-ip",
                header::HeaderValue::from_str(&format!("198.51.100.{index}"))
                    .expect("adresse de test valide"),
            );
            let (status, _, _) = send(&app, request).await;
            assert_ne!(status, StatusCode::TOO_MANY_REQUESTS, "request {index}");
        }
    }

    #[tokio::test]
    async fn rate_limit_lets_a_console_visit_load_more_than_the_auth_budget() {
        let app = router(state());
        let client = [203, 0, 113, 11];
        // One visit loads the page, its assets and many API calls; twice the
        // strict budget must still go through.
        for index in 0..AUTH_RATE_LIMIT_MAX * 2 {
            let path = if index % 2 == 0 {
                "/admin/api/overview"
            } else {
                "/admin/assets/index.css"
            };
            let (status, _, _) = send(&app, from_client(request(Method::GET, path), client)).await;
            assert_ne!(
                status,
                StatusCode::TOO_MANY_REQUESTS,
                "request {index} {path}"
            );
        }
    }

    #[tokio::test]
    async fn rate_limit_caps_the_admin_console_page_with_the_general_budget() {
        let app = router(state());
        let client = [203, 0, 113, 12];
        // The page, its assets and the JSON API share the general budget: the
        // console redirect itself is already counted.
        let (status, _, _) = send(&app, from_client(request(Method::GET, "/admin"), client)).await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
        for index in 1..RATE_LIMIT_MAX {
            let (status, _, _) =
                send(&app, from_client(request(Method::GET, "/admin/"), client)).await;
            assert_ne!(
                status,
                StatusCode::TOO_MANY_REQUESTS,
                "requête {index} acceptée"
            );
        }

        // The next request is refused with the general budget, not the strict
        // login budget.
        let (status, headers, body) =
            send(&app, from_client(request(Method::GET, "/admin/"), client)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            body,
            serde_json::json!({
                "error": "too_many_requests",
                "message": "Trop de requêtes. Réessayez dans un instant.",
            })
        );
        assert_eq!(
            *headers.get("x-ratelimit-limit").unwrap(),
            RATE_LIMIT_MAX.to_string()
        );
        assert_eq!(headers.get("x-ratelimit-remaining").unwrap(), "0");
        assert!(headers.get(header::RETRY_AFTER).is_some());
    }

    #[tokio::test]
    async fn rate_limit_compte_les_sondes_socket_io_a_part() {
        let app = router(state());
        let client = [203, 0, 113, 30];
        // A 2.x overlay polling `/socket.io/` on the same IP...
        for _ in 0..LEGACY_RATE_LIMIT_MAX {
            send(
                &app,
                from_client(request(Method::GET, "/socket.io/"), client),
            )
            .await;
        }
        let (status, _, _) = send(
            &app,
            from_client(request(Method::GET, "/socket.io/"), client),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // ...does not use up the budget of a 3.x overlay.
        let (status, _, _) = send(&app, from_client(request(Method::GET, "/health"), client)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn rate_limit_compte_les_medias_a_part() {
        let app = router(state());
        let client = [203, 0, 113, 20];

        // Le budget général est épuisé par les appels JSON.
        for index in 0..RATE_LIMIT_MAX {
            let (status, _, _) =
                send(&app, from_client(request(Method::GET, "/health"), client)).await;
            assert_eq!(status, StatusCode::OK, "requête {index} acceptée");
        }
        let (status, _, _) = send(&app, from_client(request(Method::GET, "/health"), client)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

        // Un téléchargement média n'est pas concerné par ce budget : la requête
        // atteint le handler (403/404, URL non signée) avec le plafond média.
        let (status, headers, _) = send(
            &app,
            from_client(
                request(
                    Method::GET,
                    "/overlay/media/00000000-0000-4000-8000-000000000000",
                ),
                client,
            ),
        )
        .await;
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            *headers.get("x-ratelimit-limit").unwrap(),
            MEDIA_RATE_LIMIT_MAX.to_string()
        );
    }

    #[test]
    fn journalisation_ignore_la_query_string() {
        let uri: axum::http::Uri = "/admin/api/overview?token=SECRET"
            .parse()
            .expect("URI valide");
        assert_eq!(request_path(&uri), "/admin/api/overview");
    }

    #[tokio::test]
    async fn request_timeout_returns_408_but_exempts_streaming_routes() {
        async fn slow() -> &'static str {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            "ok"
        }
        let app = Router::new()
            .route("/slow", axum::routing::get(slow))
            .route("/admin/api/overview", axum::routing::get(slow))
            .route("/overlay/media/abc", axum::routing::get(slow))
            .route("/overlay/media-board/media/abc", axum::routing::get(slow))
            .route("/overlay/media-board/items", axum::routing::post(slow))
            .route("/overlay/media-board/uploads", axum::routing::post(slow))
            .route(
                "/overlay/media-board/uploads/abc/parts",
                axum::routing::put(slow),
            )
            .layer(middleware::from_fn_with_state(
                TimeoutPolicy {
                    request: std::time::Duration::from_millis(20),
                    ingest: std::time::Duration::from_millis(20),
                },
                enforce_timeout,
            ));

        let (status, _, body) = send(&app, request(Method::GET, "/slow")).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(
            body,
            serde_json::json!({
                "error": "request_timeout",
                "message": "Le serveur n'a pas répondu à temps.",
            })
        );

        // Les routes JSON ordinaires restent soumises au timeout, y compris
        // l'ajout d'un lien au board, qui répond `202` sans attendre.
        for (method, path) in [
            (Method::GET, "/admin/api/overview"),
            (Method::POST, "/overlay/media-board/items"),
            (Method::POST, "/overlay/media-board/uploads"),
        ] {
            let (status, _, _) = send(&app, request(method, path)).await;
            assert_eq!(status, StatusCode::REQUEST_TIMEOUT, "route {path}");
        }

        // Routes exemptées : le timeout ne s'applique pas.
        for (method, path) in [
            (Method::GET, "/overlay/media/abc"),
            (Method::GET, "/overlay/media-board/media/abc"),
            (Method::PUT, "/overlay/media-board/uploads/abc/parts"),
        ] {
            let (status, _, _) = send(&app, request(method, path)).await;
            assert_eq!(status, StatusCode::OK, "route exemptée {path}");
        }
    }

    #[tokio::test]
    async fn ingest_routes_keep_their_own_budget_instead_of_the_request_timeout() {
        async fn slow_ingest() -> &'static str {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            "ok"
        }
        let app = Router::new()
            .route("/ingest", axum::routing::post(slow_ingest))
            .route("/ingest/", axum::routing::post(slow_ingest))
            .route("/ingest/file", axum::routing::post(slow_ingest))
            .route("/ingest/whoami", axum::routing::get(slow_ingest))
            .route("/slow", axum::routing::get(slow_ingest))
            .layer(middleware::from_fn_with_state(
                TimeoutPolicy {
                    request: std::time::Duration::from_millis(20),
                    ingest: std::time::Duration::from_secs(5),
                },
                enforce_timeout,
            ));

        // Une ingestion dépasse le budget court des routes ordinaires mais
        // reste dans son propre budget : elle aboutit au lieu d'être coupée.
        for (method, path) in [
            (Method::POST, "/ingest"),
            (Method::POST, "/ingest/"),
            (Method::POST, "/ingest/file"),
            (Method::GET, "/ingest/whoami"),
        ] {
            let (status, _, _) = send(&app, request(method, path)).await;
            assert_eq!(status, StatusCode::OK, "route {path}");
        }

        // La même lenteur sur une route ordinaire reste coupée : le test ne
        // passe pas parce que le timeout aurait été désactivé partout.
        let (status, _, _) = send(&app, request(Method::GET, "/slow")).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    }

    #[tokio::test]
    async fn ingest_over_its_budget_returns_504_ingest_timeout() {
        async fn stuck() -> &'static str {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            "ok"
        }
        let app = Router::new()
            .route("/ingest", axum::routing::post(stuck))
            .layer(middleware::from_fn_with_state(
                TimeoutPolicy {
                    request: std::time::Duration::from_secs(5),
                    ingest: std::time::Duration::from_millis(20),
                },
                enforce_timeout,
            ));

        let (status, _, body) = send(&app, request(Method::POST, "/ingest")).await;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body["error"], "ingest_timeout");
        assert_eq!(
            body["message"],
            crate::http::error::message_for("ingest_timeout").expect("message du contrat")
        );
    }

    #[test]
    fn timeout_scope_routes_ingest_to_its_own_budget() {
        for path in ["/ingest", "/ingest/", "/ingest/file", "/ingest/whoami"] {
            assert_eq!(
                timeout_scope(&Method::POST, path),
                Some(TimeoutScope::Ingest),
                "route {path}"
            );
        }
        // Un chemin seulement préfixé par « ingest » n'est pas une ingestion.
        assert_eq!(
            timeout_scope(&Method::GET, "/ingestion-log"),
            Some(TimeoutScope::Request)
        );
        assert_eq!(timeout_scope(&Method::GET, "/ws"), None);
        assert_eq!(timeout_scope(&Method::GET, "/overlay/media/abc"), None);
        assert_eq!(
            timeout_scope(&Method::GET, "/overlay/media-board/media/abc"),
            None
        );
        assert_eq!(
            timeout_scope(&Method::PUT, "/overlay/media-board/uploads/abc/parts"),
            None
        );
        // Seul le fragment d'upload est exempté, pas la fin d'upload.
        assert_eq!(
            timeout_scope(&Method::POST, "/overlay/media-board/uploads/abc/parts"),
            Some(TimeoutScope::Request)
        );
        assert_eq!(
            timeout_scope(&Method::GET, "/admin/api/overview"),
            Some(TimeoutScope::Request)
        );
    }

    #[test]
    fn ingest_budget_adds_its_grace_to_the_media_timeout() {
        let policy = TimeoutPolicy::from_config(&config(&[
            ("API_URL", "http://localhost:3000"),
            ("MEDIA_INGEST_TIMEOUT_MS", "90000"),
        ]));
        assert_eq!(policy.request, REQUEST_TIMEOUT);
        assert_eq!(
            policy.ingest,
            std::time::Duration::from_millis(90_000) + INGEST_TIMEOUT_GRACE
        );

        // Valeur par défaut du serveur : 240 s de média + la marge.
        let policy = TimeoutPolicy::from_config(&state().config);
        assert_eq!(policy.ingest, std::time::Duration::from_secs(300));
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
        assert_eq!(
            body,
            serde_json::json!({
                "error": "payload_too_large",
                "message": "Le corps de la requête est trop volumineux.",
            })
        );
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
