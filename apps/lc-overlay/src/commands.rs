//! Commandes exposées au bridge (`window.livechatOverlay`) et aux pages du
//! shell. Les noms suivent le contrat figé de `docs/rust/overlay-bridge.md`
//! §6.1 (`overlay_get_config`, `overlay_renderer_ready`, ...).
//!
//! Phase 8b : la télémétrie du renderer est traduite en trames WebSocket v2,
//! l'appairage passe par `POST {serverUrl}/overlay/pair/consume`, et les
//! déclenchements de média du board utilisent les trames `media.trigger` /
//! `stop.request` (plus de stub `socket_unavailable`).

use std::time::Duration;

use lc_protocol::{
    AckOutcome, AckPayload, ClientMessage, MediaTrigger, MediaTriggerPayload, PlaybackEndedPayload,
    PlaybackEndedReason, PlaybackState as ProtocolPlaybackState, PlaybackStatePayload,
    StopRequestPayload, StopRequestSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State, WebviewWindow, Wry};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::config::{MediaBindings, OverlayConfigView};
use crate::events;
use crate::net::api;
use crate::net::pairing::{self, PairingRequest};
use crate::net::upload::{self, UploadEntry, UploadRegistry};
use crate::net::ws::{self, NetCommand};
use crate::state::{AppState, GuildRemoval};
use crate::tray;
use crate::updater::{self, UpdateStatus};
use crate::windows;

/// How long `media_board_trigger` waits for the server `ack`.
const TRIGGER_ACK_TIMEOUT: Duration = Duration::from_secs(10);

/// Matrice d'autorisation IPC par fenêtre (AUD-A6-008).
///
/// Chaque page ne peut appeler que les commandes dont elle a besoin : une page
/// compromise (ex. contenu tiers) ne peut pas récupérer le token via
/// `overlay_get_config` depuis une autre fenêtre, ni déclencher l'updater ou
/// l'écriture du board. Les commandes non listées sont refusées.
const COMMAND_MATRIX: &[(&str, &[&str])] = &[
    (
        "overlay",
        &[
            "overlay_get_config",
            "overlay_renderer_ready",
            "overlay_report_error",
            "overlay_report_playback_state",
            "overlay_report_playback_stop",
        ],
    ),
    (
        "board",
        &[
            "overlay_get_config",
            "overlay_report_error",
            "media_board_get_bindings",
            "media_board_set_bindings",
            "media_board_trigger",
            "media_board_stop",
            "overlay_check_updates",
            "overlay_install_update",
            "pairing_consume",
            "guilds_list",
            "guild_select",
            "guild_set_muted",
            "guild_leave",
            "guild_api",
            "guild_media_url",
            "board_take_pending_view",
            "board_take_guild_removals",
            "media_upload_start",
            "media_upload_chunk",
            "media_upload_finish",
            "media_upload_cancel",
        ],
    ),
];

/// Indique si `command` est autorisée pour la fenêtre `window_label`.
pub fn command_allowed(window_label: &str, command: &str) -> bool {
    COMMAND_MATRIX
        .iter()
        .find(|(label, _)| *label == window_label)
        .is_some_and(|(_, commands)| commands.contains(&command))
}

/// Refuse l'appel si la fenêtre émettrice n'est pas autorisée (AUD-A6-008).
fn ensure_allowed(window: &WebviewWindow<Wry>, command: &str) -> Result<(), String> {
    if command_allowed(window.label(), command) {
        return Ok(());
    }
    tracing::warn!(
        window = window.label(),
        command,
        "commande IPC refusée : non autorisée pour cette fenêtre"
    );
    Err(format!(
        "commande '{command}' non autorisée pour la fenêtre '{}'",
        window.label()
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaBoardBindingsResponse {
    pub bindings: MediaBindings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaBoardSetBindingsResponse {
    pub ok: bool,
    pub bindings: MediaBindings,
    pub failed_accelerators: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaBoardActionResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Result of `media_board_trigger`, once the server has acknowledged it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaBoardTriggerResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<AckOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
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

/// Payload de `media_board_trigger`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaTriggerRequest {
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    /// Guilde visée ; par défaut la guilde active.
    #[serde(default)]
    guild_id: Option<String>,
}

/// Payload des commandes ciblant une guilde.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GuildRequest {
    #[serde(default)]
    guild_id: Option<String>,
    #[serde(default)]
    muted: Option<bool>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    asset_id: Option<String>,
}

/// Paramètres of `media_upload_start`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaUploadStartRequest {
    #[serde(default)]
    guild_id: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
}

/// Payload of `media_upload_finish` / `media_upload_cancel`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaUploadRequest {
    #[serde(default)]
    upload_id: Option<String>,
}

/// Résultat d'une commande de gestion des guildes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuildActionResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// État à jour des guildes (même forme que `guilds_list`).
    pub guilds: Value,
}

