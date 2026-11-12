//! Administration d'une guilde depuis l'overlay (rôle admin).
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `GET` | `/overlay/admin/members` | `ManageMembers` |
//! | `PATCH` | `/overlay/admin/members/{memberId}` | `ManageMembers` |
//! | `DELETE` | `/overlay/admin/members/{memberId}` | `ManageMembers` |
//! | `GET` | `/overlay/admin/invites` | `ManageInvites` |
//! | `POST` | `/overlay/admin/invites` | `ManageInvites` |
//! | `POST` | `/overlay/admin/invites/{code}/revoke` | `ManageInvites` |
//! | `PATCH` | `/overlay/admin/settings` | `ManageSettings` |
//!
//! Toutes les routes sont bornées à la guilde du jeton : un identifiant de
//! membre ou un code d'une autre guilde répond « introuvable ».
//!
//! Changer un rôle aligne `overlay_clients.session_mode` puis ferme les
//! sessions WS du membre (`1001`) : ses overlays se reconnectent aussitôt et
//! reprennent leur mode à jour dans `welcome`.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use chrono::Duration;
use lc_db::MemberChange;
use lc_domain::{Guild, GuildMember, GuildMemberId, GuildRole, PairingCode, Permission};
use lc_protocol::close_code;
use serde_json::{json, Value};

use crate::app::AppState;

use super::auth::AuthedOverlayClient;
use super::guild::{
    create_invite, invite_payload, iso, member_payload, normalize_label, remove_member, require,
    NewInvite,
};
use super::{db, db_failure, error, internal_error, json_response, now};

/// Durée de vie maximale d'une invitation (un an).
const INVITE_MAX_TTL_HOURS: i64 = 24 * 365;
/// Nombre maximal d'utilisations d'une invitation.
const INVITE_MAX_USES: i64 = 1_000;
/// Longueur maximale du nom d'une guilde.
const GUILD_NAME_MAX_CHARS: usize = 60;

/// Charge un membre actif de la guilde du jeton (`404 member_not_found` sinon).
async fn load_member(
    db: &lc_db::Db,
    auth: &AuthedOverlayClient,
    member_id_raw: &str,
) -> Result<GuildMember, Response> {
    let Ok(member_id) = member_id_raw.trim().parse::<GuildMemberId>() else {
        return Err(error(StatusCode::NOT_FOUND, "member_not_found"));
    };
    match db.guild_members().get(member_id).await {
        Ok(Some(member)) if member.is_active() && &member.guild_id == auth.guild_id() => Ok(member),
        Ok(_) => Err(error(StatusCode::NOT_FOUND, "member_not_found")),
        Err(db_error) => Err(db_failure("overlay_admin.member_lookup", &db_error)),
    }
}

/// Handler `GET /overlay/admin/members`.
pub async fn list_members(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&auth, Permission::ManageMembers) {
        return response;
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let members = match db.guild_members().list_by_guild(auth.guild_id()).await {
        Ok(members) => members,
        Err(db_error) => return db_failure("overlay_admin.list_members", &db_error),
    };

    let mut items = Vec::with_capacity(members.len());
    for member in &members {
        let clients = match db.overlay_clients().list_active_by_member(member.id).await {
            Ok(clients) => clients,
            Err(db_error) => return db_failure("overlay_admin.member_clients", &db_error),
        };
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
                json!(auth.member_id() == Some(member.id)),
            );
        }
        items.push(payload);
    }

    json_response(
        StatusCode::OK,
        json!({"count": items.len(), "items": items}),
    )
}

