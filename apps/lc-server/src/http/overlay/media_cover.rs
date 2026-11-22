//! `POST /overlay/media-board/items/{itemId}/cover` — the picture the video
//! editor is showing becomes the cover of the item.
//!
//! The overlay editor plays the stored video: the frame on screen at that
//! moment is the one the user wants on the card. The server reads that frame
//! again from the video file (`lc_media::poster::extract_poster_at`) and
//! records it as the asset's cover, the picture the board card and the detail
//! panel show.
//!
//! ## Request
//!
//! ```json
//! { "atSec": 2.4 }
//! ```
//!
//! `atSec` (seconds, required, `>= 0`) is the position of the editor's player.
//! It is clamped inside the video: a position past the end gives the last
//! frame, never nothing.
//!
//! ## Response
//!
//! `200 {"cover": {"width": 640, "height": 360, "mime": "image/jpeg"}}`. Every
//! overlay showing the item reloads its card (`board_changed` `updated`), and
//! the picture replaces the previous one at the same signed URL, which is why
//! the board payload carries its size as a version (see
//! [`super::media_board`]).
//!
//! ## Guards, in order
//!
//! 1. `403 read_only_session` (guest session), before anything is validated;
//! 2. `400 invalid_payload` + `message` for a malformed body;
//! 3. `404 item_not_found` when the item is not on the caller's board;
//! 4. `403 forbidden` without the right to edit the item;
//! 5. `404 media_not_found` / `media_not_found_on_disk` when the media is gone;
//! 6. `400 invalid_payload` when the media is not a video;
//! 7. `422 cover_unavailable` when the frame cannot be read or written.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_domain::MediaKind;
use lc_media::poster::{extract_poster_at, PosterSource};

use crate::app::AppState;
use crate::media::thumbnails;

use super::auth::AuthedOverlayClient;
use super::media_board::{ensure_actor_can_edit, ensure_writable, json_rejection_error};
use super::{audit as overlay_audit, db, db_failure, error, json_response, ActorInfo};

/// Handler for `POST /overlay/media-board/items/{itemId}/cover`.
pub async fn set_cover(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(item_id_raw): Path<String>,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let body = match body {
        Ok(Json(value)) => value,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "corps JSON invalide sur POST /overlay/media-board/items/{{id}}/cover"
            );
            return json_rejection_error(rejection);
        }
    };
    let actor = ActorInfo::from_auth(&state, &auth);
    apply_cover(&state, &actor, &item_id_raw, body).await
}

/// Cover pipeline shared with any other caller that already holds an actor.
///
/// The request is fully validated before the video file is touched: a guest,
/// a broken body, a foreign item or an image never reaches `ffmpeg`.
pub(crate) async fn apply_cover(
    state: &AppState,
    actor: &ActorInfo,
    item_id_raw: &str,
    body: serde_json::Value,
) -> Response {
    if actor.is_read_only() {
        return error(StatusCode::FORBIDDEN, "read_only_session");
    }
    let at_sec = match parse_request(body) {
        Ok(at_sec) => at_sec,
        Err(message) => return invalid_payload(message),
    };

    let db = match db(state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let item = match item_id_raw.trim().parse::<lc_domain::MediaBoardItemId>() {
        Ok(item_id) => match db.media_board().get(item_id).await {
            Ok(Some(item)) if &item.guild_id == actor.guild_id() => item,
            Ok(_) => return error(StatusCode::NOT_FOUND, "item_not_found"),
            Err(db_error) => {
                return db_failure("overlay_media_cover.item_lookup", &db_error);
            }
        },
        Err(_) => return error(StatusCode::NOT_FOUND, "item_not_found"),
    };
    if let Err(response) = ensure_actor_can_edit(db, actor, item.id).await {
        return response;
    }

    let asset = match db.media_assets().get(item.media_asset_id).await {
        Ok(Some(asset)) if asset.is_ready() => asset,
        Ok(_) => return super::media::media_not_found(),
        Err(db_error) => return db_failure("overlay_media_cover.asset_lookup", &db_error),
    };
    if asset.kind != MediaKind::Video {
        return invalid_payload("only a video can take its picture from a frame".to_owned());
    }
    let input = match super::media::resolve_media_file(state, &asset).await {
        Ok(path) => path,
        Err(response) => return response,
    };
    let Some(context) = thumbnails::context_for_state(state) else {
        return super::internal_error();
    };
    let source = PosterSource {
        input: &input,
        width: optional_side(asset.width),
        height: optional_side(asset.height),
        duration_sec: asset.duration_sec.map(|seconds| seconds as f64),
    };
    let Some(cover) = extract_poster_at(
        context.config.as_ref(),
        &context.storage,
        &asset.source_hash,
        &source,
        at_sec,
    )
    .await
    else {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "cover_unavailable");
    };
    if !thumbnails::store_cover(&context, &asset, &cover, "editor").await {
        return super::internal_error();
    }

    let creator = overlay_audit::creator_member(db, item.id).await;
    overlay_audit::record_moderation(
        state,
        actor,
        creator,
        "overlay.media_cover",
        "POST",
        &format!(
            "/overlay/media-board/items/{}/cover",
            item.id.to_canonical()
        ),
    )
    .await;

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "cover": {
                "width": cover.width,
                "height": cover.height,
                "mime": cover.mime,
            }
        }),
    )
}

/// Reads `{"atSec": <seconds>}`; the error is the `message` of the
/// `400 invalid_payload` response.
fn parse_request(body: serde_json::Value) -> Result<f64, String> {
    let serde_json::Value::Object(object) = body else {
        return Err("a JSON object is expected".to_owned());
    };
    let Some(at_sec) = object.get("atSec").and_then(serde_json::Value::as_f64) else {
        return Err("atSec is required: the position of the player, in seconds".to_owned());
    };
    if !at_sec.is_finite() || at_sec < 0.0 {
        return Err("atSec must be a positive number of seconds".to_owned());
    }
    Ok(at_sec)
}

/// An asset dimension as the poster sampler expects it.
fn optional_side(value: Option<i64>) -> Option<u32> {
    value.and_then(|value| u32::try_from(value).ok())
}

/// `400 invalid_payload` with a message.
fn invalid_payload(message: String) -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({"error": "invalid_payload", "message": message}),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use serde_json::json;

    use super::*;

    #[test]
    fn request_body_is_validated() {
        assert_eq!(parse_request(json!({"atSec": 2.4})).unwrap(), 2.4);
        assert_eq!(parse_request(json!({"atSec": 0})).unwrap(), 0.0);

        for (body, message) in [
            (json!([]), "a JSON object is expected"),
            (
                json!({}),
                "atSec is required: the position of the player, in seconds",
            ),
            (
                json!({"atSec": "2"}),
                "atSec is required: the position of the player, in seconds",
            ),
            (
                json!({"atSec": null}),
                "atSec is required: the position of the player, in seconds",
            ),
            (
                json!({"atSec": -1}),
                "atSec must be a positive number of seconds",
            ),
        ] {
            assert_eq!(parse_request(body).err().unwrap(), message);
        }
    }

    #[test]
    fn an_asset_side_of_any_size_is_read_as_a_pixel_count() {
        assert_eq!(optional_side(Some(640)), Some(640));
        assert_eq!(optional_side(Some(-4)), None);
        assert_eq!(optional_side(None), None);
    }
}