/// Guilde visée par une commande : celle demandée, sinon la guilde active.
fn target_guild(state: &AppState, requested: Option<&str>) -> Option<String> {
    requested
        .map(str::trim)
        .filter(|guild_id| !guild_id.is_empty())
        .map(ToString::to_string)
        .or_else(|| state.active_guild())
}

/// Envoie une trame à une guilde précise.
fn send_protocol_to(
    app: &AppHandle<Wry>,
    guild_id: &str,
    message: ClientMessage,
    label: &str,
) -> bool {
    let sent = ws::send_command_to(app, guild_id, NetCommand::Send(message));
    if !sent {
        tracing::debug!(
            action = label,
            guild_id,
            "trame non envoyée : connexion WebSocket inactive"
        );
    }
    sent
}

/// Envoie une trame de télémétrie à la guilde dont le média est à l'écran
/// (à défaut, la guilde active).
fn send_playback_protocol(app: &AppHandle<Wry>, message: ClientMessage, label: &str) -> bool {
    let state = app.state::<AppState>();
    match state.playback_guild().or_else(|| state.active_guild()) {
        Some(guild_id) => send_protocol_to(app, &guild_id, message, label),
        None => false,
    }
}

/// Répercute un changement de guildes : raccourcis, settings renderer, tray,
/// événement `overlay://guilds`.
fn publish_guilds(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    crate::shortcuts::reapply_for_session(app);
    if state.overlay_ready() {
        if let Err(error) = events::emit_settings(app, &state.view()) {
            tracing::debug!(%error, "échec d'émission des settings après changement de guilde");
        }
    }
    tray::refresh_tray(app);
    if let Err(error) = events::emit_guilds_changed(app) {
        tracing::debug!(%error, "échec d'émission de l'état des guildes");
    }
}

