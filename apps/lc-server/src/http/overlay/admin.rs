//! Administration d'une guilde depuis l'overlay (rôle admin).
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `GET` | `/overlay/admin/members` | `ManageMembers` |
//! | `PATCH` | `/overlay/admin/members/{memberId}` | `ManageMembers` |
//! | `DELETE` | `/overlay/admin/members/{memberId}` | `ManageMembers` |
//! | `POST` | `/overlay/admin/members/{memberId}/ban` | `ManageMembers` |
//! | `GET` | `/overlay/admin/bans` | `ManageMembers` |
//! | `DELETE` | `/overlay/admin/bans/{banId}` | `ManageMembers` |
//! | `GET` | `/overlay/admin/invites` | `ManageInvites` |
//! | `POST` | `/overlay/admin/invites` | `ManageInvites` |
//! | `POST` | `/overlay/admin/invites/{code}/revoke` | `ManageInvites` |
//! | `GET` | `/overlay/admin/pairing-codes` | `ManageInvites` |
//! | `POST` | `/overlay/admin/pairing-codes/{code}/revoke` | `ManageInvites` |
//! | `DELETE` | `/overlay/admin/pairing-codes/expired` | `ManageInvites` |
//! | `PATCH` | `/overlay/admin/settings` | `ManageSettings` |
//! | `POST` | `/overlay/admin/media-board/clear` | `ManageSettings` |
//! | `GET` | `/overlay/admin/audit` | `ManageSettings` |
//!
//! Toutes les routes sont bornées à la guilde du jeton : un identifiant de
//! membre ou un code d'une autre guilde répond « introuvable ».
//!
//! Bannir retire le membre comme une exclusion (fermeture `4402
//! member_banned`) et garde son compte Discord lié hors de la guilde : plus
//! de `/pair-code`, de code consommé ni de commande du bot. Un membre sans
//! compte Discord lié n'est identifiable par rien d'autre : le bannissement
//! se réduit alors à l'exclusion.
//!
//! Changer un rôle aligne `overlay_clients.session_mode` puis ferme les
//! sessions WS du membre (`1001`) : ses overlays se reconnectent aussitôt et
//! reprennent leur mode à jour dans `welcome`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_db::MemberChange;
use lc_domain::{
    Guild, GuildBan, GuildBanId, GuildMember, GuildMemberId, GuildRole, PairingCode,
    PairingCodeEntry, Permission,
};
use lc_protocol::{close_code, close_reason};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use time::Duration;

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::board_clear;
use super::guild::{
    create_invite, invite_payload, iso, member_payload, normalize_label, remove_member, require,
    NewInvite,
};
use super::ingest_code::INGEST_CODE_CREATOR_PREFIX;
use super::{db, db_failure, error, internal_error, is_conflict, json_response, now};
use crate::http::pairing_codes;

/// Durée de vie maximale d'une invitation (un an).
const INVITE_MAX_TTL_HOURS: i64 = 24 * 365;
/// Nombre maximal d'utilisations d'une invitation.
const INVITE_MAX_USES: i64 = 1_000;
/// Longueur maximale du nom d'une guilde.
const GUILD_NAME_MAX_CHARS: usize = 60;
/// Taille de page par défaut du journal de la guilde.
const AUDIT_DEFAULT_LIMIT: usize = 50;
/// Taille de page maximale du journal de la guilde.
const AUDIT_MAX_LIMIT: usize = 200;

/// Loads an active member of the guild (`404 member_not_found` otherwise).
pub(crate) async fn load_guild_member(
    db: &lc_db::Db,
    guild_id: &lc_domain::GuildId,
    member_id_raw: &str,
) -> Result<GuildMember, Response> {
    let Ok(member_id) = member_id_raw.trim().parse::<GuildMemberId>() else {
        return Err(error(StatusCode::NOT_FOUND, "member_not_found"));
    };
    match db.guild_members().get(member_id).await {
        Ok(Some(member)) if member.is_active() && &member.guild_id == guild_id => Ok(member),
        Ok(_) => Err(error(StatusCode::NOT_FOUND, "member_not_found")),
        Err(db_error) => Err(db_failure("overlay_admin.member_lookup", &db_error)),
    }
}

