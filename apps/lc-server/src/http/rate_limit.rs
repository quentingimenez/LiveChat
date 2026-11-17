//! Limitation de débit par adresse IP.
//!
//! ## Choix d'implémentation
//!
//! Le legacy utilisait `@fastify/rate-limit` : 100 requêtes par fenêtre de
//! 15 minutes, cache de 10 000 entrées, exemption des URLs `/admin*` et, en
//! développement, des adresses loopback (`server/src/server.ts`, cf.
//! `docs/rust/audit-dynamique.md`). Le limiteur est ré-implémenté en mémoire
//! plutôt qu'avec `tower_governor` :
//!
//! - une fenêtre fixe suffit au contrat (le legacy n'utilisait pas non plus de
//!   fenêtre glissante) et rend les tests déterministes ;
//! - une seule exemption subsiste (loopback en développement) : l'exemption
//!   `/admin*` du legacy est **supprimée** (AUD-COR-10), le panneau est limité
//!   comme les autres routes et un bruteforce reçoit `429` ;
//! - aucune dépendance supplémentaire n'est ajoutée.
//!
//! Each request is counted in one of four budgets ([`Bucket`]), per client:
//!
//! - **auth** ([`AUTH_RATE_LIMIT_MAX`]): the routes that check a guessable
//!   secret: pairing-code consumption and the admin login routes
//!   (`/admin/auth/secret`, `/admin/auth/discord*`). This is the budget that
//!   stops a brute force, so it stays at the legacy value. The rest of the
//!   admin console (page, assets, `/admin/api/**`, the event stream, the
//!   session probe) is counted in the general budget: it needs a 256-bit
//!   session cookie, and one console visit loads several assets and API
//!   calls, which used up the auth budget in a few screens;
//! - **media** ([`MEDIA_RATE_LIMIT_MAX`]): `GET`/`HEAD` on the two media
//!   routes. The board grid loads one thumbnail per card (and several `Range`
//!   requests per video); the URLs are HMAC-signed, so a large budget does not
//!   help a brute force;
//! - **legacy** ([`LEGACY_RATE_LIMIT_MAX`]): `/socket.io*`, polled every few
//!   seconds by 2.x overlays that only ever get `426`. They must not use up
//!   the budget of a 3.x overlay on the same IP;
//! - **general** ([`RATE_LIMIT_MAX`]): everything else, i.e. the overlay API,
//!   the `/ws` upgrade, the extension ingestion and the admin console behind
//!   its login. These routes require a
//!   256-bit bearer token, so their budget is sized for normal board use: on
//!   2026-11-13 an admin testing the board (mute toggles, tabs, focus) sent
//!   102 requests in 15 minutes, hit the old shared limit of 100, and then
//!   could not reconnect the WebSocket because `/ws` shared that budget.
//!
//! L'adresse IP est lue depuis `ConnectInfo<SocketAddr>` (jamais fourni par le
//! client). Derrière un reverse proxy, `TRUST_PROXY_HEADERS=true` active la
//! lecture de `X-Real-IP` puis du dernier `X-Forwarded-For` (voir
//! [`super::proxy`] ; le premier est écrit par le client) — configuration explicite exigée par le
//! point S6 du plan de migration : le serveur ne fait jamais confiance à cet
//! en-tête par défaut.
//!
//! Les réponses portent `x-ratelimit-limit`, `x-ratelimit-remaining` et
//! `x-ratelimit-reset`, et les 429 un `Retry-After`, comme le faisait Fastify.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::Config;
use crate::http::error::{json_error, REQUEST_ERROR};

/// General budget per window and per client (overlay API, `/ws`, ingestion).
pub const RATE_LIMIT_MAX: u32 = 1000;

/// Budget of the routes that check a guessable secret (pairing codes, admin
/// panel), per window and per client.
pub const AUTH_RATE_LIMIT_MAX: u32 = 100;

/// Budget de téléchargements média (`GET`/`HEAD` sur les deux routes média) par
/// fenêtre et par client, compté à part de [`RATE_LIMIT_MAX`].
pub const MEDIA_RATE_LIMIT_MAX: u32 = 1000;

/// Budget of the legacy `/socket.io*` probes per window and per client.
pub const LEGACY_RATE_LIMIT_MAX: u32 = 100;

