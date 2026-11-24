//! Session authentication for the admin console.
//!
//! Section 1 of `docs/rust/admin-console-api.md` replaces the legacy
//! `Authorization: Bearer <ADMIN_UI_TOKEN>` check with a cookie session:
//!
//! 1. when neither the secret login nor the Discord login is configured, every
//!    protected route answers `503 admin_ui_disabled`;
//! 2. `ADMIN_UI_LOCAL_ONLY` (and `ADMIN_TRUST_FORWARDED_FOR`) still apply;
//! 3. the request must carry a valid `lc_admin_session` cookie, otherwise it
//!    receives `401 unauthorized`;
//! 4. mutations additionally require a trusted `Origin`/`Referer` (AUD-COR-14).
//!
//! Authentication is an extractor ([`AdminAccess`]) rather than a middleware,
//! like before: the admin router is built without state (`Router<AppState>`) and
//! every JSON handler takes `_: AdminAccess`, so no `/admin/api/**` route can
//! forget the check. Static assets stay public (they contain no data).
//!
//! The session is exposed to handlers through [`AdminAccess::session`] (method,
//! display name, expiry).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use subtle::ConstantTimeEq as _;

use crate::app::AppState;
use crate::http::error::ApiError;

use super::session::{self, AdminSession};

/// Proof that the request passed the admin access checks.
///
/// Extracted by every `/admin/api/**` handler: when extraction fails, axum
/// returns the corresponding 503/403/401 response and the handler never runs.
#[derive(Debug, Clone)]
pub struct AdminAccess {
    /// Authenticated session (never contains the raw cookie token).
    ///
    /// Handlers that need the actor (audit) can read it; JSON handlers only
    /// prove the access check today.
    #[allow(dead_code)]
    pub session: AdminSession,
}

/// Enabled admin login methods, as published by `GET /admin/auth/session`.
#[derive(Debug, Clone, Copy, Default)]
pub struct AuthMethods {
    /// Discord OAuth2 login is configured and has an allow-list.
    pub discord: bool,
    /// Secret login is configured (`ADMIN_UI_TOKEN` non-empty).
    pub secret: bool,
}

impl AuthMethods {
    /// `true` when at least one method is available.
    pub fn any(self) -> bool {
        self.discord || self.secret
    }

    /// JSON body of the `methods` field.
    pub fn payload(self) -> Value {
        json!({ "discord": self.discord, "secret": self.secret })
    }
}

/// Current login methods for a given state.
pub fn auth_methods(state: &AppState) -> AuthMethods {
    AuthMethods {
        discord: state.config.admin_discord_login_enabled(),
        secret: state.config.admin_secret_login_enabled(),
    }
}

impl FromRequestParts<AppState> for AdminAccess {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !auth_methods(state).any() {
            return Err(disabled_response());
        }

        check_local_only(
            state,
            &parts.headers,
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0.ip()),
        )?;

        let admin_session = current_session(state, &parts.headers).await?;
        let Some(admin_session) = admin_session else {
            // Only clear a cookie that was actually presented: a request without
            // one must not receive a stray `Set-Cookie`.
            return Err(if has_session_cookie(&parts.headers) {
                unauthorized_response_with_cleared_cookie(state.config.cookies_secure())
            } else {
                unauthorized_response()
            });
        };

        // AUD-COR-14: defence in depth against CSRF. Cookies are ambient
        // credentials, so mutations must originate from the panel (or API_URL
        // behind a reverse proxy).
        if is_mutation(&parts.method) && !origin_allowed(&parts.headers, state) {
            return Err(invalid_origin_response());
        }

        Ok(Self {
            session: admin_session,
        })
    }
}

