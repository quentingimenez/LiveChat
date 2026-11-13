//! Shell notification icon and context menu.
//!
//! The tray menu is deliberately minimal: a status header, access to the board
//! and its settings, the global mute, one checkbox per paired guild and quit.
//! Device settings (autostart, text, display, volume, updates) live in
//! [`crate::device_settings`] for the future Settings page.
//!
//! Menu decisions are separated from rendering: [`menu_model`] produces a pure,
//! testable representation and `build_menu` translates it into Tauri items.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, Ordering};

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, TrayIcon, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::config::OverlayConfig;
use crate::device_settings;
use crate::events;
use crate::net::ws::ConnectionStatus;
use crate::state::AppState;
use crate::windows;

pub const TRAY_ID: &str = "main-tray";

const ID_HEADER: &str = "tray.header";
const ID_OPEN_BOARD: &str = "tray.open_board";
const ID_SETTINGS: &str = "tray.settings";
const ID_GENERAL_MUTE: &str = "tray.general_mute";
const ID_GUILD_PREFIX: &str = "tray.guild.";
const ID_QUIT: &str = "tray.quit";
const GUILD_LABEL_MAX_CHARS: usize = 48;

/// A menu entry, independent from Tauri so the model is testable without a
/// runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MenuEntry {
    Header(String),
    Separator,
    Action {
        id: String,
        label: String,
    },
    Check {
        id: String,
        label: String,
        checked: bool,
        enabled: bool,
    },
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}

/// Label of a guild entry: name (or identifier) followed by the status when the
/// guild should be live but is not connected.
///
/// A muted guild (or every guild during the general mute) is disconnected on
/// purpose: its unchecked box already says so, so no status is appended.
fn guild_entry_label(
    guild_name: Option<&str>,
    guild_id: &str,
    status: ConnectionStatus,
    expects_connection: bool,
) -> String {
    let name = match guild_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_string(),
        None => format!("Guilde {guild_id}"),
    };
    let label = if !expects_connection || status == ConnectionStatus::Connected {
        name
    } else {
        format!("{name} — {}", status.label())
    };
    truncate(&label, GUILD_LABEL_MAX_CHARS)
}

/// Tray icon matching the taskbar theme: the logo's framing corners must
/// contrast with the taskbar (docs/brand/README.md).
fn tray_icon(light_taskbar: bool) -> Image<'static> {
    if light_taskbar {
        tauri::include_image!("icons/tray-light.png")
    } else {
        tauri::include_image!("icons/tray-dark.png")
    }
}