/// Durée de la fenêtre de limitation.
pub const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Clé de comptage : adresse connue, ou seau partagé quand elle est inconnue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ClientKey {
    Known(IpAddr),
    Unknown,
}

/// Fenêtre de comptage d'un client.
#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    count: u32,
}

/// État protégé du limiteur.
#[derive(Debug)]
struct Inner {
    windows: HashMap<ClientKey, Window>,
    last_cleanup: Instant,
}

/// Décision de limitation pour une requête.
#[derive(Debug, Clone, Copy)]
struct Decision {
    allowed: bool,
    limit: u32,
    remaining: u32,
    reset_after: Duration,
}

/// Limiteur de débit en mémoire, cloneable (état partagé).
#[derive(Debug, Clone)]
struct RateLimiter {
    inner: Arc<Mutex<Inner>>,
    max: u32,
    window: Duration,
}

impl RateLimiter {
    fn new(max: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                windows: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
            max,
            window,
        }
    }

    /// Verrou take-no-panic : un empoisonnement (panic d'un test antérieur) ne
    /// doit pas abattre le serveur ; l'état est simplement réutilisé.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn check_at(&self, key: ClientKey, now: Instant) -> Decision {
        let mut inner = self.lock();

        // Purge opportuniste des fenêtres expirées, au plus une fois par
        // fenêtre : la mémoire reste bornée même sous attaque distribuée.
        let needs_cleanup = now
            .checked_duration_since(inner.last_cleanup)
            .is_some_and(|elapsed| elapsed >= self.window);
        if needs_cleanup {
            inner.windows.retain(|_, window| {
                now.checked_duration_since(window.started)
                    .is_some_and(|elapsed| elapsed < self.window)
            });
            inner.last_cleanup = now;
        }

        let window = inner.windows.entry(key).or_insert(Window {
            started: now,
            count: 0,
        });
        let elapsed = now
            .checked_duration_since(window.started)
            .unwrap_or_default();
        if elapsed >= self.window {
            window.started = now;
            window.count = 0;
        }
        let elapsed = now
            .checked_duration_since(window.started)
            .unwrap_or_default();
        let reset_after = self.window.saturating_sub(elapsed);

        if window.count >= self.max {
            Decision {
                allowed: false,
                limit: self.max,
                remaining: 0,
                reset_after,
            }
        } else {
            window.count += 1;
            Decision {
                allowed: true,
                limit: self.max,
                remaining: self.max - window.count,
                reset_after,
            }
        }
    }

    /// Nombre de clients actuellement suivis (diagnostic/tests).
    #[cfg(test)]
    fn tracked_clients(&self) -> usize {
        self.lock().windows.len()
    }
}

/// Budget a request is counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    General,
    Auth,
    Media,
    Legacy,
}

impl Bucket {
    fn name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Auth => "auth",
            Self::Media => "media",
            Self::Legacy => "legacy",
        }
    }
}

/// État du middleware de limitation, clonable pour `from_fn_with_state`.
#[derive(Debug, Clone)]
pub struct RateLimitState {
    general: RateLimiter,
    auth: RateLimiter,
    media: RateLimiter,
    legacy: RateLimiter,
    development: bool,
    trust_proxy_headers: bool,
}

impl RateLimitState {
    /// Construit l'état depuis la configuration du serveur.
    pub fn from_config(config: &Config) -> Self {
        Self {
            general: RateLimiter::new(RATE_LIMIT_MAX, RATE_LIMIT_WINDOW),
            auth: RateLimiter::new(AUTH_RATE_LIMIT_MAX, RATE_LIMIT_WINDOW),
            media: RateLimiter::new(MEDIA_RATE_LIMIT_MAX, RATE_LIMIT_WINDOW),
            legacy: RateLimiter::new(LEGACY_RATE_LIMIT_MAX, RATE_LIMIT_WINDOW),
            development: config.is_development(),
            trust_proxy_headers: config.trust_proxy_headers,
        }
    }

    fn limiter(&self, bucket: Bucket) -> &RateLimiter {
        match bucket {
            Bucket::General => &self.general,
            Bucket::Auth => &self.auth,
            Bucket::Media => &self.media,
            Bucket::Legacy => &self.legacy,
        }
    }
}

