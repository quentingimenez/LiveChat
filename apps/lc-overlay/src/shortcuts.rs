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

use crate::config::{normalize_media_bindings, MediaBindings, ShortcutSettings};
use crate::net::ws::{self, NetCommand};
use crate::state::AppState;
use crate::windows::OVERLAY_LABEL;

/// Raccourci de secours pour recharger l'overlay (valeur legacy à confirmer).
pub const MANUAL_RELOAD_ACCELERATOR: &str = "CommandOrControl+Shift+Alt+R";

/// Invalid global shortcut configuration, mapped to a `settings_set` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutError {
    pub field: String,
    pub message: String,
}

impl ShortcutError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

fn parse_accelerator(accelerator: &str) -> Option<Shortcut> {
    accelerator.trim().parse::<Shortcut>().ok()
}

/// Compares accelerators by their parsed shortcut identity, so aliases such as
/// `Ctrl` and `Control` are treated as the same key combination.
pub fn accelerators_match(left: &str, right: &str) -> bool {
    match (parse_accelerator(left), parse_accelerator(right)) {
        (Some(left), Some(right)) => left.id() == right.id(),
        _ => left.trim().eq_ignore_ascii_case(right.trim()),
    }
}

fn shortcut_matches(configured: Option<&str>, shortcut: &Shortcut) -> bool {
    configured.is_some_and(|accelerator| {
        parse_accelerator(accelerator).is_some_and(|parsed| parsed.id() == shortcut.id())
    })
}

/// Validates the persisted stop / mute-all shortcuts against the media
/// bindings before anything is registered or saved.
pub fn validate_shortcut_settings(
    shortcuts: &ShortcutSettings,
    media_bindings: &MediaBindings,
) -> Result<(), ShortcutError> {
    let entries = [
        ("shortcuts.stop", shortcuts.stop.as_deref()),
        ("shortcuts.muteAll", shortcuts.mute_all.as_deref()),
    ];
    for (field, accelerator) in entries {
        let Some(accelerator) = accelerator else {
            continue;
        };
        if parse_accelerator(accelerator).is_none() {
            return Err(ShortcutError::new(
                field,
                "raccourci invalide ou non pris en charge",
            ));
        }
        if accelerators_match(accelerator, MANUAL_RELOAD_ACCELERATOR) {
            return Err(ShortcutError::new(
                field,
                "raccourci réservé au rechargement de l'overlay",
            ));
        }
        if media_bindings
            .keys()
            .any(|binding| accelerators_match(accelerator, binding))
        {
            return Err(ShortcutError::new(
                field,
                "raccourci déjà utilisé par un média du board",
            ));
        }
    }
    if let (Some(stop), Some(mute_all)) = (shortcuts.stop.as_deref(), shortcuts.mute_all.as_deref())
    {
        if accelerators_match(stop, mute_all) {
            return Err(ShortcutError::new(
                "shortcuts.muteAll",
                "raccourci déjà utilisé par l'arrêt de lecture",
            ));
        }
    }
    Ok(())
}

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
    let config = state.config_snapshot();
    let guest_mode = config.guest_mode;
    let configured_shortcuts = config.shortcuts;
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
        let reserved = configured_shortcuts
            .stop
            .as_deref()
            .is_some_and(|stop| accelerators_match(accelerator, stop))
            || configured_shortcuts
                .mute_all
                .as_deref()
                .is_some_and(|mute_all| accelerators_match(accelerator, mute_all));
        if reserved {
            failed.push(accelerator.clone());
        } else if register_one(app, accelerator) {
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
    let config = app.state::<AppState>().config_snapshot();
    if shortcut_matches(config.shortcuts.mute_all.as_deref(), shortcut) {
        crate::tray::toggle_general_mute(app);
        crate::tray::refresh_tray(app);
        return;
    }
    if shortcut_matches(config.shortcuts.stop.as_deref(), shortcut) {
        let sent = crate::commands::stop_active_media(app);
        tracing::info!(sent, "arrêt de lecture demandé par le raccourci global");
        return;
    }
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

fn unregister_configured_shortcuts(app: &AppHandle<Wry>, shortcuts: &ShortcutSettings) {
    for accelerator in [shortcuts.stop.as_deref(), shortcuts.mute_all.as_deref()]
        .into_iter()
        .flatten()
    {
        unregister_one(app, accelerator);
    }
}

fn register_configured_shortcut(
    app: &AppHandle<Wry>,
    field: &str,
    accelerator: &str,
) -> Result<(), ShortcutError> {
    if register_one(app, accelerator) {
        Ok(())
    } else {
        Err(ShortcutError::new(
            field,
            "le système a refusé d'enregistrer ce raccourci",
        ))
    }
}

fn restore_previous_shortcuts(app: &AppHandle<Wry>, previous: &ShortcutSettings) {
    for (field, accelerator) in [
        ("shortcuts.stop", previous.stop.as_deref()),
        ("shortcuts.muteAll", previous.mute_all.as_deref()),
    ] {
        if let Some(accelerator) = accelerator {
            if let Err(error) = register_configured_shortcut(app, field, accelerator) {
                tracing::warn!(
                    field = error.field,
                    message = error.message,
                    "restauration d'un raccourci global impossible"
                );
            }
        }
    }
}

/// Replaces the configured stop / mute-all registrations transactionally.
///
/// The previous shortcuts are unregistered first so shortcuts can be moved
/// between the two actions. If the OS refuses a new accelerator, every new
/// registration is removed and the previous ones are restored.
pub fn replace_configured_shortcuts(
    app: &AppHandle<Wry>,
    previous: &ShortcutSettings,
    next: &ShortcutSettings,
) -> Result<(), ShortcutError> {
    if previous == next {
        return Ok(());
    }
    unregister_configured_shortcuts(app, previous);
    let registration = next
        .stop
        .as_deref()
        .map(|accelerator| register_configured_shortcut(app, "shortcuts.stop", accelerator))
        .unwrap_or(Ok(()))
        .and_then(|()| {
            next.mute_all
                .as_deref()
                .map(|accelerator| {
                    register_configured_shortcut(app, "shortcuts.muteAll", accelerator)
                })
                .unwrap_or(Ok(()))
        });
    if let Err(error) = registration {
        unregister_configured_shortcuts(app, next);
        restore_previous_shortcuts(app, previous);
        return Err(error);
    }
    Ok(())
}

/// Registers the persisted global shortcuts at startup.
pub fn register_configured_shortcuts(app: &AppHandle<Wry>) {
    let shortcuts = app.state::<AppState>().config_snapshot().shortcuts;
    for (field, accelerator) in [
        ("shortcuts.stop", shortcuts.stop.as_deref()),
        ("shortcuts.muteAll", shortcuts.mute_all.as_deref()),
    ] {
        if let Some(accelerator) = accelerator {
            if let Err(error) = register_configured_shortcut(app, field, accelerator) {
                tracing::warn!(
                    field = error.field,
                    accelerator,
                    message = error.message,
                    "raccourci global non enregistré au démarrage"
                );
            }
        }
    }
}

/// Restaure les bindings persistés au démarrage (mode invité exclus).
pub fn restore_bindings(app: &AppHandle<Wry>) {
    register_manual_reload(app);
    reapply_for_session(app);
    register_configured_shortcuts(app);
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
