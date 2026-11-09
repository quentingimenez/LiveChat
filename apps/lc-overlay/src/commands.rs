//! Commandes exposées au bridge (`window.livechatOverlay`) et aux pages du
//! shell. Les noms suivent le contrat figé de `docs/rust/overlay-bridge.md`
//! §6.1 (`overlay_get_config`, `overlay_renderer_ready`, ...).
//!
//! Phase 8a : les commandes de télémétrie se contentent de journaliser ; le
//! client WebSocket et la sémantique métier (appairage HTTP, mémoire, médias)
//! sont branchés en Phase 8b sans changer cette surface.

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State, Wry};

use crate::config::{MemeBindings, OverlayConfigView};
use crate::events;
use crate::state::AppState;
use crate::tray;
use crate::updater::{self, UpdateStatus};

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
}

fn not_implemented(feature: &str) -> String {
    format!("{feature} sera disponible en Phase 8b (client WebSocket / appairage HTTP)")
}

/// `overlay_get_config` — projection 8 champs consommée par le renderer.
#[tauri::command]
pub fn overlay_get_config(state: State<'_, AppState>) -> OverlayConfigView {
    state.view()
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

/// `overlay_report_error` — erreur remontée par le renderer.
#[tauri::command]
pub fn overlay_report_error(payload: Value) {
    tracing::warn!(payload = %payload, "erreur signalée par le renderer overlay");
}

/// `overlay_report_playback_state` — télémétrie de lecture (branchement 8b).
#[tauri::command]
pub fn overlay_report_playback_state(payload: Value) {
    tracing::debug!(payload = %payload, "état de lecture reporté par le renderer");
}

/// `overlay_report_playback_stop` — fin de lecture reportée par le renderer.
#[tauri::command]
pub fn overlay_report_playback_stop(payload: Value) {
    tracing::debug!(payload = %payload, "arrêt de lecture reporté par le renderer");
}

/// `pairing_consume` — l'appairage HTTP réel arrive en Phase 8b.
#[tauri::command]
pub fn pairing_consume(
    payload: Value,
    _state: State<'_, AppState>,
) -> Result<PairingConsumeResponse, String> {
    tracing::info!(payload = %payload, "demande d'appairage reçue (Phase 8b)");
    Err(not_implemented("l'appairage"))
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

/// `meme_board_trigger` — déclenchement manuel (socket en Phase 8b).
#[tauri::command]
pub fn meme_board_trigger(payload: Value) -> MemeBoardActionResponse {
    tracing::info!(payload = %payload, "déclenchement de mémoire demandé (Phase 8b)");
    MemeBoardActionResponse {
        ok: false,
        reason: Some("socket_unavailable".to_string()),
    }
}

/// `meme_board_stop` — arrêt manuel de la mémoire courante (socket en Phase 8b).
#[tauri::command]
pub fn meme_board_stop() -> MemeBoardActionResponse {
    tracing::info!("arrêt de mémoire demandé (Phase 8b)");
    MemeBoardActionResponse {
        ok: false,
        reason: Some("socket_unavailable".to_string()),
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