/// Middleware axum de limitation de débit.
pub async fn enforce(
    State(state): State<RateLimitState>,
    request: Request,
    next: Next,
) -> Response {
    let ip = client_ip(&state, &request);
    if is_exempt(&state, ip) {
        return next.run(request).await;
    }

    let bucket = bucket_for(request.method(), request.uri().path());
    let decision = state
        .limiter(bucket)
        .check_at(client_key(ip), Instant::now());
    if !decision.allowed {
        tracing::warn!(
            client = ?ip,
            bucket = %bucket.name(),
            path = %request.uri().path(),
            retry_after_secs = decision.reset_after.as_secs(),
            "limite de débit dépassée"
        );
        let mut response = json_error(StatusCode::TOO_MANY_REQUESTS, REQUEST_ERROR);
        insert_rate_limit_headers(&mut response, decision);
        insert_numeric_header(
            &mut response,
            header::RETRY_AFTER,
            decision.reset_after.as_secs().max(1),
        );
        return response;
    }

    let mut response = next.run(request).await;
    insert_rate_limit_headers(&mut response, decision);
    response
}

fn client_key(ip: Option<IpAddr>) -> ClientKey {
    match ip {
        Some(ip) => ClientKey::Known(ip),
        None => ClientKey::Unknown,
    }
}

/// Budget of a request, from its method and path.
fn bucket_for(method: &Method, path: &str) -> Bucket {
    if is_media_request(method, path) {
        Bucket::Media
    } else if is_auth_path(path) {
        Bucket::Auth
    } else if has_path_prefix(path, "/socket.io") {
        Bucket::Legacy
    } else {
        Bucket::General
    }
}

/// Routes that check a guessable secret: pairing-code consumption (overlay and
/// extension) and the admin login routes (secret form, Discord OAuth start and
/// callback).
fn is_auth_path(path: &str) -> bool {
    has_path_prefix(path, "/admin/auth/secret")
        || has_path_prefix(path, "/admin/auth/discord")
        || has_path_prefix(path, "/overlay/pair/consume")
        || has_path_prefix(path, "/ingest/pair/consume")
}

/// `path` is `prefix` itself or a sub-path of it (`/admin`, `/admin/…`, but
/// not `/administration`).
fn has_path_prefix(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Vrai pour un téléchargement média `GET`/`HEAD` : `/overlay/media/{asset_id}`
/// ou `/overlay/media-board/media/{asset_id}`, avec `{asset_id}` réduit à un
/// unique segment de chemin non vide. Ces requêtes seules relèvent de
/// [`MEDIA_RATE_LIMIT_MAX`].
fn is_media_request(method: &Method, path: &str) -> bool {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return false;
    }
    ["/overlay/media/", "/overlay/media-board/media/"]
        .iter()
        .any(|prefix| {
            path.strip_prefix(prefix)
                .is_some_and(|asset_id| !asset_id.is_empty() && !asset_id.contains('/'))
        })
}

/// Exemption : uniquement les adresses loopback en mode développement (tests
/// locaux, parité legacy `server/src/server.ts`).
///
/// `/admin*` n'est plus exempté (AUD-COR-10) : le panneau admin est soumis au
/// même budget que le reste et un bruteforce reçoit `429 Too Many Requests`.
fn is_exempt(state: &RateLimitState, ip: Option<IpAddr>) -> bool {
    state.development && ip.is_some_and(|address| address.is_loopback())
}

fn client_ip(state: &RateLimitState, request: &Request) -> Option<IpAddr> {
    if state.trust_proxy_headers {
        if let Some(ip) = super::proxy::proxied_client_ip(request.headers()) {
            return Some(canonical_ip(ip));
        }
    }
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| canonical_ip(info.0.ip()))
}

/// Normalise une IPv4 encapsulée en IPv6 (`::ffff:a.b.c.d`).
///
/// Une socket d'écoute dual-stack présente les clients IPv4 sous cette forme.
/// Sans normalisation, ces clients ne seraient pas reconnus comme loopback en
/// mode développement (l'exemption ne s'appliquerait pas) et seraient comptés
/// séparément de leur adresse IPv4 réelle.
fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(v4) => IpAddr::V4(v4),
    }
}

