//! Persistent audit log of admin mutations.
//!
//! Every mutating request under `/admin/api/**` or `/admin/auth/**` is written
//! to `admin_audit` by the `admin_headers` middleware. Only stable metadata is
//! stored: action name, HTTP method, path (never the query string), response
//! status, actor (from the session cookie when authenticated), optional guild id
//! parsed from the path and the client IP. Headers and request bodies are never
//! persisted: they may contain secrets.
//!
//! The action name is derived from the method and the route template (for
//! instance `guild.purge`), so it stays stable when a parameter changes.
//! Rows older than [`AUDIT_RETENTION_DAYS`] are deleted at startup.

use std::net::IpAddr;

use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lc_db::{Db, DbError};
use lc_domain::GuildId;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::FromRow;

use crate::app::AppState;

use super::auth::{self, AdminAccess};
use super::session::AdminSession;
use super::{iso_from_ms, require_db, AdminError};

/// Audit retention (90 days).
pub(crate) const AUDIT_RETENTION_DAYS: i64 = 90;

/// Default page size of `GET /admin/api/audit`.
const AUDIT_DEFAULT_LIMIT: usize = 100;

/// Maximum page size of `GET /admin/api/audit` (defensive bound).
const AUDIT_MAX_LIMIT: usize = 500;

/// Maximum number of rows exported as CSV.
const AUDIT_EXPORT_LIMIT: usize = 10_000;

/// Actor of an authenticated mutation.
#[derive(Debug, Clone)]
pub(crate) struct AuditActor {
    /// Session method (`secret` or `discord`).
    pub method: String,
    /// Display name; the session method is used when the session has no name.
    pub name: Option<String>,
}

/// One stored audit row.
#[derive(Debug, Clone, FromRow)]
struct AuditRow {
    id: i64,
    at: i64,
    actor_method: Option<String>,
    actor_name: Option<String>,
    action: String,
    method: String,
    path: String,
    status: i64,
    guild_id: Option<String>,
    client_ip: Option<String>,
}

/// Sub-router of the audit routes.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/api/audit", get(list))
        .route("/admin/api/audit/export", get(export))
}

impl AuditActor {
    /// Actor of an existing or freshly created session.
    pub(crate) fn from_session(session: &AdminSession) -> Self {
        Self {
            method: session.method.clone(),
            name: session
                .display_name
                .clone()
                .or_else(|| Some(session.method.clone())),
        }
    }
}

/// Status written to the audit row instead of the HTTP status.
///
/// A login handler inserts it as a response extension when the HTTP status
/// does not say how the attempt ended: the Discord callback always answers
/// `302`, towards the dashboard or back to the login page with an error.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AuditStatus(pub u16);

/// Path of the Discord OAuth2 callback, audited although it is a `GET`.
const DISCORD_CALLBACK_PATH: &str = "/admin/auth/discord/callback";

/// `true` when the request must leave an audit row: every mutation, plus the
/// Discord callback that opens a session.
pub(crate) fn audited_request(method: &Method, path: &str) -> bool {
    matches!(
        *method,
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    ) || (*method == Method::GET && path == DISCORD_CALLBACK_PATH)
}

