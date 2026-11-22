//! File d'attente d'une guilde, vue et modérée depuis l'overlay.
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `GET` | `/overlay/queue` | tout membre (`ViewQueue`) |
//! | `GET` | `/overlay/queue/history` | tout membre (`ViewQueue`) |
//! | `POST` | `/overlay/queue/reorder` | admin (`ModerateQueue`) |
//! | `DELETE` | `/overlay/queue/{jobId}` | admin (`ModerateQueue`) |
//! | `DELETE` | `/overlay/queue` | admin (`ModerateQueue`) |
//!
//! La file est lue dans l'ordre explicite `queue_position ASC`, départagé par
//! `submission_date ASC` puis `id ASC`, le job `PLAYING` en tête : un média
//! n'interrompt jamais la lecture en cours, il passe derrière elle. Le
//! réordonnancement ne touche que les jobs `PENDING` et exige la liste
//! complète de la file : un client qui envoie un ordre périmé reçoit
//! `409 stale_queue` et recharge.
//!
//! L'`origin` d'un job dit d'où il vient : `board` (item de board), `once`
//! (média joué une fois ou ingéré par un client, donc sans item de board) ou
//! `direct` (texte seul). Un job sans item de board n'a pas de `title` : le
//! payload porte `sourceTitle` et un objet `media` (format, dimensions,
//! taille, source) pour que l'historique reste lisible.
//!
//! Retirer un job en attente le passe `FAILED` : c'est la seule transition
//! `PENDING → terminal` admise par la machine d'état
//! (`JobStatus::can_transition_to`). Le job en cours ne se retire pas ici : il
//! s'arrête par `stop.request` (WS), qui libère proprement la lecture.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_db::Db;
use lc_domain::{GuildId, Permission, PlaybackJobId};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::sqlite::SqliteRow;
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

/// Colonnes communes à la file et à l'historique : le job, son média et
/// l'item de board éventuel dont il provient.
///
/// `media_board_items` est unique par `(guild_id, media_asset_id)` : la
/// jointure renvoie au plus une ligne et ne duplique jamais un job. Un média
/// joué une fois (`once`) ou ingéré par un client n'a pas d'item de board :
/// `board_item_id` et `title` sont alors `NULL`, `source_title` portant le
/// titre de la source.
const JOB_SELECT: &str = "SELECT j.id, j.status, j.text, j.author_name, \
        j.author_image, j.duration_sec, j.submission_date, j.started_at, \
        j.finished_at, j.media_asset_id, \
        b.id AS board_item_id, b.title AS title, \
        a.kind AS media_kind, a.source_title, a.mime, \
        a.duration_sec AS media_duration_sec, a.width, a.height, \
        a.is_vertical, a.size_bytes, a.source_url \
   FROM playback_jobs j \
   LEFT JOIN media_assets a ON a.id = j.media_asset_id \
   LEFT JOIN media_board_items b ON b.guild_id = j.guild_id \
                               AND b.media_asset_id = j.media_asset_id";

/// Origine d'un job : `board` (item de board), `once` (média joué une fois ou
/// ingéré par un client, sans item de board) ou `direct` (texte seul).
fn job_origin(media_asset_id: Option<&str>, board_item_id: Option<&str>) -> &'static str {
    if board_item_id.is_some() {
        "board"
    } else if media_asset_id.is_some() {
        "once"
    } else {
        "direct"
    }
}

/// Média d'un job, sous la même forme que le média d'un item de board, ou
/// `null` pour un job direct.
fn media_payload(row: &SqliteRow) -> Result<Value, sqlx::Error> {
    let Some(kind) = row.try_get::<Option<String>, _>("media_kind")? else {
        return Ok(Value::Null);
    };
    let is_vertical = row
        .try_get::<Option<i64>, _>("is_vertical")?
        .is_some_and(|value| value != 0);
    Ok(json!({
        "id": row.try_get::<Option<String>, _>("media_asset_id")?,
        "kind": kind.to_ascii_lowercase(),
        "mime": row.try_get::<Option<String>, _>("mime")?,
        "durationSec": row.try_get::<Option<i64>, _>("media_duration_sec")?,
        "width": row.try_get::<Option<i64>, _>("width")?,
        "height": row.try_get::<Option<i64>, _>("height")?,
        "isVertical": is_vertical,
        "sizeBytes": row.try_get::<Option<i64>, _>("size_bytes")?,
        "sourceUrl": row.try_get::<Option<String>, _>("source_url")?,
    }))
}

/// Job de la file ou de l'historique : origine, titre résolu et détails du
/// média. Un job sans item de board n'a pas de `title` mais garde
/// `sourceTitle` et `media`.
fn job_payload(row: &SqliteRow) -> Result<Value, sqlx::Error> {
    let status: String = row.try_get("status")?;
    let media_asset_id: Option<String> = row.try_get("media_asset_id")?;
    let board_item_id: Option<String> = row.try_get("board_item_id")?;
    let kind: Option<String> = row.try_get("media_kind")?;
    let origin = job_origin(media_asset_id.as_deref(), board_item_id.as_deref());
    Ok(json!({
        "id": row.try_get::<String, _>("id")?,
        "status": status.to_ascii_lowercase(),
        "title": row.try_get::<Option<String>, _>("title")?,
        "sourceTitle": row.try_get::<Option<String>, _>("source_title")?,
        "text": row.try_get::<Option<String>, _>("text")?,
        "authorName": row.try_get::<Option<String>, _>("author_name")?,
        "authorImage": row.try_get::<Option<String>, _>("author_image")?,
        "durationSec": row.try_get::<i64, _>("duration_sec")?,
        "mediaAssetId": media_asset_id,
        "origin": origin,
        "kind": kind.map(|kind| kind.to_ascii_lowercase()),
        "media": media_payload(row)?,
        "submittedAt": iso_ms(Some(row.try_get::<i64, _>("submission_date")?)),
        "startedAt": iso_ms(row.try_get::<Option<i64>, _>("started_at")?),
        "finishedAt": iso_ms(row.try_get::<Option<i64>, _>("finished_at")?),
    }))
}

