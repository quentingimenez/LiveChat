//! Admin console sessions: storage and cookie helpers.
//!
//! The contract is section 1 of `docs/rust/admin-console-api.md`:
//!
//! - a session cookie carries 32 random bytes encoded with base64url;
//! - the database only stores the SHA-256 hexadecimal digest of that value;
//! - a session expires at a seven-day absolute deadline **and** after 24 hours
//!   of inactivity (`last_seen_at`, refreshed on every authenticated request);
//! - expired or idle rows are removed on every lookup and once at startup.
//!
//! Raw SQL lives here (like [`super::queries`]) because the repositories of
//! `lc-db` have no session entity: the table exists only for the admin panel.

use axum::http::{header, HeaderMap};
use lc_db::{Db, DbError};
use rand::RngCore as _;
use sha2::{Digest as _, Sha256};
use sqlx::FromRow;
use time::UtcDateTime;

use super::oauth::DiscordIdentity;

/// Name of the session cookie.
pub const SESSION_COOKIE_NAME: &str = "lc_admin_session";
/// Name of the short-lived OAuth2 `state` cookie.
pub const OAUTH_STATE_COOKIE_NAME: &str = "lc_admin_oauth_state";
/// Absolute session lifetime in seconds (7 days, also the cookie `Max-Age`).
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// Idle session lifetime in seconds (24 hours without any authenticated use).
pub const SESSION_IDLE_TTL_SECS: i64 = 24 * 60 * 60;
/// OAuth2 `state` cookie lifetime in seconds (10 minutes).
pub const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;
/// Absolute expiry in milliseconds (stored in SQLite).
const SESSION_TTL_MS: i64 = SESSION_TTL_SECS * 1000;
/// Idle expiry in milliseconds (compared against `last_seen_at`).
const SESSION_IDLE_TTL_MS: i64 = SESSION_IDLE_TTL_SECS * 1000;

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
    /// Last authenticated use in milliseconds since the Unix epoch.
    pub last_seen_at: i64,
}

/// Generates a raw session token (32 random bytes, base64url without padding).
pub fn generate_token() -> String {
    use base64::Engine as _;

    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Length of a [`generate_token`] value: 32 bytes, base64url, no padding.
const TOKEN_LEN: usize = 43;

/// Whether `value` has the shape of a [`generate_token`] value.
///
/// Routes that reuse a token they issued earlier (the OAuth2 `state`, pinned in
/// a cookie) check this first: a value of any other shape comes from a foreign
/// or damaged cookie and is replaced instead of being reflected back into the
/// authorization URL.
pub fn is_token(value: &str) -> bool {
    value.len() == TOKEN_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
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
            last_seen_at: now_ms,
        },
    ))
}

/// Millisecond cutoffs at or before which a session is invalid at `now_ms`:
/// the absolute deadline and the idle window. Single source of truth shared by
/// [`find`]'s guard and [`delete_expired`]'s bulk cleanup, so the strict
/// boundary (valid at idle-1, expired at idle) can never drift between them.
fn expiry_cutoffs_ms(now_ms: i64) -> (i64, i64) {
    (now_ms, now_ms.saturating_sub(SESSION_IDLE_TTL_MS))
}

