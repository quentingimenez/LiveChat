//! `POST /overlay/profile/discord-link` — links the current member to a
//! Discord account with a `/pair-code` code.
//!
//! A member created from the admin panel or an invitation has no Discord
//! account, so the bot refuses their commands. They run `/pair-code` on
//! Discord and type the code in « Mon profil »: the code's author becomes the
//! member's Discord account. No overlay is paired.
//!
//! Body: `{"code": "K7QX-M2PD"}` (dashes and case ignored).
//!
//! | Status | Body |
//! |--------|------|
//! | 200 | `{"linked": true, "promoted": bool, "member": {…}}` |
//! | 400 | `invalid_payload` |
//! | 404 | `pairing_code_invalid_or_expired` (unknown, used, expired, another guild, read-only invitation) |
//! | 409 | `member_required`, `discord_already_linked` (another member of the guild has this account) |
//!
//! A code issued by a Discord administrator (`grants_admin`) also makes the
//! member an admin, as it would when pairing an overlay. The code is read
//! before any decision and consumed only once everything else is valid.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_domain::{GuildRole, PairingCode, PairingMode};
use lc_protocol::close_code;
use serde::Deserialize;
use serde_json::json;

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::guild::member_payload;
use super::{db, db_failure, error, is_conflict, json_response, now};

const CODE_INVALID: &str = "pairing_code_invalid_or_expired";

#[derive(Debug, Deserialize)]
pub(super) struct LinkRequest {
    #[serde(default)]
    code: Option<String>,
}

/// A Discord user id (snowflake), as opposed to an extension code author.
fn is_snowflake(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// Handler `POST /overlay/profile/discord-link`.
pub async fn link_discord(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<LinkRequest>, JsonRejection>,
) -> Response {
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let raw = payload.code.as_deref().map(str::trim).unwrap_or_default();
    if raw.is_empty() {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    }
    let Ok(code) = PairingCode::parse(&raw.replace('-', "")) else {
        return error(StatusCode::NOT_FOUND, CODE_INVALID);
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);

    let entry = match db.pairing_codes().get(&code).await {
        Ok(Some(entry)) if entry.is_usable_at(now) => entry,
        Ok(_) => return error(StatusCode::NOT_FOUND, CODE_INVALID),
        Err(db_error) => return db_failure("overlay_discord_profile.lookup", &db_error),
    };
    let user_id = entry.created_by_discord_user_id.clone();
    if entry.mode != PairingMode::Normal
        || !is_snowflake(&user_id)
        || entry.guild_id != member.guild_id
    {
        return error(StatusCode::NOT_FOUND, CODE_INVALID);
    }
    match db
        .guild_members()
        .find_active_by_discord_user(&member.guild_id, &user_id)
        .await
    {
        Ok(Some(other)) if other.id != member.id => {
            return error(StatusCode::CONFLICT, "discord_already_linked");
        }
        Ok(_) => {}
        Err(db_error) => return db_failure("overlay_discord_profile.owner", &db_error),
    }

    let entry = match db.pairing_codes().consume(&code, now).await {
        Ok(Some(entry)) => entry,
        Ok(None) => return error(StatusCode::NOT_FOUND, CODE_INVALID),
        Err(db_error) => return db_failure("overlay_discord_profile.consume", &db_error),
    };
    match db.guild_members().link_discord(member.id, &user_id).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(db_error) if is_conflict(&db_error) => {
            return error(StatusCode::CONFLICT, "discord_already_linked");
        }
        Err(db_error) => return db_failure("overlay_discord_profile.link", &db_error),
    }
    let promoted = entry.grants_admin && member.role != GuildRole::Admin;
    if promoted {
        if let Err(db_error) = db
            .guild_members()
            .set_role(member.id, GuildRole::Admin)
            .await
        {
            return db_failure("overlay_discord_profile.promote", &db_error);
        }
        // As for a role change from the admin panel: align the overlays'
        // mode and reconnect them so `welcome` carries it.
        if let Err(db_error) = db
            .overlay_clients()
            .update_mode_by_member(member.id, GuildRole::Admin.session_mode())
            .await
        {
            return db_failure("overlay_discord_profile.session_mode", &db_error);
        }
        match db.overlay_clients().list_active_by_member(member.id).await {
            Ok(clients) => {
                for client in clients {
                    state
                        .admin_hub
                        .close_client(&client.id.as_uuid(), close_code::GOING_AWAY);
                }
            }
            Err(db_error) => return db_failure("overlay_discord_profile.clients", &db_error),
        }
    }
    let stored = match db.guild_members().get(member.id).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(db_error) => return db_failure("overlay_discord_profile.reload", &db_error),
    };
    tracing::info!(member_id = %member.id, promoted, "compte Discord lié au membre");
    json_response(
        StatusCode::OK,
        json!({"linked": true, "promoted": promoted, "member": member_payload(&stored)}),
    )
}
