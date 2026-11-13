//! Squelette des raccourcis globaux du shell.
//!
//! Phase 8a : enregistre/désenregistre les accélérateurs `mediaBindings` sans
//! émettre aucun média (le client WebSocket arrive en Phase 8b). Un raccourci
//! « recharger l'overlay » est également posé ; sa valeur exacte sera alignée
//! sur le legacy lors du branchement du renderer.

use lc_protocol::{ClientMessage, MediaTrigger, MediaTriggerPayload};
use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::config::{normalize_media_bindings, MediaBindings};
use crate::net::ws::{self, NetCommand};
use crate::state::AppState;
use crate::windows::OVERLAY_LABEL;

/// Raccourci de secours pour recharger l'overlay (valeur legacy à confirmer).
pub const MANUAL_RELOAD_ACCELERATOR: &str = "CommandOrControl+Shift+Alt+R";

/// Résultat d'application des bindings du board média (même forme que le legacy).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaBindingsApplyResult {
    pub ok: bool,
    pub applied_bindings: MediaBindings,
    pub failed_accelerators: Vec<String>,
}

impl MediaBindingsApplyResult {
    fn success(applied_bindings: MediaBindings, failed_accelerators: Vec<String>) -> Self {
        Self {
            ok: failed_accelerators.is_empty(),
            applied_bindings,
            failed_accelerators,
        }
    }
}

fn register_one(app: &AppHandle<Wry>, accelerator: &str) -> bool {
    match app.global_shortcut().register(accelerator) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(accelerator, %error, "échec d'enregistrement d'un raccourci global");
            false
        }
    }
}

fn unregister_one(app: &AppHandle<Wry>, accelerator: &str) {
    if let Err(error) = app.global_shortcut().unregister(accelerator) {
        tracing::debug!(accelerator, %error, "raccourci non désenregistré");
    }
}

fn unregister_bindings(app: &AppHandle<Wry>, bindings: &MediaBindings) {
    for accelerator in bindings.keys() {
        unregister_one(app, accelerator);
    }
}

/// Applique `next` avec la sémantique `applyMediaBindings` du legacy :
/// en mode invité, rien n'est enregistré ; en mode strict, un échec annule
/// l'ensemble et restaure les bindings précédents.
pub fn apply_media_bindings(
    app: &AppHandle<Wry>,
    next: MediaBindings,
    strict: bool,
    persist: bool,
) -> MediaBindingsApplyResult {
    let state = app.state::<AppState>();
    let normalized = normalize_media_bindings(&next);
    let guest_mode = state.config_snapshot().guest_mode;
    let previous = state.active_bindings();
    unregister_bindings(app, &previous);

    if guest_mode {
        state.set_active_bindings(MediaBindings::default());
        if persist {
            if let Err(error) = state.update_config(|config| {
                config.media_bindings = MediaBindings::default();
            }) {
                tracing::warn!(%error, "échec de sauvegarde des bindings (mode invité)");
            }
        }
        tracing::info!("mode invité : aucun raccourci du board média enregistré");
        return MediaBindingsApplyResult::success(MediaBindings::default(), Vec::new());
    }

    let mut applied = MediaBindings::default();
    let mut failed = Vec::new();
    for (accelerator, item_id) in &normalized {
        if register_one(app, accelerator) {
            applied.insert(accelerator.clone(), item_id.clone());
        } else {
            failed.push(accelerator.clone());
        }
    }

    if strict && !failed.is_empty() {
        unregister_bindings(app, &applied);
        let mut rolled_back = MediaBindings::default();
        for (accelerator, item_id) in &previous {
            if register_one(app, accelerator) {
                rolled_back.insert(accelerator.clone(), item_id.clone());
            }
        }
        state.set_active_bindings(rolled_back.clone());
        tracing::warn!(
            failed = failed.len(),
            "bindings restaurés après échec en mode strict"
        );
        return MediaBindingsApplyResult {
            ok: false,
            applied_bindings: rolled_back,
            failed_accelerators: failed,
        };
    }

    state.set_active_bindings(applied.clone());
    if persist {
        if let Err(error) = state.update_config(|config| {
            config.media_bindings = applied.clone();
        }) {
            tracing::warn!(%error, "échec de sauvegarde des bindings du board média");
        }
    }
    MediaBindingsApplyResult::success(applied, failed)
}