fn guild_action(state: &AppState, reason: Option<&str>) -> GuildActionResponse {
    GuildActionResponse {
        ok: reason.is_none(),
        reason: reason.map(ToString::to_string),
        guilds: state.guilds_payload(),
    }
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

/// Converts a server `ack` into the board-facing trigger response (pure).
fn trigger_response_from_ack(ack: AckPayload) -> MediaBoardTriggerResponse {
    if ack.ok {
        return MediaBoardTriggerResponse {
            ok: true,
            reason: None,
            message: None,
            outcome: ack.outcome,
            position: ack.position,
            job_id: ack.job_id,
        };
    }
    let (reason, message) = match ack.error {
        Some(error) => (error.code, Some(error.message)),
        None => ("trigger_refused".to_string(), None),
    };
    MediaBoardTriggerResponse {
        ok: false,
        reason: Some(reason),
        message,
        outcome: None,
        position: None,
        job_id: None,
    }
}

/// Failure response for `media_board_trigger` (no ack involved).
fn trigger_failure(reason: &str) -> MediaBoardTriggerResponse {
    MediaBoardTriggerResponse {
        ok: false,
        reason: Some(reason.to_string()),
        message: None,
        outcome: None,
        position: None,
        job_id: None,
    }
}

/// A REST 401 means the guild token must be revalidated: ask the WebSocket
/// supervisor to reconnect (a 4401 close then purges the guild).
fn reconnect_on_unauthorized(app: &AppHandle<Wry>, guild_id: &str, response: &api::ApiResponse) {
    if response.status != 401 {
        return;
    }
    tracing::info!(
        guild_id,
        "401 REST : reconnexion WebSocket pour revalider le jeton"
    );
    if !ws::send_command_to(app, guild_id, NetCommand::Reconnect) {
        tracing::debug!(guild_id, "reconnexion non transmise : superviseur absent");
    }
}

fn upload_id_from_payload(payload: &Value) -> Option<String> {
    let parsed = serde_json::from_value::<MediaUploadRequest>(payload.clone()).unwrap_or_default();
    parsed
        .upload_id
        .map(|upload_id| upload_id.trim().to_string())
        .filter(|upload_id| !upload_id.is_empty())
}

/// Projette la configuration pour une fenêtre donnée (AUD-V2-O-002).
///
/// Le `clientToken` n'est utile qu'au board (via `authorizedFetch`) : il est
/// retiré de la projection de la fenêtre overlay (moindre privilège), et absent
/// pour tout label inconnu.
fn config_view_for_window(window_label: &str, mut view: OverlayConfigView) -> OverlayConfigView {
    if window_label != windows::BOARD_LABEL {
        view.client_token = None;
    }
    view
}

/// `board_take_pending_view` — view requested by the tray before the board was
/// loaded, consumed once (`null` when absent).
#[tauri::command]
pub fn board_take_pending_view(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    ensure_allowed(&window, "board_take_pending_view")?;
    Ok(state.take_pending_board_view())
}

/// `board_take_guild_removals` — guilds purged automatically since the last
/// call (consumed once, empty when nothing was removed).
#[tauri::command]
pub fn board_take_guild_removals(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
) -> Result<Vec<GuildRemoval>, String> {
    ensure_allowed(&window, "board_take_guild_removals")?;
    Ok(state.take_guild_removals())
}

/// `overlay_get_config` — projection 8 champs consommée par le renderer.
#[tauri::command]
pub fn overlay_get_config(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
) -> Result<OverlayConfigView, String> {
    ensure_allowed(&window, "overlay_get_config")?;
    let view = config_view_for_window(window.label(), state.view());
    tracing::debug!(
        window = window.label(),
        has_server_url = view.server_url.is_some(),
        has_token = view.client_token.is_some(),
        has_guild = view.guild_id.is_some(),
        "overlay_get_config demandé par le renderer"
    );
    Ok(view)
}

/// `overlay_renderer_ready` — handshake : envoie `overlay://settings` puis
/// rejoue les événements bufferisés.
#[tauri::command]
pub fn overlay_renderer_ready(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ensure_allowed(&window, "overlay_renderer_ready")?;
    state.mark_overlay_ready();
    tracing::info!("renderer overlay prêt : envoi des settings et rejeu du buffer");
    if let Err(error) = events::emit_settings(&app, &state.view()) {
        tracing::warn!(%error, "échec d'émission des settings initiaux");
    }
    events::flush_pending(&app);
    Ok(())
}

/// `overlay_report_error` — erreur remontée par le renderer, relayée au serveur.
#[tauri::command]
pub fn overlay_report_error(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<(), String> {
    ensure_allowed(&window, "overlay_report_error")?;
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
    send_playback_protocol(
        &app,
        ClientMessage::Error(lc_protocol::ClientErrorPayload {
            job_id: parsed.job_id.unwrap_or_default(),
            code,
            message,
        }),
        "overlay_report_error",
    );
    Ok(())
}

/// `overlay_report_playback_state` — position de lecture, limitée à 1 msg/s.
#[tauri::command]
pub fn overlay_report_playback_state(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<(), String> {
    ensure_allowed(&window, "overlay_report_playback_state")?;
    let parsed =
        serde_json::from_value::<RendererPlaybackStatePayload>(payload.clone()).unwrap_or_default();
    let state = app.state::<AppState>();
    let Some(playback_state) = parsed.state.as_deref().and_then(playback_state_from_label) else {
        tracing::debug!(payload = %payload, "état de lecture ignoré (état inconnu)");
        return Ok(());
    };
    let job_id = parsed
        .job_id
        .filter(|job_id| !job_id.trim().is_empty())
        .or_else(|| state.playback_job())
        .unwrap_or_default();
    if job_id.is_empty() {
        tracing::debug!("état de lecture ignoré : aucun job courant");
        return Ok(());
    }
    if !state.try_record_playback_state() {
        tracing::trace!("playback.state limité à 1 msg/s");
        return Ok(());
    }
    let remaining_ms = parsed
        .remaining_ms
        .filter(|remaining| remaining.is_finite() && *remaining > 0.0)
        .map(|remaining| (remaining as u64).min(lc_protocol::MAX_PLAYBACK_REMAINING_MS));
    send_playback_protocol(
        &app,
        ClientMessage::PlaybackState(PlaybackStatePayload {
            job_id,
            state: playback_state,
            remaining_ms,
        }),
        "overlay_report_playback_state",
    );
    Ok(())
}

/// `overlay_report_playback_stop` — fin de lecture envoyée comme `playback.ended`.
#[tauri::command]
pub fn overlay_report_playback_stop(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<(), String> {
    ensure_allowed(&window, "overlay_report_playback_stop")?;
    let parsed = serde_json::from_value::<RendererPlaybackStopPayload>(payload).unwrap_or_default();
    let state = app.state::<AppState>();
    let job_id = parsed
        .job_id
        .filter(|job_id| !job_id.trim().is_empty())
        .or_else(|| state.playback_job())
        .unwrap_or_default();
    if job_id.is_empty() {
        tracing::debug!("arrêt de lecture ignoré : aucun job courant");
        return Ok(());
    }
    // La trame part avant de libérer l'écran : elle doit atteindre la guilde
    // propriétaire du job, pas celle du `play` suivant.
    send_playback_protocol(
        &app,
        ClientMessage::PlaybackEnded(PlaybackEndedPayload {
            job_id,
            reason: ended_reason_from_label(parsed.reason.as_deref()),
        }),
        "overlay_report_playback_stop",
    );
    state.clear_playback();
    ws::advance_local_queue(&app);
    Ok(())
}

/// `pairing_consume` — appairage HTTP réel puis reconnexion WebSocket.
#[tauri::command]
pub async fn pairing_consume(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<PairingConsumeResponse, String> {
    ensure_allowed(&window, "pairing_consume")?;
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
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
    };
    let device_name = text("deviceName");
    let display_name = text("displayName");
    let avatar = text("avatar");
    tracing::info!(server_url = %server_url, "demande d'appairage reçue");
    let outcome = pairing::consume(
        &state,
        PairingRequest {
            server_url,
            code,
            device_name,
            display_name,
            avatar,
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
    tray::refresh_tray(&app);
    if state.config_snapshot().enabled {
        windows::show_overlay(&app);
    }
    ws::request_reconnect(&app);
    if let Err(error) = events::emit_guilds_changed(&app) {
        tracing::debug!(%error, "échec d'émission de l'état des guildes après appairage");
    }
    Ok(PairingConsumeResponse {
        ok: true,
        server_url: outcome.server_url,
        guild_id: outcome.guild_id,
        guest_mode: Some(outcome.guest_mode),
        message: outcome.message,
    })
}

/// `media_board_get_bindings` — bindings actuellement enregistrés.
#[tauri::command]
pub fn media_board_get_bindings(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
) -> Result<MediaBoardBindingsResponse, String> {
    ensure_allowed(&window, "media_board_get_bindings")?;
    Ok(MediaBoardBindingsResponse {
        bindings: state.active_bindings(),
    })
}

/// `media_board_set_bindings` — applique puis persiste les bindings (strict).
#[tauri::command]
pub fn media_board_set_bindings(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<MediaBoardSetBindingsResponse, String> {
    ensure_allowed(&window, "media_board_set_bindings")?;
    let bindings = payload
        .get("bindings")
        .cloned()
        .map(serde_json::from_value::<MediaBindings>)
        .transpose()
        .map_err(|error| format!("bindings invalides : {error}"))?
        .unwrap_or_default();
    let result = crate::shortcuts::apply_media_bindings(&app, bindings, true, true);
    tray::refresh_tray(&app);
    Ok(MediaBoardSetBindingsResponse {
        ok: result.ok,
        bindings: result.applied_bindings,
        failed_accelerators: result.failed_accelerators,
    })
}

/// `media_board_trigger` — déclenchement manuel via `media.trigger`, puis
/// attente de l'`ack` du serveur (rang en file, `jobId`, refus).
#[tauri::command]
pub async fn media_board_trigger(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<MediaBoardTriggerResponse, String> {
    ensure_allowed(&window, "media_board_trigger")?;
    let parsed = serde_json::from_value::<MediaTriggerRequest>(payload).unwrap_or_default();
    let item_id = parsed
        .item_id
        .map(|item_id| item_id.trim().to_string())
        .filter(|item_id| !item_id.is_empty());
    let Some(item_id) = item_id else {
        return Ok(trigger_failure("invalid_item_id"));
    };
    let trigger = match parsed.trigger.as_deref() {
        Some("shortcut") => MediaTrigger::Shortcut,
        _ => MediaTrigger::Ui,
    };
    let (reply, receiver) = oneshot::channel();
    let sent = {
        let state = app.state::<AppState>();
        match target_guild(&state, parsed.guild_id.as_deref()) {
            Some(guild_id) => ws::send_command_to(
                &app,
                &guild_id,
                NetCommand::SendWithAck {
                    message: ClientMessage::MediaTrigger(MediaTriggerPayload { item_id, trigger }),
                    reply,
                },
            ),
            None => false,
        }
    };
    if !sent {
        return Ok(trigger_failure("socket_unavailable"));
    }
    match tokio::time::timeout(TRIGGER_ACK_TIMEOUT, receiver).await {
        Ok(Ok(ack)) => Ok(trigger_response_from_ack(ack)),
        Ok(Err(_)) => Ok(trigger_failure("socket_unavailable")),
        Err(_) => Ok(trigger_failure("ack_timeout")),
    }
}

/// `media_board_stop` — arrêt manuel via `stop.request`.
#[tauri::command]
pub fn media_board_stop(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Option<Value>,
) -> Result<MediaBoardActionResponse, String> {
    ensure_allowed(&window, "media_board_stop")?;
    let parsed = payload
        .map(|payload| serde_json::from_value::<GuildRequest>(payload).unwrap_or_default())
        .unwrap_or_default();
    let state = app.state::<AppState>();
    let sent = match target_guild(&state, parsed.guild_id.as_deref()) {
        Some(guild_id) => send_protocol_to(
            &app,
            &guild_id,
            ClientMessage::StopRequest(StopRequestPayload {
                job_id: None,
                source: Some(StopRequestSource::User),
            }),
            "media_board_stop",
        ),
        None => false,
    };
    Ok(MediaBoardActionResponse {
        ok: sent,
        reason: if sent {
            None
        } else {
            Some("socket_unavailable".to_string())
        },
    })
}

/// `guilds_list` — guildes rejointes et état de connexion (sans jeton).
#[tauri::command]
pub fn guilds_list(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    ensure_allowed(&window, "guilds_list")?;
    Ok(state.guilds_payload())
}

/// `guild_select` — change la guilde active (board, raccourcis, tray).
#[tauri::command]
pub fn guild_select(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<GuildActionResponse, String> {
    ensure_allowed(&window, "guild_select")?;
    let parsed = serde_json::from_value::<GuildRequest>(payload).unwrap_or_default();
    let Some(guild_id) = parsed.guild_id.filter(|id| !id.trim().is_empty()) else {
        return Ok(guild_action(&state, Some("invalid_guild_id")));
    };
    let mut known = false;
    state.update_config(|config| known = config.activate(&guild_id))?;
    if !known {
        return Ok(guild_action(&state, Some("unknown_guild")));
    }
    publish_guilds(&app);
    Ok(guild_action(&state, None))
}

/// Applies a guild mute then broadcasts the change.
///
/// Shared by the tray checkbox and the `guild_set_muted` command. Returns
/// `false` when the guild is unknown.
pub fn set_guild_muted(app: &AppHandle<Wry>, guild_id: &str, muted: bool) -> Result<bool, String> {
    let guild_id = guild_id.trim();
    if guild_id.is_empty() {
        return Ok(false);
    }
    let state = app.state::<AppState>();
    let mut known = false;
    state.update_config(|config| {
        known = config.update_membership(guild_id, |membership| membership.muted = muted);
    })?;
    if !known {
        return Ok(false);
    }
    ws::ensure_supervisor(app, guild_id);
    ws::send_command_to(app, guild_id, NetCommand::Reconnect);
    publish_guilds(app);
    Ok(true)
}

/// `guild_set_muted` — coupe ou rétablit une guilde. Une guilde coupée est
/// déconnectée : plus aucun de ses médias ne s'affiche.
#[tauri::command]
pub fn guild_set_muted(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<GuildActionResponse, String> {
    ensure_allowed(&window, "guild_set_muted")?;
    let parsed = serde_json::from_value::<GuildRequest>(payload).unwrap_or_default();
    let (Some(guild_id), Some(muted)) = (
        target_guild(&state, parsed.guild_id.as_deref()),
        parsed.muted,
    ) else {
        return Ok(guild_action(&state, Some("invalid_payload")));
    };
    if !set_guild_muted(&app, &guild_id, muted)? {
        return Ok(guild_action(&state, Some("unknown_guild")));
    }
    Ok(guild_action(&state, None))
}

/// `guild_leave` — quitte une guilde : départ côté serveur puis retrait local.
///
/// Si le serveur refuse (`last_admin` : dernier admin de la guilde), l'adhésion
/// est conservée. Si le serveur est injoignable ou ne connaît plus le jeton,
/// l'adhésion est retirée localement malgré tout.
#[tauri::command]
pub async fn guild_leave(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<GuildActionResponse, String> {
    ensure_allowed(&window, "guild_leave")?;
    let parsed = serde_json::from_value::<GuildRequest>(payload).unwrap_or_default();
    let Some(guild_id) = parsed.guild_id.filter(|id| !id.trim().is_empty()) else {
        return Ok(guild_action(&state, Some("invalid_guild_id")));
    };
    let Some(membership) = state.config_snapshot().membership(&guild_id).cloned() else {
        return Ok(guild_action(&state, Some("unknown_guild")));
    };
    match api::request(&membership, "POST", "/overlay/leave", None).await {
        Ok(response) if response.status == 409 => {
            let reason = response
                .body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("conflict")
                .to_string();
            return Ok(guild_action(&state, Some(&reason)));
        }
        Ok(response) => {
            tracing::info!(guild_id = %guild_id, status = response.status, "départ de la guilde");
        }
        Err(error) => {
            tracing::warn!(guild_id = %guild_id, %error, "départ non confirmé par le serveur");
        }
    }
    state.update_config(|config| {
        config.leave(&guild_id);
    })?;
    // Réveille le superviseur : il constate le retrait et s'arrête.
    ws::send_command_to(&app, &guild_id, NetCommand::Reconnect);
    publish_guilds(&app);
    if !state.config_snapshot().has_pairing_config() {
        windows::hide_overlay(&app);
    }
    Ok(guild_action(&state, None))
}

/// `guild_api` — appel REST authentifié `/overlay/...` pour une guilde. Le
/// jeton reste dans le shell.
#[tauri::command]
pub async fn guild_api(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<api::ApiResponse, String> {
    ensure_allowed(&window, "guild_api")?;
    let parsed = serde_json::from_value::<GuildRequest>(payload).unwrap_or_default();
    let guild_id = target_guild(&state, parsed.guild_id.as_deref())
        .ok_or_else(|| "aucune guilde".to_string())?;
    let membership = state
        .config_snapshot()
        .membership(&guild_id)
        .cloned()
        .ok_or_else(|| "guilde inconnue".to_string())?;
    let response = api::request(
        &membership,
        parsed.method.as_deref().unwrap_or("GET"),
        parsed.path.as_deref().unwrap_or_default(),
        parsed.body,
    )
    .await?;
    reconnect_on_unauthorized(&app, &guild_id, &response);
    Ok(response)
}

/// `guild_media_url` — URL d'un média du board pour `<img>`/`<video>`.
#[tauri::command]
pub fn guild_media_url(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
    payload: Value,
) -> Result<String, String> {
    ensure_allowed(&window, "guild_media_url")?;
    let parsed = serde_json::from_value::<GuildRequest>(payload).unwrap_or_default();
    let guild_id = target_guild(&state, parsed.guild_id.as_deref())
        .ok_or_else(|| "aucune guilde".to_string())?;
    let membership = state
        .config_snapshot()
        .membership(&guild_id)
        .cloned()
        .ok_or_else(|| "guilde inconnue".to_string())?;
    api::board_media_url(&membership, parsed.asset_id.as_deref().unwrap_or_default())
}

/// `media_upload_start` — begins a raw, streamed file upload to the board.
#[tauri::command]
pub fn media_upload_start(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
    uploads: State<'_, UploadRegistry>,
    payload: Value,
) -> Result<upload::MediaUploadStartResponse, String> {
    ensure_allowed(&window, "media_upload_start")?;
    let parsed = serde_json::from_value::<MediaUploadStartRequest>(payload).unwrap_or_default();
    let guild_id = target_guild(&state, parsed.guild_id.as_deref())
        .ok_or_else(|| "guilde inconnue".to_string())?;
    let membership = state
        .config_snapshot()
        .membership(&guild_id)
        .cloned()
        .ok_or_else(|| "guilde inconnue".to_string())?;
    let filename = upload::validate_filename(parsed.filename.as_deref())?;
    let insecure = crate::net::tls::insecure_tls_enabled();
    let url = upload::upload_url(
        &membership.server_url,
        &filename,
        parsed.title.as_deref(),
        parsed.message.as_deref(),
        insecure,
    )?;
    let content_type = upload::normalize_content_type(parsed.content_type.as_deref());
    let client = upload::build_client(insecure)?;
    let (sender, receiver) = mpsc::channel::<upload::UploadChunk>(4);
    let token = membership.client_token;
    let task = tauri::async_runtime::spawn(async move {
        upload::run_upload(client, url, &token, &content_type, receiver).await
    });
    let upload_id = Uuid::new_v4().to_string();
    uploads.insert(
        upload_id.clone(),
        UploadEntry {
            guild_id,
            sender,
            task,
        },
    )?;
    Ok(upload::MediaUploadStartResponse::new(upload_id))
}

/// `media_upload_chunk` — appends raw bytes to an in-flight upload.
#[tauri::command]
pub async fn media_upload_chunk(
    window: WebviewWindow<Wry>,
    uploads: State<'_, UploadRegistry>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    ensure_allowed(&window, "media_upload_chunk")?;
    let upload_id = request
        .headers()
        .get("x-upload-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|upload_id| !upload_id.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "envoi inconnu".to_string())?;
    let bytes = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.clone(),
        tauri::ipc::InvokeBody::Json(_) => return Err("corps binaire attendu".to_string()),
    };
    if bytes.is_empty() {
        return Err("morceau vide".to_string());
    }
    if bytes.len() > upload::MAX_CHUNK_BYTES {
        return Err("morceau trop volumineux".to_string());
    }
    // The registry lock is released before awaiting the send.
    let sender = uploads.sender(&upload_id)?;
    sender
        .send(Ok(bytes))
        .await
        .map_err(|_| "upload_closed".to_string())
}

/// `media_upload_finish` — ends the body and returns the server response
/// (same shape as `guild_api`).
#[tauri::command]
pub async fn media_upload_finish(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    uploads: State<'_, UploadRegistry>,
    payload: Value,
) -> Result<api::ApiResponse, String> {
    ensure_allowed(&window, "media_upload_finish")?;
    let upload_id = upload_id_from_payload(&payload).ok_or_else(|| "envoi inconnu".to_string())?;
    let UploadEntry {
        guild_id,
        sender,
        task,
    } = uploads
        .take(&upload_id)
        .ok_or_else(|| "envoi inconnu".to_string())?;
    // Closing the sender terminates the streamed request body.
    drop(sender);
    match task.await {
        Ok(Ok(response)) => {
            reconnect_on_unauthorized(&app, &guild_id, &response);
            Ok(response)
        }
        Ok(Err(message)) => Err(message),
        Err(error) => Err(format!("envoi du fichier interrompu : {error}")),
    }
}

/// `media_upload_cancel` — aborts an in-flight upload (`true` if it existed).
#[tauri::command]
pub fn media_upload_cancel(
    window: WebviewWindow<Wry>,
    uploads: State<'_, UploadRegistry>,
    payload: Value,
) -> Result<bool, String> {
    ensure_allowed(&window, "media_upload_cancel")?;
    let Some(upload_id) = upload_id_from_payload(&payload) else {
        return Ok(false);
    };
    Ok(uploads.cancel(&upload_id))
}

/// `overlay_check_updates` — vérifie les mises à jour et renvoie un état lisible.
#[tauri::command]
pub async fn overlay_check_updates(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
) -> Result<UpdateStatus, String> {
    ensure_allowed(&window, "overlay_check_updates")?;
    Ok(updater::check_for_update(&app).await)
}

/// `overlay_install_update` — télécharge/installe la mise à jour disponible.
#[tauri::command]
pub async fn overlay_install_update(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
) -> Result<UpdateStatus, String> {
    ensure_allowed(&window, "overlay_install_update")?;
    Ok(updater::install_update(&app).await)
}

#[cfg(test)]
mod tests {
    use super::command_allowed;

    #[test]
    fn command_matrix_is_window_scoped() {
        // Fenêtre overlay : télémetrie et handshake uniquement.
        assert!(command_allowed("overlay", "overlay_get_config"));
        assert!(command_allowed("overlay", "overlay_renderer_ready"));
        assert!(command_allowed("overlay", "overlay_report_error"));
        assert!(!command_allowed("overlay", "pairing_consume"));
        assert!(!command_allowed("overlay", "media_board_set_bindings"));
        assert!(!command_allowed("overlay", "overlay_install_update"));

        // Board : gestion des médias du board et mises à jour.
        assert!(command_allowed("board", "media_board_set_bindings"));
        assert!(command_allowed("board", "media_board_trigger"));
        assert!(command_allowed("board", "overlay_check_updates"));
        assert!(command_allowed("board", "overlay_install_update"));
        // Le board est l'application principale : il rejoint et gère les guildes.
        assert!(command_allowed("board", "pairing_consume"));
        assert!(command_allowed("board", "guild_api"));
        assert!(command_allowed("board", "guild_leave"));
        assert!(command_allowed("board", "board_take_pending_view"));
        assert!(command_allowed("board", "board_take_guild_removals"));
        assert!(command_allowed("board", "media_upload_start"));
        assert!(command_allowed("board", "media_upload_chunk"));
        assert!(command_allowed("board", "media_upload_finish"));
        assert!(command_allowed("board", "media_upload_cancel"));
        assert!(!command_allowed("board", "overlay_renderer_ready"));
        // Les commandes de guilde restent fermées aux autres fenêtres.
        for command in [
            "guilds_list",
            "guild_select",
            "guild_set_muted",
            "guild_leave",
            "guild_api",
            "guild_media_url",
            "board_take_guild_removals",
            "media_upload_start",
            "media_upload_chunk",
            "media_upload_finish",
            "media_upload_cancel",
        ] {
            assert!(!command_allowed("overlay", command), "{command}");
        }

        // The pending view can only be consumed by the board.
        assert!(!command_allowed("overlay", "board_take_pending_view"));

        // Fenêtre inconnue : tout est refusé.
        assert!(!command_allowed("unknown", "overlay_get_config"));
        assert!(!command_allowed("unknown", "board_take_pending_view"));
        assert!(!command_allowed("", "overlay_get_config"));
    }

    #[test]
    fn config_view_is_window_scoped() {
        use crate::config::{MediaBindings, OverlayConfigView};

        fn view_with_token() -> OverlayConfigView {
            OverlayConfigView {
                server_url: Some("https://example.test".to_string()),
                client_token: Some("secret-token".to_string()),
                guild_id: Some("42".to_string()),
                client_id: Some("client-1".to_string()),
                guest_mode: false,
                show_text: true,
                volume: 1.0,
                media_bindings: MediaBindings::new(),
            }
        }

        // Board : le token est nécessaire à `authorizedFetch`.
        let board = super::config_view_for_window("board", view_with_token());
        assert_eq!(board.client_token.as_deref(), Some("secret-token"));

        // Overlay et label inconnu : token retiré, reste de la projection intact.
        for label in ["overlay", "unknown"] {
            let view = super::config_view_for_window(label, view_with_token());
            assert!(
                view.client_token.is_none(),
                "le token ne doit pas être exposé à la fenêtre '{label}'"
            );
            assert_eq!(view.server_url.as_deref(), Some("https://example.test"));
            assert_eq!(view.guild_id.as_deref(), Some("42"));
        }
    }

    #[test]
    fn ack_is_projected_into_the_trigger_response() {
        use lc_protocol::{AckError, AckOutcome, AckPayload};

        let queued = super::trigger_response_from_ack(AckPayload {
            ref_id: "cli-1".to_string(),
            ok: true,
            error: None,
            outcome: Some(AckOutcome::Queued),
            position: Some(2),
            job_id: Some("job-42".to_string()),
        });
        assert!(queued.ok);
        assert_eq!(queued.outcome, Some(AckOutcome::Queued));
        assert_eq!(queued.position, Some(2));
        assert_eq!(queued.job_id.as_deref(), Some("job-42"));
        assert_eq!(
            serde_json::to_value(&queued).expect("json"),
            serde_json::json!({
                "ok": true,
                "outcome": "queued",
                "position": 2,
                "jobId": "job-42",
            })
        );

        // Ancien serveur : ok sans outcome/position/jobId.
        let playing = super::trigger_response_from_ack(AckPayload {
            ref_id: "cli-2".to_string(),
            ok: true,
            error: None,
            outcome: Some(AckOutcome::Playing),
            position: None,
            job_id: None,
        });
        assert_eq!(
            serde_json::to_value(&playing).expect("json"),
            serde_json::json!({ "ok": true, "outcome": "playing" })
        );

        let refused = super::trigger_response_from_ack(AckPayload {
            ref_id: "cli-3".to_string(),
            ok: false,
            error: Some(AckError {
                code: "read_only".to_string(),
                message: "session invitée".to_string(),
            }),
            outcome: None,
            position: None,
            job_id: None,
        });
        assert!(!refused.ok);
        assert_eq!(refused.reason.as_deref(), Some("read_only"));
        assert_eq!(refused.message.as_deref(), Some("session invitée"));

        let bare_refusal = super::trigger_response_from_ack(AckPayload {
            ref_id: "cli-4".to_string(),
            ok: false,
            error: None,
            outcome: None,
            position: None,
            job_id: None,
        });
        assert!(!bare_refusal.ok);
        assert_eq!(bare_refusal.reason.as_deref(), Some("trigger_refused"));
        assert!(bare_refusal.message.is_none());
    }
}
