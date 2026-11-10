//! Commandes exposées au bridge (`window.livechatOverlay`) et aux pages du
//! shell. Les noms suivent le contrat figé de `docs/rust/overlay-bridge.md`
//! §6.1 (`overlay_get_config`, `overlay_renderer_ready`, ...).
//!
//! Phase 8b : la télémétrie du renderer est traduite en trames WebSocket v2,
//! l'appairage passe par `POST {serverUrl}/overlay/pair/consume`, et les
//! déclenchements de mémoire utilisent les trames `meme.trigger` /
//! `stop.request` (plus de stub `socket_unavailable`).

use lc_protocol::{
    ClientMessage, MemeTrigger, MemeTriggerPayload, PlaybackEndedPayload, PlaybackEndedReason,
    PlaybackState as ProtocolPlaybackState, PlaybackStatePayload, StopRequestPayload,
    StopRequestSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State, Wry};

use crate::config::{MemeBindings, OverlayConfigView};
use crate::events;
use crate::net::pairing::{self, PairingRequest};
use crate::net::ws::{self, NetCommand};
use crate::state::AppState;
use crate::tray;
use crate::updater::{self, UpdateStatus};
use crate::windows;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemeBoardBindingsResponse {
    pub bindings: MemeBindings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemeBoardSetBindingsResponse {
    pub ok: bool,
    pub bindings: MemeBindings,
    pub failed_accelerators: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemeBoardActionResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingConsumeResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guest_mode: Option<bool>,
    pub message: String,
}

/// Payload de `overlay_report_error` (télémétrie renderer legacy).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RendererErrorPayload {
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// Payload de `overlay_report_playback_state`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RendererPlaybackStatePayload {
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    remaining_ms: Option<f64>,
}

/// Payload de `overlay_report_playback_stop`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RendererPlaybackStopPayload {
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// Payload de `meme_board_trigger`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemeTriggerRequest {
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
}

fn send_protocol(app: &AppHandle<Wry>, message: ClientMessage, label: &str) -> bool {
    let sent = ws::send_command(app, NetCommand::Send(message));
    if !sent {
        tracing::debug!(
            action = label,
            "trame non envoyée : connexion WebSocket inactive"
        );
    }
    sent
}

fn playback_state_from_label(label: &str) -> Option<ProtocolPlaybackState> {
    match label.trim().to_ascii_lowercase().as_str() {
        "playing" | "play" => Some(ProtocolPlaybackState::Playing),
        "paused" | "pause" => Some(ProtocolPlaybackState::Paused),
        _ => None,
    }
}

fn ended_reason_from_label(label: Option<&str>) -> PlaybackEndedReason {
    match label.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("manual") => PlaybackEndedReason::Manual,
        Some("error") => PlaybackEndedReason::Error,
        _ => PlaybackEndedReason::Completed,
    }
}

/// `overlay_get_config` — projection 8 champs consommée par le renderer.
#[tauri::command]
pub fn overlay_get_config(state: State<'_, AppState>) -> OverlayConfigView {
    let view = state.view();
    tracing::debug!(
        has_server_url = view.server_url.is_some(),
        has_token = view.client_token.is_some(),
        has_guild = view.guild_id.is_some(),
        "overlay_get_config demandé par le renderer"
    );
    view
}

/// `overlay_renderer_ready` — handshake : envoie `overlay://settings` puis
/// rejoue les événements bufferisés.
#[tauri::command]
pub fn overlay_renderer_ready(app: AppHandle<Wry>, state: State<'_, AppState>) {
    state.mark_overlay_ready();
    tracing::info!("renderer overlay prêt : envoi des settings et rejeu du buffer");
    if let Err(error) = events::emit_settings(&app, &state.view()) {
        tracing::warn!(%error, "échec d'émission des settings initiaux");
    }
    events::flush_pending(&app);
}

