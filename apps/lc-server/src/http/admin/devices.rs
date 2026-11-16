//! Remote device control from the admin console.
//!
//! Implements section 6 of `docs/rust/admin-console-api.md` on the admin side:
//!
//! - `PATCH /admin/api/overlay-clients/{id}` updates the label and/or the
//!   session mode of an overlay client, applies the new mode to its live
//!   sockets and sends `session.changed`;
//! - `PATCH /admin/api/overlay-clients/{id}/guild-settings` sends a
//!   `device.settings` frame and answers `202 {"delivered": bool}`.
//!
//! Both routes only see active (non-revoked) clients, like the overview.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::patch;
use axum::Json;
use axum::Router;
use lc_domain::{OverlayClientId, OverlaySessionMode};
use lc_protocol::{
    DeviceSettingsPayload, DeviceSettingsStatePayload, ServerMessage, SessionChangedPayload,
    SessionMode,
};
use serde::Deserialize;
use serde_json::json;

use super::auth::AdminAccess;
use super::hub::admin_hub;
use super::overview::overlay_client_payload;
use super::queries;
use super::{require_db, AdminError};
use crate::app::AppState;

const INVALID_PAYLOAD: &str = "invalid_payload";
const OVERLAY_NOT_FOUND: &str = "overlay_client_not_found";
/// Maximum label length, in characters (`1..=64` after trimming).
const LABEL_MAX_CHARS: usize = 64;

/// Router of the device-control routes, merged by [`super::router`].
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/api/overlay-clients/{client_id}",
            patch(update_client),
        )
        .route(
            "/admin/api/overlay-clients/{client_id}/guild-settings",
            patch(update_guild_settings),
        )
}

/// Body of `PATCH /admin/api/overlay-clients/{id}`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateClientRequest {
    #[serde(default)]
    label: Option<String>,
    /// Unknown values are rejected by serde (`400 invalid_payload`).
    #[serde(default)]
    session_mode: Option<SessionMode>,
}

/// Body of `PATCH /admin/api/overlay-clients/{id}/guild-settings`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuildSettingsRequest {
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    media_muted: Option<bool>,
    #[serde(default)]
    paused: Option<bool>,
}

/// Resolves an active overlay client from the raw path segment.
async fn active_client(
    state: &AppState,
    raw_client_id: &str,
) -> Result<queries::OverlayClientRow, AdminError> {
    let raw_client_id = raw_client_id.trim();
    let client_id = OverlayClientId::parse(raw_client_id)
        .map_err(|_| AdminError::not_found(OVERLAY_NOT_FOUND))?;
    let db = require_db(state)?;
    let canonical = client_id.to_canonical();
    queries::overlay_client(db, &canonical)
        .await
        .map_err(|error| AdminError::db("lecture du client overlay", error))?
        .ok_or_else(|| AdminError::not_found(OVERLAY_NOT_FOUND))
}

/// `PATCH /admin/api/overlay-clients/{id}`.
async fn update_client(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_client_id): Path<String>,
    payload: Result<Json<UpdateClientRequest>, JsonRejection>,
) -> Result<Response, AdminError> {
    let client = active_client(&state, &raw_client_id).await?;
    let Ok(Json(payload)) = payload else {
        return Err(AdminError::bad_request(INVALID_PAYLOAD));
    };
    if payload.label.is_none() && payload.session_mode.is_none() {
        return Err(AdminError::bad_request(INVALID_PAYLOAD));
    }

    let label = match payload.label.as_deref() {
        Some(raw) => {
            Some(normalize_label(raw).ok_or_else(|| AdminError::bad_request(INVALID_PAYLOAD))?)
        }
        None => None,
    };

    let db = require_db(&state)?;
    let client_id =
        OverlayClientId::parse(&client.id).map_err(|_| AdminError::not_found(OVERLAY_NOT_FOUND))?;
    let hub = admin_hub(&state);

    if let Some(label) = &label {
        db.overlay_clients()
            .update_label(client_id, label)
            .await
            .map_err(|error| AdminError::db("mise à jour du libellé de l'overlay", error))?;
    }

    let mut changed_mode = None;
    if let Some(mode) = payload.session_mode {
        db.overlay_clients()
            .update_mode(client_id, domain_session_mode(mode))
            .await
            .map_err(|error| AdminError::db("mise à jour du mode de session", error))?;
        changed_mode = Some(mode);
    }

    let client_uuid = client_id.as_uuid();
    if let Some(mode) = changed_mode {
        // Live sockets must apply the mode before receiving the notification:
        // the next stop/media command is checked against it.
        let live_updated = hub.set_session_mode(&client_uuid, mode);
        let delivered = hub.send_to_client(
            &client_uuid,
            ServerMessage::SessionChanged(SessionChangedPayload { session_mode: mode }),
        );
        tracing::info!(
            client_id = %client.id,
            session_mode = ?mode,
            live_updated,
            delivered,
            "mode de session d'un overlay modifié depuis le panneau"
        );
    }

    let updated = active_client(&state, &raw_client_id).await?;
    let guild_settings: Option<DeviceSettingsStatePayload> = hub.device_settings(&client_uuid);
    let connected = hub.is_connected(&client_uuid);
    Ok(Json(overlay_client_payload(
        &updated,
        connected,
        guild_settings.as_ref(),
    ))
    .into_response())
}

