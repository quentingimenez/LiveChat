//! `POST /overlay/media-board/cache` — read-only global cache lookup.
//!
//! The add-media dialog uses it to offer « Ignorer le cache serveur » only when
//! the server already has a reusable media for the typed link. The legacy
//! check used the board of the current guild, which missed media ingested by
//! another guild or by an ingest client.
//!
//! ## Security contract
//!
//! - Bearer-authenticated overlay client only; read-only sessions are refused.
//! - The caller needs `AddMedia` **or** `TriggerPlayback`, and must see the
//!   board of its own guild. The lookup is global but never crosses that
//!   authorization boundary for the caller.
//! - The response is only `{"cached": bool}`: no asset id, title, path, guild
//!   or timestamp. It cannot be used to enumerate the cache content.
//! - The URL is validated as a public `http(s)` link (no credentials, no
//!   loopback/private literal host) and bounded to [`MAX_URL_CHARS`]. No
//!   network request is made: only the same canonical `source_hash` used by
//!   ingestion is computed. A TikTok short link therefore only matches a cache
//!   entry written for that short URL, not one written after resolution.
//! - A lookup is a read: [`assets::is_cached`] never extends the cache lifetime.
//!   `Cache-Control: no-store` keeps the boolean out of intermediary caches.
//! - The route body is bounded to [`MAX_BODY_BYTES`]; the global per-IP rate
//!   limiter applies like every other overlay route.

use std::net::IpAddr;

use axum::extract::{FromRequest, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use lc_db::Db;
use lc_domain::rights::{Action, RightsRole, Surface};
use lc_media::source::MediaSource;
use serde::Deserialize;
use url::Url;

use crate::app::AppState;
use crate::media::assets;

use super::auth::AuthedOverlayClient;
use super::media_board::{ensure_board_visible_for, json_rejection_error, media_service_error};
use super::{db, error, json_response, now};

/// Maximum body accepted by the route; the URL itself is capped separately.
pub(super) const MAX_BODY_BYTES: usize = 8 * 1024;

/// Maximum URL length kept in the dialog and accepted by the lookup.
const MAX_URL_CHARS: usize = 2048;

/// Bodies of `POST /overlay/media-board/cache`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CacheLookupRequest {
    /// Link whose cache entry is probed.
    #[serde(default)]
    url: Option<String>,
}

/// Handler `POST /overlay/media-board/cache`.
pub async fn lookup_cache(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    request: Request,
) -> Response {
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    // Check the session before reading the body: a read-only client must not
    // learn whether its payload would have been valid.
    if let Err(response) = ensure_lookup_allowed(&state, &auth, db).await {
        return response;
    }
    let payload = match Json::<CacheLookupRequest>::from_request(request, &state).await {
        Ok(Json(payload)) => payload,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "corps JSON invalide sur POST /overlay/media-board/cache"
            );
            return json_rejection_error(rejection);
        }
    };
    let Some(source) = payload.url.as_deref().and_then(lookup_source) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "invalid_payload", "message": "url is invalid"}),
        );
    };

    let cached = match assets::is_cached(db, &source.source_hash, now(&state)).await {
        Ok(cached) => cached,
        Err(service_error) => return media_service_error(&service_error),
    };
    tracing::debug!(
        client_id = %auth.client.id.to_canonical(),
        cached,
        "media cache lookup"
    );

    let mut response = json_response(StatusCode::OK, serde_json::json!({"cached": cached}));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// `AddMedia` or `TriggerPlayback`, board visible, never a read-only session.
async fn ensure_lookup_allowed(
    state: &AppState,
    auth: &AuthedOverlayClient,
    db: &Db,
) -> Result<(), Response> {
    if auth.is_read_only() {
        return Err(error(StatusCode::FORBIDDEN, "read_only_session"));
    }
    let role: RightsRole = auth.role().into();
    let rights = state.rights.snapshot();
    let allowed = rights.allowed(Surface::Overlay, Action::AddMedia, role)
        || rights.allowed(Surface::Overlay, Action::TriggerPlayback, role);
    if !allowed {
        return Err(error(StatusCode::FORBIDDEN, "forbidden"));
    }
    ensure_board_visible_for(state, db, auth.guild_id(), auth.role()).await
}

/// Validates a public `http(s)` link and computes the ingestion source hash.
fn lookup_source(raw: &str) -> Option<MediaSource> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_URL_CHARS {
        return None;
    }
    let parsed = Url::parse(trimmed).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    if parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !is_public_host(&parsed)
    {
        return None;
    }
    MediaSource::from_raw(trimmed)
}

/// Rejects literal loopback/private hosts. DNS names are not resolved: the
/// lookup never makes a network request, so a public name is accepted as is.
fn is_public_host(url: &Url) -> bool {
    let Some(raw_host) = url.host_str() else {
        return false;
    };
    // IPv6 hosts are bracketed in URLs; strip the brackets before parsing.
    let host = raw_host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(raw_host)
        .trim_end_matches('.');
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return false;
    }
    let Ok(ip) = host.parse::<IpAddr>() else {
        return true;
    };
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast())
        }
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || first & 0xfe00 == 0xfc00
                || first & 0xffc0 == 0xfe80)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::lookup_source;

    #[test]
    fn accepts_public_http_links_and_canonicalizes_them() {
        let source =
            lookup_source(" https://example.test/watch?v=1#part ").expect("lien public accepté");
        assert_eq!(source.canonical_url, "https://example.test/watch?v=1");
    }

    #[test]
    fn rejects_non_http_and_private_targets() {
        for url in [
            "",
            "file:///etc/passwd",
            "ftp://example.test/clip",
            "https://user:pass@example.test/clip",
            "http://localhost/clip",
            "http://127.0.0.1/clip",
            "http://[::1]/clip",
            "http://10.0.0.1/clip",
        ] {
            assert!(lookup_source(url).is_none(), "URL refusée : {url}");
        }
    }

    #[test]
    fn rejects_overlong_urls() {
        let url = format!("https://example.test/{}", "a".repeat(2048));
        assert!(lookup_source(&url).is_none());
    }
}
