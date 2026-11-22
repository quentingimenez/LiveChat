//! `POST /overlay/media-board/items/{itemId}/edit` — edit the audio or video
//! of a board item with ffmpeg (cut, crop, rotate, speed, volume, fades,
//! reverse, audio extraction, GIF export).
//!
//! ## Request
//!
//! ```json
//! { "replace": false, "title": "Chat qui tombe (court)", "recipe": { ... } }
//! ```
//!
//! - `recipe`: an [`lc_media::edit::EditRecipe`] (required object; every field
//!   is optional);
//! - `replace` (default `false`): `false` adds the result as a new board item,
//!   `true` replaces the media of the edited item, which keeps its id, title,
//!   message, author, date, favorites and picture. A replacement must produce
//!   the same kind of media (`output: "same"`);
//! - `title` (copy only, ≤ 240 characters): title of the new item; by default
//!   the original title followed by « (édité) ». The message is copied from
//!   the original.
//!
//! ## Response
//!
//! `202 {"ingestionId": "..."}`. The edit runs in the detached ingestion task
//! shared with the link and upload routes (see [`super::media_ingestion`]):
//! its result arrives as `media.ingestion` (`created:true` for a copy,
//! `created:false` + the item's id for a replacement) and `board.changed`
//! (`added` / `updated`), and stays readable through
//! `GET /overlay/media-board/ingestions`.
//!
//! ## Guards, in order
//!
//! 1. `403 read_only_session` (guest session), before anything is validated;
//! 2. `400 invalid_payload` + `message` for a malformed body;
//! 3. `404 item_not_found` when the item is not on the caller's board;
//! 4. `403 forbidden`: a replacement needs the right to edit the item, a copy
//!    the right to add media;
//! 5. `404 media_not_found` / `media_not_found_on_disk` when the media is gone;
//! 6. `400 invalid_payload` + `message` when the recipe does not fit the media
//!    (an image is refused: images are edited on the board itself);
//! 7. `429 too_many_ingestions`.
//!
//! ## Body size
//!
//! The annotations of a video travel as a base64 PNG, so this route accepts
//! [`MAX_EDIT_BODY_BYTES`] (8 MiB) instead of the global 1 MiB
//! ([`crate::app::MAX_REQUEST_BODY_BYTES`]). The overlay itself is limited to
//! 4 MiB once decoded by [`lc_media::edit`].

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_domain::Permission;
use lc_media::edit::{EditRecipe, EditSource};

use crate::app::AppState;
use crate::ingestions::StartIngestionError;

use super::auth::AuthedOverlayClient;
use super::media_board::{
    ensure_actor_can_edit, ensure_writable, json_rejection_error, too_many_ingestions,
    truncate_chars,
};
use super::media_ingestion::{spawn_ingestion, BoardTarget, IngestionInput};
use super::{audit as overlay_audit, db, db_failure, error, json_response, now, ActorInfo};

/// Longest title of a board item.
const TITLE_MAX_CHARS: usize = 240;
/// Largest body accepted on this route: an annotation overlay is a base64 PNG
/// of at most 4 MiB once decoded, and base64 adds a third on top of it.
pub const MAX_EDIT_BODY_BYTES: usize = 8 * 1024 * 1024;
/// Longest name given to the edited file (it is only displayed).
const FILENAME_STEM_MAX_CHARS: usize = 120;
/// Suffix of the default title of an edited copy.
const EDITED_SUFFIX: &str = " (édité)";
/// Title of an edited copy whose original has no title.
const EDITED_DEFAULT_TITLE: &str = "Média édité";
/// Duration assumed for an asset stored without one; the real duration is
/// checked again once the file has been probed.
const UNKNOWN_DURATION_SEC: f64 = 86_400.0;

/// Parsed request body.
struct EditRequest {
    replace: bool,
    title: Option<String>,
    recipe: EditRecipe,
}

