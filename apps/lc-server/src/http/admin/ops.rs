//! Operational admin routes: health, ingestion history and media maintenance.
//!
//! The public contract lives in `docs/rust/admin-console-api.md` (sections 3
//! and 4). This module only wires read-only diagnostics and two explicit
//! maintenance actions; every handler is protected by [`AdminAccess`] exactly
//! like the rest of `/admin/api/**`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lc_db::Db;
use lc_domain::GuildId;
use lc_media::storage::{is_staged_file_name, MediaStorage};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::FromRow;

use crate::app::AppState;
use crate::http::health::{HealthState, DATABASE_CHECK_TIMEOUT};

use super::auth::AdminAccess;
use super::{opt_iso, require_db, to_iso, AdminError};
use crate::ingestions::{
    FinishedIngestion, IngestionEntry, IngestionErrorInfo, IngestionStatus, HISTORY_LIMIT,
};

/// Maximum accepted `limit` for `GET /admin/api/ingestions`.
const INGESTIONS_DEFAULT_LIMIT: usize = 50;

/// Timeout of one binary version probe.
#[cfg_attr(test, allow(dead_code))]
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// Lifetime of a cached binary version.
const VERSION_CACHE_TTL: ChronoDuration = ChronoDuration::seconds(60);

/// Number of ingestion failures in the last hour that degrades the status to
/// `warn`.
const INGESTION_FAILURE_WARN_THRESHOLD: usize = 3;

/// Outcome of one binary version probe.
#[derive(Debug, Clone)]
pub(crate) struct BinaryProbeResult {
    /// `true` when the binary answered successfully.
    pub ok: bool,
    /// Version parsed from the first output line, when available.
    pub version: Option<String>,
    /// Readable detail shown in the health check.
    pub detail: String,
}

impl BinaryProbeResult {
    /// Successful probe with a readable detail.
    fn ok(detail: impl Into<String>, version: Option<String>) -> Self {
        Self {
            ok: true,
            version,
            detail: detail.into(),
        }
    }

    /// Failed probe; the detail is never a secret (only a command error).
    #[cfg_attr(test, allow(dead_code))]
    fn error(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            version: None,
            detail: detail.into(),
        }
    }
}

/// Injectable version probe (tests never spawn a real binary).
#[async_trait]
pub(crate) trait BinaryProbe: Send + Sync {
    /// Probes `binary` (`--version`, then `-version`).
    async fn probe(&self, binary: &str, now: DateTime<Utc>) -> BinaryProbeResult;
}

/// Cached result of one binary probe.
#[derive(Debug, Clone)]
struct CachedProbe {
    at: DateTime<Utc>,
    result: BinaryProbeResult,
}

/// System implementation: runs the binary once, caches the result for 60 s.
#[derive(Debug, Default)]
pub(crate) struct SystemBinaryProbe {
    cache: Mutex<HashMap<String, CachedProbe>>,
}

impl SystemBinaryProbe {
    /// Creates an empty probe cache.
    pub fn new() -> Self {
        Self::default()
    }

    fn cached(&self, binary: &str, now: DateTime<Utc>) -> Option<BinaryProbeResult> {
        let cache = self.cache.lock().ok()?;
        let cached = cache.get(binary)?;
        (now - cached.at < VERSION_CACHE_TTL).then(|| cached.result.clone())
    }

    fn store(&self, binary: &str, now: DateTime<Utc>, result: &BinaryProbeResult) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(
                binary.to_owned(),
                CachedProbe {
                    at: now,
                    result: result.clone(),
                },
            );
        }
    }
}

#[async_trait]
impl BinaryProbe for SystemBinaryProbe {
    async fn probe(&self, binary: &str, now: DateTime<Utc>) -> BinaryProbeResult {
        if let Some(cached) = self.cached(binary, now) {
            return cached;
        }

        // Unit tests inject a fake probe. This guard is a second safety net so
        // no test can accidentally spawn ffmpeg/yt-dlp/piper on a build agent.
        #[cfg(test)]
        let result = BinaryProbeResult::ok("test probe", Some("test".to_owned()));
        #[cfg(not(test))]
        let result = match run_version(binary, "--version").await {
            Ok(result) => result,
            Err(first_error) => match run_version(binary, "-version").await {
                Ok(result) => result,
                Err(second_error) => {
                    BinaryProbeResult::error(format!("{first_error}; {second_error}"))
                }
            },
        };

        self.store(binary, now, &result);
        result
    }
}

