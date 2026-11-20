//! Appareils personnels d'un membre, administrés depuis « Mon profil ».
//!
//! Chaque membre voit les appareils rattachés à son propre profil et peut les
//! révoquer sans droit d'administration de guilde :
//!
//! | Méthode | Chemin | Effet |
//! |---------|--------|-------|
//! | `GET` | `/overlay/devices` | liste ses overlays, extensions et raccourcis iOS actifs |
//! | `DELETE` | `/overlay/devices/overlay/{clientId}` | révoque un de ses overlays et ferme sa socket (`4402`) |
//! | `DELETE` | `/overlay/devices/ingest/{clientId}` | révoque un de ses clients d'ingestion |
//!
//! Un appareil d'un autre membre ou d'une autre guilde répond exactement comme
//! un identifiant inconnu (`404 device_not_found`). L'appareil appelant ne peut
//! pas se révoquer lui-même (`409 cannot_revoke_self`) : c'est la session qui
//! affiche le dialogue.
//!
//! La propriété d'un client d'ingestion se lit dans
//! `created_by_discord_user_id` : [`super::guild::member_code_authors`] fournit
//! les deux formes écrites par les flux d'appairage (`member:<id>` pour un code
//! émis depuis l'overlay, identifiant Discord pour un code `/pair-code`).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use lc_domain::{GuildMember, IngestClientId, OverlayClientId};
use lc_protocol::close_code;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::http::admin::hub::admin_hub;

use super::auth::AuthedOverlayClient;
use super::guild::{iso, member_code_authors};
use super::ingest_code::ingest_client_kind;
use super::{db, db_failure, error, json_response, now};

/// Appareil inconnu, déjà révoqué ou appartenant à quelqu'un d'autre.
const DEVICE_NOT_FOUND: &str = "device_not_found";

/// Liste les appareils actifs du membre courant.
pub async fn list_personal_devices(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
) -> Response {
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    match personal_devices_payload(&state, &auth, member).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(response) => response,
    }
}

async fn personal_devices_payload(
    state: &AppState,
    auth: &AuthedOverlayClient,
    member: &GuildMember,
) -> Result<Value, Response> {
    let db = db(state)?;
    let hub = admin_hub(state);
    let mut items = Vec::new();

    let overlays = db
        .overlay_clients()
        .list_active_by_member(member.id)
        .await
        .map_err(|db_error| db_failure("overlay_personal_devices.list_overlays", &db_error))?;
    for overlay in overlays {
        // Un rattachement incohérent (membre d'une autre guilde) ne doit jamais
        // exposer ni rendre révocable un overlay hors de la guilde du jeton.
        if overlay.guild_id != member.guild_id {
            continue;
        }
        let (overlay_version, platform) = db
            .overlay_clients()
            .client_info(overlay.id)
            .await
            .map_err(|db_error| db_failure("overlay_personal_devices.client_info", &db_error))?
            .unwrap_or((None, None));
        items.push(json!({
            "id": overlay.id.to_canonical(),
            "kind": "overlay",
            "label": overlay.label,
            "createdAt": iso(Some(overlay.created_at)),
            "lastSeenAt": iso(overlay.last_seen_at),
            "connected": hub.is_connected(&overlay.id.as_uuid()),
            "sessionMode": overlay.session_mode.as_wire_str(),
            "overlayVersion": overlay_version,
            "platform": platform,
            "isSelf": overlay.id == auth.client.id,
        }));
    }

    let authors = member_code_authors(member);
    let ingest_clients = db
        .ingest_clients()
        .list_by_guild(&member.guild_id)
        .await
        .map_err(|db_error| db_failure("overlay_personal_devices.list_ingest", &db_error))?;
    for client in ingest_clients {
        if !client.is_active()
            || !authors
                .iter()
                .any(|author| author == &client.created_by_discord_user_id)
        {
            continue;
        }
        items.push(json!({
            "id": client.id.to_canonical(),
            "kind": ingest_client_kind(&client.label),
            "label": client.label,
            "createdAt": iso(Some(client.created_at)),
            "lastSeenAt": iso(client.last_seen_at),
            "connected": false,
            "isSelf": false,
        }));
    }

    Ok(json!({"count": items.len(), "items": items}))
}

/// Révoque un overlay du membre courant.
pub async fn revoke_personal_overlay(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(raw_client_id): Path<String>,
) -> Response {
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    let Ok(client_id) = raw_client_id.trim().parse::<OverlayClientId>() else {
        return error(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND);
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let owned = match db.overlay_clients().list_active_by_member(member.id).await {
        Ok(clients) => clients,
        Err(db_error) => {
            return db_failure("overlay_personal_devices.revoke_list_overlays", &db_error)
        }
    };
    if !owned
        .iter()
        .any(|client| client.id == client_id && client.guild_id == member.guild_id)
    {
        return error(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND);
    }
    if client_id == auth.client.id {
        return error(StatusCode::CONFLICT, "cannot_revoke_self");
    }
    let revoked = match db.overlay_clients().revoke(client_id, now(&state)).await {
        Ok(revoked) => revoked,
        Err(db_error) => return db_failure("overlay_personal_devices.revoke_overlay", &db_error),
    };
    if revoked {
        let closed = admin_hub(&state).close_client(&client_id.as_uuid(), close_code::REVOKED);
        tracing::info!(
            guild_id = %auth.guild_id(),
            client_id = %client_id.to_canonical(),
            closed,
            "appareil personnel révoqué depuis Mon profil"
        );
    }
    json_response(StatusCode::OK, json!({"found": true, "revoked": revoked}))
}

/// Révoque un client d'ingestion (extension ou raccourci iOS) du membre courant.
pub async fn revoke_personal_ingest(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(raw_client_id): Path<String>,
) -> Response {
    let Some(member) = auth.member.as_ref() else {
        return error(StatusCode::CONFLICT, "member_required");
    };
    let Ok(client_id) = raw_client_id.trim().parse::<IngestClientId>() else {
        return error(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND);
    };
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let clients = match db.ingest_clients().list_by_guild(&member.guild_id).await {
        Ok(clients) => clients,
        Err(db_error) => {
            return db_failure("overlay_personal_devices.revoke_list_ingest", &db_error)
        }
    };
    let authors = member_code_authors(member);
    let Some(client) = clients.iter().find(|client| {
        client.id == client_id
            && client.is_active()
            && authors
                .iter()
                .any(|author| author == &client.created_by_discord_user_id)
    }) else {
        return error(StatusCode::NOT_FOUND, DEVICE_NOT_FOUND);
    };
    let revoked = match db.ingest_clients().revoke(client.id, now(&state)).await {
        Ok(revoked) => revoked,
        Err(db_error) => return db_failure("overlay_personal_devices.revoke_ingest", &db_error),
    };
    json_response(StatusCode::OK, json!({"found": true, "revoked": revoked}))
}
