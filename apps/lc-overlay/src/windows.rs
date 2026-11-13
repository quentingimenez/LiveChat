//! Cycle de vie et politiques des deux fenêtres du shell :
//! `overlay` (plein écran transparent, click-through, topmost) et `board`
//! (1180×780, qui porte aussi l'appairage).

use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent, Wry};

use crate::config::OverlayConfig;
use crate::display::{self, DisplayInfo, DisplaySelection};
use crate::state::AppState;
#[cfg(windows)]
use crate::win32;

pub const OVERLAY_LABEL: &str = "overlay";
pub const BOARD_LABEL: &str = "board";

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

/// Le board ne doit passer au-dessus de l'overlay que s'il est visible et qu'il
/// détient le focus clavier : c'est la seule situation où l'utilisateur
/// interagit avec lui alors qu'un média plein écran couvre l'écran.
pub fn board_should_float(visible: bool, focused: bool) -> bool {
    visible && focused
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

    #[cfg(windows)]
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

    #[cfg(windows)]
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
        #[cfg(windows)]
        match window.hwnd() {
            Ok(hwnd) => {
                let _styles = win32::ensure_overlay_ex_style(hwnd);
                if let Err(error) = win32::reassert_topmost(hwnd) {
                    tracing::debug!(%error, "SetWindowPos topmost tick en échec");
                }
            }
            Err(error) => tracing::debug!(%error, "hwnd overlay perdu (tick)"),
        }

        // Le board au focus doit rester au-dessus de l'overlay topmost : on le
        // réaffirme après l'overlay, sans jamais toucher aux flags de ce dernier.
        if let Some(board) = app.get_webview_window(BOARD_LABEL) {
            let visible = board.is_visible().unwrap_or(false);
            let focused = board.is_focused().unwrap_or(false);
            if board_should_float(visible, focused) {
                if let Err(error) = board.set_always_on_top(true) {
                    tracing::debug!(%error, "set_always_on_top board tick en échec");
                }
                #[cfg(windows)]
                match board.hwnd() {
                    Ok(hwnd) => {
                        if let Err(error) = win32::reassert_topmost(hwnd) {
                            tracing::debug!(%error, "SetWindowPos topmost board tick en échec");
                        }
                    }
                    Err(error) => tracing::debug!(%error, "hwnd board perdu (tick)"),
                }
            }
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

/// Délai laissé à Windows pour terminer le changement de fenêtre active avant
/// de replacer le board derrière la nouvelle fenêtre au premier plan.
#[cfg(windows)]
const BOARD_BLUR_SETTLE_DELAY: Duration = Duration::from_millis(40);

/// Le board passe au-dessus de l'overlay pendant qu'il a le focus, puis
/// redevient une fenêtre normale dès qu'il le perd.
fn track_board_focus(app: &AppHandle<Wry>) {
    let Some(window) = window(app, BOARD_LABEL) else {
        tracing::warn!("fenêtre board absente");
        return;
    };
    let target = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(focused) = event {
            if let Err(error) = target.set_always_on_top(*focused) {
                tracing::debug!(%error, "échec set_always_on_top board");
            }
            #[cfg(windows)]
            if !*focused {
                settle_board_behind_foreground(target.clone());
            }
        }
    });
}

/// Retirer le mode « toujours au-dessus » (`HWND_NOTOPMOST`) place le board
/// devant toutes les fenêtres normales, y compris celle que l'utilisateur vient
/// d'activer : on le range juste derrière elle, comme une fenêtre ordinaire.
#[cfg(windows)]
fn settle_board_behind_foreground(board: WebviewWindow<Wry>) {
    std::thread::spawn(move || {
        std::thread::sleep(BOARD_BLUR_SETTLE_DELAY);
        if board.is_focused().unwrap_or(false) {
            return;
        }
        match board.hwnd() {
            Ok(hwnd) => {
                if !win32::place_behind_foreground(hwnd) {
                    tracing::debug!("board laissé en place : fenêtre au premier plan non éligible");
                }
            }
            Err(error) => tracing::debug!(%error, "hwnd board indisponible après perte du focus"),
        }
    });
}

/// Les fenêtres du shell ne se détruisent pas à la fermeture : elles se cachent.
pub fn install_window_events(app: &AppHandle<Wry>) {
    hide_on_close(app, BOARD_LABEL);
    track_board_focus(app);
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

/// Deuxième lancement : ouverture explicite de la board.
///
/// `--pairing` reste accepté pour la compatibilité des raccourcis existants :
/// il ouvre aussi la board, qui porte désormais l'appairage.
pub fn handle_second_instance(app: &AppHandle<Wry>, args: &[String]) {
    if args.iter().any(|arg| arg == "--pairing") {
        tracing::info!("--pairing reçu : ouverture de la board (fenêtre d'appairage supprimée)");
    }
    show_board(app);
}

/// Point d'entrée de l'affichage initial, appelé depuis `setup`.
pub fn bootstrap_windows(app: &AppHandle<Wry>) {
    install_window_events(app);
    let _display = position_overlay_on_selected_display(app);
    apply_overlay_flags(app);
    // First launch without a guild: open the board so pairing can happen (it
    // replaces the former `pairing` window).
    if app
        .state::<AppState>()
        .config_snapshot()
        .memberships
        .is_empty()
    {
        show_board(app);
    }
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

    #[test]
    fn board_floats_only_when_visible_and_focused() {
        assert!(board_should_float(true, true));
        assert!(!board_should_float(true, false));
        assert!(!board_should_float(false, true));
        assert!(!board_should_float(false, false));
    }
}
