//! Board extras: personal favorites and pictures of audio items.
//!
//! | Method | Route | Body |
//! |---|---|---|
//! | `PUT` | `/overlay/media-board/items/{itemId}/favorite` | `{"favorite": bool}` |
//! | `PUT` | `/overlay/media-board/items/{itemId}/artwork?filename=` | raw image file |
//! | `DELETE` | `/overlay/media-board/items/{itemId}/artwork` | — |
//!
//! Favorites belong to the member (they follow them on every device). The
//! picture of an item is an ordinary image asset, pinned like the board's
//! media; whether overlays show it is `artworkOnOverlay` on `PATCH` of the
//! item.

use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use futures_util::StreamExt as _;
use lc_domain::enums::MediaKind;
use lc_domain::ids::MediaBoardItemId;
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::app::AppState;
use crate::media::assets;
use crate::media::service::MediaService;

use super::auth::AuthedOverlayClient;
use super::media_board::{ensure_can_edit, ensure_writable, media_service_error};
use super::media_upload::UploadTempDir;
use super::{db, db_failure, error, internal_error, json_response, now};

/// Largest picture accepted for an item (bytes).
pub(super) const MAX_ARTWORK_BYTES: u64 = 10 * 1024 * 1024;

/// Body of `PUT …/favorite`.
#[derive(Debug, Deserialize)]
pub struct FavoriteBody {
    favorite: bool,
}

/// Query of `PUT …/artwork`.
#[derive(Debug, Default, Deserialize)]
pub struct ArtworkQuery {
    #[serde(default)]
    filename: Option<String>,
}

/// Handler `PUT /overlay/media-board/items/{itemId}/favorite`.
pub async fn put_favorite(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
    body: Result<Json<FavoriteBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let Some(member_id) = auth.member_id() else {
        return error(StatusCode::FORBIDDEN, "member_required");
    };
    let Ok(item_id) = item_id_raw.trim().parse::<MediaBoardItemId>() else {
        return error(StatusCode::NOT_FOUND, "item_not_found");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == auth.guild_id() => {}
        Ok(_) => return error(StatusCode::NOT_FOUND, "item_not_found"),
        Err(db_error) => return db_failure("overlay_board_extras.favorite_lookup", &db_error),
    }
    if let Err(db_error) = db
        .media_board()
        .set_favorite(member_id, item_id, body.favorite, now(&state))
        .await
    {
        return db_failure("overlay_board_extras.favorite_save", &db_error);
    }
    json_response(
        StatusCode::OK,
        serde_json::json!({ "itemId": item_id.to_canonical(), "favorite": body.favorite }),
    )
}

/// Handler `PUT /overlay/media-board/items/{itemId}/artwork`: the body is the
/// raw image file.
pub async fn put_artwork(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
    query: Result<Query<ArtworkQuery>, QueryRejection>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let Ok(Query(query)) = query else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let filename = query
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && name.chars().count() <= 255
                && !name.contains(['/', '\\'])
                && !name.contains("..")
                && !name.chars().any(char::is_control)
        })
        .unwrap_or("artwork")
        .to_owned();
    let Ok(item_id) = item_id_raw.trim().parse::<MediaBoardItemId>() else {
        return error(StatusCode::NOT_FOUND, "item_not_found");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let item = match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == auth.guild_id() => item,
        Ok(_) => return error(StatusCode::NOT_FOUND, "item_not_found"),
        Err(db_error) => return db_failure("overlay_board_extras.artwork_lookup", &db_error),
    };
    if let Err(response) = ensure_can_edit(&state, &auth, item_id).await {
        return response;
    }
    let service = match MediaService::for_state(&state) {
        Ok(service) => service,
        Err(service_error) => return media_service_error(&service_error),
    };
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let temp = match UploadTempDir::create().await {
        Ok(temp) => temp,
        Err(io_error) => {
            tracing::error!(error = %io_error, "dossier temporaire de l'image impossible");
            return internal_error();
        }
    };
    let temp_path = temp.file_path();
    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(file) => file,
        Err(io_error) => {
            tracing::error!(error = %io_error, "fichier temporaire de l'image impossible");
            return internal_error();
        }
    };
    let mut stream = body.into_data_stream();
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return error(StatusCode::BAD_REQUEST, "invalid_payload");
        };
        total = total.saturating_add(chunk.len() as u64);
        if total > MAX_ARTWORK_BYTES {
            return error(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large");
        }
        if let Err(io_error) = file.write_all(&chunk).await {
            tracing::error!(error = %io_error, "écriture de l'image impossible");
            return internal_error();
        }
        hasher.update(&chunk);
    }
    if file.flush().await.is_err() {
        return internal_error();
    }
    drop(file);
    if total == 0 {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    }
    let sha256 = format!("{:x}", hasher.finalize());

    let asset = match service
        .ingest_file(&temp_path, &filename, content_type.as_deref(), &sha256)
        .await
    {
        Ok(asset) => asset,
        Err(service_error) => return media_service_error(&service_error),
    };
    if asset.kind != MediaKind::Image {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media");
    }
    let asset = match assets::pin(db, asset.id, now(&state)).await {
        Ok(asset) => asset,
        Err(service_error) => return media_service_error(&service_error),
    };
    if let Err(db_error) = db.media_board().set_artwork(item.id, Some(asset.id)).await {
        return db_failure("overlay_board_extras.artwork_save", &db_error);
    }
    broadcast_updated(&state, &auth, item_id);
    json_response(
        StatusCode::OK,
        serde_json::json!({ "itemId": item_id.to_canonical(), "artworkAssetId": asset.id.to_canonical() }),
    )
}

/// Handler `DELETE /overlay/media-board/items/{itemId}/artwork`.
pub async fn delete_artwork(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let Ok(item_id) = item_id_raw.trim().parse::<MediaBoardItemId>() else {
        return error(StatusCode::NOT_FOUND, "item_not_found");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    match db.media_board().get(item_id).await {
        Ok(Some(item)) if &item.guild_id == auth.guild_id() => {}
        Ok(_) => return error(StatusCode::NOT_FOUND, "item_not_found"),
        Err(db_error) => return db_failure("overlay_board_extras.artwork_lookup", &db_error),
    }
    if let Err(response) = ensure_can_edit(&state, &auth, item_id).await {
        return response;
    }
    if let Err(db_error) = db.media_board().set_artwork(item_id, None).await {
        return db_failure("overlay_board_extras.artwork_clear", &db_error);
    }
    broadcast_updated(&state, &auth, item_id);
    json_response(
        StatusCode::OK,
        serde_json::json!({ "itemId": item_id.to_canonical() }),
    )
}

fn broadcast_updated(state: &AppState, auth: &AuthedOverlayClient, item_id: MediaBoardItemId) {
    state.admin_hub.broadcast_to_guild(
        auth.guild_id(),
        ServerMessage::BoardChanged(BoardChangedPayload {
            change: BoardChange::Updated,
            item_id: item_id.to_canonical(),
        }),
    );
}
