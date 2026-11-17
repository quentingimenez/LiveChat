//! Admin authentication routes.
//!
//! Implements the endpoints of section 1 of
//! `docs/rust/admin-console-api.md`:
//!
//! | Route | Effect |
//! |---|---|
//! | `GET /admin/auth/session` | current session or `401` + available methods |
//! | `POST /admin/auth/secret` | secret login, sets the session cookie |
//! | `GET /admin/auth/discord` | Discord OAuth2 redirect |
//! | `GET /admin/auth/discord/callback` | state check, code exchange, allow-list |
//! | `POST /admin/auth/logout` | deletes the session and clears the cookie |
//!
//! `POST /admin/auth/secret` and `POST /admin/auth/logout` are mutations and use
//! the same `Origin`/`Referer` check as the API. The cookie and session logic
//! lives in [`super::session`]; the Discord exchange is injected through
//! [`super::oauth::AdminOAuth`].

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::app::AppState;

use super::audit::{AuditActor, AuditStatus};
use super::auth::{self, AuthMethods};
use super::oauth;
use super::session;
use super::{iso_from_ms, AdminError};

/// Builds the authentication sub-router (wrapped by `secure(...)` in `mod.rs`).
pub(crate) fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/admin/auth/session", get(session_info))
        .route("/admin/auth/secret", post(secret_login))
        .route("/admin/auth/discord", get(discord_login))
        .route("/admin/auth/discord/callback", get(discord_callback))
        .route("/admin/auth/logout", post(logout))
}

/// Direct peer address when the server was started with connect info.
///
/// A dedicated extractor is needed because `Option<ConnectInfo<…>>` is only
/// available when the inner extractor implements `OptionalFromRequestParts`,
/// which `ConnectInfo` does not. A missing address is treated as a remote
/// client by the local-only check, never as a handler failure.
#[derive(Debug, Clone, Copy)]
struct PeerIp(Option<IpAddr>);

impl<S> FromRequestParts<S> for PeerIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0.ip()),
        ))
    }
}

/// `GET /admin/auth/session`: current session details or `401`.
async fn session_info(State(state): State<AppState>, peer: PeerIp, headers: HeaderMap) -> Response {
    if let Err(response) = auth::check_local_only(&state, &headers, peer.0) {
        return response;
    }

    let methods = auth::auth_methods(&state);
    let admin_session = match auth::current_session(&state, &headers).await {
        Ok(Some(session)) => session,
        Ok(None) => return unauthorized_session(methods),
        Err(response) => return response,
    };

    let user = if admin_session.method == "discord" {
        json!({
            "id": admin_session.discord_user_id.clone().unwrap_or_default(),
            "name": admin_session.display_name.clone().unwrap_or_default(),
            "avatarUrl": admin_session.avatar_url,
        })
    } else {
        Value::Null
    };
    let expires_at = iso_from_ms(admin_session.expires_at).unwrap_or_default();

    auth::json_status(
        StatusCode::OK,
        json!({
            "method": admin_session.method,
            "user": user,
            "expiresAt": expires_at,
        }),
    )
}

/// `POST /admin/auth/secret` body.
#[derive(Debug, Deserialize)]
struct SecretLoginBody {
    secret: Option<String>,
}