/// Loads the session carried by the request cookie, if any.
///
/// `Ok(None)` covers a missing cookie, an unknown token, and an expired token.
/// Database failures become a neutral `500 internal_error` response.
#[allow(clippy::result_large_err)]
pub async fn current_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<AdminSession>, Response> {
    let Some(token) = session::cookie_value(headers, session::SESSION_COOKIE_NAME) else {
        return Ok(None);
    };
    let Some(db) = state.db.as_ref() else {
        tracing::error!("admin session lookup without a configured database");
        return Ok(None);
    };
    let now_ms = lc_domain::datetime_to_ms(state.clock.now());

    session::find(db, &token, now_ms).await.map_err(|error| {
        tracing::error!(error = %error, "admin session lookup failed");
        internal_error_response()
    })
}

/// Applies `ADMIN_UI_LOCAL_ONLY` to auth and API routes.
///
/// `peer_ip` is the direct connection address (`ConnectInfo`). When
/// `ADMIN_TRUST_FORWARDED_FOR` is enabled **and** that peer is listed in
/// `TRUSTED_PROXIES`, the `X-Forwarded-For` chain decides instead: declared
/// proxy hops are skipped from right to left and the first other hop is the
/// client. Any other peer is judged on its own address, never on headers it can
/// forge. A chain with no client hop (missing header, invalid entry met before
/// the client, or only declared proxies) fails closed, as does a missing peer
/// address.
#[allow(clippy::result_large_err)]
pub fn check_local_only(
    state: &AppState,
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
) -> Result<(), Response> {
    let peer_ip = peer_ip.map(crate::http::proxy::canonical_ip);
    let loopback = if state.config.admin_trust_forwarded_for
        && crate::http::proxy::is_trusted_proxy(peer_ip, &state.config.trusted_proxies)
    {
        // The declared proxy speaks for the client. If it forwards no usable
        // address, fail closed instead of judging the proxy itself.
        crate::http::proxy::forwarded_client_ip(peer_ip, headers, &state.config.trusted_proxies)
            .is_some_and(is_loopback)
    } else {
        peer_ip.is_some_and(is_loopback)
    };

    if state.config.admin_ui_local_only && !loopback {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "admin_local_only").into_response());
    }

    Ok(())
}

/// Methods considered mutations (mandatory origin check).
///
/// `http::Method` exposes no pattern-matchable variant: the uppercase textual
/// representation is stable for standard methods.
pub fn is_mutation(method: &Method) -> bool {
    matches!(method.as_str(), "POST" | "PATCH" | "PUT" | "DELETE")
}

/// Origin announced by the request: `Origin`, otherwise the `Referer` origin.
fn request_origin(headers: &HeaderMap) -> Option<String> {
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let origin = origin.trim();
        if !origin.is_empty() {
            return Some(origin.to_owned());
        }
    }

    headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(referer_origin)
}

/// Extracts `scheme://authority` from a `Referer` (the path is ignored).
fn referer_origin(referer: &str) -> Option<String> {
    let (scheme, rest) = referer.trim().split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|authority| !authority.is_empty())?;
    Some(format!("{scheme}://{authority}"))
}

/// Normalized authority of an origin (`http(s)://host[:port]`), lowercase.
///
/// Returns `None` for an opaque origin (`null`), a non-HTTP(S) scheme, or an
/// empty value: those can never match a legitimate panel request.
fn canonical_authority(origin: &str) -> Option<String> {
    let (scheme, rest) = origin.trim().split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?.trim();
    if authority.is_empty() || authority.eq_ignore_ascii_case("null") {
        return None;
    }
    Some(authority.to_ascii_lowercase())
}

/// Normalized `Host` header authority (lowercase).
fn request_host(headers: &HeaderMap) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Is the mutation origin trusted?
///
/// Accepts the `Host` authority (same origin as the panel) or the public origin
/// derived from `API_URL` (reverse proxy rewriting `Host`). A missing origin,
/// `null`, or a foreign origin is rejected.
pub fn origin_allowed(headers: &HeaderMap, state: &AppState) -> bool {
    let Some(origin) = request_origin(headers) else {
        return false;
    };
    let Some(origin_authority) = canonical_authority(&origin) else {
        return false;
    };

    if let Some(host) = request_host(headers) {
        if origin_authority == host {
            return true;
        }
    }

    canonical_authority(&state.config.api_origin()).is_some_and(|api| api == origin_authority)
}

