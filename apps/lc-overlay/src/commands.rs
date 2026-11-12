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
use tauri::{AppHandle, Manager, State, WebviewWindow, Wry};

use crate::config::{MemeBindings, OverlayConfigView};
use crate::events;
use crate::net::api;
use crate::net::pairing::{self, PairingRequest};
use crate::net::ws::{self, NetCommand};
use crate::state::AppState;
use crate::tray;
use crate::updater::{self, UpdateStatus};
use crate::windows;

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
            "meme_board_get_bindings",
            "meme_board_set_bindings",
            "meme_board_trigger",
            "meme_board_stop",
            "overlay_check_updates",
            "overlay_install_update",
            "pairing_consume",
            "guilds_list",
            "guild_select",
            "guild_set_muted",
            "guild_leave",
            "guild_api",
            "guild_media_url",
        ],
    ),
    (
        "pairing",
        &[
            "overlay_get_config",
            "overlay_report_error",
            "pairing_consume",
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

/// Projette la configuration pour une fenêtre donnée (AUD-V2-O-002).
///
/// Le `clientToken` n'est utile qu'au board (via `authorizedFetch`) : il est
/// retiré des projections des fenêtres overlay et pairing (moindre privilège),
/// et absent pour tout label inconnu.
fn config_view_for_window(window_label: &str, mut view: OverlayConfigView) -> OverlayConfigView {
    if window_label != windows::BOARD_LABEL {
        view.client_token = None;
    }
    view
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

/// `meme_board_get_bindings` — bindings actuellement enregistrés.
#[tauri::command]
pub fn meme_board_get_bindings(
    window: WebviewWindow<Wry>,
    state: State<'_, AppState>,
) -> Result<MemeBoardBindingsResponse, String> {
    ensure_allowed(&window, "meme_board_get_bindings")?;
    Ok(MemeBoardBindingsResponse {
        bindings: state.active_bindings(),
    })
}

/// `meme_board_set_bindings` — applique puis persiste les bindings (strict).
#[tauri::command]
pub fn meme_board_set_bindings(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<MemeBoardSetBindingsResponse, String> {
    ensure_allowed(&window, "meme_board_set_bindings")?;
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
pub fn meme_board_trigger(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Value,
) -> Result<MemeBoardActionResponse, String> {
    ensure_allowed(&window, "meme_board_trigger")?;
    let parsed = serde_json::from_value::<MemeTriggerRequest>(payload).unwrap_or_default();
    let item_id = parsed
        .item_id
        .map(|item_id| item_id.trim().to_string())
        .filter(|item_id| !item_id.is_empty());
    let Some(item_id) = item_id else {
        return Ok(MemeBoardActionResponse {
            ok: false,
            reason: Some("invalid_item_id".to_string()),
        });
    };
    let trigger = match parsed.trigger.as_deref() {
        Some("shortcut") => MemeTrigger::Shortcut,
        _ => MemeTrigger::Ui,
    };
    let state = app.state::<AppState>();
    let sent = match target_guild(&state, parsed.guild_id.as_deref()) {
        Some(guild_id) => send_protocol_to(
            &app,
            &guild_id,
            ClientMessage::MemeTrigger(MemeTriggerPayload { item_id, trigger }),
            "meme_board_trigger",
        ),
        None => false,
    };
    Ok(MemeBoardActionResponse {
        ok: sent,
        reason: if sent {
            None
        } else {
            Some("socket_unavailable".to_string())
        },
    })
}

/// `meme_board_stop` — arrêt manuel via `stop.request`.
#[tauri::command]
pub fn meme_board_stop(
    window: WebviewWindow<Wry>,
    app: AppHandle<Wry>,
    payload: Option<Value>,
) -> Result<MemeBoardActionResponse, String> {
    ensure_allowed(&window, "meme_board_stop")?;
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
            "meme_board_stop",
        ),
        None => false,
    };
    Ok(MemeBoardActionResponse {
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
    let mut known = false;
    state.update_config(|config| {
        known = config.update_membership(&guild_id, |membership| membership.muted = muted);
    })?;
    if !known {
        return Ok(guild_action(&state, Some("unknown_guild")));
    }
    ws::ensure_supervisor(&app, &guild_id);
    ws::send_command_to(&app, &guild_id, NetCommand::Reconnect);
    publish_guilds(&app);
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
    api::request(
        &membership,
        parsed.method.as_deref().unwrap_or("GET"),
        parsed.path.as_deref().unwrap_or_default(),
        parsed.body,
    )
    .await
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
        assert!(!command_allowed("overlay", "meme_board_set_bindings"));
        assert!(!command_allowed("overlay", "overlay_install_update"));

        // Board : gestion des mémos et mises à jour.
        assert!(command_allowed("board", "meme_board_set_bindings"));
        assert!(command_allowed("board", "meme_board_trigger"));
        assert!(command_allowed("board", "overlay_check_updates"));
        assert!(command_allowed("board", "overlay_install_update"));
        // Le board est l'application principale : il rejoint et gère les guildes.
        assert!(command_allowed("board", "pairing_consume"));
        assert!(command_allowed("board", "guild_api"));
        assert!(command_allowed("board", "guild_leave"));
        assert!(!command_allowed("board", "overlay_renderer_ready"));
        // Les commandes de guilde restent fermées aux autres fenêtres.
        for command in [
            "guilds_list",
            "guild_select",
            "guild_set_muted",
            "guild_leave",
            "guild_api",
            "guild_media_url",
        ] {
            assert!(!command_allowed("overlay", command), "{command}");
            assert!(!command_allowed("pairing", command), "{command}");
        }

        // Pairing : appairage uniquement.
        assert!(command_allowed("pairing", "pairing_consume"));
        assert!(command_allowed("pairing", "overlay_get_config"));
        assert!(!command_allowed("pairing", "meme_board_trigger"));

        // Fenêtre inconnue : tout est refusé.
        assert!(!command_allowed("unknown", "overlay_get_config"));
        assert!(!command_allowed("", "overlay_get_config"));
    }

    #[test]
    fn config_view_is_window_scoped() {
        use crate::config::{MemeBindings, OverlayConfigView};

        fn view_with_token() -> OverlayConfigView {
            OverlayConfigView {
                server_url: Some("https://example.test".to_string()),
                client_token: Some("secret-token".to_string()),
                guild_id: Some("42".to_string()),
                client_id: Some("client-1".to_string()),
                guest_mode: false,
                show_text: true,
                volume: 1.0,
                meme_bindings: MemeBindings::new(),
            }
        }

        // Board : le token est nécessaire à `authorizedFetch`.
        let board = super::config_view_for_window("board", view_with_token());
        assert_eq!(board.client_token.as_deref(), Some("secret-token"));

        // Overlay et pairing : token retiré, reste de la projection intact.
        for label in ["overlay", "pairing", "unknown"] {
            let view = super::config_view_for_window(label, view_with_token());
            assert!(
                view.client_token.is_none(),
                "le token ne doit pas être exposé à la fenêtre '{label}'"
            );
            assert_eq!(view.server_url.as_deref(), Some("https://example.test"));
            assert_eq!(view.guild_id.as_deref(), Some("42"));
        }
    }
}
