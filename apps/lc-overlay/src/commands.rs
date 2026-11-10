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
    send_protocol(
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
    send_protocol(
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
    state.clear_playback();
    send_protocol(
        &app,
        ClientMessage::PlaybackEnded(PlaybackEndedPayload {
            job_id,
            reason: ended_reason_from_label(parsed.reason.as_deref()),
        }),
        "overlay_report_playback_stop",
    );
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
    let sent = send_protocol(
        &app,
        ClientMessage::MemeTrigger(MemeTriggerPayload { item_id, trigger }),
        "meme_board_trigger",
    );
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
) -> Result<MemeBoardActionResponse, String> {
    ensure_allowed(&window, "meme_board_stop")?;
    let sent = send_protocol(
        &app,
        ClientMessage::StopRequest(StopRequestPayload {
            job_id: None,
            source: Some(StopRequestSource::User),
        }),
        "meme_board_stop",
    );
    Ok(MemeBoardActionResponse {
        ok: sent,
        reason: if sent {
            None
        } else {
            Some("socket_unavailable".to_string())
        },
    })
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
        assert!(!command_allowed("board", "pairing_consume"));
        assert!(!command_allowed("board", "overlay_renderer_ready"));

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
