//! Admin console sessions: storage and cookie helpers.
//!
//! The contract is section 1 of `docs/rust/admin-console-api.md`:
//!
//! - a session cookie carries 32 random bytes encoded with base64url;
//! - the database only stores the SHA-256 hexadecimal digest of that value;
//! - sessions expire after seven days (absolute deadline);
//! - expired rows are removed on every lookup and once at startup.
//!
//! Raw SQL lives here (like [`super::queries`]) because the repositories of
//! `lc-db` have no session entity: the table exists only for the admin panel.

use axum::http::{header, HeaderMap};
use chrono::Utc;
use lc_db::{Db, DbError};
use rand::RngCore as _;
use sha2::{Digest as _, Sha256};
use sqlx::FromRow;

use super::oauth::DiscordIdentity;

/// Name of the session cookie.
pub const SESSION_COOKIE_NAME: &str = "lc_admin_session";
/// Name of the short-lived OAuth2 `state` cookie.
pub const OAUTH_STATE_COOKIE_NAME: &str = "lc_admin_oauth_state";
/// Session lifetime in seconds (7 days, also used as the cookie `Max-Age`).
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// OAuth2 `state` cookie lifetime in seconds (10 minutes).
pub const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;
/// Session lifetime in milliseconds (absolute expiry stored in SQLite).
const SESSION_TTL_MS: i64 = SESSION_TTL_SECS * 1000;

/// A persisted admin session (never contains the raw cookie token).
#[derive(Debug, Clone, FromRow)]
pub struct AdminSession {
    /// Authentication method: `"secret"` or `"discord"`.
    pub method: String,
    /// Discord user id for a Discord session, `None` for the secret method.
    pub discord_user_id: Option<String>,
    /// Display name (global name or username) for a Discord session.
    pub display_name: Option<String>,
    /// Discord avatar URL for a Discord session.
    pub avatar_url: Option<String>,
    /// Absolute expiry time in milliseconds since the Unix epoch.
    pub expires_at: i64,
}

/// Generates a raw session token (32 random bytes, base64url without padding).
pub fn generate_token() -> String {
    use base64::Engine as _;

    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// SHA-256 hexadecimal digest stored in `admin_sessions.token_hash`.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Creates a session authenticated with the shared admin secret.
pub async fn create_secret(db: &Db, now_ms: i64) -> Result<(String, AdminSession), DbError> {
    create(db, "secret", None, now_ms).await
}

/// Creates a session authenticated through Discord OAuth2.
pub async fn create_discord(
    db: &Db,
    identity: &DiscordIdentity,
    now_ms: i64,
) -> Result<(String, AdminSession), DbError> {
    create(
        db,
        "discord",
        Some((
            identity.id.as_str(),
            identity.name.as_str(),
            identity.avatar_url.as_deref(),
        )),
        now_ms,
    )
    .await
}

/// Inserts a session row and returns the raw token plus the stored row.
async fn create(
    db: &Db,
    method: &str,
    user: Option<(&str, &str, Option<&str>)>,
    now_ms: i64,
) -> Result<(String, AdminSession), DbError> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let expires_at = now_ms.saturating_add(SESSION_TTL_MS);
    let (discord_user_id, display_name, avatar_url) = match user {
        Some((id, name, avatar)) => (
            Some(id.to_owned()),
            Some(name.to_owned()),
            avatar.map(str::to_owned),
        ),
        None => (None, None, None),
    };

    sqlx::query(
        "INSERT INTO admin_sessions ( \
            token_hash, method, discord_user_id, display_name, avatar_url, \
            created_at, expires_at, last_seen_at \
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(&token_hash)
    .bind(method)
    .bind(discord_user_id.as_deref())
    .bind(display_name.as_deref())
    .bind(avatar_url.as_deref())
    .bind(now_ms)
    .bind(expires_at)
    .bind(now_ms)
    .execute(db.pool())
    .await?;

    Ok((
        token,
        AdminSession {
            method: method.to_owned(),
            discord_user_id,
            display_name,
            avatar_url,
            expires_at,
        },
    ))
}

/// Looks a raw token up, deletes expired rows, and refreshes `last_seen_at`.
///
/// `Ok(None)` covers an unknown, expired, or empty token: callers answer `401`
/// without distinguishing the cases.
pub async fn find(db: &Db, token: &str, now_ms: i64) -> Result<Option<AdminSession>, DbError> {
    if token.is_empty() {
        return Ok(None);
    }

    delete_expired(db, now_ms).await?;
    let token_hash = hash_token(token);
    let session = sqlx::query_as::<_, AdminSession>(
        "SELECT method, discord_user_id, display_name, avatar_url, expires_at \
         FROM admin_sessions WHERE token_hash = ?1",
    )
    .bind(&token_hash)
    .fetch_optional(db.pool())
    .await?;

    let Some(session) = session else {
        return Ok(None);
    };
    if session.expires_at <= now_ms {
        delete_token_hash(db, &token_hash).await?;
        return Ok(None);
    }

    sqlx::query("UPDATE admin_sessions SET last_seen_at = ?1 WHERE token_hash = ?2")
        .bind(now_ms)
        .bind(&token_hash)
        .execute(db.pool())
        .await?;

    Ok(Some(session))
}

/// Deletes the session identified by a raw token; returns the number of rows.
pub async fn delete_by_token(db: &Db, token: &str) -> Result<u64, DbError> {
    if token.is_empty() {
        return Ok(0);
    }
    delete_token_hash(db, &hash_token(token)).await
}

/// Deletes every session whose absolute expiry is at or before `now_ms`.
pub async fn delete_expired(db: &Db, now_ms: i64) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= ?1")
        .bind(now_ms)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Startup cleanup: expired sessions are removed when the server starts.
pub async fn purge_expired(db: &Db) -> Result<u64, DbError> {
    delete_expired(db, Utc::now().timestamp_millis()).await
}

async fn delete_token_hash(db: &Db, token_hash: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM admin_sessions WHERE token_hash = ?1")
        .bind(token_hash)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Reads a cookie value from the `Cookie` header(s).
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for pair in value.split(';') {
            let Some((key, candidate)) = pair.trim().split_once('=') else {
                continue;
            };
            let candidate = candidate.trim();
            if key.trim() == name && !candidate.is_empty() {
                return Some(candidate.to_owned());
            }
        }
    }

    None
}

