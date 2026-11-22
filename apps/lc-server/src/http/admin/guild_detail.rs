//! Guild detail and transposed guild routes of the admin console.
//!
//! Section 3 of `docs/rust/admin-console-api.md`:
//!
//! - `GET /admin/api/guilds/{id}`: one `overview.guilds[]` entry enriched with
//!   the guild member/invite/ban/queue lists and the board quota;
//! - `GET /admin/api/guilds/{id}/media-board/items`: the overlay board list.
//!
//! Section 4, "Transposées de l'overlay": the ten routes below reuse the body
//! of the overlay handlers through the shared functions of `overlay::*`. The
//! JSON payloads and error codes are those of the overlay routes; only the
//! authentication differs (`AdminAccess` instead of the overlay bearer token).
//! The admin console acts as a server operator: there is no calling member, so
//! `isSelf` is always `false`, edit permissions are bypassed and a device code
//! requires `memberId` in the body.

use axum::body::to_bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use lc_domain::{GuildId, OverlayClientId};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::http::overlay::{
    admin as overlay_admin, discord_link, guild as overlay_guild, media_board, media_edit,
    queue as overlay_queue, ActorInfo,
};

use super::auth::AdminAccess;
use super::{megabytes_to_bytes, require_db, to_non_empty_string, AdminError};

/// Validates the guild id from the path (`400 invalid_guild_id` otherwise).
fn parse_guild_id(raw: &str) -> Result<GuildId, AdminError> {
    GuildId::parse(raw).map_err(|_| AdminError::bad_request("invalid_guild_id"))
}

/// Maps the shared overlay result to an admin HTTP response.
///
/// Error responses produced by the extracted overlay logic are returned
/// untouched, so the status and `{"error": code}` body stay identical.
fn to_admin_response(result: Result<Value, Response>) -> Result<Response, AdminError> {
    match result {
        Ok(payload) => Ok(Json(payload).into_response()),
        Err(response) => Ok(response),
    }
}

/// Operator actor for the admin console.
///
/// The overlay client is used to sign media URLs and to tag an ingestion; the
/// first active client of the guild is reused when there is one so its media
/// URLs are resolvable, otherwise an unsaved placeholder is used.
async fn operator_actor(state: &AppState, guild_id: &GuildId) -> Result<ActorInfo, AdminError> {
    let db = require_db(state)?;
    let clients = db
        .overlay_clients()
        .list_by_guild(guild_id)
        .await
        .map_err(|error| AdminError::db("admin guild detail overlay clients", error))?;
    Ok(
        match clients.into_iter().find(|client| client.is_active()) {
            Some(client) => ActorInfo::operator_with_client(state, client),
            None => ActorInfo::operator(state, guild_id),
        },
    )
}

/// One `overview.guilds[]` entry, or `None` when the guild is unknown.
async fn overview_entry(
    state: &AppState,
    access: AdminAccess,
    guild_id: &GuildId,
) -> Result<Option<Value>, AdminError> {
    let response = super::overview::overview(State(state.clone()), access).await?;
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| AdminError::db("guild detail overview body", error))?;
    let payload: Value = serde_json::from_slice(&bytes)
        .map_err(|error| AdminError::db("guild detail overview JSON", error))?;
    Ok(payload
        .get("guilds")
        .and_then(Value::as_array)
        .and_then(|guilds| {
            guilds
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(guild_id.as_str()))
        })
        .cloned())
}