fn taskbar_is_light() -> bool {
    #[cfg(windows)]
    {
        crate::win32::taskbar_uses_light_theme()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Last taskbar theme applied to the icon: 0 = none yet, 1 = dark, 2 = light.
static TRAY_ICON_THEME: AtomicU8 = AtomicU8::new(0);

/// Swaps the tray icon when the taskbar theme changed since the last call.
fn sync_tray_icon(tray: &TrayIcon<Wry>) {
    let light = taskbar_is_light();
    let theme = if light { 2 } else { 1 };
    if TRAY_ICON_THEME.swap(theme, Ordering::Relaxed) == theme {
        return;
    }
    if let Err(error) = tray.set_icon(Some(tray_icon(light))) {
        tracing::debug!(%error, "échec de mise à jour de l'icône tray");
    }
}

fn connection_statuses(state: &AppState) -> BTreeMap<String, ConnectionStatus> {
    state
        .net_snapshots()
        .into_iter()
        .map(|(guild_id, snapshot)| (guild_id, snapshot.status))
        .collect()
}

/// "LiveChat — N/M guildes connectées", the menu header.
fn connection_summary(
    config: &OverlayConfig,
    statuses: &BTreeMap<String, ConnectionStatus>,
) -> String {
    let paired = config.memberships.len();
    let connected = config
        .memberships
        .iter()
        .filter(|membership| {
            statuses.get(&membership.guild_id).copied() == Some(ConnectionStatus::Connected)
        })
        .count();
    if paired == 0 {
        "LiveChat — aucune guilde".to_string()
    } else {
        format!("LiveChat — {connected}/{paired} guildes connectées")
    }
}

/// Hover text of the tray icon: the menu header, or the general mute, which
/// is what matters most when it is on.
fn tooltip_text(config: &OverlayConfig, statuses: &BTreeMap<String, ConnectionStatus>) -> String {
    if config.enabled {
        connection_summary(config, statuses)
    } else {
        "LiveChat — sourdine générale".to_string()
    }
}

/// Pure menu model, in the exact expected order.
fn menu_model(
    config: &OverlayConfig,
    statuses: &BTreeMap<String, ConnectionStatus>,
) -> Vec<MenuEntry> {
    let mut entries = vec![
        MenuEntry::Header(connection_summary(config, statuses)),
        MenuEntry::Separator,
        MenuEntry::Action {
            id: ID_OPEN_BOARD.to_string(),
            label: "Ouvrir la board".to_string(),
        },
        MenuEntry::Action {
            id: ID_SETTINGS.to_string(),
            label: "Paramètres…".to_string(),
        },
        MenuEntry::Separator,
        MenuEntry::Check {
            id: ID_GENERAL_MUTE.to_string(),
            label: "Sourdine générale".to_string(),
            checked: !config.enabled,
            enabled: true,
        },
    ];

    for membership in &config.memberships {
        let status = statuses
            .get(&membership.guild_id)
            .copied()
            .unwrap_or(ConnectionStatus::Disabled);
        entries.push(MenuEntry::Check {
            id: format!("{ID_GUILD_PREFIX}{}", membership.guild_id),
            label: guild_entry_label(
                membership.guild_name.as_deref(),
                &membership.guild_id,
                status,
                config.enabled && !membership.muted,
            ),
            checked: !membership.muted,
            enabled: config.enabled,
        });
    }

    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::Action {
        id: ID_QUIT.to_string(),
        label: "Quitter".to_string(),
    });
    entries
}

fn tray_tooltip(state: &AppState) -> String {
    tooltip_text(&state.config_snapshot(), &connection_statuses(state))
}

/// Builds the menu from the current state (config + connections).
pub fn build_menu(app: &AppHandle<Wry>, state: &AppState) -> tauri::Result<Menu<Wry>> {
    let config = state.config_snapshot();
    let statuses = connection_statuses(state);
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();
    for entry in menu_model(&config, &statuses) {
        let item: Box<dyn IsMenuItem<Wry>> = match entry {
            MenuEntry::Header(label) => Box::new(MenuItem::with_id(
                app,
                ID_HEADER,
                label,
                false,
                None::<&str>,
            )?),
            MenuEntry::Separator => Box::new(PredefinedMenuItem::separator(app)?),
            MenuEntry::Action { id, label } => {
                Box::new(MenuItem::with_id(app, id, label, true, None::<&str>)?)
            }
            MenuEntry::Check {
                id,
                label,
                checked,
                enabled,
            } => Box::new(CheckMenuItem::with_id(
                app,
                id,
                label,
                enabled,
                checked,
                None::<&str>,
            )?),
        };
        items.push(item);
    }
    let refs = items
        .iter()
        .map(|item| item.as_ref())
        .collect::<Vec<&dyn IsMenuItem<Wry>>>();
    Menu::with_items(app, &refs)
}

/// Crée l'icône dans la zone de notification (une seule fois au démarrage).
///
/// L'état applicatif doit déjà être géré (`app.manage`) au moment de l'appel.
pub fn create_tray(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let state = app.state::<AppState>();
    let menu = build_menu(app, &state)?;

    let mut builder = tauri::tray::TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(tray_tooltip(&state))
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                windows::show_board(tray.app_handle());
            }
        });
    let light = taskbar_is_light();
    TRAY_ICON_THEME.store(if light { 2 } else { 1 }, Ordering::Relaxed);
    builder = builder.icon(tray_icon(light));
    let tray = builder.build(app)?;
    tracing::info!(
        id = TRAY_ID,
        "icône et menu de la zone de notification créés"
    );
    drop(tray);
    Ok(())
}

/// Reconstruit le menu (après changement de config ou d'état de connexion).
pub fn refresh_tray(app: &AppHandle<Wry>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        tracing::debug!("tray introuvable au moment du rafraîchissement");
        return;
    };
    let state = app.state::<AppState>();
    let menu = match build_menu(app, &state) {
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
    sync_tray_icon(&tray);
    let net = state.net_snapshot();
    tracing::debug!(status = %net.status.label(), "tray rafraîchi");
}

/// Applies the connection/window effect of general mute, exactly like the tray.
pub(crate) fn apply_general_mute_effect(app: &AppHandle<Wry>, enabled: bool) {
    if enabled
        && app
            .state::<AppState>()
            .config_snapshot()
            .has_pairing_config()
    {
        windows::show_overlay(app);
    } else {
        // AUD-A6-006 : désactivation immédiate, lecture interrompue et fenêtre
        // masquée (aucun événement bufferisé ne sera rejoué).
        events::stop_overlay_now(app, "disabled");
        windows::hide_overlay(app);
    }
    crate::net::ws::request_reconnect(app);
}

