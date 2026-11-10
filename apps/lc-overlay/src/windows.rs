//! Cycle de vie et politiques des trois fenêtres du shell :
//! `overlay` (plein écran transparent, click-through, topmost), `board`
//! (1180×780) et `pairing` (460×580).

use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent, Wry};

use crate::config::OverlayConfig;
use crate::display::{self, DisplayInfo, DisplaySelection};
use crate::state::AppState;
use crate::win32;

pub const OVERLAY_LABEL: &str = "overlay";
pub const BOARD_LABEL: &str = "board";
pub const PAIRING_LABEL: &str = "pairing";

/// Même cadence que le legacy Electron (`setInterval(..., 2000)`).
pub const KEEP_ON_TOP_INTERVAL: Duration = Duration::from_secs(2);

fn window(app: &AppHandle<Wry>, label: &str) -> Option<WebviewWindow<Wry>> {
    app.get_webview_window(label)
}

/// Décision pure d'affichage de l'overlay : il ne doit être visible que si
/// l'utilisateur l'a activé **et** si l'appareil est appairé (AUD-A6-006).
/// Sans appairage, aucun job ne peut arriver ; afficher la fenêtre ne ferait
/// qu'exposer un cadre topmost inutile.
pub fn overlay_should_show(config: &OverlayConfig) -> bool {
    config.enabled && config.has_pairing_config()
}

/// Applique la décision [`overlay_should_show`] à l'état courant.
fn overlay_should_show_now(app: &AppHandle<Wry>) -> bool {
    overlay_should_show(&app.state::<AppState>().config_snapshot())
}

/// Liste les moniteurs visibles avec leur index Tauri et l'index du primaire.
pub fn collect_displays(app: &AppHandle<Wry>) -> (Vec<DisplayInfo>, Option<usize>) {
    let monitors = match app.available_monitors() {
        Ok(monitors) => monitors,
        Err(error) => {
            tracing::warn!(%error, "impossible d'énumérer les écrans");
            return (Vec::new(), None);
        }
    };
    let primary = app.primary_monitor().ok().flatten();
    let primary_geometry = primary.as_ref().map(|monitor| {
        (
            monitor.position().x,
            monitor.position().y,
            monitor.size().width,
            monitor.size().height,
        )
    });
    let displays = monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| {
            DisplayInfo::new(
                index,
                monitor.name().cloned(),
                monitor.position().x,
                monitor.position().y,
                monitor.size().width,
                monitor.size().height,
                monitor.scale_factor(),
            )
        })
        .collect::<Vec<_>>();
    let primary_index = primary_geometry.and_then(|geometry| {
        monitors.iter().position(|monitor| {
            (
                monitor.position().x,
                monitor.position().y,
                monitor.size().width,
                monitor.size().height,
            ) == geometry
        })
    });
    (displays, primary_index)
}

/// Écran cible d'après la configuration persistée.
pub fn selected_display(app: &AppHandle<Wry>) -> Option<DisplayInfo> {
    let (displays, primary_index) = collect_displays(app);
    if displays.is_empty() {
        return None;
    }
    let config = app.state::<AppState>().config_snapshot();
    display::select_display(&config, &displays, primary_index)
        .and_then(|index| displays.into_iter().nth(index))
}

/// Mémoire de sélection à écrire dans la configuration (uniquement si changé).
fn persist_display_selection(app: &AppHandle<Wry>, display: &DisplayInfo, config: &OverlayConfig) {
    let selection = DisplaySelection::from_display(display);
    let already_saved = config.display_index == selection.display_index
        && config.display_key == selection.display_key
        && config.display_label == selection.display_label;
    if already_saved {
        return;
    }
    let state = app.state::<AppState>();
    if let Err(error) = state.update_config(|config| {
        config.display_index = selection.display_index;
        config.display_key = selection.display_key.clone();
        config.display_label = selection.display_label.clone();
    }) {
        tracing::warn!(%error, "échec de persistance de l'écran sélectionné");
    }
}

/// Positionne et dimensionne l'overlay sur l'écran sélectionné.
pub fn position_overlay_on_selected_display(app: &AppHandle<Wry>) -> Option<DisplayInfo> {
    let Some(window) = window(app, OVERLAY_LABEL) else {
        tracing::warn!("fenêtre overlay absente");
        return None;
    };
    let Some(target) = selected_display(app) else {
        tracing::warn!("aucun écran disponible");
        return None;
    };
    let (x, y, width, height) = target.overlay_bounds();
    if let Err(error) = window.set_size(PhysicalSize::new(width, height)) {
        tracing::warn!(%error, "échec set_size overlay");
    }
    if let Err(error) = window.set_position(PhysicalPosition::new(x, y)) {
        tracing::warn!(%error, "échec set_position overlay");
    }
    let config = app.state::<AppState>().config_snapshot();
    persist_display_selection(app, &target, &config);
    tracing::info!(
        display_index = target.index,
        display_key = %target.key(),
        x,
        y,
        width,
        height,
        "overlay positionné sur l'écran sélectionné"
    );
    Some(target)
}