/// Enregistre le raccourci de rechargement manuel de l'overlay.
pub fn register_manual_reload(app: &AppHandle<Wry>) {
    if !register_one(app, MANUAL_RELOAD_ACCELERATOR) {
        tracing::warn!(
            accelerator = MANUAL_RELOAD_ACCELERATOR,
            "raccourci de rechargement indisponible"
        );
    }
}

/// Recharge l'overlay si la touche de rechargement manuel est pressée.
pub fn handle_shortcut_pressed(
    app: &AppHandle<Wry>,
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
) {
    let is_manual_reload = MANUAL_RELOAD_ACCELERATOR
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map(|manual| manual.id() == shortcut.id())
        .unwrap_or(false);
    if !is_manual_reload {
        if let Some(item_id) = board_item_for_shortcut(app, shortcut) {
            let sent = ws::send_command(
                app,
                NetCommand::Send(ClientMessage::MediaTrigger(MediaTriggerPayload {
                    item_id: item_id.clone(),
                    trigger: MediaTrigger::Shortcut,
                })),
            );
            tracing::info!(item_id = %item_id, sent, "déclenchement de média du board demandé");
        } else {
            tracing::debug!(
                shortcut = %shortcut.into_string(),
                "raccourci global sans action associée"
            );
        }
        return;
    }
    let enabled = app.state::<AppState>().config_snapshot().enabled;
    if !enabled {
        tracing::debug!("raccourci de rechargement ignoré : overlay désactivé");
        return;
    }
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        if let Err(error) = window.eval("window.location.reload()") {
            tracing::warn!(%error, "échec du rechargement de l'overlay");
        }
    }
}

/// Restaure les bindings persistés au démarrage (mode invité exclus).
pub fn restore_bindings(app: &AppHandle<Wry>) {
    register_manual_reload(app);
    reapply_for_session(app);
}

/// Réapplique les bindings du board média selon la session serveur courante
/// (`welcome`/`settings`/purge d'appairage peuvent changer `guestMode`).
pub fn reapply_for_session(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    let config = state.config_snapshot();
    let bindings = if config.guest_mode {
        MediaBindings::default()
    } else {
        config.media_bindings
    };
    let result = apply_media_bindings(app, bindings, false, false);
    tracing::info!(
        applied = result.applied_bindings.len(),
        failed = result.failed_accelerators.len(),
        "raccourcis du board média réappliqués"
    );
}

/// Item du board média associé à un raccourci global pressé, s'il existe.
fn board_item_for_shortcut(app: &AppHandle<Wry>, shortcut: &Shortcut) -> Option<String> {
    let state = app.state::<AppState>();
    let config = state.config_snapshot();
    if !config.enabled || config.guest_mode {
        return None;
    }
    for (accelerator, item_id) in config.media_bindings.iter() {
        if let Ok(parsed) = accelerator.parse::<Shortcut>() {
            if parsed.id() == shortcut.id() {
                return Some(item_id.clone());
            }
        }
    }
    None
}

/// Désenregistre tous les raccourcis (utilisé à la fermeture et dans les tests).
pub fn unregister_all(app: &AppHandle<Wry>) {
    if let Err(error) = app.global_shortcut().unregister_all() {
        tracing::warn!(%error, "échec de désenregistrement global des raccourcis");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_is_ok_only_without_failures() {
        assert!(MediaBindingsApplyResult::success(MediaBindings::default(), Vec::new()).ok);
        assert!(
            !MediaBindingsApplyResult::success(
                MediaBindings::default(),
                vec!["Ctrl+1".to_string()]
            )
            .ok
        );
    }
}