/// `POST /admin/auth/secret`: fallback login with `ADMIN_UI_TOKEN`.
async fn secret_login(
    State(state): State<AppState>,
    peer: PeerIp,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(response) = guard_route(&state, &headers, peer) {
        return response;
    }
    let methods = auth::auth_methods(&state);
    if !methods.secret {
        return auth::method_disabled_response();
    }
    // Check the origin before the secret: a cross-site form must not be able
    // to probe the secret or plant a session.
    if !auth::origin_allowed(&headers, &state) {
        return auth::invalid_origin_response();
    }

    let Ok(payload) = serde_json::from_slice::<SecretLoginBody>(&body) else {
        return AdminError::bad_request("invalid_payload").into_response();
    };
    let Some(candidate) = payload.secret else {
        return AdminError::bad_request("invalid_payload").into_response();
    };

    let expected = state.config.admin_ui_token.expose_secret().trim();
    if !auth::constant_time_eq(expected, &candidate) {
        return auth::invalid_secret_response();
    }

    let Some(db) = state.db.as_ref() else {
        tracing::error!("secret admin login without a configured database");
        return auth::internal_error_response();
    };
    let now_ms = lc_domain::datetime_to_ms(state.clock.now());
    match session::create_secret(db, now_ms).await {
        Ok((token, admin_session)) => {
            let mut response = with_cookies(
                StatusCode::NO_CONTENT,
                &[session::session_cookie(
                    &token,
                    state.config.cookies_secure(),
                )],
            );
            response
                .extensions_mut()
                .insert(AuditActor::from_session(&admin_session));
            response
        }
        Err(error) => {
            tracing::error!(error = %error, "admin secret session creation failed");
            auth::internal_error_response()
        }
    }
}

/// `GET /admin/auth/discord`: redirect to Discord with a `state` cookie.
async fn discord_login(
    State(state): State<AppState>,
    peer: PeerIp,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = guard_route(&state, &headers, peer) {
        return response;
    }
    if !auth::auth_methods(&state).discord {
        return auth::method_disabled_response();
    }
    let Some(client_id) = state.config.discord_client_id.as_deref() else {
        return auth::method_disabled_response();
    };

    let state_token = session::generate_token();
    let Ok(mut authorize) = Url::parse(oauth::AUTHORIZE_URL) else {
        return auth::internal_error_response();
    };
    authorize
        .query_pairs_mut()
        .append_pair("client_id", client_id.trim())
        .append_pair("response_type", "code")
        .append_pair("scope", "identify")
        .append_pair("state", &state_token)
        .append_pair("redirect_uri", &state.config.admin_discord_redirect_uri());

    redirect_response(
        authorize.as_str(),
        &[session::oauth_state_cookie(
            &state_token,
            state.config.cookies_secure(),
        )],
    )
}

/// Query string of the OAuth2 callback.
#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

/// `GET /admin/auth/discord/callback`: verify, exchange, allow-list, redirect.
async fn discord_callback(
    State(state): State<AppState>,
    peer: PeerIp,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if let Err(response) = guard_route(&state, &headers, peer) {
        return response;
    }
    if !auth::auth_methods(&state).discord {
        return auth::method_disabled_response();
    }

    let cookie_state = session::cookie_value(&headers, session::OAUTH_STATE_COOKIE_NAME);
    let state_matches = match (cookie_state.as_deref(), query.state.as_deref()) {
        (Some(expected), Some(candidate)) => auth::constant_time_eq(expected, candidate),
        _ => false,
    };
    if !state_matches {
        return login_error(&state, "oauth_state", StatusCode::BAD_REQUEST);
    }

    let Some(code) = query
        .code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    else {
        return login_error(&state, "oauth_failed", StatusCode::BAD_REQUEST);
    };
    let redirect_uri = state.config.admin_discord_redirect_uri();
    let identity = match state.admin_oauth.exchange(code, &redirect_uri).await {
        Ok(identity) => identity,
        Err(error) => {
            tracing::warn!(error = %error, "Discord admin login exchange failed");
            return login_error(&state, "oauth_failed", StatusCode::BAD_GATEWAY);
        }
    };

    if !state
        .config
        .admin_discord_user_ids
        .iter()
        .any(|allowed| allowed == &identity.id)
    {
        tracing::warn!(
            discord_user_id = %identity.id,
            "Discord account not allowed to open an admin session"
        );
        // The journal names the refused account: a refused login is exactly
        // what an administrator wants to see.
        let mut response = login_error(&state, "not_allowed", StatusCode::FORBIDDEN);
        response.extensions_mut().insert(AuditActor {
            method: "discord".to_owned(),
            name: Some(identity.name.clone()),
        });
        return response;
    }

    let Some(db) = state.db.as_ref() else {
        tracing::error!("Discord admin login without a configured database");
        return auth::internal_error_response();
    };
    let now_ms = lc_domain::datetime_to_ms(state.clock.now());
    match session::create_discord(db, &identity, now_ms).await {
        Ok((token, admin_session)) => {
            let secure = state.config.cookies_secure();
            let mut response = redirect_response(
                "/admin/#/dashboard",
                &[
                    session::session_cookie(&token, secure),
                    session::clear_oauth_state_cookie(secure),
                ],
            );
            response
                .extensions_mut()
                .insert(AuditActor::from_session(&admin_session));
            response
                .extensions_mut()
                .insert(AuditStatus(StatusCode::OK.as_u16()));
            response
        }
        Err(error) => {
            tracing::error!(error = %error, "admin Discord session creation failed");
            auth::internal_error_response()
        }
    }
}

