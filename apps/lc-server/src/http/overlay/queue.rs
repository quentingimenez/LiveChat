//! File d'attente d'une guilde, vue et modérée depuis l'overlay.
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `GET` | `/overlay/queue` | tout membre (`ViewQueue`) |
//! | `GET` | `/overlay/queue/history` | tout membre (`ViewQueue`) |
//! | `DELETE` | `/overlay/queue/{jobId}` | admin (`ModerateQueue`) |
//! | `DELETE` | `/overlay/queue` | admin (`ModerateQueue`) |
//!
//! La file est lue dans l'ordre du scheduler (`priority DESC`,
//! `submission_date ASC`, `id ASC`), le job `PLAYING` en tête.
//!
//! Retirer un job en attente le passe `FAILED` : c'est la seule transition
//! `PENDING → terminal` admise par la machine d'état
//! (`JobStatus::can_transition_to`). Le job en cours ne se retire pas ici : il
//! s'arrête par `stop.request` (WS), qui libère proprement la lecture.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use lc_db::Db;
use lc_domain::{GuildId, Permission, PlaybackJobId};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row as _;

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::guild::require;
use super::{audit as overlay_audit, db, db_failure, json_response, now};

/// Nombre maximal de jobs renvoyés (garde-fou d'affichage).
const QUEUE_LIMIT: i64 = 200;
/// Taille de page par défaut de l'historique.
const HISTORY_DEFAULT_LIMIT: i64 = 30;
/// Taille de page maximale de l'historique.
const HISTORY_MAX_LIMIT: i64 = 100;

/// Date ISO 8601 à partir de millisecondes epoch, ou `null`.
fn iso_ms(millis: Option<i64>) -> Value {
    millis
        .and_then(lc_domain::ms_to_datetime)
        .map(|date| json!(lc_domain::time::to_rfc3339_millis(date)))
        .unwrap_or(Value::Null)
}

/// Lit la file (job en cours puis jobs en attente) en une requête.
async fn load_queue(db: &Db, guild_id: &GuildId) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT j.id, j.status, j.text, j.author_name, j.author_image, j.duration_sec, \
                j.submission_date, j.started_at, j.media_asset_id, \
                a.kind AS media_kind, \
                (SELECT b.title FROM media_board_items b \
                  WHERE b.guild_id = j.guild_id AND b.media_asset_id = j.media_asset_id) \
                    AS title \
           FROM playback_jobs j \
           LEFT JOIN media_assets a ON a.id = j.media_asset_id \
          WHERE j.guild_id = ? AND j.status IN ('PLAYING', 'PENDING') \
          ORDER BY CASE j.status WHEN 'PLAYING' THEN 0 ELSE 1 END, \
                   j.priority DESC, j.submission_date ASC, j.id ASC \
          LIMIT ?",
    )
    .bind(guild_id.as_str())
    .bind(QUEUE_LIMIT)
    .fetch_all(db.pool())
    .await?;

    rows.iter()
        .map(|row| {
            let status: String = row.try_get("status")?;
            let kind: Option<String> = row.try_get("media_kind")?;
            let media_asset_id: Option<String> = row.try_get("media_asset_id")?;
            // Origin of the media: a board item (`mediaAssetId`) or a one-off
            // play / text job sent straight to the queue.
            let origin = if media_asset_id.is_some() {
                "board"
            } else {
                "direct"
            };
            Ok(json!({
                "id": row.try_get::<String, _>("id")?,
                "status": status.to_ascii_lowercase(),
                "title": row.try_get::<Option<String>, _>("title")?,
                "text": row.try_get::<Option<String>, _>("text")?,
                "authorName": row.try_get::<Option<String>, _>("author_name")?,
                "authorImage": row.try_get::<Option<String>, _>("author_image")?,
                "durationSec": row.try_get::<i64, _>("duration_sec")?,
                "mediaAssetId": media_asset_id,
                "origin": origin,
                "kind": kind.map(|kind| kind.to_ascii_lowercase()),
                "submittedAt": iso_ms(Some(row.try_get::<i64, _>("submission_date")?)),
                "startedAt": iso_ms(row.try_get::<Option<i64>, _>("started_at")?),
            }))
        })
        .collect()
}

/// Queue body shared by `GET /overlay/queue` and the admin guild detail.
pub(crate) async fn queue_payload(state: &AppState, guild_id: &GuildId) -> Result<Value, Response> {
    let db = db(state)?;
    let jobs = load_queue(db, guild_id).await.map_err(|sqlx_error| {
        db_failure("overlay_queue.list", &lc_db::DbError::from(sqlx_error))
    })?;
    let (playing, pending): (Vec<Value>, Vec<Value>) =
        jobs.into_iter().partition(|job| job["status"] == "playing");

    Ok(json!({
        "playing": playing.into_iter().next(),
        "pending": pending,
        "serverTime": iso_ms(Some(lc_domain::datetime_to_ms(now(state)))),
    }))
}

/// Failure of a queue mutation, reported as the overlay route does.
fn queue_db_failure(operation: &'static str, sqlx_error: sqlx::Error) -> Response {
    db_failure(operation, &lc_db::DbError::from(sqlx_error))
}

/// Removes one pending job of the guild.
pub(crate) async fn remove_job_payload(
    state: &AppState,
    guild_id: &GuildId,
    job_id_raw: &str,
) -> Result<Value, Response> {
    let Ok(job_id) = job_id_raw.trim().parse::<PlaybackJobId>() else {
        return Ok(json!({"removed": false}));
    };
    let db = db(state)?;
    let result = sqlx::query(
        "UPDATE playback_jobs SET status = 'FAILED', finished_at = ? \
         WHERE id = ? AND guild_id = ? AND status = 'PENDING'",
    )
    .bind(lc_domain::datetime_to_ms(now(state)))
    .bind(job_id.to_canonical())
    .bind(guild_id.as_str())
    .execute(db.pool())
    .await
    .map_err(|sqlx_error| queue_db_failure("overlay_queue.remove", sqlx_error))?;
    Ok(json!({"removed": result.rows_affected() > 0}))
}