/// Resolves the audit actor from the request session, if any.
///
/// Called by `admin_headers` before the handler runs: a `logout` mutation then
/// still knows who performed it. A login has no session yet: its handler
/// attaches the actor to the response instead.
pub(crate) async fn actor_for_request(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Option<AuditActor> {
    let session = auth::current_session(state, headers).await.ok().flatten()?;
    Some(AuditActor::from_session(&session))
}

/// Persists one mutation. Failures are logged and never change the response.
pub(crate) async fn record(
    state: &AppState,
    method: &str,
    path: &str,
    status: i64,
    actor: Option<AuditActor>,
    client_ip: Option<IpAddr>,
) {
    if !audited_path(path) {
        return;
    }
    let Some(db) = state.db.as_ref() else {
        return;
    };

    let action = action_for(method, path);
    let guild_id = guild_id_from_path(path);
    let at = state.clock.now().timestamp_millis();
    let client_ip = client_ip.map(|ip| ip.to_string());
    let (actor_method, actor_name) =
        actor.map_or((None, None), |actor| (Some(actor.method), actor.name));

    let result = sqlx::query(
        "INSERT INTO admin_audit \
            (at, actor_method, actor_name, action, method, path, status, guild_id, client_ip) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(at)
    .bind(actor_method.as_deref())
    .bind(actor_name.as_deref())
    .bind(&action)
    .bind(method)
    .bind(path)
    .bind(status)
    .bind(guild_id.as_deref())
    .bind(client_ip.as_deref())
    .execute(db.pool())
    .await;

    if let Err(error) = &result {
        tracing::warn!(error = %error, %action, %path, "admin audit row insert failed");
    } else if let Ok(outcome) = result {
        // Publish the new entry exactly like `GET /admin/api/audit` renders it.
        state.events.audit(json!({
            "id": outcome.last_insert_rowid(),
            "at": iso_from_ms(at).unwrap_or_default(),
            "actor": {
                "method": actor_method,
                "name": actor_name,
            },
            "action": action,
            "method": method,
            "path": path,
            "status": status,
            "guildId": guild_id,
            "clientIp": client_ip,
        }));
    }
}

/// `true` for the admin surfaces that must be audited.
fn audited_path(path: &str) -> bool {
    path.starts_with("/admin/api/") || path.starts_with("/admin/auth/")
}

/// Deletes rows older than [`AUDIT_RETENTION_DAYS`]; returns the number removed.
pub(crate) async fn purge_old(db: &Db, now: DateTime<Utc>) -> Result<u64, DbError> {
    let cutoff = (now - ChronoDuration::days(AUDIT_RETENTION_DAYS)).timestamp_millis();
    let result = sqlx::query("DELETE FROM admin_audit WHERE at < ?1")
        .bind(cutoff)
        .execute(db.pool())
        .await?;
    Ok(result.rows_affected())
}

/// Query parameters of `GET /admin/api/audit`.
#[derive(Debug, Default, Deserialize)]
pub(super) struct AuditQuery {
    query: Option<String>,
    before: Option<String>,
    limit: Option<String>,
}

impl AuditQuery {
    /// Trimmed search string (empty means no filter).
    fn search(&self) -> String {
        self.query
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    }

    /// Parsed id cursor, or `400 invalid_before`.
    fn before(&self) -> Result<Option<i64>, AdminError> {
        let Some(raw) = self.before.as_deref().map(str::trim) else {
            return Ok(None);
        };
        if raw.is_empty() {
            return Ok(None);
        }
        raw.parse::<i64>()
            .map(Some)
            .map_err(|_| AdminError::bad_request("invalid_before"))
    }

    /// Parsed page size clamped to `[1, AUDIT_MAX_LIMIT]`.
    fn limit(&self) -> Result<usize, AdminError> {
        let Some(raw) = self
            .limit
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        else {
            return Ok(AUDIT_DEFAULT_LIMIT);
        };
        let value = raw
            .parse::<usize>()
            .map_err(|_| AdminError::bad_request("invalid_limit"))?;
        Ok(value.clamp(1, AUDIT_MAX_LIMIT))
    }
}

/// `GET /admin/api/audit?query=&before=&limit=100`.
pub(super) async fn list(
    State(state): State<AppState>,
    _access: AdminAccess,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Value>, AdminError> {
    let db = require_db(&state)?;
    let before = query.before()?;
    let limit = query.limit()?;
    let search = query.search();

    let mut rows = fetch_rows(db, &search, before, limit.saturating_add(1)).await?;
    let next_before = if rows.len() > limit {
        rows.truncate(limit);
        rows.last().map(|row| row.id.to_string())
    } else {
        None
    };
    let items: Vec<Value> = rows.iter().map(row_payload).collect();

    Ok(Json(json!({
        "items": items,
        "nextBefore": next_before,
    })))
}

/// `GET /admin/api/audit/export` (CSV attachment).
pub(super) async fn export(
    State(state): State<AppState>,
    _access: AdminAccess,
    Query(query): Query<AuditQuery>,
) -> Result<Response, AdminError> {
    let db = require_db(&state)?;
    let before = query.before()?;
    let search = query.search();
    let rows = fetch_rows(db, &search, before, AUDIT_EXPORT_LIMIT).await?;

    let mut csv =
        String::from("id,at,actorMethod,actorName,action,method,path,status,guildId,clientIp\n");
    for row in &rows {
        let at = iso_from_ms(row.at).unwrap_or_default();
        csv.push_str(&csv_record(row, &at));
    }

    let headers = [
        (header::CONTENT_TYPE, HeaderValue::from_static("text/csv")),
        (
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"admin-audit.csv\""),
        ),
    ];
    Ok((headers, csv).into_response())
}

/// Reads audit rows with the optional search and id cursor, newest first.
async fn fetch_rows(
    db: &Db,
    search: &str,
    before: Option<i64>,
    limit: usize,
) -> Result<Vec<AuditRow>, AdminError> {
    sqlx::query_as::<_, AuditRow>(
        "SELECT id, at, actor_method, actor_name, action, method, path, status, \
                guild_id, client_ip \
           FROM admin_audit \
          WHERE (?1 IS NULL OR id < ?1) \
            AND (?2 = '' \
                 OR instr(lower(action), lower(?2)) > 0 \
                 OR instr(lower(path), lower(?2)) > 0 \
                 OR instr(lower(COALESCE(actor_name, '')), lower(?2)) > 0 \
                 OR instr(lower(COALESCE(actor_method, '')), lower(?2)) > 0 \
                 OR instr(COALESCE(guild_id, ''), ?2) > 0) \
          ORDER BY id DESC \
          LIMIT ?3",
    )
    .bind(before)
    .bind(search)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(db.pool())
    .await
    .map_err(|error| AdminError::db("reading admin audit", error))
}

/// JSON payload of one audit row.
fn row_payload(row: &AuditRow) -> Value {
    json!({
        "id": row.id,
        "at": iso_from_ms(row.at).unwrap_or_default(),
        "actor": {
            "method": row.actor_method,
            "name": row.actor_name,
        },
        "action": row.action,
        "method": row.method,
        "path": row.path,
        "status": row.status,
        "guildId": row.guild_id,
        "clientIp": row.client_ip,
    })
}

/// Escapes one CSV field (quotes doubled when needed).
///
/// Formula-leading values are prefixed with `'`: the actor name comes from
/// Discord and a spreadsheet must never execute it.
fn csv_field(value: &str) -> String {
    let mut value = value.to_owned();
    if value
        .chars()
        .next()
        .is_some_and(|character| matches!(character, '=' | '+' | '-' | '@' | '\t' | '\r'))
    {
        value.insert(0, '\'');
    }
    let needs_quotes = value
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\n' | '\r'));
    let escaped = value.replace('"', "\"\"");
    if needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

/// One CSV line (without the header).
fn csv_record(row: &AuditRow, at: &str) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{}\n",
        row.id,
        csv_field(at),
        csv_field(row.actor_method.as_deref().unwrap_or_default()),
        csv_field(row.actor_name.as_deref().unwrap_or_default()),
        csv_field(&row.action),
        csv_field(&row.method),
        csv_field(&row.path),
        row.status,
        csv_field(row.guild_id.as_deref().unwrap_or_default()),
        csv_field(row.client_ip.as_deref().unwrap_or_default()),
    )
}

/// Stable short action name derived from method and route template.
pub(crate) fn action_for(method: &str, path: &str) -> String {
    if let Some(action) = auth_action(method, path) {
        return action.to_owned();
    }
    if let Some(action) = guild_action(method, path) {
        return action.to_owned();
    }
    if let Some(action) = overlay_action(method, path) {
        return action.to_owned();
    }
    if let Some(action) = client_action(method, path) {
        return action.to_owned();
    }
    if let Some(action) = media_action(method, path) {
        return action.to_owned();
    }
    fallback_action(method, path)
}

fn auth_action(method: &str, path: &str) -> Option<&'static str> {
    match (method, path) {
        ("POST", "/admin/auth/secret") => Some("auth.login_secret"),
        ("GET", DISCORD_CALLBACK_PATH) => Some("auth.login_discord"),
        ("POST", "/admin/auth/logout") => Some("auth.logout"),
        _ => None,
    }
}