/// Members body shared by `GET /overlay/admin/members` and the admin guild
/// detail. `caller_member_id` only drives `isSelf`; the admin console passes
/// `None`.
pub(crate) async fn members_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    caller_member_id: Option<GuildMemberId>,
) -> Result<Value, Response> {
    let db = db(state)?;
    let members = db
        .guild_members()
        .list_by_guild(guild_id)
        .await
        .map_err(|db_error| db_failure("overlay_admin.list_members", &db_error))?;

    let mut items = Vec::with_capacity(members.len());
    for member in &members {
        let clients = db
            .overlay_clients()
            .list_active_by_member(member.id)
            .await
            .map_err(|db_error| db_failure("overlay_admin.member_clients", &db_error))?;
        let connected = clients
            .iter()
            .any(|client| state.admin_hub.is_connected(&client.id.as_uuid()));
        let last_seen_at = clients
            .iter()
            .filter_map(|client| client.last_seen_at)
            .max();
        let mut payload = member_payload(member);
        if let Some(object) = payload.as_object_mut() {
            object.insert("connected".to_owned(), json!(connected));
            object.insert("deviceCount".to_owned(), json!(clients.len()));
            object.insert("lastSeenAt".to_owned(), iso(last_seen_at));
            object.insert(
                "isSelf".to_owned(),
                json!(caller_member_id == Some(member.id)),
            );
        }
        items.push(payload);
    }

    Ok(json!({"count": items.len(), "items": items}))
}

/// Handler `GET /overlay/admin/members`.
pub async fn list_members(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    match members_payload(&state, auth.guild_id(), auth.member_id()).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Updates a member role and realigns its overlay sessions.
pub(crate) async fn update_member_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    member_id_raw: &str,
    payload: &Value,
) -> Result<Value, Response> {
    let Some(role) = payload
        .get("role")
        .and_then(Value::as_str)
        .and_then(|raw| GuildRole::from_wire_str(raw).ok())
    else {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_role"));
    };
    let db = db(state)?;
    let member = load_guild_member(db, guild_id, member_id_raw).await?;

    match db.guild_members().set_role(member.id, role).await {
        Ok(MemberChange::Applied) => {}
        Ok(MemberChange::LastAdmin) => return Err(error(StatusCode::CONFLICT, "last_admin")),
        Ok(MemberChange::NotFound) => {
            return Err(error(StatusCode::NOT_FOUND, "member_not_found"));
        }
        Err(db_error) => return Err(db_failure("overlay_admin.set_role", &db_error)),
    }

    if role != member.role {
        if let Err(db_error) = db
            .overlay_clients()
            .update_mode_by_member(member.id, role.session_mode())
            .await
        {
            return Err(db_failure("overlay_admin.sync_session_mode", &db_error));
        }
        // Reconnexion immédiate des overlays du membre : le nouveau mode est
        // repris au `welcome` suivant.
        match db.overlay_clients().list_active_by_member(member.id).await {
            Ok(clients) => {
                for client in clients {
                    state
                        .admin_hub
                        .close_client(&client.id.as_uuid(), close_code::GOING_AWAY);
                }
            }
            Err(db_error) => return Err(db_failure("overlay_admin.member_clients", &db_error)),
        }
        tracing::info!(
            guild_id = %member.guild_id,
            member_id = %member.id,
            role = role.as_wire_str(),
            "rôle de membre modifié"
        );
    }

    Ok(json!({
        "updated": true,
        "member": {"id": member.id.to_canonical(), "role": role.as_wire_str()},
    }))
}

