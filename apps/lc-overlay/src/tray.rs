//! Icône de la zone de notification et menu contextuel du shell.
//!
//! Équivalent simplifié du `tray-menu-service` Electron : accès rapide à
//! l'overlay, au board et à l'appairage, interrupteurs de configuration,
//! sélection de l'écran, volume, vérification des mises à jour et quitter.

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};

use crate::display::{self, DisplayInfo};
use crate::events;
use crate::state::AppState;
use crate::windows;

pub const TRAY_ID: &str = "main-tray";

const ID_SHOW_OVERLAY: &str = "tray.show_overlay";
const ID_OPEN_BOARD: &str = "tray.open_board";
const ID_OPEN_PAIRING: &str = "tray.open_pairing";
const ID_TOGGLE_ENABLED: &str = "tray.toggle_enabled";
const ID_TOGGLE_AUTOSTART: &str = "tray.toggle_autostart";
const ID_TOGGLE_SHOW_TEXT: &str = "tray.toggle_show_text";
const ID_RECONNECT: &str = "tray.reconnect";
const ID_DISPLAY_PREFIX: &str = "tray.display.";
const ID_VOLUME_UP: &str = "tray.volume.up";
const ID_VOLUME_DOWN: &str = "tray.volume.down";
const ID_VOLUME_RESET: &str = "tray.volume.reset";
const ID_CHECK_UPDATES: &str = "tray.check_updates";
const ID_QUIT: &str = "tray.quit";

fn display_label(selected: Option<&DisplayInfo>, displays: &[DisplayInfo]) -> String {
    match selected {
        Some(display) => format!(
            "Écran sélectionné : {}",
            display::format_display_menu_label(display)
        ),
        None if displays.is_empty() => "Écran sélectionné : aucun écran détecté".to_string(),
        None => "Écran sélectionné : écran principal".to_string(),
    }
}

fn volume_label(volume: f64) -> String {
    format!("Volume audio ({:.0} %)", (volume * 100.0).round())
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}

fn connection_detail(state: &AppState) -> String {
    let config = state.config_snapshot();
    if !config.enabled {
        return "Overlay désactivé (aucune lecture)".to_string();
    }
    if !config.has_pairing_config() {
        return "Appairage requis".to_string();
    }
    let net = state.net_snapshot();
    if let Some(error) = net.last_error.as_deref().filter(|error| !error.is_empty()) {
        return format!("Dernière erreur : {}", truncate(error, 70));
    }
    if let Some(reason) = net.reason.as_deref().filter(|reason| !reason.is_empty()) {
        return format!("Détail : {}", truncate(reason, 70));
    }
    if !net.peers.is_empty() {
        return format!("Pairs connectés : {}", net.peers.len());
    }
    if let Some(version) = net.server_version.as_deref() {
        return format!("Serveur : v{version}");
    }
    if net.guild_id.is_some() {
        return "Appareil appairé".to_string();
    }
    if config.has_pairing_config() {
        // Appareil configuré mais serveur pas encore joint (welcome non reçu).
        return "Appareil appairé (en attente du serveur)".to_string();
    }
    "Aucun appareil appairé".to_string()
}

/// L'entrée « Afficher l'overlay » n'a de sens que si l'overlay est activé et
/// appairé (AUD-A6-006) : sinon la fenêtre resterait masquée.
fn show_overlay_item_enabled(config: &crate::config::OverlayConfig) -> bool {
    config.enabled && config.has_pairing_config()
}

fn tray_tooltip(state: &AppState) -> String {
    let config = state.config_snapshot();
    let net = state.net_snapshot();
    let mut tooltip = if config.enabled {
        format!("LiveChat — overlay ({})", net.status.label())
    } else {
        "LiveChat — overlay (désactivé)".to_string()
    };
    if let Some(version) = net.server_version.as_deref() {
        tooltip.push_str(&format!(" — serveur v{version}"));
    }
    tooltip
}