/// Runs `binary <flag>` with a three-second timeout and parses its version.
#[cfg(not(test))]
async fn run_version(binary: &str, flag: &str) -> Result<BinaryProbeResult, String> {
    let mut command = tokio::process::Command::new(binary);
    command.arg(flag).kill_on_drop(true);
    let output = match tokio::time::timeout(VERSION_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Err(format!("cannot run `{binary} {flag}`: {error}"));
        }
        Err(_) => {
            return Err(format!(
                "`{binary} {flag}` timed out after {} s",
                VERSION_TIMEOUT.as_secs()
            ));
        }
    };

    if !output.status.success() {
        return Err(format!(
            "`{binary} {flag}` exited with status {}",
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = if stdout.trim().is_empty() {
        stderr.as_ref()
    } else {
        stdout.as_ref()
    };
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if line.is_empty() {
        return Ok(BinaryProbeResult::ok(
            format!("`{binary} {flag}` answered without a version"),
            None,
        ));
    }
    Ok(BinaryProbeResult::ok(
        line.to_owned(),
        parse_version_token(line),
    ))
}

/// First token that looks like a version number (`7.1.1`, `2025.01.15`).
#[cfg_attr(test, allow(dead_code))]
fn parse_version_token(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+'))
        });
        let candidate = token.trim_start_matches('v');
        let mut characters = candidate.chars();
        let starts_with_digit = characters
            .next()
            .is_some_and(|value| value.is_ascii_digit());
        (starts_with_digit && candidate.contains('.')).then(|| candidate.to_owned())
    })
}

/// Sub-router of the operational routes.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/api/health", get(health))
        .route("/admin/api/ingestions", get(ingestions))
        .route("/admin/api/media/cache/purge", post(purge_cache))
        .route("/admin/api/media/orphans/clean", post(clean_orphans))
}

/// One check of `GET /admin/api/health`.
#[derive(Debug, serde::Serialize)]
pub(super) struct HealthCheck {
    id: &'static str,
    status: &'static str,
    detail: String,
    version: Option<String>,
    /// Number behind the check, when it has one (failed ingestions).
    count: Option<u64>,
}

/// Body of `GET /admin/api/health`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HealthResponse {
    status: &'static str,
    checks: Vec<HealthCheck>,
    uptime_sec: i64,
    server_version: &'static str,
}

/// `GET /admin/api/health`.
pub(super) async fn health(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Json<HealthResponse> {
    let now = state.clock.now();
    let (database, ffmpeg, ytdlp, piper) = tokio::join!(
        database_check(state.db.as_ref()),
        binary_check(
            "ffmpeg",
            &state.config.ffmpeg_binary,
            state.binary_probe.as_ref(),
            now
        ),
        binary_check(
            "ytdlp",
            &state.config.ytdlp_binary,
            state.binary_probe.as_ref(),
            now
        ),
        binary_check(
            "piper",
            &state.config.piper_bin,
            state.binary_probe.as_ref(),
            now
        ),
    );
    let checks = vec![
        database,
        discord_check(state.discord.state()),
        ffmpeg,
        ytdlp,
        piper,
        ingestions_check(&state, now),
    ];
    let degraded = checks.iter().any(|check| check.status == "error");

    Json(HealthResponse {
        status: if degraded { "degraded" } else { "ok" },
        checks,
        uptime_sec: (now - state.started_at).num_seconds().max(0),
        server_version: env!("CARGO_PKG_VERSION"),
    })
}

/// Runs a simple `SELECT 1` query with the health timeout.
async fn database_check(db: Option<&Db>) -> HealthCheck {
    let Some(db) = db else {
        return HealthCheck {
            id: "database",
            status: "warn",
            detail: "database is not configured".to_owned(),
            version: None,
            count: None,
        };
    };
    match tokio::time::timeout(
        DATABASE_CHECK_TIMEOUT,
        sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(db.pool()),
    )
    .await
    {
        Ok(Ok(_)) => HealthCheck {
            id: "database",
            status: "ok",
            detail: "database reachable".to_owned(),
            version: None,
            count: None,
        },
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "admin health database check failed");
            HealthCheck {
                id: "database",
                status: "error",
                detail: "database query failed".to_owned(),
                version: None,
                count: None,
            }
        }
        Err(_) => {
            tracing::warn!("admin health database check timed out");
            HealthCheck {
                id: "database",
                status: "error",
                detail: "database query timed out".to_owned(),
                version: None,
                count: None,
            }
        }
    }
}