/// Applies the global mute: disables the overlay and stops the current playback
/// (the same logic as the former `toggle_enabled`, inverted).
pub(crate) fn toggle_general_mute(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    let updated = state.update_config(|config| config.enabled = !config.enabled);
    match updated {
        Ok(config) => {
            tracing::info!(
                enabled = config.enabled,
                "sourdine générale modifiée depuis le tray"
            );
            apply_general_mute_effect(app, config.enabled);
            device_settings::publish_settings(app);
        }
        Err(error) => tracing::warn!(%error, "échec de sauvegarde de `enabled`"),
    }
    if let Err(error) = events::emit_guilds_changed(app) {
        tracing::debug!(%error, "échec d'émission de l'état des guildes");
    }
}

/// Toggles a guild's mute from the tray.
fn toggle_guild_mute(app: &AppHandle<Wry>, guild_id: &str) {
    let state = app.state::<AppState>();
    let Some(membership) = state.config_snapshot().membership(guild_id).cloned() else {
        tracing::warn!(guild_id, "guilde inconnue pour la sourdine du tray");
        refresh_tray(app);
        return;
    };
    match crate::commands::set_guild_muted(app, guild_id, !membership.muted) {
        // `publish_guilds` already rebuilt the menu.
        Ok(true) => return,
        Ok(false) => tracing::warn!(guild_id, "guilde inconnue pour la sourdine du tray"),
        Err(error) => tracing::warn!(%error, guild_id, "échec de la sourdine de guilde"),
    }
    refresh_tray(app);
}

/// Opens the board on its Settings view, remembering the request in case the
/// webview is not loaded yet.
fn open_settings(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    state.set_pending_board_view("settings");
    windows::show_board(app);
    if let Err(error) = events::emit_board_navigate(app, "settings") {
        tracing::debug!(%error, "échec d'émission de overlay://navigate");
    }
}

/// Gère les entrées du menu tray.
pub fn handle_menu_event(app: &AppHandle<Wry>, id: &str) {
    match id {
        ID_OPEN_BOARD => windows::show_board(app),
        ID_SETTINGS => open_settings(app),
        ID_GENERAL_MUTE => toggle_general_mute(app),
        ID_QUIT => {
            app.exit(0);
            return;
        }
        other if other.starts_with(ID_GUILD_PREFIX) => {
            toggle_guild_mute(app, other.trim_start_matches(ID_GUILD_PREFIX));
            return;
        }
        other => tracing::debug!(id = other, "entrée de menu tray ignorée"),
    }
    refresh_tray(app);
}

#[cfg(test)]
mod tests {
    use super::{
        guild_entry_label, menu_model, tooltip_text, MenuEntry, ID_GENERAL_MUTE, ID_OPEN_BOARD,
        ID_QUIT, ID_SETTINGS,
    };
    use crate::config::{Membership, OverlayConfig};
    use crate::net::ws::ConnectionStatus;
    use std::collections::BTreeMap;

    fn membership(guild_id: &str, guild_name: Option<&str>, muted: bool) -> Membership {
        Membership {
            server_url: "https://example.test".to_string(),
            guild_id: guild_id.to_string(),
            guild_name: guild_name.map(ToString::to_string),
            client_token: format!("token-{guild_id}"),
            muted,
            ..Membership::default()
        }
    }

    fn config_with(memberships: Vec<Membership>, enabled: bool) -> OverlayConfig {
        let mut config = OverlayConfig::default();
        config.enabled = enabled;
        config.memberships = memberships;
        config
    }

    fn statuses(entries: &[(&str, ConnectionStatus)]) -> BTreeMap<String, ConnectionStatus> {
        entries
            .iter()
            .map(|(guild_id, status)| ((*guild_id).to_string(), *status))
            .collect()
    }

    fn action(id: &str, label: &str) -> MenuEntry {
        MenuEntry::Action {
            id: id.to_string(),
            label: label.to_string(),
        }
    }

    fn check(id: &str, label: &str, checked: bool, enabled: bool) -> MenuEntry {
        MenuEntry::Check {
            id: id.to_string(),
            label: label.to_string(),
            checked,
            enabled,
        }
    }