fn guild_action(method: &str, path: &str) -> Option<&'static str> {
    if path == "/admin/api/guilds" {
        return (method == "POST").then_some("guild.create");
    }
    let rest = path.strip_prefix("/admin/api/guilds/")?;
    let mut segments = rest.split('/');
    let _guild_id = segments.next()?;
    let suffix: Vec<&str> = segments.collect();
    match (method, suffix.as_slice()) {
        ("POST", ["admin-code"]) => Some("guild.admin_code"),
        ("PATCH", ["settings"]) => Some("guild.settings"),
        ("POST", ["stop"]) => Some("guild.stop"),
        ("POST", ["purge"]) => Some("guild.purge"),
        ("POST", ["revoke-clients"]) => Some("guild.revoke_clients"),
        ("POST", ["media-board", "clear"]) => Some("guild.board_clear"),
        ("POST", ["discord-unlink"]) => Some("guild.discord_unlink"),
        ("POST", ["discord-leave"]) => Some("guild.discord_leave"),
        ("POST", ["discord-link-code"]) => Some("guild.discord_link_code"),
        ("POST", ["device-code"]) => Some("guild.device_code"),
        ("POST", ["invites"]) => Some("guild.invite_create"),
        ("POST", ["invites", _, "revoke"]) => Some("guild.invite_revoke"),
        ("PATCH", ["members", _]) => Some("guild.member_update"),
        ("DELETE", ["members", _]) => Some("guild.member_remove"),
        ("POST", ["members", _, "ban"]) => Some("guild.member_ban"),
        ("DELETE", ["bans", _]) => Some("guild.ban_remove"),
        ("DELETE", ["queue"]) => Some("guild.queue_clear"),
        ("DELETE", ["queue", _]) => Some("guild.queue_remove"),
        ("PATCH", ["media-board", "items", _]) => Some("guild.media_update"),
        ("DELETE", ["media-board", "items", _]) => Some("guild.media_remove"),
        ("POST", ["media-board", "items", _, "edit"]) => Some("guild.media_edit"),
        _ => None,
    }
}

