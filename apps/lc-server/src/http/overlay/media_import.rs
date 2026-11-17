//! Route `POST /overlay/media-board/import`: copy board items from another
//! guild the same device belongs to.
//!
//! The request is authenticated twice: the bearer is the **target** overlay
//! client (where the items are added), and `X-LiveChat-Source-Token` carries
//! the token of the **source** overlay client (whose board the items come
//! from). Both tokens live in the overlay shell, never in its web layer.
//!
//! A copy is a new board item pointing to the same media asset (no download,
//! no transcode): it goes through [`super::media_board::complete_board_item`]
//! like any other addition, so deduplication, storage quota, pinning and the
//! author follow the usual rules. Title, message and picture are copied.
//!
//! Body `{"itemIds": [...]}` (1 to [`MAX_IMPORT_ITEMS`] ids). Answer `200
//! {"results": [{sourceItemId, status, itemId?, error?, message?}]}` with
//! `status` among `added`, `already`, `not_found`, `failed`, in request order.
//!
//! Synchronous errors: `403 read_only_session` and `403 forbidden` (target
//! may not add media, or source guest may not view its board), `400
//! invalid_payload`, `401 invalid_source_token`.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use lc_db::Db;
use lc_domain::{MediaBoardItem, MediaBoardItemId, Permission};
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};
use serde::Deserialize;
use serde_json::{json, Value};
use time::UtcDateTime;

use crate::app::AppState;
use crate::media::assets;

use super::auth::{authenticate_token, AuthedOverlayClient};
use super::media_board::{
    complete_board_item, ensure_board_visible_for, ensure_writable, json_rejection_error, ItemTitle,
};
use super::{db, error, json_response, now, ActorInfo};

/// Header carrying the source overlay client's token.
pub const SOURCE_TOKEN_HEADER: &str = "x-livechat-source-token";

/// Most items copied by one request.
pub const MAX_IMPORT_ITEMS: usize = 50;

/// Body of `POST /overlay/media-board/import`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub item_ids: Vec<String>,
}

/// Handler `POST /overlay/media-board/import`.
pub async fn import_items(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    headers: HeaderMap,
    payload: Result<Json<ImportRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    if !ActorInfo::from_auth(&state, &auth).can(Permission::AddMedia) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    let payload = match payload {
        Ok(Json(payload)) => payload,
        Err(rejection) => return json_rejection_error(rejection),
    };
    let item_ids = match parse_item_ids(&payload.item_ids) {
        Ok(item_ids) => item_ids,
        Err(message) => return invalid_payload(message),
    };
    let db = match db(&state) {
        Ok(db) => db.clone(),
        Err(response) => return response,
    };
    let at = now(&state);

    let Some(source_token) = headers
        .get(SOURCE_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return invalid_source_token();
    };
    let source = match authenticate_token(&db, source_token, at).await {
        Ok(Some(source)) => source,
        Ok(None) => return invalid_source_token(),
        Err(response) => return response,
    };
    if source.guild_id() == auth.guild_id() {
        return invalid_payload("source and target are the same guild");
    }
    if let Err(response) =
        ensure_board_visible_for(&state, &db, source.guild_id(), source.role()).await
    {
        return response;
    }

    let mut results = Vec::with_capacity(item_ids.len());
    for item_id in item_ids {
        results.push(import_one(&state, &db, &auth, &source, item_id).await);
    }
    json_response(StatusCode::OK, json!({ "results": results }))
}

/// Copies one source item to the target board and describes the outcome.
async fn import_one(
    state: &AppState,
    db: &Db,
    auth: &AuthedOverlayClient,
    source: &AuthedOverlayClient,
    item_id: MediaBoardItemId,
) -> Value {
    let source_item_id = item_id.to_canonical();
    let item = match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == source.guild_id() => item,
        Ok(_) => return json!({ "sourceItemId": source_item_id, "status": "not_found" }),
        Err(db_error) => {
            tracing::error!(error = %db_error, "cannot read a board item to import");
            return failed(&source_item_id, "internal_error", None);
        }
    };
    let asset = match db.media_assets().get(item.media_asset_id).await {
        Ok(Some(asset)) if asset.is_ready() => asset,
        Ok(_) => return failed(&source_item_id, "media_not_ready", None),
        Err(db_error) => {
            tracing::error!(error = %db_error, "cannot read the media to import");
            return failed(&source_item_id, "internal_error", None);
        }
    };
    let at = now(state);
    let completed = match complete_board_item(
        state,
        auth,
        db,
        asset,
        // L'import garde le titre de l'item source, même vide : pas de repli
        // sur le titre source de l'asset.
        ItemTitle::Copied(item.title.clone()),
        item.message.clone(),
        at,
    )
    .await
    {
        Ok(completed) => completed,
        Err(failure) => return failed(&source_item_id, failure.code, failure.message),
    };
    let new_id = completed.item.id.to_canonical();
    if !completed.created {
        return json!({ "sourceItemId": source_item_id, "status": "already", "itemId": new_id });
    }
    copy_artwork(db, &item, &completed.item, at).await;
    state.admin_hub.broadcast_to_guild(
        auth.guild_id(),
        ServerMessage::BoardChanged(BoardChangedPayload {
            change: BoardChange::Added,
            item_id: new_id.clone(),
        }),
    );
    json!({ "sourceItemId": source_item_id, "status": "added", "itemId": new_id })
}

/// Gives the copy the source item's picture and display time. The copy is already on the
/// board, so a failure here is logged and does not fail the import.
async fn copy_artwork(db: &Db, source: &MediaBoardItem, copy: &MediaBoardItem, at: UtcDateTime) {
    if let Some(artwork_id) = source.artwork_asset_id {
        if let Err(pin_error) = assets::pin(db, artwork_id, at).await {
            tracing::warn!(error = %pin_error, "cannot pin the picture of an imported item");
            return;
        }
        if let Err(db_error) = db
            .media_board()
            .set_artwork(copy.id, Some(artwork_id))
            .await
        {
            tracing::warn!(error = %db_error, "picture of an imported item not copied");
            return;
        }
    }
    if source.artwork_on_overlay {
        if let Err(db_error) = db.media_board().set_artwork_on_overlay(copy.id, true).await {
            tracing::warn!(error = %db_error, "picture option of an imported item not copied");
        }
    }
    if source.display_duration_sec.is_some() {
        if let Err(db_error) = db
            .media_board()
            .set_display_duration(copy.id, source.display_duration_sec)
            .await
        {
            tracing::warn!(error = %db_error, "display time of an imported item not copied");
        }
    }
}

/// Validates and deduplicates the ids, keeping their order.
fn parse_item_ids(raw: &[String]) -> Result<Vec<MediaBoardItemId>, &'static str> {
    if raw.is_empty() {
        return Err("itemIds must not be empty");
    }
    if raw.len() > MAX_IMPORT_ITEMS {
        return Err("too many itemIds (50 at most)");
    }
    let mut ids: Vec<MediaBoardItemId> = Vec::with_capacity(raw.len());
    for value in raw {
        let id = MediaBoardItemId::parse(value.trim()).map_err(|_| "invalid item id")?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn failed(source_item_id: &str, code: &str, message: Option<String>) -> Value {
    let mut result = json!({ "sourceItemId": source_item_id, "status": "failed", "error": code });
    if let Some(message) = message {
        result["message"] = Value::String(message);
    }
    result
}

fn invalid_payload(message: &str) -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "invalid_payload", "message": message }),
    )
}

fn invalid_source_token() -> Response {
    error(StatusCode::UNAUTHORIZED, "invalid_source_token")
}