/// Applique les politiques de l'overlay (click-through, topmost, styles Win32)
/// puis l'affiche sans lui donner le focus.
pub fn apply_overlay_flags(app: &AppHandle<Wry>) {
    let Some(window) = window(app, OVERLAY_LABEL) else {
        tracing::warn!("fenêtre overlay absente");
        return;
    };
    let tauri_results = [
        (
            "set_ignore_cursor_events",
            window.set_ignore_cursor_events(true),
        ),
        ("set_focusable", window.set_focusable(false)),
        ("set_always_on_top", window.set_always_on_top(true)),
        ("set_skip_taskbar", window.set_skip_taskbar(true)),
    ];
    for (call, result) in tauri_results {
        if let Err(error) = result {
            tracing::warn!(call, %error, "échec d'application d'une politique de fenêtre");
        }
    }

    match window.hwnd() {
        Ok(hwnd) => {
            let before = win32::ex_style(hwnd);
            let after = win32::ensure_overlay_ex_style(hwnd);
            tracing::info!(
                before = %win32::describe_ex_style(before),
                after = %win32::describe_ex_style(after),
                "styles étendus overlay appliqués"
            );
        }
        Err(error) => tracing::warn!(%error, "hwnd overlay indisponible"),
    }

    if !overlay_should_show_now(app) {
        // AUD-A6-006 : `enabled=false` (ou appareil non appairé) ⇒ la fenêtre
        // reste masquée, aucune lecture, aucun topmost.
        if let Err(error) = window.hide() {
            tracing::warn!(%error, "échec hide overlay (overlay désactivé)");
        }
        tracing::debug!("overlay désactivé ou non appairé : fenêtre masquée");
        return;
    }

    if let Err(error) = window.show() {
        tracing::warn!(%error, "échec show overlay");
    }

    if let Ok(hwnd) = window.hwnd() {
        if let Err(error) = win32::reassert_topmost(hwnd) {
            tracing::warn!(%error, "échec réaffirmation topmost initiale");
        }
    }
}

/// Maintient l'overlay au-dessus : réaffirme styles + `HWND_TOPMOST` toutes les 2 s.
pub fn spawn_topmost_guard(app: AppHandle<Wry>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(KEEP_ON_TOP_INTERVAL);
        let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
            break;
        };
        if !overlay_should_show_now(&app) {
            // Overlay désactivé : aucune réaffirmation topmost ni lecture.
            continue;
        }
        if let Err(error) = window.set_always_on_top(true) {
            tracing::debug!(%error, "set_always_on_top tick en échec");
        }
        match window.hwnd() {
            Ok(hwnd) => {
                let _styles = win32::ensure_overlay_ex_style(hwnd);
                if let Err(error) = win32::reassert_topmost(hwnd) {
                    tracing::debug!(%error, "SetWindowPos topmost tick en échec");
                }
            }
            Err(error) => tracing::debug!(%error, "hwnd overlay perdu (tick)"),
        }
    });
}

fn hide_on_close(app: &AppHandle<Wry>, label: &str) {
    let Some(window) = window(app, label) else {
        return;
    };
    let target = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(error) = target.hide() {
                tracing::warn!(%error, "échec hide sur CloseRequested");
            }
        }
    });
}

/// Les fenêtres du shell ne se détruisent pas à la fermeture : elles se cachent.
pub fn install_window_events(app: &AppHandle<Wry>) {
    hide_on_close(app, BOARD_LABEL);
    hide_on_close(app, PAIRING_LABEL);
    hide_on_close(app, OVERLAY_LABEL);
}

fn reveal(app: &AppHandle<Wry>, label: &str, focus: bool) {
    let Some(window) = window(app, label) else {
        tracing::warn!(label, "fenêtre absente au moment de l'affichage");
        return;
    };
    if window.is_minimized().unwrap_or(false) {
        if let Err(error) = window.unminimize() {
            tracing::warn!(label, %error, "échec unminimize");
        }
    }
    if let Err(error) = window.show() {
        tracing::warn!(label, %error, "échec show");
    }
    if focus {
        if let Err(error) = window.set_focus() {
            tracing::warn!(label, %error, "échec set_focus");
        }
    }
}

pub fn show_overlay(app: &AppHandle<Wry>) {
    if !overlay_should_show_now(app) {
        tracing::info!("affichage overlay ignoré : overlay désactivé ou appareil non appairé");
        hide_overlay(app);
        return;
    }
    reveal(app, OVERLAY_LABEL, false);
    apply_overlay_flags(app);
}

/// Masque l'overlay (utilisé au boot désactivé et lors du toggle tray).
pub fn hide_overlay(app: &AppHandle<Wry>) {
    if let Some(window) = window(app, OVERLAY_LABEL) {
        if let Err(error) = window.hide() {
            tracing::warn!(%error, "échec hide overlay");
        }
    }
}

pub fn show_board(app: &AppHandle<Wry>) {
    reveal(app, BOARD_LABEL, true);
}

pub fn show_pairing(app: &AppHandle<Wry>) {
    reveal(app, PAIRING_LABEL, true);
}

/// Deuxième lancement : demande d'ouverture explicite (`--pairing`/`--board`).
pub fn handle_second_instance(app: &AppHandle<Wry>, args: &[String]) {
    let wants_pairing = args.iter().any(|arg| arg == "--pairing");
    if wants_pairing {
        show_pairing(app);
    } else {
        show_board(app);
    }
}

/// Point d'entrée de l'affichage initial, appelé depuis `setup`.
pub fn bootstrap_windows(app: &AppHandle<Wry>) {
    install_window_events(app);
    let _display = position_overlay_on_selected_display(app);
    apply_overlay_flags(app);
    spawn_topmost_guard(app.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OverlayConfig;

    fn paired_config() -> OverlayConfig {
        let mut config = OverlayConfig::default();
        config.server_url = Some("https://example.test".to_string());
        config.client_token = Some("token".to_string());
        config.guild_id = Some("guild".to_string());
        config
    }

    #[test]
    fn overlay_is_hidden_when_disabled_or_not_paired() {
        let mut config = paired_config();
        assert!(overlay_should_show(&config));

        config.enabled = false;
        assert!(!overlay_should_show(&config), "enabled=false => masqué");

        let mut unpaired = paired_config();
        unpaired.client_token = None;
        assert!(!overlay_should_show(&unpaired), "sans token => masqué");
    }
}