/// `overlay_report_error` — erreur remontée par le renderer, relayée au serveur.
#[tauri::command]
pub fn overlay_report_error(app: AppHandle<Wry>, payload: Value) {
    let parsed =
        serde_json::from_value::<RendererErrorPayload>(payload.clone()).unwrap_or_default();
    let code = parsed
        .code
        .filter(|code| !code.trim().is_empty())
        .unwrap_or_else(|| "renderer_error".to_string());
    let message = parsed
        .message
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| payload.to_string());
    tracing::warn!(%code, %message, "erreur signalée par le renderer overlay");
    send_protocol(
        &app,
        ClientMessage::Error(lc_protocol::ClientErrorPayload {
            job_id: parsed.job_id.unwrap_or_default(),
            code,
            message,
        }),
        "overlay_report_error",
    );
}

/// `overlay_report_playback_state` — position de lecture, limitée à 1 msg/s.
#[tauri::command]
pub fn overlay_report_playback_state(app: AppHandle<Wry>, payload: Value) {
    let parsed =
        serde_json::from_value::<RendererPlaybackStatePayload>(payload.clone()).unwrap_or_default();
    let state = app.state::<AppState>();
    let Some(playback_state) = parsed.state.as_deref().and_then(playback_state_from_label) else {
        tracing::debug!(payload = %payload, "état de lecture ignoré (état inconnu)");
        return;
    };
    let job_id = parsed
        .job_id
        .filter(|job_id| !job_id.trim().is_empty())
        .or_else(|| state.playback_job())
        .unwrap_or_default();
    if job_id.is_empty() {
        tracing::debug!("état de lecture ignoré : aucun job courant");
        return;
    }
    if !state.try_record_playback_state() {
        tracing::trace!("playback.state limité à 1 msg/s");
        return;
    }
    let remaining_ms = parsed
        .remaining_ms
        .filter(|remaining| remaining.is_finite() && *remaining > 0.0)
        .map(|remaining| (remaining as u64).min(lc_protocol::MAX_PLAYBACK_REMAINING_MS));
    send_protocol(
        &app,
        ClientMessage::PlaybackState(PlaybackStatePayload {
            job_id,
            state: playback_state,
            remaining_ms,
        }),
        "overlay_report_playback_state",
    );
}

/// `overlay_report_playback_stop` — fin de lecture envoyée comme `playback.ended`.
#[tauri::command]
pub fn overlay_report_playback_stop(app: AppHandle<Wry>, payload: Value) {
    let parsed = serde_json::from_value::<RendererPlaybackStopPayload>(payload).unwrap_or_default();
    let state = app.state::<AppState>();
    let job_id = parsed
        .job_id
        .filter(|job_id| !job_id.trim().is_empty())
        .or_else(|| state.playback_job())
        .unwrap_or_default();
    if job_id.is_empty() {
        tracing::debug!("arrêt de lecture ignoré : aucun job courant");
        return;
    }
    state.clear_playback();
    send_protocol(
        &app,
        ClientMessage::PlaybackEnded(PlaybackEndedPayload {
            job_id,
            reason: ended_reason_from_label(parsed.reason.as_deref()),
        }),
        "overlay_report_playback_stop",
    );
}