/// Handler `PATCH /overlay/admin/members/{memberId}` (corps `{"role": "…"}`).
pub async fn update_member(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(member_id_raw): Path<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    match update_member_payload(&state, auth.guild_id(), &member_id_raw, &payload).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Removes a member (shared by the overlay and the admin console).
pub(crate) async fn remove_member_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    member_id_raw: &str,
) -> Result<Value, Response> {
    let db = db(state)?;
    let member = load_guild_member(db, guild_id, member_id_raw).await?;
    match remove_member(state, db, &member, now(state), close_reason::MEMBER_REMOVED).await {
        Ok(MemberChange::Applied) => {
            tracing::info!(
                guild_id = %member.guild_id,
                member_id = %member.id,
                "membre exclu"
            );
            Ok(json!({"removed": true}))
        }
        Ok(MemberChange::LastAdmin) => Err(error(StatusCode::CONFLICT, "last_admin")),
        Ok(MemberChange::NotFound) => Err(error(StatusCode::NOT_FOUND, "member_not_found")),
        Err(response) => Err(response),
    }
}

/// Handler `DELETE /overlay/admin/members/{memberId}` : exclusion.
pub async fn remove_member_handler(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(member_id_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    match remove_member_payload(&state, auth.guild_id(), &member_id_raw).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Who bans: the calling member (overlay) or the server operator (console).
#[derive(Clone, Copy, Debug)]
pub(crate) enum BanActor<'a> {
    /// An admin of the guild, from their overlay.
    Member(&'a GuildMember),
    /// The server operator, from the admin console.
    Operator,
}

/// JSON of a ban.
fn ban_payload(ban: &GuildBan) -> Value {
    json!({
        "id": ban.id.to_canonical(),
        "memberId": ban.member_id.map(GuildMemberId::to_canonical),
        "displayName": ban.display_name,
        "avatar": ban.avatar,
        "discordUserId": ban.discord_user_id,
        "bannedBy": ban.banned_by,
        "createdAt": iso(Some(ban.created_at)),
    })
}

/// Bans a member (shared by the overlay and the admin console): removes them
/// like an exclusion, then records the ban.
pub(crate) async fn ban_member_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    member_id_raw: &str,
    actor: BanActor<'_>,
) -> Result<Value, Response> {
    let db = db(state)?;
    let member = load_guild_member(db, guild_id, member_id_raw).await?;
    if let BanActor::Member(caller) = actor {
        if caller.id == member.id {
            return Err(error(StatusCode::CONFLICT, "cannot_ban_self"));
        }
    }
    let at = now(state);
    match remove_member(state, db, &member, at, close_reason::MEMBER_BANNED).await? {
        MemberChange::Applied => {}
        MemberChange::LastAdmin => return Err(error(StatusCode::CONFLICT, "last_admin")),
        MemberChange::NotFound => return Err(error(StatusCode::NOT_FOUND, "member_not_found")),
    }
    let ban = GuildBan {
        id: GuildBanId::new_v4(),
        guild_id: member.guild_id.clone(),
        member_id: Some(member.id),
        discord_user_id: member.discord_user_id.clone(),
        display_name: member.display_name.clone(),
        avatar: member.avatar.clone(),
        banned_by: match actor {
            BanActor::Member(caller) => Some(caller.display_name.clone()),
            BanActor::Operator => None,
        },
        created_at: at,
    };
    match db.guild_bans().create(&ban).await {
        Ok(()) => {}
        // The account was already banned (a ban lifted then a concurrent
        // one): the member is removed, the existing ban stands.
        Err(db_error) if is_conflict(&db_error) => {}
        Err(db_error) => return Err(db_failure("overlay_admin.create_ban", &db_error)),
    }
    tracing::info!(
        guild_id = %member.guild_id,
        member_id = %member.id,
        discord_linked = member.discord_user_id.is_some(),
        "membre banni"
    );
    Ok(json!({"banned": true, "ban": ban_payload(&ban)}))
}

/// Handler `POST /overlay/admin/members/{memberId}/ban`.
pub async fn ban_member_handler(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(member_id_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    let Some(caller) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    match ban_member_payload(
        &state,
        auth.guild_id(),
        &member_id_raw,
        BanActor::Member(caller),
    )
    .await
    {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Bans body shared by `GET /overlay/admin/bans` and the admin guild detail.
pub(crate) async fn bans_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
) -> Result<Value, Response> {
    let db = db(state)?;
    let bans = db
        .guild_bans()
        .list_by_guild(guild_id)
        .await
        .map_err(|db_error| db_failure("overlay_admin.list_bans", &db_error))?;
    let items: Vec<Value> = bans.iter().map(ban_payload).collect();
    Ok(json!({"count": items.len(), "items": items}))
}

/// Handler `GET /overlay/admin/bans`.
pub async fn list_bans(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    match bans_payload(&state, auth.guild_id()).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Lifts a ban (shared by the overlay and the admin console). The person
/// needs a new code to come back.
pub(crate) async fn unban_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    ban_id_raw: &str,
) -> Result<Value, Response> {
    let db = db(state)?;
    let Ok(ban_id) = ban_id_raw.trim().parse::<GuildBanId>() else {
        return Err(error(StatusCode::NOT_FOUND, "ban_not_found"));
    };
    match db.guild_bans().delete(guild_id, ban_id).await {
        Ok(true) => {
            tracing::info!(guild_id = %guild_id, ban_id = %ban_id, "bannissement levé");
            Ok(json!({"unbanned": true}))
        }
        Ok(false) => Err(error(StatusCode::NOT_FOUND, "ban_not_found")),
        Err(db_error) => Err(db_failure("overlay_admin.delete_ban", &db_error)),
    }
}

/// Handler `DELETE /overlay/admin/bans/{banId}`.
pub async fn unban_handler(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(ban_id_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    match unban_payload(&state, auth.guild_id(), &ban_id_raw).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Invites body shared by `GET /overlay/admin/invites` and the admin guild
/// detail.
pub(crate) async fn invites_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
) -> Result<Value, Response> {
    let db = db(state)?;
    let now = now(state);
    let invites = db
        .guild_invites()
        .list_by_guild(guild_id)
        .await
        .map_err(|db_error| db_failure("overlay_admin.list_invites", &db_error))?;
    // Device codes are personal: they are never listed.
    let items: Vec<Value> = invites
        .iter()
        .filter(|invite| invite.member_id.is_none())
        .map(|invite| invite_payload(invite, now))
        .collect();
    Ok(json!({"count": items.len(), "items": items}))
}

/// Handler `GET /overlay/admin/invites`.
pub async fn list_invites(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageInvites) {
        return response;
    }
    match invites_payload(&state, auth.guild_id()).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Lit un entier optionnel strictement positif borné : `Ok(None)` si absent ou
/// `null`, `Err(())` si mal typé ou hors bornes.
fn optional_bounded(payload: &Value, key: &str, max: i64) -> Result<Option<i64>, ()> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_i64() {
            Some(number) if (1..=max).contains(&number) => Ok(Some(number)),
            _ => Err(()),
        },
    }
}

/// Creates an invite for the guild (shared by the overlay and the admin
/// console).
pub(crate) async fn create_invite_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    created_by_member_id: Option<GuildMemberId>,
    payload: &Value,
) -> Result<Value, Response> {
    let Some(role) = payload
        .get("role")
        .and_then(Value::as_str)
        .and_then(|raw| GuildRole::from_wire_str(raw).ok())
    else {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_role"));
    };
    let Ok(expires_in_hours) = optional_bounded(payload, "expiresInHours", INVITE_MAX_TTL_HOURS)
    else {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_expiry"));
    };
    let Ok(max_uses) = optional_bounded(payload, "maxUses", INVITE_MAX_USES) else {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_max_uses"));
    };
    let db = db(state)?;
    let now = now(state);
    let invite = NewInvite {
        guild_id: guild_id.clone(),
        role,
        member_id: None,
        max_uses,
        created_by_member_id,
        expires_at: expires_in_hours.map(|hours| now + Duration::hours(hours)),
    };
    match create_invite(db, invite, now).await {
        Ok(Some(invite)) => Ok(json!({
            "created": true,
            "invite": invite_payload(&invite, now),
        })),
        Ok(None) => Err(internal_error()),
        Err(db_error) => Err(db_failure("overlay_admin.create_invite", &db_error)),
    }
}

/// Handler `POST /overlay/admin/invites`.
///
/// Corps : `{"role": "admin|member|guest", "expiresInHours": n|null,
/// "maxUses": n|null}` (`null` = jamais / illimité).
pub async fn create_invite_handler(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageInvites) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    match create_invite_payload(&state, auth.guild_id(), auth.member_id(), &payload).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Revokes an invite of the guild (shared by the overlay and the admin
/// console).
pub(crate) async fn revoke_invite_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    code_raw: &str,
) -> Result<Value, Response> {
    let Ok(code) = PairingCode::parse(&code_raw.replace('-', "")) else {
        return Err(error(StatusCode::NOT_FOUND, "invite_not_found"));
    };
    let db = db(state)?;
    match db.guild_invites().get(&code).await {
        Ok(Some(invite)) if &invite.guild_id == guild_id => {}
        Ok(_) => return Err(error(StatusCode::NOT_FOUND, "invite_not_found")),
        Err(db_error) => return Err(db_failure("overlay_admin.invite_lookup", &db_error)),
    }
    match db.guild_invites().revoke(guild_id, &code, now(state)).await {
        Ok(revoked) => Ok(json!({"found": true, "revoked": revoked})),
        Err(db_error) => Err(db_failure("overlay_admin.revoke_invite", &db_error)),
    }
}

/// Handler `POST /overlay/admin/invites/{code}/revoke`.
pub async fn revoke_invite(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(code_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageInvites) {
        return response;
    }
    match revoke_invite_payload(&state, auth.guild_id(), &code_raw).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Nature d'un code d'appairage telle que l'overlay l'affiche : `extension`
/// pour un code émis depuis un overlay (auteur `member:<id>`), `discord` pour
/// un code `/pair-code` émis par le bot.
fn pairing_kind(code: &PairingCodeEntry) -> &'static str {
    if code
        .created_by_discord_user_id
        .starts_with(INGEST_CODE_CREATOR_PREFIX)
    {
        "extension"
    } else {
        "discord"
    }
}

/// Codes body shared by `GET /overlay/admin/pairing-codes`: every code of the
/// guild (Discord `/pair-code` and browser-extension codes), newest first,
/// with its kind and the member display name of an extension code's author.
async fn pairing_codes_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
) -> Result<Value, Response> {
    let db = db(state)?;
    let now = now(state);
    let codes = pairing_codes::list(db, Some(guild_id))
        .await
        .map_err(|db_error| db_failure("overlay_admin.list_pairing_codes", &db_error))?;
    let members = db
        .guild_members()
        .list_by_guild(guild_id)
        .await
        .map_err(|db_error| db_failure("overlay_admin.pairing_members", &db_error))?;
    let names: HashMap<String, String> = members
        .into_iter()
        .map(|member| (member.id.to_canonical(), member.display_name))
        .collect();

    let items: Vec<Value> = codes
        .iter()
        .map(|code| {
            let kind = pairing_kind(code);
            let author_name = if kind == "extension" {
                code.created_by_discord_user_id
                    .strip_prefix(INGEST_CODE_CREATOR_PREFIX)
                    .and_then(|member_id| names.get(member_id).cloned())
                    .or_else(|| code.author_name.clone())
            } else {
                code.author_name.clone()
            };
            let mut payload = pairing_codes::payload(code, now);
            payload["kind"] = json!(kind);
            // The current member name wins over the one stored at issue time.
            payload["authorName"] = json!(author_name);
            payload
        })
        .collect();

    Ok(json!({"count": items.len(), "items": items}))
}