/// Lit la file (job en cours puis jobs en attente) en une requête.
async fn load_queue(db: &Db, guild_id: &GuildId) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "{JOB_SELECT} \
          WHERE j.guild_id = ? AND j.status IN ('PLAYING', 'PENDING') \
          ORDER BY CASE j.status WHEN 'PLAYING' THEN 0 ELSE 1 END, \
                   j.queue_position ASC, j.submission_date ASC, j.id ASC \
          LIMIT ?"
    ))
    .bind(guild_id.as_str())
    .bind(QUEUE_LIMIT)
    .fetch_all(db.pool())
    .await?;

    rows.iter().map(job_payload).collect()
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

/// Body of `POST /overlay/queue/reorder` : la file complète, dans le nouvel
/// ordre. Un identifiant inconnu ou une liste incomplète est refusé.
#[derive(Debug, Deserialize)]
pub(crate) struct ReorderRequest {
    #[serde(rename = "jobIds")]
    job_ids: Vec<String>,
}

/// Applique un nouvel ordre aux jobs `PENDING` de la guilde.
///
/// La liste doit être exactement l'ensemble des jobs en attente : si la file a
/// changé entre la lecture du client et l'envoi (job ajouté, retiré ou lancé),
/// rien n'est écrit et la réponse est `409 stale_queue`.
pub(crate) async fn reorder_queue_payload(
    state: &AppState,
    guild_id: &GuildId,
    raw_ids: &[String],
) -> Result<Value, Response> {
    let mut ordered = Vec::with_capacity(raw_ids.len());
    for raw in raw_ids {
        let Ok(id) = raw.trim().parse::<PlaybackJobId>() else {
            return Err(super::error(StatusCode::BAD_REQUEST, "invalid_job_id"));
        };
        ordered.push(id.to_canonical());
    }
    let unique: std::collections::HashSet<&String> = ordered.iter().collect();
    if unique.len() != ordered.len() {
        return Err(super::error(StatusCode::BAD_REQUEST, "duplicate_job_id"));
    }

    let db = db(state)?;
    let pending: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM playback_jobs WHERE guild_id = ? AND status = 'PENDING' \
         ORDER BY queue_position ASC, submission_date ASC, id ASC",
    )
    .bind(guild_id.as_str())
    .fetch_all(db.pool())
    .await
    .map_err(|sqlx_error| queue_db_failure("overlay_queue.reorder", sqlx_error))?;
    if pending.len() != ordered.len() || !pending.iter().all(|id| ordered.contains(id)) {
        return Err(super::error(StatusCode::CONFLICT, "stale_queue"));
    }

    let mut tx = db
        .pool()
        .begin()
        .await
        .map_err(|sqlx_error| queue_db_failure("overlay_queue.reorder", sqlx_error))?;
    for (index, id) in ordered.iter().enumerate() {
        let updated = sqlx::query(
            "UPDATE playback_jobs SET queue_position = ? \
             WHERE id = ? AND guild_id = ? AND status = 'PENDING'",
        )
        .bind(index as i64 + 1)
        .bind(id)
        .bind(guild_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|sqlx_error| queue_db_failure("overlay_queue.reorder", sqlx_error))?;
        if updated.rows_affected() != 1 {
            drop(tx.rollback().await);
            return Err(super::error(StatusCode::CONFLICT, "stale_queue"));
        }
    }
    tx.commit()
        .await
        .map_err(|sqlx_error| queue_db_failure("overlay_queue.reorder", sqlx_error))?;

    Ok(json!({"reordered": true, "count": ordered.len()}))
}

/// Handler `POST /overlay/queue/reorder` : réordonne les jobs en attente.
pub async fn reorder_queue(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<ReorderRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ModerateQueue) {
        return response;
    }
    let Ok(Json(body)) = payload else {
        return super::error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    match reorder_queue_payload(&state, auth.guild_id(), &body.job_ids).await {
        Ok(payload) => {
            overlay_audit::record_queue_action(
                &state,
                &auth,
                "overlay.queue_reorder",
                "/overlay/queue/reorder",
            )
            .await;
            json_response(StatusCode::OK, payload)
        }
        Err(response) => response,
    }
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

/// Dernières lectures terminées ou échouées de la guilde, de la plus récente
/// à la plus ancienne : `{count, items: [job_payload]}`, chaque item portant
/// son `origin`, son `sourceTitle` et son `media`.
async fn history_payload(db: &Db, guild_id: &GuildId, limit: i64) -> Result<Value, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "{JOB_SELECT} \
          WHERE j.guild_id = ? AND j.status IN ('DONE', 'FAILED') \
            AND j.finished_at IS NOT NULL \
          ORDER BY j.finished_at DESC, j.id DESC \
          LIMIT ?"
    ))
    .bind(guild_id.as_str())
    .bind(limit)
    .fetch_all(db.pool())
    .await?;

    let items: Vec<Value> = rows
        .iter()
        .map(job_payload)
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
