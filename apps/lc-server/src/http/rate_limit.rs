//! Limitation de débit par adresse IP.
//!
//! ## Choix d'implémentation
//!
//! Le legacy utilisait `@fastify/rate-limit` : 100 requêtes par fenêtre de
//! 15 minutes, cache de 10 000 entrées, exemption des URLs `/admin*` et, en
//! développement, des adresses loopback (`server/src/server.ts`, cf.
//! `docs/rust/audit-dynamique.md`). Ces paramètres sont conservés, mais le
//! limiteur est ré-implémenté en mémoire plutôt qu'avec `tower_governor` :
//!
//! - une fenêtre fixe suffit au contrat (le legacy n'utilisait pas non plus de
//!   fenêtre glissante) et rend les tests déterministes ;
//! - les exemptions dépendent du chemin et de l'environnement, ce qu'un
//!   middleware générique exprimerait plus mal ;
//! - aucune dépendance supplémentaire n'est ajoutée.
//!
//! L'adresse IP est lue depuis `ConnectInfo<SocketAddr>` (jamais fourni par le
//! client). Derrière un reverse proxy, `TRUST_PROXY_HEADERS=true` active la
//! lecture du premier `X-Forwarded-For` — configuration explicite exigée par le
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
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::Config;
use crate::http::error::{json_error, REQUEST_ERROR};

/// Nombre maximal de requêtes par fenêtre et par client.
pub const RATE_LIMIT_MAX: u32 = 100;

/// Durée de la fenêtre de limitation.
pub const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Préfixe d'URL exempté (parité legacy : tout `/admin*`).
const ADMIN_PATH_PREFIX: &str = "/admin";

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

/// État du middleware de limitation, clonable pour `from_fn_with_state`.
#[derive(Debug, Clone)]
pub struct RateLimitState {
    limiter: RateLimiter,
    development: bool,
    trust_proxy_headers: bool,
}

impl RateLimitState {
    /// Construit l'état depuis la configuration du serveur.
    pub fn from_config(config: &Config) -> Self {
        Self {
            limiter: RateLimiter::new(RATE_LIMIT_MAX, RATE_LIMIT_WINDOW),
            development: config.is_development(),
            trust_proxy_headers: config.trust_proxy_headers,
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
    if is_exempt(request.uri().path(), &state, ip) {
        return next.run(request).await;
    }

    let decision = state.limiter.check_at(client_key(ip), Instant::now());
    if !decision.allowed {
        tracing::warn!(
            client = ?ip,
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

/// Exemptions : `/admin*` (l'admin UI a ses propres contrôles) et, en
/// développement seulement, les adresses loopback (tests locaux).
fn is_exempt(path: &str, state: &RateLimitState, ip: Option<IpAddr>) -> bool {
    path.starts_with(ADMIN_PATH_PREFIX)
        || (state.development && ip.is_some_and(|address| address.is_loopback()))
}

fn client_ip(state: &RateLimitState, request: &Request) -> Option<IpAddr> {
    if state.trust_proxy_headers {
        if let Some(ip) = forwarded_ip(request) {
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

/// Premier `X-Forwarded-For` exploitable (uniquement si `TRUST_PROXY_HEADERS`).
fn forwarded_ip(request: &Request) -> Option<IpAddr> {
    request
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .and_then(|value| value.parse::<IpAddr>().ok())
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
    fn admin_paths_and_dev_loopback_are_exempt() {
        let production =
            RateLimitState::from_config(&config(&[("API_URL", "http://localhost:3000")]));
        let loopback = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(is_exempt("/admin", &production, loopback));
        assert!(is_exempt("/admin/overlay", &production, loopback));
        assert!(!is_exempt("/health", &production, loopback));

        let development = RateLimitState::from_config(&config(&[
            ("API_URL", "http://localhost:3000"),
            ("NODE_ENV", "development"),
        ]));
        assert!(is_exempt("/health", &development, loopback));
        assert!(!is_exempt(
            "/health",
            &development,
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
        ));
    }

    #[test]
    fn forwarded_ip_is_used_only_when_enabled() {
        let request = HttpRequest::builder()
            .uri("/health")
            .header("x-forwarded-for", "203.0.113.7, 10.0.0.1")
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
}