/// `pairing_consume` — appairage HTTP réel puis reconnexion WebSocket.
#[tauri::command]
pub async fn pairing_consume(
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<PairingConsumeResponse, String> {
    let server_url = payload
        .get("serverUrl")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let code = payload
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let device_name = payload
        .get("deviceName")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    tracing::info!(server_url = %server_url, "demande d'appairage reçue");
    let outcome = pairing::consume(
        &state,
        PairingRequest {
            server_url,
            code,
            device_name,
        },
    )
    .await;
    if !outcome.ok {
        tracing::warn!(message = %outcome.message, "appairage refusé");
        return Ok(PairingConsumeResponse {
            ok: false,
            server_url: None,
            guild_id: None,
            guest_mode: None,
            message: outcome.message,
        });
    }
    crate::shortcuts::reapply_for_session(&app);
    if state.overlay_ready() {
        if let Err(error) = events::emit_settings(&app, &state.view()) {
            tracing::debug!(%error, "échec d'émission des settings après appairage");
        }
    }
    if let Some(window) = app.get_webview_window(windows::PAIRING_LABEL) {
        if let Err(error) = window.hide() {
            tracing::debug!(%error, "échec de fermeture de la fenêtre d'appairage");
        }
    }
    tray::refresh_tray(&app);
    if state.config_snapshot().enabled {
        windows::show_overlay(&app);
    }
    ws::request_reconnect(&app);
    Ok(PairingConsumeResponse {
        ok: true,
        server_url: outcome.server_url,
        guild_id: outcome.guild_id,
        guest_mode: Some(outcome.guest_mode),
        message: outcome.message,
    })
}

/// `meme_board_get_bindings` — bindings actuellement enregistrés.
#[tauri::command]
pub fn meme_board_get_bindings(state: State<'_, AppState>) -> MemeBoardBindingsResponse {
    MemeBoardBindingsResponse {
        bindings: state.active_bindings(),
    }
}

/// `meme_board_set_bindings` — applique puis persiste les bindings (strict).
#[tauri::command]
pub fn meme_board_set_bindings(
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<MemeBoardSetBindingsResponse, String> {
    let bindings = payload
        .get("bindings")
        .cloned()
        .map(serde_json::from_value::<MemeBindings>)
        .transpose()
        .map_err(|error| format!("bindings invalides : {error}"))?
        .unwrap_or_default();
    let result = crate::shortcuts::apply_meme_bindings(&app, bindings, true, true);
    tray::refresh_tray(&app);
    Ok(MemeBoardSetBindingsResponse {
        ok: result.ok,
        bindings: result.applied_bindings,
        failed_accelerators: result.failed_accelerators,
    })
}

/// `meme_board_trigger` — déclenchement manuel via `meme.trigger`.
#[tauri::command]
pub fn meme_board_trigger(app: AppHandle<Wry>, payload: Value) -> MemeBoardActionResponse {
    let parsed = serde_json::from_value::<MemeTriggerRequest>(payload).unwrap_or_default();
    let item_id = parsed
        .item_id
        .map(|item_id| item_id.trim().to_string())
        .filter(|item_id| !item_id.is_empty());
    let Some(item_id) = item_id else {
        return MemeBoardActionResponse {
            ok: false,
            reason: Some("invalid_item_id".to_string()),
        };
    };
    let trigger = match parsed.trigger.as_deref() {
        Some("shortcut") => MemeTrigger::Shortcut,
        _ => MemeTrigger::Ui,
    };
    let sent = send_protocol(
        &app,
        ClientMessage::MemeTrigger(MemeTriggerPayload { item_id, trigger }),
        "meme_board_trigger",
    );
    MemeBoardActionResponse {
        ok: sent,
        reason: if sent {
            None
        } else {
            Some("socket_unavailable".to_string())
        },
    }
}

/// `meme_board_stop` — arrêt manuel via `stop.request`.
#[tauri::command]
pub fn meme_board_stop(app: AppHandle<Wry>) -> MemeBoardActionResponse {
    let sent = send_protocol(
        &app,
        ClientMessage::StopRequest(StopRequestPayload {
            job_id: None,
            source: Some(StopRequestSource::User),
        }),
        "meme_board_stop",
    );
    MemeBoardActionResponse {
        ok: sent,
        reason: if sent {
            None
        } else {
            Some("socket_unavailable".to_string())
        },
    }
}

/// `overlay_check_updates` — vérifie les mises à jour et renvoie un état lisible.
#[tauri::command]
pub async fn overlay_check_updates(app: AppHandle<Wry>) -> UpdateStatus {
    updater::check_for_update(&app).await
}

/// `overlay_install_update` — télécharge/installe la mise à jour disponible.
#[tauri::command]
pub async fn overlay_install_update(app: AppHandle<Wry>) -> UpdateStatus {
    updater::install_update(&app).await
}
