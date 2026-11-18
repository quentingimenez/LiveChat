//! Appareils (overlays) de la guilde, administrés depuis l'overlay.
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `GET` | `/overlay/admin/devices` | `ManageMembers` |
//! | `PATCH` | `/overlay/admin/devices/{clientId}` | `ManageMembers` |
//! | `DELETE` | `/overlay/admin/devices/{clientId}` | `ManageMembers` |
//! | `PATCH` | `/overlay/admin/devices/{clientId}/guild-settings` | `ManageMembers` |
//! | `POST` | `/overlay/admin/device-codes` | `ManageMembers` |
//!
//! Toutes les routes sont bornées à la guilde du jeton : un appareil d'une
//! autre guilde répond exactement comme un identifiant inconnu (`404
//! device_not_found`). La révocation a le même effet que celle du panneau
//! serveur (jeton révoqué, socket fermée `4402`), mais l'appareil appelant ne
//! peut pas se révoquer lui-même (`409 cannot_revoke_self`).
//!
//! Le corps de `PATCH …/guild-settings` est validé par le même helper que la
//! route du panneau (`admin::devices::validate_guild_settings`) et poussé en
//! direct par `device.settings`, comme depuis la console.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use lc_domain::{GuildMember, GuildMemberId, OverlayClientId, Permission};
use lc_protocol::{close_code, ServerMessage, SessionChangedPayload, SessionMode};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::http::admin::devices::{
    domain_session_mode, normalize_device_label, validate_guild_settings, GuildSettingsRequest,
};
use crate::http::admin::hub::admin_hub;
use crate::http::admin::overview::{overlay_client_payload, session_mode_wire};
use crate::http::admin::queries::{self, OverlayClientRow};

use super::admin::load_guild_member;
use super::auth::AuthedOverlayClient;
use super::guild::{create_device_code_payload, require};
use super::{db, db_failure, error, json_response, now};

/// Longueur maximale du nom d'un appareil depuis l'overlay.
const DEVICE_LABEL_MAX_CHARS: usize = 40;
/// Appareil inconnu ou appartenant à une autre guilde.
const DEVICE_NOT_FOUND: &str = "device_not_found";

/// Erreur JSON du module.
fn failure(status: StatusCode, code: &'static str) -> Response {
    error(status, code)
}

/// Corps de `PATCH /overlay/admin/devices/{clientId}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateDeviceRequest {
    #[serde(default)]
    label: Option<String>,
    /// Une valeur inconnue est refusée par serde (`400 invalid_payload`).
    #[serde(default)]
    session_mode: Option<SessionMode>,
}

/// Résout un appareil **actif** de la guilde (`404 device_not_found` sinon).
async fn active_device(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    raw_client_id: &str,
) -> Result<OverlayClientRow, Response> {
    let Ok(client_id) = raw_client_id.trim().parse::<OverlayClientId>() else {
        return Err(failure(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND));
    };
    let db = db(state)?;
    match queries::overlay_client(db, &client_id.to_canonical()).await {
        Ok(Some(row)) if row.guild_id == guild_id.as_str() => Ok(row),
        Ok(_) => Err(failure(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND)),
        Err(db_error) => Err(db_failure("overlay_devices.lookup", &db_error)),
    }
}

/// Membre actif rattaché à un appareil, s'il y en a un.
async fn device_member(
    db: &lc_db::Db,
    row: &OverlayClientRow,
) -> Result<Option<GuildMember>, Response> {
    let Ok(client_id) = row.id.parse::<OverlayClientId>() else {
        return Ok(None);
    };
    db.guild_members()
        .get_for_overlay_client(client_id)
        .await
        .map_err(|db_error| db_failure("overlay_devices.member", &db_error))
}

/// Rôle effectif de l'appareil : celui du membre rattaché, sinon celui que
/// porte son mode de session (même repli que le détail du panneau).
fn device_role(member: Option<&GuildMember>, stored_mode: &str) -> &'static str {
    match member {
        Some(member) => member.role.as_wire_str(),
        None if session_mode_wire(stored_mode) == "invite_read_only" => "guest",
        None => "member",
    }
}