/// Builds the session `Set-Cookie` value.
pub fn session_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/admin; Max-Age={SESSION_TTL_SECS}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Builds the `Set-Cookie` value that removes the session cookie.
pub fn clear_session_cookie(secure: bool) -> String {
    let mut cookie =
        format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Strict; Path=/admin; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Builds the OAuth2 `state` `Set-Cookie` value.
pub fn oauth_state_cookie(state: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{OAUTH_STATE_COOKIE_NAME}={state}; HttpOnly; SameSite=Lax; \
         Path=/admin/auth; Max-Age={OAUTH_STATE_TTL_SECS}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Builds the `Set-Cookie` value that removes the OAuth2 `state` cookie.
pub fn clear_oauth_state_cookie(secure: bool) -> String {
    let mut cookie =
        format!("{OAUTH_STATE_COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path=/admin/auth; Max-Age=0");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn tokens_are_base64url_and_hashes_are_sha256_hex() {
        let first = generate_token();
        let second = generate_token();

        assert_eq!(first.len(), 43, "32 bytes without padding");
        assert!(
            first
                .chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || character == '-'
                    || character == '_'),
            "base64url alphabet: {first}"
        );
        assert_ne!(first, second);

        assert_eq!(hash_token("abc").len(), 64);
        assert!(hash_token("abc").chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash_token("abc"), hash_token("abc"));
        assert_ne!(hash_token("abc"), hash_token("abd"));
    }

    #[test]
    fn session_cookie_carries_the_contract_attributes() {
        let cookie = session_cookie("token", false);
        assert_eq!(
            cookie,
            "lc_admin_session=token; HttpOnly; SameSite=Strict; Path=/admin; Max-Age=604800"
        );
        assert!(session_cookie("token", true).ends_with("; Secure"));
        assert!(clear_session_cookie(false).contains("Max-Age=0"));
    }

    #[test]
    fn oauth_state_cookie_carries_the_contract_attributes() {
        let cookie = oauth_state_cookie("state", false);
        assert_eq!(
            cookie,
            "lc_admin_oauth_state=state; HttpOnly; SameSite=Lax; Path=/admin/auth; Max-Age=600"
        );
        assert!(oauth_state_cookie("state", true).ends_with("; Secure"));
        assert!(clear_oauth_state_cookie(false).contains("Max-Age=0"));
    }

    #[test]
    fn cookie_value_finds_a_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; lc_admin_session=abc; empty= "),
        );

        assert_eq!(
            cookie_value(&headers, SESSION_COOKIE_NAME).as_deref(),
            Some("abc")
        );
        assert_eq!(cookie_value(&headers, "empty"), None);
        assert_eq!(cookie_value(&headers, "missing"), None);
    }

    #[tokio::test]
    async fn a_session_lasts_seven_days_then_is_no_longer_found() {
        let db = lc_db::Db::open_in_memory(&lc_db::DbConfig::default())
            .await
            .unwrap();
        let now_ms = 1_800_000_000_000;
        let (token, session) = create_secret(&db, now_ms).await.unwrap();
        assert_eq!(session.expires_at, now_ms + 7 * 24 * 60 * 60 * 1000);

        let day_six = now_ms + 6 * 24 * 60 * 60 * 1000;
        assert!(find(&db, &token, day_six).await.unwrap().is_some());
        let day_eight = now_ms + 8 * 24 * 60 * 60 * 1000;
        assert!(find(&db, &token, day_eight).await.unwrap().is_none());
    }
}