/// Fails every pending job of the guild.
pub(crate) async fn clear_queue_payload(
    state: &AppState,
    guild_id: &GuildId,
) -> Result<Value, Response> {
    let db = db(state)?;
    let result = sqlx::query(
        "UPDATE playback_jobs SET status = 'FAILED', finished_at = ? \
         WHERE guild_id = ? AND status = 'PENDING'",
    )
    .bind(lc_domain::datetime_to_ms(now(state)))
    .bind(guild_id.as_str())
    .execute(db.pool())
    .await
    .map_err(|sqlx_error| queue_db_failure("overlay_queue.clear", sqlx_error))?;
    Ok(json!({"removedCount": result.rows_affected()}))
}

/// Handler `GET /overlay/queue`.
pub async fn get_queue(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ViewQueue) {
        return response;
    }
    match queue_payload(&state, auth.guild_id()).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Query parameters of `GET /overlay/queue/history`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct HistoryQuery {
    limit: Option<String>,
}

impl HistoryQuery {
    /// Page size requested, clamped to `[1, HISTORY_MAX_LIMIT]`.
    fn limit(&self) -> Result<i64, ()> {
        let Some(raw) = self
            .limit
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(HISTORY_DEFAULT_LIMIT);
        };
        let value = raw.parse::<i64>().map_err(|_| ())?;
        Ok(value.clamp(1, HISTORY_MAX_LIMIT))
    }
}

/// Last finished/failed plays of the guild, newest first:
/// `{count, items: [{id, status, title, text, authorName, kind, origin,
/// finishedAt}]}`.
async fn history_payload(db: &Db, guild_id: &GuildId, limit: i64) -> Result<Value, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT j.id, j.status, j.text, j.author_name, j.duration_sec, j.finished_at, \
                j.media_asset_id, a.kind AS media_kind, \
                (SELECT b.title FROM media_board_items b \
                  WHERE b.guild_id = j.guild_id AND b.media_asset_id = j.media_asset_id) \
                    AS title \
           FROM playback_jobs j \
           LEFT JOIN media_assets a ON a.id = j.media_asset_id \
          WHERE j.guild_id = ? AND j.status IN ('DONE', 'FAILED') \
            AND j.finished_at IS NOT NULL \
          ORDER BY j.finished_at DESC, j.id DESC \
          LIMIT ?",
    )
    .bind(guild_id.as_str())
    .bind(limit)
    .fetch_all(db.pool())
    .await?;

    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            let status: String = row.try_get("status")?;
            let media_asset_id: Option<String> = row.try_get("media_asset_id")?;
            let kind: Option<String> = row.try_get("media_kind")?;
            Ok(json!({
                "id": row.try_get::<String, _>("id")?,
                "status": status.to_ascii_lowercase(),
                "title": row.try_get::<Option<String>, _>("title")?,
                "text": row.try_get::<Option<String>, _>("text")?,
                "authorName": row.try_get::<Option<String>, _>("author_name")?,
                "durationSec": row.try_get::<i64, _>("duration_sec")?,
                "kind": kind.map(|kind| kind.to_ascii_lowercase()),
                "origin": if media_asset_id.is_some() { "board" } else { "direct" },
                "finishedAt": iso_ms(row.try_get::<Option<i64>, _>("finished_at")?),
            }))
        })
        .collect::<Result<Vec<Value>, sqlx::Error>>()?;

    Ok(json!({"count": items.len(), "items": items}))
}

/// Handler `GET /overlay/queue/history?limit=30`.
pub async fn get_history(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Query(query): Query<HistoryQuery>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ViewQueue) {
        return response;
    }
    let Ok(limit) = query.limit() else {
        return super::error(StatusCode::BAD_REQUEST, "invalid_limit");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    match history_payload(db, auth.guild_id(), limit).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(sqlx_error) => db_failure("overlay_queue.history", &lc_db::DbError::from(sqlx_error)),
    }
}

/// Handler `DELETE /overlay/queue/{jobId}` : retire un job en attente.
pub async fn remove_job(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(job_id_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ModerateQueue) {
        return response;
    }
    match remove_job_payload(&state, auth.guild_id(), &job_id_raw).await {
        Ok(payload) => {
            if payload.get("removed").and_then(Value::as_bool) == Some(true) {
                overlay_audit::record_queue_action(
                    &state,
                    &auth,
                    "overlay.queue_remove",
                    &format!("/overlay/queue/{job_id_raw}"),
                )
                .await;
            }
            json_response(StatusCode::OK, payload)
        }
        Err(response) => response,
    }
}

/// Handler `DELETE /overlay/queue` : vide les jobs en attente de la guilde.
pub async fn clear_queue(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ModerateQueue) {
        return response;
    }
    match clear_queue_payload(&state, auth.guild_id()).await {
        Ok(payload) => {
            if payload
                .get("removedCount")
                .and_then(Value::as_i64)
                .is_some_and(|count| count > 0)
            {
                overlay_audit::record_queue_action(
                    &state,
                    &auth,
                    "overlay.queue_clear",
                    "/overlay/queue",
                )
                .await;
            }
            json_response(StatusCode::OK, payload)
        }
        Err(response) => response,
    }
}