/// Une entrée d'appareil : le payload du panneau, enrichi du membre et de
/// `isSelf` (l'appareil appelant).
async fn device_payload(
    state: &AppState,
    db: &lc_db::Db,
    row: &OverlayClientRow,
    caller_client_id: Option<OverlayClientId>,
) -> Result<Value, Response> {
    let hub = admin_hub(state);
    let client_id = row.id.parse::<OverlayClientId>().ok();
    let socket_id = client_id.as_ref().map(OverlayClientId::as_uuid);
    let connected = socket_id.is_some_and(|id| hub.is_connected(&id));
    let guild_settings = socket_id.and_then(|id| hub.device_settings(&id));
    let member = device_member(db, row).await?;

    let mut payload = overlay_client_payload(row, connected, guild_settings.as_ref());
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "memberId".to_owned(),
            member
                .as_ref()
                .map_or(Value::Null, |member| json!(member.id.to_canonical())),
        );
        object.insert(
            "memberName".to_owned(),
            member
                .as_ref()
                .map_or(Value::Null, |member| json!(member.display_name)),
        );
        object.insert(
            "role".to_owned(),
            json!(device_role(member.as_ref(), &row.session_mode)),
        );
        object.insert(
            "isSelf".to_owned(),
            json!(caller_client_id.is_some() && client_id == caller_client_id),
        );
    }
    Ok(payload)
}

/// Appareils actifs de la guilde (triés par nom, comme l'aperçu du panneau).
async fn devices_payload(
    state: &AppState,
    guild_id: &lc_domain::GuildId,
    caller_client_id: Option<OverlayClientId>,
) -> Result<Value, Response> {
    let db = db(state)?;
    let rows = queries::overlay_clients(db)
        .await
        .map_err(|db_error| db_failure("overlay_devices.list", &db_error))?;
    let mut items = Vec::new();
    for row in rows.iter().filter(|row| row.guild_id == guild_id.as_str()) {
        items.push(device_payload(state, db, row, caller_client_id).await?);
    }
    Ok(json!({"count": items.len(), "items": items}))
}