/// Edit pipeline shared by the overlay route and the admin console.
///
/// The request is fully validated (and refused for a read-only actor) before
/// the detached ingestion is registered; the response is the overlay
/// `202 {"ingestionId"}` body.
pub(crate) async fn start_edit(
    state: &AppState,
    actor: &ActorInfo,
    item_id_raw: &str,
    body: serde_json::Value,
) -> Response {
    if actor.is_read_only() {
        return error(StatusCode::FORBIDDEN, "read_only_session");
    }
    let request = match parse_request(body) {
        Ok(request) => request,
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
                return db_failure("overlay_media_edit.item_lookup", &db_error);
            }
        },
        Err(_) => return error(StatusCode::NOT_FOUND, "item_not_found"),
    };
    if request.replace {
        if let Err(response) = ensure_actor_can_edit(db, actor, item.id).await {
            return response;
        }
    } else if !actor.can(Permission::AddMedia) {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }

    let asset = match db.media_assets().get(item.media_asset_id).await {
        Ok(Some(asset)) if asset.is_ready() => asset,
        Ok(_) => return super::media::media_not_found(),
        Err(db_error) => return db_failure("overlay_media_edit.asset_lookup", &db_error),
    };
    // The stored duration is rounded up to a second and nothing is known of
    // the streams yet: the task checks the recipe again on the probed file.
    // The stored picture size is enough to validate the annotations early.
    let source = EditSource {
        kind: asset.kind,
        duration_sec: asset
            .duration_sec
            .filter(|duration| *duration > 0)
            .map_or(UNKNOWN_DURATION_SEC, |duration| duration as f64),
        has_audio: true,
        width: asset.width.and_then(|width| u32::try_from(width).ok()),
        height: asset.height.and_then(|height| u32::try_from(height).ok()),
    };
    let plan = match request.recipe.plan(&source) {
        Ok(plan) => plan,
        Err(message) => return invalid_payload(message),
    };
    if request.replace && plan.output_kind() != asset.kind {
        return invalid_payload(
            "a replacement must produce the same kind of media as the original".to_owned(),
        );
    }
    let source_path = match super::media::resolve_media_file(state, &asset).await {
        Ok(path) => path,
        Err(response) => return response,
    };

    let ingested_at = now(state);
    let authed = actor.to_authed(ingested_at);
    let ingestion_id = match state.ingestions.start(
        actor.guild_id().clone(),
        actor.client.id.as_uuid(),
        ingested_at,
    ) {
        Ok(ingestion_id) => ingestion_id,
        Err(StartIngestionError::TooManyProcessing) => return too_many_ingestions(),
    };

    let (target, title, message, name) = if request.replace {
        (
            BoardTarget::Replace(item.id),
            None,
            None,
            item.title.clone().unwrap_or_default(),
        )
    } else {
        let title = request
            .title
            .unwrap_or_else(|| match item.title.as_deref() {
                Some(original) => {
                    let room = TITLE_MAX_CHARS.saturating_sub(EDITED_SUFFIX.chars().count());
                    format!("{}{EDITED_SUFFIX}", truncate_chars(original, room))
                }
                None => EDITED_DEFAULT_TITLE.to_owned(),
            });
        (
            BoardTarget::NewItem,
            Some(title.clone()),
            item.message.clone(),
            title,
        )
    };
    spawn_ingestion(
        state.clone(),
        authed,
        ingestion_id.clone(),
        IngestionInput::Edit {
            source_path,
            kind: asset.kind,
            recipe: Box::new(request.recipe),
            filename_stem: filename_stem(&name),
        },
        target,
        title,
        message,
    );

    if request.replace {
        let creator = overlay_audit::creator_member(db, item.id).await;
        overlay_audit::record_moderation(
            state,
            actor,
            creator,
            "overlay.media_edit",
            "POST",
            &format!("/overlay/media-board/items/{}/edit", item.id.to_canonical()),
        )
        .await;
    }

    json_response(
        StatusCode::ACCEPTED,
        serde_json::json!({ "ingestionId": ingestion_id }),
    )
}