fn overlay_action(method: &str, path: &str) -> Option<&'static str> {
    let rest = path.strip_prefix("/admin/api/overlay-clients/")?;
    let mut segments = rest.split('/');
    let _client_id = segments.next()?;
    let suffix: Vec<&str> = segments.collect();
    match (method, suffix.as_slice()) {
        ("POST", ["revoke"]) => Some("overlay.revoke"),
        ("PATCH", []) => Some("overlay.update"),
        ("PATCH", ["guild-settings"]) => Some("overlay.guild_settings"),
        _ => None,
    }
}

fn client_action(method: &str, path: &str) -> Option<&'static str> {
    if path == "/admin/api/ingest-clients" {
        return (method == "POST").then_some("ingest_client.create");
    }
    if let Some(rest) = path.strip_prefix("/admin/api/ingest-clients/") {
        let mut segments = rest.split('/');
        let _client_id = segments.next()?;
        let suffix: Vec<&str> = segments.collect();
        if method == "POST" && suffix == ["revoke"] {
            return Some("ingest_client.revoke");
        }
    }
    if path == "/admin/api/pairing-codes/expired" && method == "DELETE" {
        return Some("pairing.expired_clean");
    }
    if let Some(rest) = path.strip_prefix("/admin/api/pairing-codes/") {
        let mut segments = rest.split('/');
        let _code = segments.next()?;
        let suffix: Vec<&str> = segments.collect();
        if method == "POST" && suffix == ["revoke"] {
            return Some("pairing.revoke");
        }
    }
    None
}