/// Handler `GET /overlay/admin/pairing-codes`.
pub async fn list_pairing_codes(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageInvites) {
        return response;
    }
    match pairing_codes_payload(&state, auth.guild_id()).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Revokes a pairing code of the guild (overlay side).
///
/// A code belonging to another guild answers exactly like an unknown code
/// (`404 pairing_code_not_found`): the route never reveals that it exists.
async fn revoke_pairing_code_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    code_raw: &str,
) -> Result<Value, Response> {
    let Ok(code) = PairingCode::parse(code_raw) else {
        return Err(error(StatusCode::NOT_FOUND, "pairing_code_not_found"));
    };
    let db = db(state)?;
    let entry = match db.pairing_codes().get(&code).await {
        Ok(Some(entry)) if &entry.guild_id == guild_id => entry,
        Ok(_) => return Err(error(StatusCode::NOT_FOUND, "pairing_code_not_found")),
        Err(db_error) => return Err(db_failure("overlay_admin.pairing_lookup", &db_error)),
    };
    let now_ms = lc_domain::datetime_to_ms(now(state));
    let revoked = pairing_codes::revoke(db, code.as_str(), now_ms)
        .await
        .map_err(|db_error| db_failure("overlay_admin.revoke_pairing_code", &db_error))?;
    if revoked > 0 {
        tracing::info!(
            code = code.as_str(),
            guild_id = %entry.guild_id,
            "code de pairing révoqué depuis l'overlay"
        );
    }
    Ok(json!({"revoked": revoked > 0}))
}