/// Response used when no admin login method is configured.
pub fn disabled_response() -> Response {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "admin_ui_disabled").into_response()
}

/// Response used for a missing or invalid session.
pub fn unauthorized_response() -> Response {
    ApiError::unauthorized().into_response()
}

/// `401` for a session cookie that exists but is no longer usable (expired,
/// idle, or unknown).
///
/// The stale cookie is cleared in the same response: the server already deleted
/// the row, so keeping the browser-side cookie until its seven-day `Max-Age`
/// would only make every later request carry a dead credential.
pub fn unauthorized_response_with_cleared_cookie(secure: bool) -> Response {
    with_cleared_session_cookie(unauthorized_response(), secure)
}

/// Adds the `Set-Cookie` that clears the session cookie to `response`.
pub fn with_cleared_session_cookie(mut response: Response, secure: bool) -> Response {
    if let Ok(cleared) = HeaderValue::from_str(&session::clear_session_cookie(secure)) {
        response.headers_mut().insert(header::SET_COOKIE, cleared);
    }
    response
}

/// `true` when the request carries a session cookie, usable or not.
pub fn has_session_cookie(headers: &HeaderMap) -> bool {
    session::cookie_value(headers, session::SESSION_COOKIE_NAME).is_some()
}

/// Response used when the shared secret does not match.
pub fn invalid_secret_response() -> Response {
    ApiError::new(StatusCode::UNAUTHORIZED, "invalid_secret").into_response()
}

/// Response used when a specific login method is disabled.
pub fn method_disabled_response() -> Response {
    ApiError::new(StatusCode::NOT_FOUND, "method_disabled").into_response()
}

/// Response used when a mutation origin is missing or foreign.
pub fn invalid_origin_response() -> Response {
    ApiError::new(StatusCode::FORBIDDEN, "invalid_origin").into_response()
}

/// Neutral internal error (details are only logged).
pub fn internal_error_response() -> Response {
    ApiError::internal().into_response()
}

/// JSON response with the given status.
pub fn json_status(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Constant-time comparison (length first, like the legacy check).
pub fn constant_time_eq(expected: &str, candidate: &str) -> bool {
    let expected = expected.as_bytes();
    let candidate = candidate.as_bytes();

    if expected.len() != candidate.len() {
        return false;
    }

    expected.ct_eq(candidate).into()
}

/// Loopback address? True for `127.0.0.0/8`, `::1` and `::ffff:127.0.0.1`.
fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(address) => address.is_loopback(),
        IpAddr::V6(address) => {
            address.is_loopback()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|mapped: Ipv4Addr| mapped.is_loopback())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn comparaison_temps_constant() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "secreT"));
        assert!(!constant_time_eq("secret", "secret "));
        assert!(!constant_time_eq("secret", ""));
    }

    #[test]
    fn loopback_accepte_les_formes_attendues() {
        assert!(is_loopback(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_loopback(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 5))));
        assert!(is_loopback("::1".parse().expect("IPv6 valide")));
        assert!(is_loopback("::ffff:127.0.0.1".parse().expect("IPv6 mappé")));
        assert!(!is_loopback(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
        assert!(!is_loopback("fe80::1".parse().expect("IPv6 valide")));
    }

    #[test]
    fn empty_or_null_origin_authorities_are_refused() {
        assert_eq!(canonical_authority("http://"), None);
        assert_eq!(canonical_authority("https:///admin"), None);
        assert_eq!(canonical_authority("http://null"), None);
        assert_eq!(canonical_authority("null"), None);
        assert_eq!(
            canonical_authority("HTTP://Admin.Example:8443/x"),
            Some("admin.example:8443".to_owned())
        );
        assert_eq!(
            canonical_authority("https://livechat.example"),
            Some("livechat.example".to_owned())
        );
        assert_eq!(canonical_authority("ftp://livechat.example"), None);
    }

    #[test]
    fn internal_errors_answer_500_without_details() {
        let response = internal_error_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }
}