/// Handler for `POST /overlay/media-board/items/{itemId}/edit`.
pub async fn edit_item(
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
                "corps JSON invalide sur POST /overlay/media-board/items/{{id}}/edit"
            );
            return json_rejection_error(rejection);
        }
    };
    let actor = ActorInfo::from_auth(&state, &auth);
    start_edit(&state, &actor, &item_id_raw, body).await
}

/// Reads `{replace?, title?, recipe}`; the error is the `message` of the
/// `400 invalid_payload` response.
fn parse_request(body: serde_json::Value) -> Result<EditRequest, String> {
    let serde_json::Value::Object(mut object) = body else {
        return Err("a JSON object is expected".to_owned());
    };
    let replace = match object.get("replace") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(replace)) => *replace,
        Some(_) => return Err("replace must be a boolean".to_owned()),
    };
    let title = match object.get("title") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(title)) => {
            Some(truncate_chars(title, TITLE_MAX_CHARS)).filter(|title| !title.is_empty())
        }
        Some(_) => return Err("title must be a string".to_owned()),
    };
    let recipe = match object.remove("recipe") {
        Some(recipe @ serde_json::Value::Object(_)) => serde_json::from_value::<EditRecipe>(recipe)
            .map_err(|error| format!("recipe is invalid: {error}"))?,
        Some(_) => return Err("recipe must be an object".to_owned()),
        None => return Err("recipe is required".to_owned()),
    };
    Ok(EditRequest {
        replace,
        title,
        recipe,
    })
}

/// Display name of the edited file, without extension: the title without
/// path separators, control characters or `..`.
fn filename_stem(title: &str) -> String {
    let mut stem: String = title
        .chars()
        .map(|character| {
            if character == '/' || character == '\\' || character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    while stem.contains("..") {
        stem = stem.replace("..", ".");
    }
    let stem: String = stem
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(FILENAME_STEM_MAX_CHARS)
        .collect();
    let stem = stem.trim_matches(|character: char| character == '.' || character.is_whitespace());
    if stem.is_empty() {
        "media".to_owned()
    } else {
        stem.to_owned()
    }
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
    fn filename_stem_never_looks_like_a_path() {
        assert_eq!(
            filename_stem("Chat qui tombe (édité)"),
            "Chat qui tombe (édité)"
        );
        assert_eq!(filename_stem("../..\\etc/passwd"), "etc passwd");
        assert_eq!(filename_stem("a\u{0}b\nc"), "a b c");
        assert_eq!(filename_stem("wait for it...."), "wait for it");
        assert_eq!(filename_stem("   "), "media");
        assert_eq!(filename_stem(""), "media");
        assert_eq!(filename_stem(&"x".repeat(500)).chars().count(), 120);
    }

    #[test]
    fn request_body_is_validated() {
        let request = parse_request(json!({"recipe": {}})).unwrap();
        assert!(!request.replace);
        assert_eq!(request.title, None);
        assert_eq!(request.recipe, EditRecipe::default());

        let request =
            parse_request(json!({"replace": true, "title": "  Court  ", "recipe": {"speed": 2}}))
                .unwrap();
        assert!(request.replace);
        assert_eq!(request.title.as_deref(), Some("Court"));
        assert_eq!(request.recipe.speed, 2.0);

        let blank = parse_request(json!({"title": "   ", "recipe": {}})).unwrap();
        assert_eq!(blank.title, None);

        for (body, message) in [
            (json!([]), "a JSON object is expected"),
            (json!({}), "recipe is required"),
            (json!({"recipe": 3}), "recipe must be an object"),
            (
                json!({"recipe": {}, "replace": "yes"}),
                "replace must be a boolean",
            ),
            (json!({"recipe": {}, "title": 3}), "title must be a string"),
        ] {
            assert_eq!(parse_request(body).err().unwrap(), message);
        }
        let error = parse_request(json!({"recipe": {"speed": "fast"}}))
            .err()
            .unwrap();
        assert!(error.starts_with("recipe is invalid: "), "{error}");
    }
}