/// Handler `POST /overlay/admin/pairing-codes/{code}/revoke`.
pub async fn revoke_pairing_code(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(code_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageInvites) {
        return response;
    }
    match revoke_pairing_code_payload(&state, auth.guild_id(), &code_raw).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Handler `DELETE /overlay/admin/pairing-codes/expired` : purge des codes de
/// la guilde qui sont utilisés ou expirés.
pub async fn purge_pairing_codes(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageInvites) {
        return response;
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now_ms = lc_domain::datetime_to_ms(now(&state));
    match pairing_codes::delete_expired(db, Some(auth.guild_id().as_str()), now_ms).await {
        Ok(deleted) => json_response(StatusCode::OK, json!({"deleted": deleted})),
        Err(db_error) => db_failure("overlay_admin.purge_pairing_codes", &db_error),
    }
}

/// Lit une durée optionnelle : absent → `None` ; `null` ou `<= 0` efface
/// (`Some(None)`) ; entier positif → `Some(Some(n))` ; autre → erreur.
fn duration_field(payload: &Value, key: &str) -> Result<Option<Option<i64>>, ()> {
    match payload.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => match value.as_i64() {
            Some(number) if number <= 0 => Ok(Some(None)),
            Some(number) if number <= 86_400 => Ok(Some(Some(number))),
            _ => Err(()),
        },
    }
}

