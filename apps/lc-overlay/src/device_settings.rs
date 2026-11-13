//! Device settings removed from the tray menu.
//!
//! These operations (autostart, text display, display selection, volume,
//! updates) are no longer exposed in the compact tray menu. They remain
//! available from Rust, with unchanged behaviour, for the future board Settings
//! page.

use tauri::{AppHandle, Manager, Wry};

use crate::display::{self, DisplayInfo};
use crate::events;
use crate::state::AppState;
use crate::windows;

pub fn display_label(selected: Option<&DisplayInfo>, displays: &[DisplayInfo]) -> String {
    match selected {
        Some(display) => format!(
            "Écran sélectionné : {}",
            display::format_display_menu_label(display)
        ),
        None if displays.is_empty() => "Écran sélectionné : aucun écran détecté".to_string(),
        None => "Écran sélectionné : écran principal".to_string(),
    }
}

pub fn volume_label(volume: f64) -> String {
    format!("Volume audio ({:.0} %)", (volume * 100.0).round())
}

pub fn autostart_enabled(app: &AppHandle<Wry>) -> bool {
    use tauri_plugin_autostart::ManagerExt as AutostartExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

pub fn set_autostart(app: &AppHandle<Wry>, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt as AutostartExt;
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|error| error.to_string())?;
    tracing::info!(enabled, "démarrage automatique modifié");
    Ok(())
}

pub fn toggle_autostart(app: &AppHandle<Wry>) {
    let enabled = !autostart_enabled(app);
    if let Err(error) = set_autostart(app, enabled) {
        tracing::warn!(%error, "échec de modification du démarrage automatique");
    }
}

pub(crate) fn publish_settings(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    if let Err(error) = events::emit_settings(app, &state.view()) {
        tracing::debug!(%error, "échec d'émission overlay://settings");
    }
    if let Err(error) = events::emit_device_settings(app) {
        tracing::debug!(%error, "échec d'émission overlay://device-settings");
    }
}

pub fn toggle_show_text(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    let updated = state.update_config(|config| config.show_text = !config.show_text);
    match updated {
        Ok(config) => {
            tracing::info!(show_text = config.show_text, "affichage du texte modifié");
            publish_settings(app);
        }
        Err(error) => tracing::warn!(%error, "échec de sauvegarde de `showText`"),
    }
}

pub fn select_display(app: &AppHandle<Wry>, index: usize) {
    let (displays, _primary) = windows::collect_displays(app);
    let Some(target_display) = displays.get(index).cloned() else {
        tracing::warn!(index, "écran inconnu sélectionné depuis le tray");
        return;
    };
    let state = app.state::<AppState>();
    let selection = display::DisplaySelection::from_display(&target_display);
    let result = state.update_config(|config| {
        config.display_index = selection.display_index;
        config.display_key = selection.display_key.clone();
        config.display_label = selection.display_label.clone();
    });
    match result {
        Ok(_) => {
            tracing::info!(display_key = %target_display.key(), "écran sélectionné depuis le tray");
            let _positioned = windows::position_overlay_on_selected_display(app);
            publish_settings(app);
        }
        Err(error) => tracing::warn!(%error, "échec de sauvegarde de l'écran sélectionné"),
    }
}

pub fn adjust_volume(app: &AppHandle<Wry>, delta: f64) {
    let state = app.state::<AppState>();
    let result =
        state.update_config(|config| config.volume = (config.volume + delta).clamp(0.0, 1.0));
    match result {
        Ok(config) => {
            tracing::info!(volume = config.volume, "volume modifié depuis le tray");
            publish_settings(app);
        }
        Err(error) => tracing::warn!(%error, "échec de sauvegarde du volume"),
    }
}

pub fn reset_volume(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    match state.update_config(|config| config.volume = 1.0) {
        Ok(config) => {
            tracing::info!(volume = config.volume, "volume réinitialisé");
            publish_settings(app);
        }
        Err(error) => tracing::warn!(%error, "échec de sauvegarde du volume"),
    }
}

/// Libellé et état de l'entrée « mises à jour » du menu tray (AUD-V2-O-001).
///
/// Quand l'updater est désactivé pour la release, l'entrée est **grisée** et
/// annonce l'état au lieu de laisser un clic déclencher une requête réseau.
pub fn update_menu_entry(updater_enabled: bool) -> (&'static str, bool) {
    if updater_enabled {
        ("Vérifier les mises à jour", true)
    } else {
        ("Mises à jour désactivées pour cette version", false)
    }
}

/// Vérifie les mises à jour depuis la future page Paramètres.
///
/// Le flag `updater::UPDATER_ENABLED_FOR_RELEASE` est consulté **avant** toute
/// tâche asynchrone : désactivé, aucun appel au plugin n'est effectué (donc
/// aucune requête réseau), conformément à AUD-V2-O-001.
pub fn spawn_update_check(app: &AppHandle<Wry>) {
    if !crate::updater::UPDATER_ENABLED_FOR_RELEASE {
        tracing::info!(
            "vérification des mises à jour ignorée : updater désactivé pour cette version"
        );
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let status = crate::updater::check_for_update(&handle).await;
        match status.state.as_str() {
            "available" => tracing::info!(
                version = status.available_version.as_deref().unwrap_or("?"),
                "mise à jour disponible"
            ),
            "up_to_date" => tracing::info!("aucune mise à jour disponible"),
            "error" => tracing::warn!(
                message = %status.message,
                "échec de la vérification des mises à jour"
            ),
            other => tracing::debug!(state = other, message = %status.message, "updater"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::update_menu_entry;

    #[test]
    fn update_entry_is_disabled_when_updater_is_off() {
        let (label, enabled) = update_menu_entry(false);
        assert!(!enabled, "l'entrée tray doit être grisée (AUD-V2-O-001)");
        assert!(label.contains("désactivées"));
    }

    #[test]
    fn update_entry_is_active_when_updater_is_on() {
        let (label, enabled) = update_menu_entry(true);
        assert!(enabled);
        assert_eq!(label, "Vérifier les mises à jour");
    }
}