/// Maps the existing Discord [`HealthState`] to the admin health vocabulary.
fn discord_check(state: HealthState) -> HealthCheck {
    match state {
        HealthState::Ok => HealthCheck {
            id: "discord",
            status: "ok",
            detail: "bot connected".to_owned(),
            version: None,
            count: None,
        },
        HealthState::Down => HealthCheck {
            id: "discord",
            status: "error",
            detail: "bot configured but not connected".to_owned(),
            version: None,
            count: None,
        },
        HealthState::Disabled => HealthCheck {
            id: "discord",
            status: "warn",
            detail: "Discord is not configured".to_owned(),
            version: None,
            count: None,
        },
    }
}

/// Probes one external binary and converts the result to a health check.
async fn binary_check(
    id: &'static str,
    binary: &str,
    probe: &dyn BinaryProbe,
    now: DateTime<Utc>,
) -> HealthCheck {
    let result = probe.probe(binary, now).await;
    HealthCheck {
        id,
        status: if result.ok { "ok" } else { "error" },
        detail: result.detail,
        version: result.version,
        count: None,
    }
}

/// Warns when more than three ingestions failed during the last hour.
fn ingestions_check(state: &AppState, now: DateTime<Utc>) -> HealthCheck {
    let since = now - ChronoDuration::hours(1);
    let failures = state.ingestions.failure_count_since(since);
    let status = if failures > INGESTION_FAILURE_WARN_THRESHOLD {
        "warn"
    } else {
        "ok"
    };
    HealthCheck {
        id: "ingestions",
        status,
        detail: format!("{failures} failure(s) in the last hour"),
        version: None,
        count: Some(u64::try_from(failures).unwrap_or(u64::MAX)),
    }
}

/// Query parameters of `GET /admin/api/ingestions`.
#[derive(Debug, Deserialize)]
pub(super) struct IngestionsQuery {
    limit: Option<usize>,
}

/// Unified row rendered by the ingestion history endpoint.
struct IngestionRow {
    id: String,
    guild_id: GuildId,
    item_id: Option<String>,
    status: &'static str,
    error: Option<String>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

/// `GET /admin/api/ingestions?limit=50`.
pub(super) async fn ingestions(
    State(state): State<AppState>,
    _access: AdminAccess,
    Query(query): Query<IngestionsQuery>,
) -> Result<Json<Value>, AdminError> {
    let limit = query
        .limit
        .unwrap_or(INGESTIONS_DEFAULT_LIMIT)
        .clamp(1, HISTORY_LIMIT);
    let now = state.clock.now();

    let mut rows: Vec<IngestionRow> = state
        .ingestions
        .finished()
        .into_iter()
        .map(finished_row)
        .collect();
    rows.extend(state.ingestions.active(now).into_iter().map(active_row));
    rows.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    rows.truncate(limit);

    let guild_names = guild_names(state.db.as_ref(), &rows).await?;
    let source_urls = source_urls(state.db.as_ref(), &rows).await?;

    let items: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let source_url = row
                .item_id
                .as_deref()
                .and_then(|item_id| source_urls.get(item_id))
                .cloned()
                .unwrap_or_default();
            let source = if source_url.starts_with("upload:") {
                "upload"
            } else if source_url.is_empty() {
                "unknown"
            } else {
                "link"
            };
            let duration_ms = row
                .finished_at
                .unwrap_or(now)
                .signed_duration_since(row.started_at)
                .num_milliseconds()
                .max(0);
            json!({
                "id": row.id,
                "guildId": row.guild_id.as_str(),
                "guildName": guild_names.get(row.guild_id.as_str()).cloned().flatten(),
                "source": source,
                "url": source_url,
                "status": row.status,
                "error": row.error,
                "startedAt": to_iso(row.started_at),
                "finishedAt": opt_iso(row.finished_at),
                "durationMs": duration_ms,
            })
        })
        .collect();

    Ok(Json(json!({ "items": items })))
}

/// Converts a finished ingestion to the unified rendering row.
fn finished_row(entry: FinishedIngestion) -> IngestionRow {
    let status = match entry.status {
        IngestionStatus::Ready => "done",
        IngestionStatus::Failed => "failed",
        IngestionStatus::Processing => "running",
    };
    IngestionRow {
        id: entry.ingestion_id,
        guild_id: entry.guild_id,
        item_id: entry.item_id,
        status,
        error: entry.error.map(ingestion_error_text),
        started_at: entry.started_at,
        finished_at: Some(entry.finished_at),
    }
}