/// Complete guild detail entry (overview entry plus section 3 enrichments).
///
/// `overlayVersion`, `platform` and `guildSettings` are intentionally left out:
/// they are added by another task (protocol `hello` / device settings).
pub(super) async fn detail_entry(
    state: &AppState,
    access: AdminAccess,
    guild_id: &GuildId,
) -> Result<Option<Value>, AdminError> {
    let Some(mut entry) = overview_entry(state, access, guild_id).await? else {
        return Ok(None);
    };
    let db = require_db(state)?;
    let guild_access = db
        .guilds()
        .access(guild_id)
        .await
        .map_err(|error| AdminError::db("admin guild detail access", error))?;
    let board_quota_mb = db
        .guilds()
        .board_quota_mb(guild_id)
        .await
        .map_err(|error| AdminError::db("admin guild detail board quota", error))?;

    // Members, invites and queue reuse the very payload of their overlay route.
    let members = overlay_admin::members_payload(state, guild_id, None)
        .await
        .map_err(|_| AdminError::Internal)?;
    let invites = overlay_admin::invites_payload(state, guild_id)
        .await
        .map_err(|_| AdminError::Internal)?;
    let bans = overlay_admin::bans_payload(state, guild_id)
        .await
        .map_err(|_| AdminError::Internal)?;
    let queue = overlay_queue::queue_payload(state, guild_id)
        .await
        .map_err(|_| AdminError::Internal)?;

    // Enrich every overlay client with its attached member and effective role.
    if let Some(clients) = entry
        .get_mut("overlays")
        .and_then(Value::as_object_mut)
        .and_then(|overlays| overlays.get_mut("clients"))
        .and_then(Value::as_array_mut)
    {
        for client in clients {
            let Some(raw_id) = client.get("id").and_then(Value::as_str) else {
                continue;
            };
            let Ok(client_id) = raw_id.parse::<OverlayClientId>() else {
                continue;
            };
            let member = db
                .guild_members()
                .get_for_overlay_client(client_id)
                .await
                .map_err(|error| AdminError::db("admin guild detail client member", error))?;
            let (member_id, role) = match member {
                Some(member) => (
                    json!(member.id.to_canonical()),
                    json!(member.role.as_wire_str()),
                ),
                None => {
                    let role = match client.get("sessionMode").and_then(Value::as_str) {
                        Some("invite_read_only") => "guest",
                        _ => "member",
                    };
                    (Value::Null, json!(role))
                }
            };
            if let Some(object) = client.as_object_mut() {
                object.insert("memberId".to_owned(), member_id);
                object.insert("role".to_owned(), role);
            }
        }
    }

    if let Some(discord) = entry.get_mut("discord").and_then(Value::as_object_mut) {
        // No Discord presence feed is exposed to the server yet: `null` is the
        // contract's "unknown" value.
        discord.insert("botPresent".to_owned(), Value::Null);
    }
    entry["guestsCanViewBoard"] = json!(guild_access
        .as_ref()
        .is_none_or(|access| access.guests_can_view_board));
    entry["boardQuotaBytes"] = board_quota_mb
        .and_then(|megabytes| u64::try_from(megabytes).ok())
        .map(megabytes_to_bytes)
        .map_or(Value::Null, |bytes| json!(bytes));
    entry["members"] = members;
    entry["invites"] = invites;
    entry["bans"] = bans;
    entry["queue"] = queue;

    Ok(Some(entry))
}

/// Handler `GET /admin/api/guilds/{id}`.
pub(super) async fn guild_detail(
    State(state): State<AppState>,
    access: AdminAccess,
    Path(raw_guild_id): Path<String>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    match detail_entry(&state, access, &guild_id).await? {
        Some(entry) => Ok(Json(entry).into_response()),
        None => Err(AdminError::not_found("guild_not_found")),
    }
}

/// Handler `GET /admin/api/guilds/{id}/media-board/items`.
pub(super) async fn media_board_items(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    Query(query): Query<media_board::ListQuery>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    if let Some(requested) = query
        .guild_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if requested != guild_id.as_str() {
            return Ok((StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"}))).into_response());
        }
    }
    let actor = operator_actor(&state, &guild_id).await?;
    to_admin_response(media_board::list_items_payload(&state, &actor, &query).await)
}

/// Handler `PATCH /admin/api/guilds/{id}/members/{memberId}`.
pub(super) async fn update_member(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, member_id_raw)): Path<(String, String)>,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let Ok(Json(payload)) = payload else {
        return Err(AdminError::bad_request("invalid_payload"));
    };
    to_admin_response(
        overlay_admin::update_member_payload(&state, &guild_id, &member_id_raw, &payload).await,
    )
}

/// Handler `DELETE /admin/api/guilds/{id}/members/{memberId}`.
pub(super) async fn remove_member(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, member_id_raw)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    to_admin_response(overlay_admin::remove_member_payload(&state, &guild_id, &member_id_raw).await)
}

/// Handler `POST /admin/api/guilds/{id}/members/{memberId}/ban`.
pub(super) async fn ban_member(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, member_id_raw)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    to_admin_response(
        overlay_admin::ban_member_payload(
            &state,
            &guild_id,
            &member_id_raw,
            overlay_admin::BanActor::Operator,
        )
        .await,
    )
}

/// Handler `DELETE /admin/api/guilds/{id}/bans/{banId}`.
pub(super) async fn unban(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, ban_id_raw)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    to_admin_response(overlay_admin::unban_payload(&state, &guild_id, &ban_id_raw).await)
}

/// Handler `POST /admin/api/guilds/{id}/invites`.
pub(super) async fn create_invite(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let Ok(Json(payload)) = payload else {
        return Err(AdminError::bad_request("invalid_payload"));
    };
    to_admin_response(overlay_admin::create_invite_payload(&state, &guild_id, None, &payload).await)
}

/// Handler `POST /admin/api/guilds/{id}/invites/{code}/revoke`.
pub(super) async fn revoke_invite(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, code)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    to_admin_response(overlay_admin::revoke_invite_payload(&state, &guild_id, &code).await)
}

/// Handler `POST /admin/api/guilds/{id}/discord-link-code`.
pub(super) async fn discord_link_code(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    to_admin_response(discord_link::create_link_code_payload(&state, &guild_id, None).await)
}