/// Lit un booléen optionnel : absent → `None`, mal typé → erreur.
fn bool_field(payload: &Value, key: &str) -> Result<Option<bool>, ()> {
    match payload.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(()),
    }
}

/// Lit `boardQuotaMb` : absent → `None` ; `null` ou chaîne vide → retour au
/// quota du serveur (`Some(None)`) ; entier strictement positif → `Some(Some)`.
///
/// Mêmes règles que la console du propriétaire (`PATCH
/// /admin/api/guilds/{id}/settings`) ; la borne haute (quota du serveur) est
/// vérifiée par l'appelant.
fn board_quota_field(payload: &Value) -> Result<Option<Option<i64>>, ()> {
    match payload.get("boardQuotaMb") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(raw)) if raw.trim().is_empty() => Ok(Some(None)),
        Some(Value::Number(number)) => match number.as_i64() {
            Some(megabytes) if megabytes > 0 => Ok(Some(Some(megabytes))),
            _ => Err(()),
        },
        Some(_) => Err(()),
    }
}

/// Handler `PATCH /overlay/admin/settings`.
///
/// Corps partiel : `name`, `defaultMediaTime`, `maxMediaTime`,
/// `displayMediaFull`, `guestsCanViewBoard`, `boardQuotaMb` (`null` = quota du
/// serveur, entier en Mo sinon, jamais au-dessus de la limite du serveur). Les
/// réglages de lecture sont rediffusés aux overlays connectés (`settings`).
pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageSettings) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    if !payload.is_object() {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    }
    let name = match payload.get("name") {
        None => None,
        Some(Value::String(raw)) => {
            let Some(value) = normalize_label(raw, GUILD_NAME_MAX_CHARS) else {
                return error(StatusCode::BAD_REQUEST, "invalid_name");
            };
            Some(value)
        }
        Some(_) => return error(StatusCode::BAD_REQUEST, "invalid_name"),
    };
    let (Ok(default_media_time), Ok(max_media_time), Ok(display_media_full), Ok(guests), Ok(quota)) = (
        duration_field(&payload, "defaultMediaTime"),
        duration_field(&payload, "maxMediaTime"),
        bool_field(&payload, "displayMediaFull"),
        bool_field(&payload, "guestsCanViewBoard"),
        board_quota_field(&payload),
    ) else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    if name.is_none()
        && default_media_time.is_none()
        && max_media_time.is_none()
        && display_media_full.is_none()
        && guests.is_none()
        && quota.is_none()
    {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    }
    // Le quota du serveur reste le plafond : une guilde peut le réduire, jamais
    // le dépasser (contrairement à la console propriétaire, qui édite la limite
    // de l'instance elle-même).
    let server_quota_mb =
        i64::try_from(state.runtime().media_board_max_total_mb()).unwrap_or(i64::MAX);
    if let Some(Some(megabytes)) = quota {
        if megabytes > server_quota_mb {
            return error(StatusCode::BAD_REQUEST, "invalid_board_quota");
        }
    }

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let guild_id = auth.guild_id();
    let existing = match db.guilds().get(guild_id).await {
        Ok(existing) => existing,
        Err(db_error) => return db_failure("overlay_admin.settings_lookup", &db_error),
    };

    let playback_changed =
        default_media_time.is_some() || max_media_time.is_some() || display_media_full.is_some();
    let guild = Guild {
        id: guild_id.clone(),
        name: name
            .clone()
            .or_else(|| existing.as_ref().and_then(|guild| guild.name.clone())),
        busy_until: existing.as_ref().and_then(|guild| guild.busy_until),
        default_media_time: default_media_time
            .unwrap_or_else(|| existing.as_ref().and_then(|guild| guild.default_media_time)),
        max_media_time: max_media_time
            .unwrap_or_else(|| existing.as_ref().and_then(|guild| guild.max_media_time)),
        display_media_full: display_media_full.unwrap_or_else(|| {
            existing
                .as_ref()
                .is_some_and(|guild| guild.display_media_full)
        }),
    };
    if let Err(db_error) = db.guilds().upsert(&guild).await {
        return db_failure("overlay_admin.settings_save", &db_error);
    }
    if let Some(allowed) = guests {
        if let Err(db_error) = db
            .guilds()
            .set_guests_can_view_board(guild_id, allowed)
            .await
        {
            return db_failure("overlay_admin.settings_guests", &db_error);
        }
    }
    if let Some(quota_mb) = quota {
        if let Err(db_error) = db.guilds().set_board_quota_mb(guild_id, quota_mb).await {
            return db_failure("overlay_admin.settings_quota", &db_error);
        }
    }
    if playback_changed {
        state.admin_hub.broadcast_settings(guild_id).await;
    }

    let guests_can_view_board = match db.guilds().access(guild_id).await {
        Ok(access) => access.is_none_or(|access| access.guests_can_view_board),
        Err(db_error) => return db_failure("overlay_admin.settings_reload", &db_error),
    };
    let board_quota_mb = match db.guilds().board_quota_mb(guild_id).await {
        Ok(quota_mb) => quota_mb,
        Err(db_error) => return db_failure("overlay_admin.settings_quota_reload", &db_error),
    };
    json_response(
        StatusCode::OK,
        json!({
            "updated": true,
            "guild": {
                "id": guild_id.as_str(),
                "name": guild.name,
                "guestsCanViewBoard": guests_can_view_board,
                "settings": {
                    "defaultMediaTime": guild.default_media_time,
                    "maxMediaTime": guild.max_media_time,
                    "displayMediaFull": guild.display_media_full,
                },
                "boardQuotaMb": board_quota_mb,
            },
        }),
    )
}