/// Converts an in-flight ingestion to the unified rendering row.
fn active_row(entry: IngestionEntry) -> IngestionRow {
    IngestionRow {
        id: entry.ingestion_id,
        guild_id: entry.guild_id,
        item_id: entry.item_id,
        status: "running",
        error: entry.error.map(ingestion_error_text),
        started_at: entry.created_at,
        finished_at: None,
    }
}

/// Readable error text: the message when present, otherwise the machine code.
fn ingestion_error_text(error: IngestionErrorInfo) -> String {
    error.message.unwrap_or(error.code)
}

/// Reads the display name of every guild referenced by `rows`.
async fn guild_names(
    db: Option<&Db>,
    rows: &[IngestionRow],
) -> Result<HashMap<String, Option<String>>, AdminError> {
    let Some(db) = db else {
        return Ok(HashMap::new());
    };
    let mut names = HashMap::new();
    for row in rows {
        let id = row.guild_id.as_str();
        if names.contains_key(id) {
            continue;
        }
        let name = db
            .guilds()
            .get(&row.guild_id)
            .await
            .map_err(|error| AdminError::db("reading ingestion guild name", error))?
            .and_then(|guild| guild.name);
        names.insert(id.to_owned(), name);
    }
    Ok(names)
}

/// Reads the source URL of every board item referenced by `rows`.
async fn source_urls(
    db: Option<&Db>,
    rows: &[IngestionRow],
) -> Result<HashMap<String, String>, AdminError> {
    let Some(db) = db else {
        return Ok(HashMap::new());
    };
    let mut urls = HashMap::new();
    for item_id in rows.iter().filter_map(|row| row.item_id.as_deref()) {
        if urls.contains_key(item_id) {
            continue;
        }
        let url = sqlx::query_scalar::<_, String>(
            "SELECT assets.source_url \
               FROM media_board_items AS items \
               JOIN media_assets AS assets ON assets.id = items.media_asset_id \
              WHERE items.id = ?1",
        )
        .bind(item_id)
        .fetch_optional(db.pool())
        .await
        .map_err(|error| AdminError::db("reading ingestion source URL", error))?;
        if let Some(url) = url {
            urls.insert(item_id.to_owned(), url);
        }
    }
    Ok(urls)
}

/// One media asset row used by the maintenance actions.
#[derive(Debug, FromRow)]
struct AssetRow {
    id: String,
    storage_path: String,
    size_bytes: i64,
}

/// Counters returned by the maintenance actions.
#[derive(Debug, Default)]
struct MaintenanceReport {
    removed_files: u64,
    freed_bytes: u64,
}

impl MaintenanceReport {
    fn payload(&self) -> Value {
        json!({
            "removedFiles": self.removed_files,
            "freedBytes": self.freed_bytes,
        })
    }
}

/// Resolves and canonicalizes the media root, failing closed.
fn storage_for(state: &AppState) -> Result<MediaStorage, AdminError> {
    let root = std::fs::canonicalize(&state.config.media_storage_dir).map_err(|error| {
        tracing::error!(
            directory = %state.config.media_storage_dir.display(),
            %error,
            "media maintenance: media root cannot be canonicalized"
        );
        AdminError::Internal
    })?;
    Ok(MediaStorage::from_root(root))
}

/// Canonical key of a stored reference, or `None` for an empty path.
fn reference_key(root: &Path, reference: &str) -> Option<PathBuf> {
    if reference.trim().is_empty() {
        return None;
    }
    let resolved = crate::media::confinement::resolve_reference(root, Path::new(reference));
    Some(std::fs::canonicalize(&resolved).unwrap_or(resolved))
}