/// Looks a raw token up, refreshes `last_seen_at`, and purges expired rows.
///
/// `Ok(None)` covers an unknown, expired, idle, or empty token: callers answer
/// `401` without distinguishing the cases. A successful lookup also refreshes
/// the 24 h idle window, so an actively used session stays valid until the
/// seven-day absolute deadline.
///
/// The row is judged against [`expiry_cutoffs_ms`] **before** the sweep, so the
/// per-row guard is the authority for this token: a session that crosses a
/// deadline during the round-trip is refused here, not silently refreshed. The
/// sweep that follows uses the same cutoffs, which is why it can only remove
/// the refused row and never the one just validated.
pub async fn find(db: &Db, token: &str, now_ms: i64) -> Result<Option<AdminSession>, DbError> {
    if token.is_empty() {
        return Ok(None);
    }

    let token_hash = hash_token(token);
    let session = sqlx::query_as::<_, AdminSession>(
        "SELECT method, discord_user_id, display_name, avatar_url, expires_at, last_seen_at \
         FROM admin_sessions WHERE token_hash = ?1",
    )
    .bind(&token_hash)
    .fetch_optional(db.pool())
    .await?;

    let (absolute_cutoff, idle_cutoff) = expiry_cutoffs_ms(now_ms);
    let Some(session) = session else {
        delete_expired(db, now_ms).await?;
        return Ok(None);
    };
    let usable = session.expires_at > absolute_cutoff && session.last_seen_at > idle_cutoff;

    // One sweep for every lookup: it removes the refused row (same cutoffs) and
    // any other stale row without a second statement on the happy path.
    delete_expired(db, now_ms).await?;
    if !usable {
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

/// Deletes every session that is past its absolute expiry or idle window.
///
/// Uses [`expiry_cutoffs_ms`], the same helper as [`find`]'s per-row guard, so
/// the sweep and the guard can never disagree on the strict boundary: a row the
/// guard refused is removed here, and a row the guard accepted is never
/// touched. Called on every lookup ([`find`]) and once at startup
/// ([`purge_expired`]).
pub async fn delete_expired(db: &Db, now_ms: i64) -> Result<u64, DbError> {
    let (absolute_cutoff, idle_cutoff) = expiry_cutoffs_ms(now_ms);
    let result =
        sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= ?1 OR last_seen_at <= ?2")
            .bind(absolute_cutoff)
            .bind(idle_cutoff)
            .execute(db.pool())
            .await?;

    Ok(result.rows_affected())
}

/// Startup cleanup: expired and idle sessions are removed when the server starts.
pub async fn purge_expired(db: &Db) -> Result<u64, DbError> {
    delete_expired(db, lc_domain::datetime_to_ms(UtcDateTime::now())).await
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
        assert!(is_token(&first), "base64url alphabet: {first}");
        assert!(
            first
                .chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || character == '-'
                    || character == '_'),
            "base64url alphabet: {first}"
        );
        assert_ne!(first, second);

        // Anything else is a cookie value the server never issued.
        assert!(!is_token(""), "vide");
        assert!(!is_token(&first[..42]), "tronqué");
        assert!(!is_token(&format!("{first}=")), "avec remplissage");
        assert!(!is_token("state-1"), "valeur étrangère");

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
    async fn an_active_session_still_dies_at_the_seven_day_absolute_deadline() {
        const HOUR_MS: i64 = 60 * 60 * 1000;
        const DAY_HOURS: i64 = 24;
        let db = lc_db::Db::open_in_memory(&lc_db::DbConfig::default())
            .await
            .unwrap();
        let now_ms = 1_800_000_000_000;
        let (token, session) = create_secret(&db, now_ms).await.unwrap();
        assert_eq!(session.expires_at, now_ms + SESSION_TTL_MS);

        // One visit every 23 h keeps the 24 h idle window fresh for the whole
        // week, so inactivity can never be the reason this session dies.
        for hour in (23..=6 * DAY_HOURS + 17).step_by(23) {
            let at = now_ms + hour * HOUR_MS;
            assert!(
                find(&db, &token, at).await.unwrap().is_some(),
                "visite à {hour} h : la session active reste valide"
            );
        }

        // 167 h: six hours of inactivity (fresh window) and one hour before the
        // absolute deadline, so the session must still be accepted.
        assert!(find(&db, &token, now_ms + (7 * DAY_HOURS - 1) * HOUR_MS)
            .await
            .unwrap()
            .is_some());

        // 168 h: the absolute deadline wins even though the idle window is
        // fresh — this is the assertion that isolates the seven-day bound.
        assert!(find(&db, &token, now_ms + 7 * DAY_HOURS * HOUR_MS)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn idle_window_strict_bound_and_activity_refresh() {
        const HOUR_MS: i64 = 60 * 60 * 1000;
        const IDLE_MS: i64 = 24 * HOUR_MS;
        let db = lc_db::Db::open_in_memory(&lc_db::DbConfig::default())
            .await
            .unwrap();
        let start = 1_800_000_000_000;
        let (token, _) = create_secret(&db, start).await.unwrap();

        // Two independent sessions: the IDLE-1 lookup refreshes `last_seen_at`,
        // so it cannot also be the one tested at IDLE exactly.
        assert!(find(&db, &token, start + IDLE_MS - 1)
            .await
            .unwrap()
            .is_some());
        let (idle_token, _) = create_secret(&db, start).await.unwrap();
        assert!(find(&db, &idle_token, start + IDLE_MS)
            .await
            .unwrap()
            .is_none());
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM admin_sessions WHERE token_hash = ?1")
                .bind(hash_token(&idle_token))
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(count, 0, "ligne supprimée à l'échéance de l'inactivité");

        // Activity pushes the window: at t+12 h then t+30 h (18 h idle).
        let (token, _) = create_secret(&db, start).await.unwrap();
        assert!(find(&db, &token, start + 12 * HOUR_MS)
            .await
            .unwrap()
            .is_some());
        assert!(
            find(&db, &token, start + 30 * HOUR_MS)
                .await
                .unwrap()
                .is_some(),
            "seules 18 h d'inactivité se sont écoulées"
        );
    }

    #[tokio::test]
    async fn absolute_expiry_wins_over_fresh_activity() {
        let db = lc_db::Db::open_in_memory(&lc_db::DbConfig::default())
            .await
            .unwrap();
        let start = 1_800_000_000_000;
        let (token, _) = create_secret(&db, start).await.unwrap();
        sqlx::query("UPDATE admin_sessions SET expires_at = ?1")
            .bind(start + 1)
            .execute(db.pool())
            .await
            .unwrap();

        assert!(find(&db, &token, start).await.unwrap().is_some());
        assert!(find(&db, &token, start + 1).await.unwrap().is_none());
    }
}