fn insert_rate_limit_headers(response: &mut Response, decision: Decision) {
    insert_numeric_header(
        response,
        HeaderName::from_static("x-ratelimit-limit"),
        u64::from(decision.limit),
    );
    insert_numeric_header(
        response,
        HeaderName::from_static("x-ratelimit-remaining"),
        u64::from(decision.remaining),
    );
    insert_numeric_header(
        response,
        HeaderName::from_static("x-ratelimit-reset"),
        decision.reset_after.as_secs().max(1),
    );
}

/// Insère un en-tête numérique, sans paniquer si la valeur est invalide
/// (cas impossible pour des entiers positifs, mais le code reste total).
fn insert_numeric_header(response: &mut Response, name: HeaderName, value: u64) {
    let text = value.to_string();
    if let Ok(header_value) = HeaderValue::from_str(&text) {
        response.headers_mut().insert(name, header_value);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::http::Request as HttpRequest;
    use std::net::Ipv4Addr;

    use super::*;
    use crate::config::Config;

    fn client(last_octet: u8) -> ClientKey {
        ClientKey::Known(IpAddr::V4(Ipv4Addr::new(10, 0, 0, last_octet)))
    }

    fn config(values: &[(&str, &str)]) -> Config {
        Config::from_pairs(
            values
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        )
        .expect("configuration de test valide")
    }

    #[test]
    fn allows_the_limit_then_denies() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        let now = Instant::now();

        let first = limiter.check_at(client(1), now);
        assert!(first.allowed);
        assert_eq!(first.remaining, 1);
        let second = limiter.check_at(client(1), now);
        assert!(second.allowed);
        assert_eq!(second.remaining, 0);
        let third = limiter.check_at(client(1), now);
        assert!(!third.allowed);
        assert_eq!(third.remaining, 0);
        assert!(third.reset_after > Duration::ZERO);
    }

    #[test]
    fn clients_are_counted_separately() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        assert!(limiter.check_at(client(1), now).allowed);
        assert!(!limiter.check_at(client(1), now).allowed);
        assert!(limiter.check_at(client(2), now).allowed);
    }

    #[test]
    fn window_resets_after_expiry() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let start = Instant::now();
        assert!(limiter.check_at(client(1), start).allowed);
        assert!(!limiter.check_at(client(1), start).allowed);
        let later = start + Duration::from_secs(61);
        assert!(limiter.check_at(client(1), later).allowed);
    }

    #[test]
    fn expired_windows_are_purged() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let start = Instant::now();
        for octet in 1..=10 {
            let _ = limiter.check_at(client(octet), start);
        }
        assert_eq!(limiter.tracked_clients(), 10);
        let later = start + Duration::from_secs(61);
        let _ = limiter.check_at(client(1), later);
        assert_eq!(limiter.tracked_clients(), 1);
    }

    #[test]
    fn unknown_addresses_share_a_bucket() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        assert!(limiter.check_at(ClientKey::Unknown, now).allowed);
        assert!(!limiter.check_at(ClientKey::Unknown, now).allowed);
    }

    #[test]
    fn media_paths_are_classified() {
        for (method, path) in [
            (Method::GET, "/overlay/media/abc"),
            (Method::HEAD, "/overlay/media/abc"),
            (Method::GET, "/overlay/media-board/media/abc"),
        ] {
            assert!(is_media_request(&method, path), "{method} {path}");
        }

        for (method, path) in [
            (Method::POST, "/overlay/media/abc"),
            (Method::GET, "/overlay/media"),
            (Method::GET, "/overlay/media/"),
            (Method::GET, "/overlay/media/a/b"),
            (Method::GET, "/overlay/mediax/a"),
            (Method::GET, "/overlay/media-board/items"),
            (Method::GET, "/health"),
        ] {
            assert!(!is_media_request(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn requests_are_sorted_into_buckets() {
        for (method, path, bucket) in [
            (Method::GET, "/overlay/media/abc", Bucket::Media),
            (Method::GET, "/overlay/media-board/media/abc", Bucket::Media),
            (Method::POST, "/admin/auth/secret", Bucket::Auth),
            (Method::GET, "/admin/auth/discord", Bucket::Auth),
            (Method::GET, "/admin/auth/discord/callback", Bucket::Auth),
            (Method::GET, "/admin", Bucket::General),
            (Method::GET, "/admin/", Bucket::General),
            (Method::GET, "/admin/assets/index-abc.css", Bucket::General),
            (Method::GET, "/admin/api/overview", Bucket::General),
            (Method::GET, "/admin/api/events", Bucket::General),
            (Method::GET, "/admin/auth/session", Bucket::General),
            (Method::POST, "/admin/auth/logout", Bucket::General),
            (Method::POST, "/admin/auth/secretx", Bucket::General),
            (Method::POST, "/overlay/pair/consume", Bucket::Auth),
            (Method::POST, "/ingest/pair/consume", Bucket::Auth),
            (Method::POST, "/ingest/pair/consume/", Bucket::Auth),
            (Method::GET, "/socket.io", Bucket::Legacy),
            (Method::GET, "/socket.io/", Bucket::Legacy),
            (Method::POST, "/socket.io/x", Bucket::Legacy),
            (Method::GET, "/ws", Bucket::General),
            (Method::GET, "/overlay/guild", Bucket::General),
            (Method::GET, "/overlay/queue", Bucket::General),
            (
                Method::PUT,
                "/overlay/media-board/uploads/abc/parts",
                Bucket::General,
            ),
            (Method::POST, "/ingest", Bucket::General),
            (Method::GET, "/health", Bucket::General),
            (Method::GET, "/administration", Bucket::General),
            (Method::POST, "/overlay/pair/consumer", Bucket::General),
        ] {
            assert_eq!(bucket_for(&method, path), bucket, "{method} {path}");
        }
    }

    #[test]
    fn admin_paths_are_rate_limited_and_only_dev_loopback_is_exempt() {
        let production =
            RateLimitState::from_config(&config(&[("API_URL", "http://localhost:3000")]));
        let loopback = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        // AUD-COR-10 : plus aucune exemption `/admin*`.
        assert!(!is_exempt(&production, loopback));
        assert!(!is_exempt(
            &production,
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
        ));

        let development = RateLimitState::from_config(&config(&[
            ("API_URL", "http://localhost:3000"),
            ("NODE_ENV", "development"),
        ]));
        assert!(is_exempt(&development, loopback));
        assert!(!is_exempt(
            &development,
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
        ));
    }

    #[test]
    fn forwarded_ip_is_used_only_when_enabled() {
        let request = HttpRequest::builder()
            .uri("/health")
            // First hop forged by the client, last one appended by the proxy.
            .header("x-forwarded-for", "127.0.0.1, 203.0.113.7")
            .body(axum::body::Body::empty())
            .unwrap();

        let untrusted =
            RateLimitState::from_config(&config(&[("API_URL", "http://localhost:3000")]));
        assert_eq!(client_ip(&untrusted, &request), None);

        let trusted = RateLimitState::from_config(&config(&[
            ("API_URL", "http://localhost:3000"),
            ("TRUST_PROXY_HEADERS", "true"),
        ]));
        assert_eq!(
            client_ip(&trusted, &request),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)))
        );
    }

    #[test]
    fn dual_stack_mapped_ipv4_is_canonicalized() {
        let mapped: IpAddr = "::ffff:127.0.0.1".parse().expect("adresse valide");
        assert_eq!(canonical_ip(mapped), IpAddr::V4(Ipv4Addr::LOCALHOST));

        let ipv4: IpAddr = "192.0.2.10".parse().expect("adresse valide");
        assert_eq!(canonical_ip(ipv4), ipv4);

        let ipv6: IpAddr = "2001:db8::1".parse().expect("adresse valide");
        assert_eq!(canonical_ip(ipv6), ipv6);
    }

    #[test]
    fn the_window_lasts_fifteen_minutes() {
        assert_eq!(RATE_LIMIT_WINDOW, Duration::from_secs(15 * 60));
        assert_eq!(RATE_LIMIT_WINDOW.as_secs(), 900);
    }

    #[test]
    fn the_purge_keeps_windows_that_are_still_running() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let start = Instant::now();
        let _ = limiter.check_at(client(1), start);
        let _ = limiter.check_at(client(2), start + Duration::from_secs(30));
        // The purge runs (a window has passed since the last one): client 1
        // is gone, client 2 (31 s old) is kept and still limited.
        let _ = limiter.check_at(client(3), start + Duration::from_secs(61));
        assert_eq!(limiter.tracked_clients(), 2);
        assert!(
            !limiter
                .check_at(client(2), start + Duration::from_secs(61))
                .allowed
        );
    }
}