/// Handler `POST /admin/api/guilds/{id}/device-code`.
///
/// The device is bound to the member supplied as `{"memberId": "..."}`: unlike
/// the overlay route, the admin console has no calling member.
pub(super) async fn device_code(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let Ok(Json(payload)) = payload else {
        return Err(AdminError::bad_request("invalid_payload"));
    };
    let Some(member_id_raw) = to_non_empty_string(payload.get("memberId")) else {
        return Err(AdminError::bad_request("invalid_payload"));
    };
    let db = require_db(&state)?;
    let member = match overlay_admin::load_guild_member(db, &guild_id, &member_id_raw).await {
        Ok(member) => member,
        Err(response) => return Ok(response),
    };
    to_admin_response(
        overlay_guild::create_device_code_payload(&state, &guild_id, member.id, member.role, None)
            .await,
    )
}

/// Handler `DELETE /admin/api/guilds/{id}/queue`.
pub(super) async fn clear_queue(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    to_admin_response(overlay_queue::clear_queue_payload(&state, &guild_id).await)
}

/// Handler `DELETE /admin/api/guilds/{id}/queue/{jobId}`.
pub(super) async fn remove_queue_job(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, job_id_raw)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    to_admin_response(overlay_queue::remove_job_payload(&state, &guild_id, &job_id_raw).await)
}

/// Handler `POST /admin/api/guilds/{id}/queue/reorder`.
pub(super) async fn reorder_queue(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_guild_id): Path<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let Ok(Json(body)) = payload else {
        return Err(AdminError::bad_request("invalid_payload"));
    };
    let Some(ids) = body.get("jobIds").and_then(Value::as_array) else {
        return Err(AdminError::bad_request("invalid_payload"));
    };
    let mut job_ids = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(id) = id.as_str() else {
            return Err(AdminError::bad_request("invalid_payload"));
        };
        job_ids.push(id.to_owned());
    }
    to_admin_response(overlay_queue::reorder_queue_payload(&state, &guild_id, &job_ids).await)
}

/// Handler `PATCH /admin/api/guilds/{id}/media-board/items/{itemId}`.
///
/// A body carrying an edit `recipe` (or `replace`) is handed to the overlay
/// media editor (`202 {"ingestionId"}`); otherwise the overlay item update is
/// applied (`title`, `message`, `artworkOnOverlay`).
pub(super) async fn update_media_item(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, item_id_raw)): Path<(String, String)>,
    payload: Result<Json<Value>, JsonRejection>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let Ok(Json(body)) = payload else {
        return Err(AdminError::bad_request("invalid_payload"));
    };
    let actor = ActorInfo::operator(&state, &guild_id);
    if body.get("recipe").is_some() || body.get("replace").is_some() {
        return Ok(media_edit::start_edit(&state, &actor, &item_id_raw, body).await);
    }
    to_admin_response(media_board::update_item_payload(&state, &actor, &item_id_raw, body).await)
}

/// Handler `DELETE /admin/api/guilds/{id}/media-board/items/{itemId}`.
pub(super) async fn delete_media_item(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path((raw_guild_id, item_id_raw)): Path<(String, String)>,
) -> Result<Response, AdminError> {
    let guild_id = parse_guild_id(&raw_guild_id)?;
    let actor = ActorInfo::operator(&state, &guild_id);
    to_admin_response(media_board::delete_item_payload(&state, &actor, &item_id_raw).await)
}

/// Routes of this module, merged into the secured admin API router.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/api/guilds/{guild_id}", get(guild_detail))
        .route(
            "/admin/api/guilds/{guild_id}/media-board/items",
            get(media_board_items),
        )
        .route(
            "/admin/api/guilds/{guild_id}/members/{member_id}",
            patch(update_member).delete(remove_member),
        )
        .route(
            "/admin/api/guilds/{guild_id}/members/{member_id}/ban",
            post(ban_member),
        )
        .route("/admin/api/guilds/{guild_id}/bans/{ban_id}", delete(unban))
        .route("/admin/api/guilds/{guild_id}/invites", post(create_invite))
        .route(
            "/admin/api/guilds/{guild_id}/invites/{code}/revoke",
            post(revoke_invite),
        )
        .route(
            "/admin/api/guilds/{guild_id}/discord-link-code",
            post(discord_link_code),
        )
        .route(
            "/admin/api/guilds/{guild_id}/device-code",
            post(device_code),
        )
        .route("/admin/api/guilds/{guild_id}/queue", delete(clear_queue))
        .route(
            "/admin/api/guilds/{guild_id}/queue/reorder",
            post(reorder_queue),
        )
        .route(
            "/admin/api/guilds/{guild_id}/queue/{job_id}",
            delete(remove_queue_job),
        )
        .route(
            "/admin/api/guilds/{guild_id}/media-board/items/{item_id}",
            patch(update_media_item).delete(delete_media_item),
        )
}
