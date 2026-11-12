//! File d'attente d'une guilde, vue et modérée depuis l'overlay.
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `GET` | `/overlay/queue` | tout membre |
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

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{DateTime, Utc};
use lc_db::Db;
use lc_domain::{GuildId, Permission, PlaybackJobId};
use serde_json::{json, Value};
use sqlx::Row as _;

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::guild::require;
use super::{db, db_failure, json_response, now};

/// Nombre maximal de jobs renvoyés (garde-fou d'affichage).
const QUEUE_LIMIT: i64 = 200;

/// Date ISO 8601 à partir de millisecondes epoch, ou `null`.
fn iso_ms(millis: Option<i64>) -> Value {
    millis
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|date| json!(date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)))
        .unwrap_or(Value::Null)
}

/// Lit la file (job en cours puis jobs en attente) en une requête.
async fn load_queue(db: &Db, guild_id: &GuildId) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT j.id, j.status, j.text, j.author_name, j.author_image, j.duration_sec, \
                j.submission_date, j.started_at, j.media_asset_id, \
                a.kind AS media_kind, \
                (SELECT b.title FROM meme_board_items b \
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
            Ok(json!({
                "id": row.try_get::<String, _>("id")?,
                "status": status.to_ascii_lowercase(),
                "title": row.try_get::<Option<String>, _>("title")?,
                "text": row.try_get::<Option<String>, _>("text")?,
                "authorName": row.try_get::<Option<String>, _>("author_name")?,
                "authorImage": row.try_get::<Option<String>, _>("author_image")?,
                "durationSec": row.try_get::<i64, _>("duration_sec")?,
                "mediaAssetId": row.try_get::<Option<String>, _>("media_asset_id")?,
                "kind": kind.map(|kind| kind.to_ascii_lowercase()),
                "submittedAt": iso_ms(Some(row.try_get::<i64, _>("submission_date")?)),
                "startedAt": iso_ms(row.try_get::<Option<i64>, _>("started_at")?),
            }))
        })
        .collect()
}

/// Handler `GET /overlay/queue`.
pub async fn get_queue(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&auth, Permission::ViewQueue) {
        return response;
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let jobs = match load_queue(db, auth.guild_id()).await {
        Ok(jobs) => jobs,
        Err(sqlx_error) => {
            return db_failure("overlay_queue.list", &lc_db::DbError::from(sqlx_error));
        }
    };
    let (playing, pending): (Vec<Value>, Vec<Value>) =
        jobs.into_iter().partition(|job| job["status"] == "playing");

    json_response(
        StatusCode::OK,
        json!({
            "playing": playing.into_iter().next(),
            "pending": pending,
            "serverTime": iso_ms(Some(now(&state).timestamp_millis())),
        }),
    )
}

/// Handler `DELETE /overlay/queue/{jobId}` : retire un job en attente.
pub async fn remove_job(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(job_id_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&auth, Permission::ModerateQueue) {
        return response;
    }
    let Ok(job_id) = job_id_raw.trim().parse::<PlaybackJobId>() else {
        return json_response(StatusCode::OK, json!({"removed": false}));
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let result = sqlx::query(
        "UPDATE playback_jobs SET status = 'FAILED', finished_at = ? \
         WHERE id = ? AND guild_id = ? AND status = 'PENDING'",
    )
    .bind(now(&state).timestamp_millis())
    .bind(job_id.to_canonical())
    .bind(auth.guild_id().as_str())
    .execute(db.pool())
    .await;
    match result {
        Ok(result) => json_response(
            StatusCode::OK,
            json!({"removed": result.rows_affected() > 0}),
        ),
        Err(sqlx_error) => db_failure("overlay_queue.remove", &lc_db::DbError::from(sqlx_error)),
    }
}

/// Handler `DELETE /overlay/queue` : vide les jobs en attente de la guilde.
pub async fn clear_queue(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&auth, Permission::ModerateQueue) {
        return response;
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let result = sqlx::query(
        "UPDATE playback_jobs SET status = 'FAILED', finished_at = ? \
         WHERE guild_id = ? AND status = 'PENDING'",
    )
    .bind(now(&state).timestamp_millis())
    .bind(auth.guild_id().as_str())
    .execute(db.pool())
    .await;
    match result {
        Ok(result) => json_response(
            StatusCode::OK,
            json!({"removedCount": result.rows_affected()}),
        ),
        Err(sqlx_error) => db_failure("overlay_queue.clear", &lc_db::DbError::from(sqlx_error)),
    }
}