/// Construit le menu à partir de l'état courant (config + écrans + autostart).
pub fn build_menu(
    app: &AppHandle<Wry>,
    state: &AppState,
    displays: &[DisplayInfo],
    selected_index: Option<usize>,
    autostart_enabled: bool,
) -> tauri::Result<Menu<Wry>> {
    let config = state.config_snapshot();
    let selected_display = selected_index.and_then(|index| displays.get(index));
    let net = state.net_snapshot();

    let status_connection = MenuItem::with_id(
        app,
        "tray.status.connection",
        format!("Connexion : {}", net.status.label()),
        false,
        None::<&str>,
    )?;
    let status_detail = MenuItem::with_id(
        app,
        "tray.status.detail",
        connection_detail(state),
        false,
        None::<&str>,
    )?;
    let status_version = MenuItem::with_id(
        app,
        "tray.status.version",
        format!("Overlay v{}", env!("CARGO_PKG_VERSION")),
        false,
        None::<&str>,
    )?;
    let reconnect = MenuItem::with_id(
        app,
        ID_RECONNECT,
        "Reconnecter",
        config.enabled && config.has_pairing_config(),
        None::<&str>,
    )?;
    let separator_status = PredefinedMenuItem::separator(app)?;

    let show_overlay = MenuItem::with_id(
        app,
        ID_SHOW_OVERLAY,
        "Afficher l'overlay",
        show_overlay_item_enabled(&config),
        None::<&str>,
    )?;
    let open_board = MenuItem::with_id(
        app,
        ID_OPEN_BOARD,
        "Ouvrir Meme Board",
        config.enabled && config.has_pairing_config(),
        None::<&str>,
    )?;
    let open_pairing = MenuItem::with_id(
        app,
        ID_OPEN_PAIRING,
        "Appairer l'overlay",
        true,
        None::<&str>,
    )?;
    let separator_top = PredefinedMenuItem::separator(app)?;

    let toggle_enabled = CheckMenuItem::with_id(
        app,
        ID_TOGGLE_ENABLED,
        "Overlay activé",
        true,
        config.enabled,
        None::<&str>,
    )?;
    let toggle_autostart = CheckMenuItem::with_id(
        app,
        ID_TOGGLE_AUTOSTART,
        "Lancer au démarrage",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let toggle_show_text = CheckMenuItem::with_id(
        app,
        ID_TOGGLE_SHOW_TEXT,
        "Afficher le texte",
        true,
        config.show_text,
        None::<&str>,
    )?;
    let separator_config = PredefinedMenuItem::separator(app)?;

    let display_header = MenuItem::with_id(
        app,
        "tray.display.header",
        display_label(selected_display, displays),
        false,
        None::<&str>,
    )?;
    let mut display_items = Vec::with_capacity(displays.len());
    for display in displays {
        display_items.push(CheckMenuItem::with_id(
            app,
            format!("{ID_DISPLAY_PREFIX}{}", display.index),
            display::format_display_menu_label(display),
            true,
            Some(display.index) == selected_index,
            None::<&str>,
        )?);
    }
    let separator_displays = PredefinedMenuItem::separator(app)?;

    let volume_up = MenuItem::with_id(app, ID_VOLUME_UP, "Volume +", true, None::<&str>)?;
    let volume_down = MenuItem::with_id(app, ID_VOLUME_DOWN, "Volume -", true, None::<&str>)?;
    let volume_reset = MenuItem::with_id(app, ID_VOLUME_RESET, "Volume 100 %", true, None::<&str>)?;
    let volume = Submenu::with_id_and_items(
        app,
        "tray.volume",
        volume_label(config.volume),
        true,
        &[&volume_up, &volume_down, &volume_reset],
    )?;
    let separator_media = PredefinedMenuItem::separator(app)?;

    let (update_label, update_enabled) =
        update_menu_entry(crate::updater::UPDATER_ENABLED_FOR_RELEASE);
    let check_updates = MenuItem::with_id(
        app,
        ID_CHECK_UPDATES,
        update_label,
        update_enabled,
        None::<&str>,
    )?;
    let separator_quit = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quitter", true, None::<&str>)?;

    let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![
        &status_connection,
        &status_detail,
        &status_version,
        &reconnect,
        &separator_status,
        &show_overlay,
        &open_board,
        &open_pairing,
        &separator_top,
        &toggle_enabled,
        &toggle_autostart,
        &toggle_show_text,
        &separator_config,
        &display_header,
    ];
    for item in &display_items {
        items.push(item);
    }
    items.push(&separator_displays);
    items.push(&volume);
    items.push(&separator_media);
    items.push(&check_updates);
    items.push(&separator_quit);
    items.push(&quit);

    Menu::with_items(app, &items)
}

fn autostart_enabled(app: &AppHandle<Wry>) -> bool {
    use tauri_plugin_autostart::ManagerExt as AutostartExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Crée l'icône dans la zone de notification (une seule fois au démarrage).
///
/// L'état applicatif doit déjà être géré (`app.manage`) au moment de l'appel.
pub fn create_tray(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let state = app.state::<AppState>();
    let (displays, _primary) = windows::collect_displays(app);
    let selected = windows::selected_display(app);
    let selected_index = selected.as_ref().map(|display| display.index);
    let menu = build_menu(
        app,
        &state,
        &displays,
        selected_index,
        autostart_enabled(app),
    )?;

    let mut builder = tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(tray_tooltip(&state))
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    let tray = builder.build(app)?;
    tracing::info!(
        id = TRAY_ID,
        "icône et menu de la zone de notification créés"
    );
    drop(tray);
    Ok(())
}

/// Reconstruit le menu (après changement de config, d'écran ou d'autostart).
pub fn refresh_tray(app: &AppHandle<Wry>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        tracing::debug!("tray introuvable au moment du rafraîchissement");
        return;
    };
    let state = app.state::<AppState>();
    let (displays, _primary) = windows::collect_displays(app);
    let selected = windows::selected_display(app);
    let selected_index = selected.as_ref().map(|display| display.index);
    let menu = match build_menu(
        app,
        &state,
        &displays,
        selected_index,
        autostart_enabled(app),
    ) {
        Ok(menu) => menu,
        Err(error) => {
            tracing::warn!(%error, "échec de construction du menu tray");
            return;
        }
    };
    if let Err(error) = tray.set_menu(Some(menu)) {
        tracing::warn!(%error, "échec d'application du menu tray");
    }
    if let Err(error) = tray.set_tooltip(Some(tray_tooltip(&state))) {
        tracing::debug!(%error, "échec de mise à jour du tooltip tray");
    }
    let net = state.net_snapshot();
    tracing::debug!(
        status = %net.status.label(),
        detail = %connection_detail(&state),
        "tray rafraîchi"
    );
}

fn publish_settings(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    if let Err(error) = events::emit_settings(app, &state.view()) {
        tracing::debug!(%error, "échec d'émission overlay://settings");
    }
}

fn toggle_enabled(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    let updated = state.update_config(|config| config.enabled = !config.enabled);
    match updated {
        Ok(config) => {
            tracing::info!(
                enabled = config.enabled,
                "overlay activé/désactivé depuis le tray"
            );
            if config.enabled && config.has_pairing_config() {
                windows::show_overlay(app);
            } else {
                // AUD-A6-006 : désactivation immédiate, lecture interrompue et
                // fenêtre masquée (aucun événement bufferisé ne sera rejoué).
                events::stop_overlay_now(app, "disabled");
                windows::hide_overlay(app);
            }
            publish_settings(app);
            crate::net::ws::request_reconnect(app);
        }
        Err(error) => tracing::warn!(%error, "échec de sauvegarde de `enabled`"),
    }
}

fn toggle_show_text(app: &AppHandle<Wry>) {
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

fn toggle_autostart(app: &AppHandle<Wry>) {
    use tauri_plugin_autostart::ManagerExt as AutostartExt;
    let manager = app.autolaunch();
    let currently_enabled = manager.is_enabled().unwrap_or(false);
    let result = if currently_enabled {
        manager.disable()
    } else {
        manager.enable()
    };
    match result {
        Ok(()) => tracing::info!(
            enabled = !currently_enabled,
            "démarrage automatique modifié"
        ),
        Err(error) => tracing::warn!(%error, "échec de modification du démarrage automatique"),
    }
}

fn select_display(app: &AppHandle<Wry>, index: usize) {
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
        }
        Err(error) => tracing::warn!(%error, "échec de sauvegarde de l'écran sélectionné"),
    }
}