/// `PATCH /admin/api/overlay-clients/{id}/guild-settings`.
async fn update_guild_settings(
    State(state): State<AppState>,
    _access: AdminAccess,
    Path(raw_client_id): Path<String>,
    payload: Result<Json<GuildSettingsRequest>, JsonRejection>,
) -> Result<Response, AdminError> {
    let client = active_client(&state, &raw_client_id).await?;
    let Ok(Json(payload)) = payload else {
        return Err(AdminError::bad_request(INVALID_PAYLOAD));
    };
    let device_settings = validate_guild_settings(payload)?;

    let client_id =
        OverlayClientId::parse(&client.id).map_err(|_| AdminError::not_found(OVERLAY_NOT_FOUND))?;
    let delivered = admin_hub(&state).send_to_client(
        &client_id.as_uuid(),
        ServerMessage::DeviceSettings(device_settings),
    );
    tracing::info!(
        client_id = %client.id,
        delivered,
        "réglages d'appareil envoyés à un overlay depuis le panneau"
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "delivered": delivered })),
    )
        .into_response())
}

/// Validates the guild-settings body and builds the `device.settings` payload.
fn validate_guild_settings(
    payload: GuildSettingsRequest,
) -> Result<DeviceSettingsPayload, AdminError> {
    if payload.volume.is_none() && payload.media_muted.is_none() && payload.paused.is_none() {
        return Err(AdminError::bad_request(INVALID_PAYLOAD));
    }
    if let Some(volume) = payload.volume {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(AdminError::bad_request(INVALID_PAYLOAD));
        }
    }
    Ok(DeviceSettingsPayload {
        volume: payload.volume,
        media_muted: payload.media_muted,
        paused: payload.paused,
    })
}

/// Trims and validates an overlay label (`1..=64` characters).
fn normalize_label(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let length = trimmed.chars().count();
    (1..=LABEL_MAX_CHARS)
        .contains(&length)
        .then(|| trimmed.to_owned())
}

/// Domain session mode stored on the overlay client row.
fn domain_session_mode(mode: SessionMode) -> OverlaySessionMode {
    match mode {
        SessionMode::Normal => OverlaySessionMode::Normal,
        SessionMode::InviteReadOnly => OverlaySessionMode::InviteReadOnly,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::http::admin::overview::session_mode_wire;

    #[test]
    fn labels_are_trimmed_and_bounded() {
        assert_eq!(
            normalize_label("  Salon principal  ").as_deref(),
            Some("Salon principal")
        );
        assert_eq!(normalize_label("   "), None);
        assert_eq!(normalize_label(&"x".repeat(64)), Some("x".repeat(64)));
        assert_eq!(normalize_label(&"x".repeat(65)), None);
        assert_eq!(
            normalize_label("Salon\nprincipal"),
            Some("Salon\nprincipal".to_owned())
        );
    }

    #[test]
    fn guild_settings_validation_rejects_empty_and_out_of_range_volumes() {
        assert!(validate_guild_settings(GuildSettingsRequest::default()).is_err());
        for volume in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            let payload = GuildSettingsRequest {
                volume: Some(volume),
                ..GuildSettingsRequest::default()
            };
            assert!(validate_guild_settings(payload).is_err(), "{volume}");
        }
        for volume in [0.0, 0.5, 1.0] {
            let payload = GuildSettingsRequest {
                volume: Some(volume),
                ..GuildSettingsRequest::default()
            };
            assert_eq!(
                validate_guild_settings(payload)
                    .expect("volume valide")
                    .volume,
                Some(volume)
            );
        }
        let payload = GuildSettingsRequest {
            paused: Some(true),
            ..GuildSettingsRequest::default()
        };
        assert_eq!(
            validate_guild_settings(payload)
                .expect("paused seul")
                .paused,
            Some(true)
        );
    }

    #[test]
    fn wire_session_mode_roundtrips() {
        assert_eq!(
            session_mode_wire(domain_session_mode(SessionMode::Normal).as_db_str()),
            "normal"
        );
        assert_eq!(
            session_mode_wire(domain_session_mode(SessionMode::InviteReadOnly).as_db_str()),
            "invite_read_only"
        );
    }
}