/// Handler `PATCH /overlay/admin/members/{memberId}` (corps `{"role": "…"}`).
pub async fn update_member(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(member_id_raw): Path<String>,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&auth, Permission::ManageMembers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let Some(role) = payload
        .get("role")
        .and_then(Value::as_str)
        .and_then(|raw| GuildRole::from_wire_str(raw).ok())
    else {
        return error(StatusCode::BAD_REQUEST, "invalid_role");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let member = match load_member(db, &auth, &member_id_raw).await {
        Ok(member) => member,
        Err(response) => return response,
    };

    match db.guild_members().set_role(member.id, role).await {
        Ok(MemberChange::Applied) => {}
        Ok(MemberChange::LastAdmin) => return error(StatusCode::CONFLICT, "last_admin"),
        Ok(MemberChange::NotFound) => return error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(db_error) => return db_failure("overlay_admin.set_role", &db_error),
    }

    if role != member.role {
        if let Err(db_error) = db
            .overlay_clients()
            .update_mode_by_member(member.id, role.session_mode())
            .await
        {
            return db_failure("overlay_admin.sync_session_mode", &db_error);
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
            Err(db_error) => return db_failure("overlay_admin.member_clients", &db_error),
        }
        tracing::info!(
            guild_id = %member.guild_id,
            member_id = %member.id,
            role = role.as_wire_str(),
            "rôle de membre modifié"
        );
    }

    json_response(
        StatusCode::OK,
        json!({
            "updated": true,
            "member": {"id": member.id.to_canonical(), "role": role.as_wire_str()},
        }),
    )
}

/// Handler `DELETE /overlay/admin/members/{memberId}` : exclusion.
pub async fn remove_member_handler(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(member_id_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&auth, Permission::ManageMembers) {
        return response;
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let member = match load_member(db, &auth, &member_id_raw).await {
        Ok(member) => member,
        Err(response) => return response,
    };
    match remove_member(&state, db, member.id, now(&state)).await {
        Ok(MemberChange::Applied) => {
            tracing::info!(
                guild_id = %member.guild_id,
                member_id = %member.id,
                "membre exclu"
            );
            json_response(StatusCode::OK, json!({"removed": true}))
        }
        Ok(MemberChange::LastAdmin) => error(StatusCode::CONFLICT, "last_admin"),
        Ok(MemberChange::NotFound) => error(StatusCode::NOT_FOUND, "member_not_found"),
        Err(response) => response,
    }
}

/// Handler `GET /overlay/admin/invites`.
pub async fn list_invites(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&auth, Permission::ManageInvites) {
        return response;
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);
    match db.guild_invites().list_by_guild(auth.guild_id()).await {
        Ok(invites) => {
            // Les codes « appareil » sont personnels : ils ne sont pas listés.
            let items: Vec<Value> = invites
                .iter()
                .filter(|invite| invite.member_id.is_none())
                .map(|invite| invite_payload(invite, now))
                .collect();
            json_response(
                StatusCode::OK,
                json!({"count": items.len(), "items": items}),
            )
        }
        Err(db_error) => db_failure("overlay_admin.list_invites", &db_error),
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

/// Handler `POST /overlay/admin/invites`.
///
/// Corps : `{"role": "admin|member|guest", "expiresInHours": n|null,
/// "maxUses": n|null}` (`null` = jamais / illimité).
pub async fn create_invite_handler(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&auth, Permission::ManageInvites) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let Some(role) = payload
        .get("role")
        .and_then(Value::as_str)
        .and_then(|raw| GuildRole::from_wire_str(raw).ok())
    else {
        return error(StatusCode::BAD_REQUEST, "invalid_role");
    };
    let Ok(expires_in_hours) = optional_bounded(&payload, "expiresInHours", INVITE_MAX_TTL_HOURS)
    else {
        return error(StatusCode::BAD_REQUEST, "invalid_expiry");
    };
    let Ok(max_uses) = optional_bounded(&payload, "maxUses", INVITE_MAX_USES) else {
        return error(StatusCode::BAD_REQUEST, "invalid_max_uses");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let now = now(&state);
    let invite = NewInvite {
        guild_id: auth.guild_id().clone(),
        role,
        member_id: None,
        max_uses,
        created_by_member_id: auth.member_id(),
        expires_at: expires_in_hours.map(|hours| now + Duration::hours(hours)),
    };
    match create_invite(db, invite, now).await {
        Ok(Some(invite)) => json_response(
            StatusCode::OK,
            json!({"created": true, "invite": invite_payload(&invite, now)}),
        ),
        Ok(None) => internal_error(),
        Err(db_error) => db_failure("overlay_admin.create_invite", &db_error),
    }
}

/// Handler `POST /overlay/admin/invites/{code}/revoke`.
pub async fn revoke_invite(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(code_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&auth, Permission::ManageInvites) {
        return response;
    }
    let Ok(code) = PairingCode::parse(&code_raw.replace('-', "")) else {
        return error(StatusCode::NOT_FOUND, "invite_not_found");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    match db.guild_invites().get(&code).await {
        Ok(Some(invite)) if &invite.guild_id == auth.guild_id() => {}
        Ok(_) => return error(StatusCode::NOT_FOUND, "invite_not_found"),
        Err(db_error) => return db_failure("overlay_admin.invite_lookup", &db_error),
    }
    match db
        .guild_invites()
        .revoke(auth.guild_id(), &code, now(&state))
        .await
    {
        Ok(revoked) => json_response(StatusCode::OK, json!({"found": true, "revoked": revoked})),
        Err(db_error) => db_failure("overlay_admin.revoke_invite", &db_error),
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

/// Handler `PATCH /overlay/admin/settings`.
///
/// Corps partiel : `name`, `defaultMediaTime`, `maxMediaTime`,
/// `displayMediaFull`, `guestsCanViewBoard`. Les réglages de lecture sont
/// rediffusés aux overlays connectés (`settings`).
pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&auth, Permission::ManageSettings) {
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
    let (Ok(default_media_time), Ok(max_media_time), Ok(display_media_full), Ok(guests)) = (
        duration_field(&payload, "defaultMediaTime"),
        duration_field(&payload, "maxMediaTime"),
        bool_field(&payload, "displayMediaFull"),
        bool_field(&payload, "guestsCanViewBoard"),
    ) else {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    if name.is_none()
        && default_media_time.is_none()
        && max_media_time.is_none()
        && display_media_full.is_none()
        && guests.is_none()
    {
        return error(StatusCode::BAD_REQUEST, "invalid_payload");
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
    if playback_changed {
        state.admin_hub.broadcast_settings(guild_id).await;
    }

    let guests_can_view_board = match db.guilds().access(guild_id).await {
        Ok(access) => access.is_none_or(|access| access.guests_can_view_board),
        Err(db_error) => return db_failure("overlay_admin.settings_reload", &db_error),
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
            },
        }),
    )
}