fn adjust_volume(app: &AppHandle<Wry>, delta: f64) {
    let state = app.state::<AppState>();
    let result =
        state.update_config(|config| config.volume = (config.volume + delta).clamp(0.0, 1.0));
    match result {
        Ok(config) => tracing::info!(volume = config.volume, "volume modifié depuis le tray"),
        Err(error) => tracing::warn!(%error, "échec de sauvegarde du volume"),
    }
}

fn reset_volume(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    match state.update_config(|config| config.volume = 1.0) {
        Ok(config) => tracing::info!(volume = config.volume, "volume réinitialisé"),
        Err(error) => tracing::warn!(%error, "échec de sauvegarde du volume"),
    }
}

/// Libellé et état de l'entrée « mises à jour » du menu tray (AUD-V2-O-001).
///
/// Quand l'updater est désactivé pour la release, l'entrée est **grisée** et
/// annonce l'état au lieu de laisser un clic déclencher une requête réseau.
fn update_menu_entry(updater_enabled: bool) -> (&'static str, bool) {
    if updater_enabled {
        ("Vérifier les mises à jour", true)
    } else {
        ("Mises à jour désactivées pour cette version", false)
    }
}

/// Vérifie les mises à jour depuis le tray.
///
/// Le flag `updater::UPDATER_ENABLED_FOR_RELEASE` est consulté **avant** toute
/// tâche asynchrone : désactivé, aucun appel au plugin n'est effectué (donc
/// aucune requête réseau), conformément à AUD-V2-O-001.
fn spawn_update_check(app: &AppHandle<Wry>) {
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

/// Gère les entrées du menu tray.
pub fn handle_menu_event(app: &AppHandle<Wry>, id: &str) {
    match id {
        ID_SHOW_OVERLAY => windows::show_overlay(app),
        ID_OPEN_BOARD => windows::show_board(app),
        ID_OPEN_PAIRING => windows::show_pairing(app),
        ID_TOGGLE_ENABLED => toggle_enabled(app),
        ID_TOGGLE_AUTOSTART => toggle_autostart(app),
        ID_TOGGLE_SHOW_TEXT => toggle_show_text(app),
        ID_VOLUME_UP => adjust_volume(app, 0.1),
        ID_VOLUME_DOWN => adjust_volume(app, -0.1),
        ID_VOLUME_RESET => reset_volume(app),
        ID_RECONNECT => crate::net::ws::request_reconnect(app),
        ID_CHECK_UPDATES => spawn_update_check(app),
        ID_QUIT => app.exit(0),
        other if other.starts_with(ID_DISPLAY_PREFIX) => {
            let index = other.trim_start_matches(ID_DISPLAY_PREFIX).parse::<usize>();
            match index {
                Ok(index) => select_display(app, index),
                Err(_) => tracing::warn!(id = other, "identifiant d'écran tray invalide"),
            }
        }
        other => tracing::debug!(id = other, "entrée de menu tray ignorée"),
    }
    refresh_tray(app);
}

#[cfg(test)]
mod tests {
    use super::{show_overlay_item_enabled, update_menu_entry};
    use crate::config::OverlayConfig;

    fn paired_config() -> OverlayConfig {
        let mut config = OverlayConfig::default();
        config.server_url = Some("https://example.test".to_string());
        config.client_token = Some("token".to_string());
        config.guild_id = Some("guild".to_string());
        config
    }

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

    #[test]
    fn show_overlay_item_requires_enabled_and_paired() {
        let mut config = paired_config();
        assert!(show_overlay_item_enabled(&config));

        config.enabled = false;
        assert!(!show_overlay_item_enabled(&config), "désactivé => grisé");

        let mut unpaired = paired_config();
        unpaired.client_token = None;
        assert!(
            !show_overlay_item_enabled(&unpaired),
            "non appairé => grisé"
        );
    }
}