/// Handler `POST /overlay/admin/media-board/clear`.
///
/// Corps `{"confirm": "<nom de la guilde>"}` : le nom doit correspondre
/// exactement (`400 confirmation_mismatch` sinon), à la manière de la
/// confirmation forte de la console. Le vidage lui-même est partagé avec la
/// route propriétaire (`board_clear::clear_board`).
pub async fn clear_media_board(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageSettings) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let guild_name = match db.guilds().access(auth.guild_id()).await {
        Ok(access) => access.and_then(|access| access.name),
        Err(db_error) => return db_failure("overlay_admin.board_clear_access", &db_error),
    };
    let confirmation = payload.get("confirm").and_then(Value::as_str);
    // Nom inconnu : rien ne peut correspondre, la confirmation est refusée.
    if guild_name.as_deref().is_none() || confirmation != guild_name.as_deref() {
        return error(StatusCode::BAD_REQUEST, "confirmation_mismatch");
    }
    match board_clear::clear_board(&state, auth.guild_id()).await {
        Ok(report) => json_response(
            StatusCode::OK,
            json!({
                "guildId": auth.guild_id().as_str(),
                "stop": {
                    "releasedPlayingJobs": report.released_playing_jobs,
                    "broadcast": report.broadcast,
                },
                "deleted": {
                    "mediaBoardItems": report.media_board_items,
                    "orphanMediaAssets": report.orphan_media_assets,
                    "orphanMediaFileDeleteErrors": report.orphan_media_file_delete_errors,
                },
            }),
        ),
        Err(response) => response,
    }
}

/// Query parameters of `GET /overlay/admin/audit`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AuditQuery {
    limit: Option<String>,
}

impl AuditQuery {
    /// Taille de page demandée, bornée à `[1, AUDIT_MAX_LIMIT]`.
    fn limit(&self) -> Result<usize, Response> {
        let Some(raw) = self
            .limit
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(AUDIT_DEFAULT_LIMIT);
        };
        let Ok(value) = raw.parse::<usize>() else {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_limit"));
        };
        Ok(value.clamp(1, AUDIT_MAX_LIMIT))
    }
}

/// Handler `GET /overlay/admin/audit?limit=50` : journal d'administration de
/// cette guilde, du plus récent au plus ancien.
pub async fn list_audit(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Query(query): Query<AuditQuery>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageSettings) {
        return response;
    }
    let limit = match query.limit() {
        Ok(limit) => limit,
        Err(response) => return response,
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    match crate::http::admin::audit_rows_for_guild(db, auth.guild_id().as_str(), limit).await {
        Ok(items) => json_response(
            StatusCode::OK,
            json!({"count": items.len(), "items": items}),
        ),
        Err(db_error) => db_failure("overlay_admin.audit", &db_error),
    }
}