/// `POST /admin/auth/logout`: delete the session and clear the cookie.
async fn logout(State(state): State<AppState>, peer: PeerIp, headers: HeaderMap) -> Response {
    if let Err(response) = guard_route(&state, &headers, peer) {
        return response;
    }
    if !auth::origin_allowed(&headers, &state) {
        return auth::invalid_origin_response();
    }

    if let Some(db) = state.db.as_ref() {
        if let Some(token) = session::cookie_value(&headers, session::SESSION_COOKIE_NAME) {
            if let Err(error) = session::delete_by_token(db, &token).await {
                tracing::error!(error = %error, "admin session deletion failed");
                return auth::internal_error_response();
            }
        }
    }

    with_cookies(
        StatusCode::NO_CONTENT,
        &[session::clear_session_cookie(state.config.cookies_secure())],
    )
}

/// Common guard: at least one method enabled, then local-only check.
#[allow(clippy::result_large_err)]
fn guard_route(state: &AppState, headers: &HeaderMap, peer: PeerIp) -> Result<(), Response> {
    if !auth::auth_methods(state).any() {
        return Err(auth::disabled_response());
    }

    auth::check_local_only(state, headers, peer.0)
}

/// `401` body of `GET /admin/auth/session`.
fn unauthorized_session(methods: AuthMethods) -> Response {
    auth::json_status(
        StatusCode::UNAUTHORIZED,
        json!({
            "error": "unauthorized",
            "methods": methods.payload(),
        }),
    )
}

/// Redirects to the console login page with the given error code.
/// Redirect back to the login page with `error=<code>`. `outcome` is the
/// status written to the journal: the HTTP status is always `302`.
fn login_error(state: &AppState, code: &str, outcome: StatusCode) -> Response {
    let mut response = redirect_response(
        &format!("/admin/#/login?error={code}"),
        &[session::clear_oauth_state_cookie(
            state.config.cookies_secure(),
        )],
    );
    response
        .extensions_mut()
        .insert(AuditStatus(outcome.as_u16()));
    response
}

/// `302 Found` with one or more `Set-Cookie` headers.
fn redirect_response(location: &str, cookies: &[String]) -> Response {
    let mut headers = HeaderMap::new();
    let Ok(location) = HeaderValue::from_str(location) else {
        return auth::internal_error_response();
    };
    headers.insert(header::LOCATION, location);
    append_cookies(&mut headers, cookies);

    (StatusCode::FOUND, headers).into_response()
}

/// Empty response with one or more `Set-Cookie` headers.
fn with_cookies(status: StatusCode, cookies: &[String]) -> Response {
    let mut headers = HeaderMap::new();
    append_cookies(&mut headers, cookies);

    (status, headers).into_response()
}

fn append_cookies(headers: &mut HeaderMap, cookies: &[String]) {
    for cookie in cookies {
        if let Ok(value) = HeaderValue::from_str(cookie) {
            headers.append(header::SET_COOKIE, value);
        }
    }
}