fn media_action(method: &str, path: &str) -> Option<&'static str> {
    match (method, path) {
        ("POST", "/admin/api/media/cache/purge") => Some("media.cache_purge"),
        ("POST", "/admin/api/media/orphans/clean") => Some("media.orphans_clean"),
        ("PATCH", "/admin/api/runtime-settings") => Some("settings.update"),
        ("PUT", "/admin/api/rights") => Some("rights.update"),
        _ => None,
    }
}

/// Conservative fallback for routes added later: `admin.<last segment>`.
fn fallback_action(method: &str, path: &str) -> String {
    let prefix = if path.starts_with("/admin/auth/") {
        "auth"
    } else {
        "admin"
    };
    let last = path
        .trim_matches('/')
        .rsplit('/')
        .find(|segment| !segment.is_empty() && !looks_like_identifier(segment))
        .map(|segment| {
            segment
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() {
                        character.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| method.to_ascii_lowercase());
    format!("{prefix}.{last}")
}

/// `true` for numeric ids, UUIDs and other path parameters.
fn looks_like_identifier(segment: &str) -> bool {
    segment.chars().all(|character| character.is_ascii_digit())
        || (segment.len() == 36
            && segment
                .chars()
                .all(|character| character.is_ascii_hexdigit() || character == '-'))
}

/// Parses the guild id from `/admin/api/guilds/{id}/…`.
pub(crate) fn guild_id_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/admin/api/guilds/")?;
    let raw = rest.split('/').next()?;
    GuildId::parse(raw)
        .ok()
        .map(|guild_id| guild_id.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn action_names_are_stable_for_known_routes() {
        assert_eq!(
            action_for("POST", "/admin/api/guilds/42/purge"),
            "guild.purge"
        );
        assert_eq!(
            action_for("POST", "/admin/api/guilds/42/stop"),
            "guild.stop"
        );
        assert_eq!(
            action_for("POST", "/admin/api/overlay-clients/abc/revoke"),
            "overlay.revoke"
        );
        assert_eq!(
            action_for("PATCH", "/admin/api/guilds/42/members/member-1"),
            "guild.member_update"
        );
        assert_eq!(
            action_for("DELETE", "/admin/api/guilds/42/queue/job-1"),
            "guild.queue_remove"
        );
        assert_eq!(
            action_for("POST", "/admin/api/media/cache/purge"),
            "media.cache_purge"
        );
        assert_eq!(action_for("POST", "/admin/auth/logout"), "auth.logout");
        assert_eq!(
            action_for("GET", "/admin/auth/discord/callback"),
            "auth.login_discord"
        );
    }

    #[test]
    fn mutations_and_the_discord_callback_are_audited() {
        for method in [Method::POST, Method::PATCH, Method::PUT, Method::DELETE] {
            assert!(audited_request(&method, "/admin/api/guilds"));
        }
        assert!(audited_request(
            &Method::GET,
            "/admin/auth/discord/callback"
        ));
        assert!(!audited_request(&Method::GET, "/admin/auth/discord"));
        assert!(!audited_request(&Method::GET, "/admin/api/overview"));
    }

    #[test]
    fn fallback_action_ignores_path_parameters() {
        assert_eq!(
            action_for("POST", "/admin/api/guilds/42/future-action"),
            "admin.future_action"
        );
        assert_eq!(
            action_for("DELETE", "/admin/api/123456789012345678"),
            "admin.api"
        );
    }

    #[test]
    fn guild_id_is_parsed_only_from_guild_routes() {
        assert_eq!(
            guild_id_from_path("/admin/api/guilds/42/purge").as_deref(),
            Some("42")
        );
        assert!(guild_id_from_path("/admin/api/overview").is_none());
        assert!(guild_id_from_path("/admin/api/guilds/not-a-number/stop").is_none());
    }

    #[test]
    fn csv_fields_are_escaped() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_field("a\nb"), "\"a\nb\"");
        assert_eq!(csv_field("=cmd"), "'=cmd");
    }
}