    #[test]
    fn menu_order_matches_the_new_tray() {
        let config = config_with(
            vec![
                membership("111", Some("Les Sardines"), false),
                membership("222", Some("Les Orques"), false),
            ],
            true,
        );
        let statuses = statuses(&[
            ("111", ConnectionStatus::Connected),
            ("222", ConnectionStatus::Revoked),
        ]);

        assert_eq!(
            menu_model(&config, &statuses),
            vec![
                MenuEntry::Header("LiveChat — 1/2 guildes connectées".to_string()),
                MenuEntry::Separator,
                action(ID_OPEN_BOARD, "Ouvrir la board"),
                action(ID_SETTINGS, "Paramètres…"),
                MenuEntry::Separator,
                check(ID_GENERAL_MUTE, "Sourdine générale", false, true),
                check("tray.guild.111", "Les Sardines", true, true),
                check(
                    "tray.guild.222",
                    "Les Orques — appareil révoqué",
                    true,
                    true
                ),
                MenuEntry::Separator,
                action(ID_QUIT, "Quitter"),
            ]
        );
    }

    #[test]
    fn header_reports_no_guild() {
        let model = menu_model(&OverlayConfig::default(), &BTreeMap::new());
        assert_eq!(
            model.first(),
            Some(&MenuEntry::Header("LiveChat — aucune guilde".to_string()))
        );
    }

    #[test]
    fn general_mute_is_checked_and_disables_guild_checks() {
        let config = config_with(vec![membership("111", Some("Alpha"), false)], false);
        let model = menu_model(&config, &statuses(&[("111", ConnectionStatus::Connected)]));

        assert!(model.contains(&check(ID_GENERAL_MUTE, "Sourdine générale", true, true)));
        assert!(model.contains(&check("tray.guild.111", "Alpha", true, false)));
    }

    #[test]
    fn guild_check_reflects_mute_and_status_suffix() {
        let config = config_with(
            vec![
                membership("111", Some("Alpha"), false),
                membership("222", Some("Bêta"), true),
            ],
            true,
        );
        let model = menu_model(
            &config,
            &statuses(&[
                ("111", ConnectionStatus::Connected),
                ("222", ConnectionStatus::Connecting),
            ]),
        );

        assert!(model.contains(&check("tray.guild.111", "Alpha", true, true)));
        // Muted on purpose: the unchecked box says it, no status suffix.
        assert!(model.contains(&check("tray.guild.222", "Bêta", false, true)));
    }

    #[test]
    fn live_guild_that_is_not_connected_shows_its_status() {
        let config = config_with(vec![membership("111", Some("Alpha"), false)], true);
        let model = menu_model(&config, &statuses(&[("111", ConnectionStatus::Connecting)]));
        assert!(model.contains(&check("tray.guild.111", "Alpha — connexion…", true, true)));
    }

    #[test]
    fn general_mute_hides_the_guild_statuses() {
        let config = config_with(vec![membership("111", Some("Alpha"), false)], false);
        let model = menu_model(&config, &statuses(&[("111", ConnectionStatus::Disabled)]));
        assert!(model.contains(&check("tray.guild.111", "Alpha", true, false)));
    }

    #[test]
    fn tooltip_repeats_the_header_unless_general_mute_is_on() {
        let guilds = vec![
            membership("111", Some("Alpha"), false),
            membership("222", Some("Beta"), true),
        ];
        let live = statuses(&[("111", ConnectionStatus::Connected)]);
        assert_eq!(
            tooltip_text(&config_with(guilds.clone(), true), &live),
            "LiveChat — 1/2 guildes connectées"
        );
        assert_eq!(
            tooltip_text(&config_with(guilds, false), &live),
            "LiveChat — sourdine générale"
        );
        assert_eq!(
            tooltip_text(&config_with(Vec::new(), true), &statuses(&[])),
            "LiveChat — aucune guilde"
        );
    }

    #[test]
    fn guild_label_falls_back_to_id_and_truncates() {
        assert_eq!(
            guild_entry_label(None, "111", ConnectionStatus::Disabled, true),
            "Guilde 111 — désactivé"
        );
        assert_eq!(
            guild_entry_label(None, "111", ConnectionStatus::Disabled, false),
            "Guilde 111"
        );
        assert_eq!(
            guild_entry_label(Some("  Alpha  "), "111", ConnectionStatus::Connected, true),
            "Alpha"
        );

        let long_name = "a".repeat(60);
        let label = guild_entry_label(Some(&long_name), "111", ConnectionStatus::Connected, true);
        assert_eq!(label, format!("{}…", "a".repeat(48)));
        assert_eq!(label.chars().count(), 49);
    }
}