/// Handler `GET /overlay/admin/devices`.
pub async fn list_devices(State(state): State<AppState>, auth: AuthedOverlayClient) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    match devices_payload(&state, auth.guild_id(), Some(auth.client.id)).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Handler `PATCH /overlay/admin/devices/{clientId}`.
///
/// Corps partiel : `label` (1 à 40 caractères après trim) et/ou `sessionMode`
/// (`normal` ou `invite_read_only`). Un changement de mode est appliqué aux
/// sockets vivantes puis notifié (`session.changed`).
pub async fn update_device(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(client_id_raw): Path<String>,
    payload: Result<Json<UpdateDeviceRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return failure(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    if payload.label.is_none() && payload.session_mode.is_none() {
        return failure(StatusCode::BAD_REQUEST, "invalid_payload");
    }
    let label = match payload.label.as_deref() {
        Some(raw) => match normalize_device_label(raw, DEVICE_LABEL_MAX_CHARS) {
            Some(label) => Some(label),
            None => return failure(StatusCode::BAD_REQUEST, "invalid_payload"),
        },
        None => None,
    };
    let row = match active_device(&state, auth.guild_id(), &client_id_raw).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    let Ok(client_id) = row.id.parse::<OverlayClientId>() else {
        return failure(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND);
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let hub = admin_hub(&state);

    if let Some(label) = &label {
        if let Err(db_error) = db.overlay_clients().update_label(client_id, label).await {
            return db_failure("overlay_devices.update_label", &db_error);
        }
    }
    if let Some(mode) = payload.session_mode {
        if let Err(db_error) = db
            .overlay_clients()
            .update_mode(client_id, domain_session_mode(mode))
            .await
        {
            return db_failure("overlay_devices.update_mode", &db_error);
        }
        let socket_id = client_id.as_uuid();
        // Le mode doit être appliqué avant la notification : la commande
        // suivante du client est contrôlée contre lui.
        let live_updated = hub.set_session_mode(&socket_id, mode);
        let delivered = hub.send_to_client(
            &socket_id,
            ServerMessage::SessionChanged(SessionChangedPayload { session_mode: mode }),
        );
        tracing::info!(
            guild_id = %auth.guild_id(),
            client_id = %client_id.to_canonical(),
            session_mode = ?mode,
            live_updated,
            delivered,
            "mode de session d'un appareil modifié depuis l'overlay"
        );
    }

    let updated = match active_device(&state, auth.guild_id(), &client_id_raw).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    match device_payload(&state, db, &updated, Some(auth.client.id)).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

/// Handler `DELETE /overlay/admin/devices/{clientId}` : révocation (jeton
/// invalidé, socket fermée). L'appareil appelant est refusé.
pub async fn revoke_device(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(client_id_raw): Path<String>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    let row = match active_device(&state, auth.guild_id(), &client_id_raw).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    let Ok(client_id) = row.id.parse::<OverlayClientId>() else {
        return failure(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND);
    };
    if client_id == auth.client.id {
        return failure(StatusCode::CONFLICT, "cannot_revoke_self");
    }
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let revoked = match db.overlay_clients().revoke(client_id, now(&state)).await {
        Ok(revoked) => revoked,
        Err(db_error) => return db_failure("overlay_devices.revoke", &db_error),
    };
    if revoked {
        let closed = admin_hub(&state).close_client(&client_id.as_uuid(), close_code::REVOKED);
        tracing::info!(
            guild_id = %auth.guild_id(),
            client_id = %client_id.to_canonical(),
            closed,
            "appareil révoqué depuis l'overlay"
        );
    }
    json_response(StatusCode::OK, json!({"found": true, "revoked": revoked}))
}

/// Handler `PATCH /overlay/admin/devices/{clientId}/guild-settings`.
///
/// Corps partiel `volume` (0..=1), `mediaMuted`, `paused` : mêmes règles que
/// `PATCH /admin/api/overlay-clients/{id}/guild-settings`. Répond `202
/// {"delivered": bool}`.
pub async fn update_guild_settings(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(client_id_raw): Path<String>,
    payload: Result<Json<GuildSettingsRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return failure(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let device_settings = match validate_guild_settings(payload) {
        Ok(settings) => settings,
        Err(()) => return failure(StatusCode::BAD_REQUEST, "invalid_payload"),
    };
    let row = match active_device(&state, auth.guild_id(), &client_id_raw).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    let Ok(client_id) = row.id.parse::<OverlayClientId>() else {
        return failure(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND);
    };
    let delivered = admin_hub(&state).send_to_client(
        &client_id.as_uuid(),
        ServerMessage::DeviceSettings(device_settings),
    );
    tracing::info!(
        guild_id = %auth.guild_id(),
        client_id = %client_id.to_canonical(),
        delivered,
        "réglages d'appareil envoyés depuis l'overlay"
    );
    json_response(StatusCode::ACCEPTED, json!({"delivered": delivered}))
}

/// Handler `POST /overlay/admin/device-codes` (corps `{"memberId": "…"}`).
///
/// Code à usage unique (10 minutes) rattachant un nouvel overlay à un AUTRE
/// membre de la guilde. Un membre inconnu ou d'une autre guilde répond `404
/// member_not_found`.
pub async fn create_device_code(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    payload: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Err(response) = require(&state, &auth, Permission::ManageMembers) {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return failure(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let Some(member_id_raw) = payload
        .get("memberId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return failure(StatusCode::BAD_REQUEST, "invalid_payload");
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let member: GuildMember = match load_guild_member(db, auth.guild_id(), member_id_raw).await {
        Ok(member) => member,
        Err(response) => return response,
    };
    let member_id: GuildMemberId = member.id;
    match create_device_code_payload(
        &state,
        auth.guild_id(),
        member_id,
        member.role,
        auth.member_id(),
    )
    .await
    {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}