/// `POST /admin/api/media/cache/purge`.
pub(super) async fn purge_cache(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Json<Value>, AdminError> {
    let db = require_db(&state)?;
    let storage = storage_for(&state)?;
    let root = storage.root().to_path_buf();

    let assets =
        sqlx::query_as::<_, AssetRow>("SELECT id, storage_path, size_bytes FROM media_assets")
            .fetch_all(db.pool())
            .await
            .map_err(|error| AdminError::db("listing media assets", error))?;

    let mut references: HashMap<PathBuf, usize> = HashMap::with_capacity(assets.len());
    let mut by_id: HashMap<String, AssetRow> = HashMap::with_capacity(assets.len());
    for asset in assets {
        if let Some(key) = reference_key(&root, &asset.storage_path) {
            *references.entry(key).or_insert(0) += 1;
        }
        by_id.insert(asset.id.clone(), asset);
    }

    // A cached asset is READY, not pinned by any media board (item or artwork),
    // and not referenced by a pending/playing job.
    let candidate_ids = sqlx::query_scalar::<_, String>(
        "SELECT id FROM media_assets \
          WHERE status = 'READY' \
            AND id NOT IN (SELECT media_asset_id FROM media_board_items \
                            WHERE media_asset_id IS NOT NULL) \
            AND id NOT IN (SELECT artwork_asset_id FROM media_board_items \
                            WHERE artwork_asset_id IS NOT NULL) \
            AND id NOT IN (SELECT a.cover_asset_id FROM media_assets a \
                             JOIN media_board_items b ON b.media_asset_id = a.id \
                            WHERE a.cover_asset_id IS NOT NULL) \
            AND id NOT IN (SELECT media_asset_id FROM playback_jobs \
                            WHERE media_asset_id IS NOT NULL \
                              AND status IN ('PENDING', 'PLAYING')) \
          ORDER BY last_accessed_at ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|error| AdminError::db("listing cache candidates", error))?;

    let mut report = MaintenanceReport::default();
    for id in candidate_ids {
        let Some(asset) = by_id.remove(&id) else {
            continue;
        };
        let affected = sqlx::query("DELETE FROM media_assets WHERE id = ?1")
            .bind(&id)
            .execute(db.pool())
            .await
            .map_err(|error| AdminError::db("deleting cached media", error))?
            .rows_affected();
        if affected == 0 {
            continue;
        }

        let Some(key) = reference_key(&root, &asset.storage_path) else {
            continue;
        };
        let last_reference = match references.get_mut(&key) {
            Some(count) if *count > 0 => {
                *count -= 1;
                *count == 0
            }
            _ => false,
        };
        if !last_reference {
            continue;
        }
        let removed = storage
            .remove_path(Path::new(&asset.storage_path))
            .map_err(|error| AdminError::db("removing a cached media file", error))?;
        if removed {
            report.removed_files = report.removed_files.saturating_add(1);
            // The stored size is the canonical cache accounting (same value as
            // `overview.cache.usedBytes`), even when the file itself is missing
            // or has a different on-disk size.
            report.freed_bytes = report
                .freed_bytes
                .saturating_add(u64::try_from(asset.size_bytes.max(0)).unwrap_or(0));
        }
    }

    tracing::info!(
        removed_files = report.removed_files,
        freed_bytes = report.freed_bytes,
        "admin media cache purge"
    );
    Ok(Json(report.payload()))
}

/// `POST /admin/api/media/orphans/clean`.
pub(super) async fn clean_orphans(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Json<Value>, AdminError> {
    let db = require_db(&state)?;
    let storage = storage_for(&state)?;
    let root = storage.root().to_path_buf();

    let known_paths = sqlx::query_scalar::<_, String>(
        "SELECT storage_path FROM media_assets WHERE storage_path != ''",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|error| AdminError::db("listing referenced media files", error))?;
    let known: HashSet<PathBuf> = known_paths
        .iter()
        .filter_map(|path| reference_key(&root, path))
        .collect();

    let now_ms = state.clock.now().timestamp_millis();
    let grace_ms = crate::media::purge::ORPHAN_GRACE.num_milliseconds();
    let variants = storage
        .list_variants()
        .map_err(|error| AdminError::db("listing stored media files", error))?;

    let mut report = MaintenanceReport::default();
    for variant in variants {
        let canonical =
            std::fs::canonicalize(&variant.path).unwrap_or_else(|_| variant.path.clone());
        if known.contains(&canonical) {
            continue;
        }
        // Staged files (`*.tmp-*`) can belong to an ingestion still writing its
        // output: only remove them once the orphan grace elapsed. A finalized
        // file is not written in place, so an unreferenced one is a real orphan.
        let name = variant
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if is_staged_file_name(name) && now_ms.saturating_sub(variant.modified_ms) < grace_ms {
            continue;
        }
        let removed = storage
            .remove_path(&variant.path)
            .map_err(|error| AdminError::db("removing an orphan media file", error))?;
        if removed {
            report.removed_files = report.removed_files.saturating_add(1);
            report.freed_bytes = report.freed_bytes.saturating_add(variant.size_bytes);
        }
    }

    tracing::info!(
        removed_files = report.removed_files,
        freed_bytes = report.freed_bytes,
        "admin media orphan cleanup"
    );
    Ok(Json(report.payload()))
}
