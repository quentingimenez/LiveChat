//! Cycle de vie et politiques des deux fenêtres du shell :
//! `overlay` (plein écran transparent, click-through, topmost) et `board`
//! (1180×780, qui porte aussi l'appairage).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent, Wry,
};

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

/// What to do with the board's layer after a foreground change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardLayer {
    /// Above the topmost overlay.
    Float,
    /// Back to an ordinary window, right behind the new foreground one.
    Settle,
    /// Nothing to change.
    Keep,
}

/// The board floats above the overlay only while it is visible and in the
/// foreground: the only time the user works in it while a full-screen media
/// covers the screen. As soon as another window comes to the front, a floating
/// board becomes an ordinary window again.
pub fn board_layer(visible: bool, foreground: bool, floating: bool) -> BoardLayer {
    if visible && foreground {
        BoardLayer::Float
    } else if floating {
        BoardLayer::Settle
    } else {
        BoardLayer::Keep
    }
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

fn position_overlay(app: &AppHandle<Wry>, persist: bool) -> Option<DisplayInfo> {
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
    if persist {
        let config = app.state::<AppState>().config_snapshot();
        persist_display_selection(app, &target, &config);
    }
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

/// Positionne et dimensionne l'overlay sur l'écran sélectionné.
pub fn position_overlay_on_selected_display(app: &AppHandle<Wry>) -> Option<DisplayInfo> {
    position_overlay(app, true)
}

/// Re-places the overlay after `settings_set` already persisted the selection.
pub fn position_overlay_without_persisting(app: &AppHandle<Wry>) -> Option<DisplayInfo> {
    position_overlay(app, false)
}

/// Applies click-through, non-focusable, topmost and Win32 overlay policies.
fn apply_window_policies(window: &WebviewWindow<Wry>) {
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
}

/// Applique les politiques de l'overlay (click-through, topmost, styles Win32)
/// puis l'affiche sans lui donner le focus.
pub fn apply_overlay_flags(app: &AppHandle<Wry>) {
    let Some(window) = window(app, OVERLAY_LABEL) else {
        tracing::warn!("fenêtre overlay absente");
        return;
    };
    apply_window_policies(&window);

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

/// Shows the overlay for the local settings preview, including when the device
/// is not paired or the overlay is muted. This is only used by
/// `settings_test_media`; no server event is involved.
pub fn show_overlay_preview(app: &AppHandle<Wry>) {
    let Some(window) = window(app, OVERLAY_LABEL) else {
        tracing::warn!("fenêtre overlay absente pendant l'aperçu");
        return;
    };
    apply_window_policies(&window);
    reveal(app, OVERLAY_LABEL, false);
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        if let Err(error) = win32::reassert_topmost(hwnd) {
            tracing::warn!(%error, "échec réaffirmation topmost de l'aperçu");
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

        // The overlay was just raised to the top of the topmost band: a board in
        // the foreground must go back above it, without touching the overlay.
        #[cfg(windows)]
        schedule_board_layer_sync(&app);
        #[cfg(not(windows))]
        if let Some(board) = app.get_webview_window(BOARD_LABEL) {
            let visible = board.is_visible().unwrap_or(false);
            let focused = board.is_focused().unwrap_or(false);
            if board_layer(visible, focused, false) == BoardLayer::Float {
                if let Err(error) = board.set_always_on_top(true) {
                    tracing::debug!(%error, "set_always_on_top board tick en échec");
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

/// Floats the board above the overlay while it is the foreground window and
/// puts it back among ordinary windows as soon as another one is activated.
///
/// Follows the desktop's foreground window instead of Tauri's `Focused`
/// events: on Windows those come from WebView2's `GotFocus`/`LostFocus`, and
/// `LostFocus` is not raised when the user activates another application, so
/// the board stayed in front of the window just clicked.
#[cfg(windows)]
fn track_board_layer(app: &AppHandle<Wry>) {
    let handle = app.clone();
    if !win32::watch_foreground(move |_foreground| schedule_board_layer_sync(&handle)) {
        tracing::warn!(
            "suivi du premier plan indisponible : le board ne passera pas devant l'overlay"
        );
    }
}

/// Le board passe au-dessus de l'overlay pendant qu'il a le focus, puis
/// redevient une fenêtre normale dès qu'il le perd.
#[cfg(not(windows))]
fn track_board_layer(app: &AppHandle<Wry>) {
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
        }
    });
}

/// Runs [`sync_board_layer`] from the event loop.
///
/// The foreground hook may fire inside a nested message loop (WebView2
/// creation, a native dialog) where calling back into Tauri is not safe, so
/// the work is always posted from another thread; the topmost guard uses the
/// same path, which keeps both in order on the main thread.
#[cfg(windows)]
fn schedule_board_layer_sync(app: &AppHandle<Wry>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let target = handle.clone();
        if let Err(error) = handle.run_on_main_thread(move || sync_board_layer(&target)) {
            tracing::debug!(%error, "synchronisation du plan du board impossible");
        }
    });
}

/// Applies [`board_layer`] for the current foreground window.
#[cfg(windows)]
fn sync_board_layer(app: &AppHandle<Wry>) {
    let Some(board) = app.get_webview_window(BOARD_LABEL) else {
        return;
    };
    let Ok(hwnd) = board.hwnd() else {
        return;
    };
    let visible = board.is_visible().unwrap_or(false);
    let foreground = win32::foreground_window() == Some(hwnd);
    match board_layer(visible, foreground, win32::is_topmost(hwnd)) {
        BoardLayer::Float => {
            // Keeps tao's own flag in step, then lifts the board above the
            // overlay, which the guard keeps at the top of the topmost band.
            if let Err(error) = board.set_always_on_top(true) {
                tracing::debug!(%error, "set_always_on_top board en échec");
            }
            if let Err(error) = win32::reassert_topmost(hwnd) {
                tracing::debug!(%error, "SetWindowPos topmost board en échec");
            }
            // Screen identification cards stay above the board (centred both)
            // for the few seconds they are shown.
            for (label, card) in app.webview_windows() {
                if !label.starts_with(IDENTIFY_LABEL_PREFIX) {
                    continue;
                }
                if let Ok(card_hwnd) = card.hwnd() {
                    if let Err(error) = win32::reassert_topmost(card_hwnd) {
                        tracing::debug!(label, %error, "carte d'identification sous le board");
                    }
                }
            }
        }
        BoardLayer::Settle => {
            // On the main thread Tauri applies this inline: its HWND_NOTOPMOST
            // lifts the board above every ordinary window, the one just
            // activated included, so it is moved right behind that one next.
            if let Err(error) = board.set_always_on_top(false) {
                tracing::debug!(%error, "retrait du topmost du board en échec");
            }
            if !win32::place_behind_foreground(hwnd) {
                tracing::debug!("board laissé devant : fenêtre au premier plan non éligible");
            }
        }
        BoardLayer::Keep => {}
    }
}

/// Les fenêtres du shell ne se détruisent pas à la fermeture : elles se cachent.
pub fn install_window_events(app: &AppHandle<Wry>) {
    hide_on_close(app, BOARD_LABEL);
    track_board_layer(app);
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

/// How long the display identification cards stay visible.
pub const IDENTIFY_DISPLAY_DURATION: Duration = Duration::from_secs(3);
const IDENTIFY_LABEL_PREFIX: &str = "identify-";

/// Generation counter so a previous 3-second timer can never close a newer
/// identification batch.
static IDENTIFY_GENERATION: AtomicU64 = AtomicU64::new(0);

fn close_identify_windows(app: &AppHandle<Wry>) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(IDENTIFY_LABEL_PREFIX) {
            if let Err(error) = window.destroy() {
                tracing::debug!(label, %error, "fermeture de la fenêtre d'identification impossible");
            }
        }
    }
}

/// Shows one identifying overlay per display and closes them after 3 seconds.
/// Returns the number of displays that received a card.
pub fn identify_displays(app: &AppHandle<Wry>) -> usize {
    close_identify_windows(app);
    let generation = IDENTIFY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let (displays, primary_index) = collect_displays(app);
    let mut created = 0;
    for display in &displays {
        let label = format!("{IDENTIFY_LABEL_PREFIX}{}", display.index);
        let primary = primary_index == Some(display.index);
        let initialization_script = format!(
            "window.__livechatIdentify = {};",
            serde_json::json!({
                "index": display.index,
                "label": display::format_settings_label(display, primary),
                "role": if primary { "Écran principal" } else { "Écran secondaire" },
            })
        );
        let builder =
            WebviewWindowBuilder::new(app, &label, WebviewUrl::App("identify.html".into()))
                .title("LiveChat — Identification des écrans")
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .resizable(false)
                .focused(false)
                .focusable(false)
                .visible(false)
                .initialization_script(initialization_script);
        let built = builder
            .position(display.x as f64, display.y as f64)
            .inner_size(display.width as f64, display.height as f64)
            .build();
        match built {
            Ok(window) => {
                if let Err(error) =
                    window.set_size(PhysicalSize::new(display.width, display.height))
                {
                    tracing::warn!(label, %error, "taille de la fenêtre d'identification");
                }
                if let Err(error) = window.set_position(PhysicalPosition::new(display.x, display.y))
                {
                    tracing::warn!(label, %error, "position de la fenêtre d'identification");
                }
                if let Err(error) = window.set_ignore_cursor_events(true) {
                    tracing::warn!(label, %error, "click-through de l'identification");
                }
                if let Err(error) = window.set_focusable(false) {
                    tracing::warn!(label, %error, "focus de l'identification");
                }
                // `show()` would activate the card and take the focus from the
                // board the user clicked in: show it without activation, last,
                // with no tao setter afterwards.
                #[cfg(windows)]
                match window.hwnd() {
                    Ok(hwnd) => win32::show_without_activation(hwnd),
                    Err(error) => tracing::warn!(label, %error, "hwnd de l'identification"),
                }
                #[cfg(not(windows))]
                if let Err(error) = window.show() {
                    tracing::warn!(label, %error, "affichage de l'identification");
                }
                created += 1;
            }
            Err(error) => {
                tracing::warn!(label, %error, "création de la fenêtre d'identification impossible");
            }
        }
    }

    if created > 0 {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(IDENTIFY_DISPLAY_DURATION);
            let timer_handle = handle.clone();
            if let Err(error) = handle.run_on_main_thread(move || {
                if IDENTIFY_GENERATION.load(Ordering::SeqCst) != generation {
                    return;
                }
                close_identify_windows(&timer_handle);
            }) {
                tracing::warn!(%error, "fermeture planifiée des fenêtres d'identification impossible");
            }
        });
    }
    created
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
    fn board_floats_only_when_visible_and_in_the_foreground() {
        assert_eq!(board_layer(true, true, false), BoardLayer::Float);
        assert_eq!(board_layer(true, true, true), BoardLayer::Float);
        assert_eq!(board_layer(false, true, false), BoardLayer::Keep);
        assert_eq!(board_layer(true, false, false), BoardLayer::Keep);
    }

    #[test]
    fn a_floating_board_settles_once_another_window_is_in_front() {
        // The case that left the board above the window the user had clicked.
        assert_eq!(board_layer(true, false, true), BoardLayer::Settle);
        // Hidden or minimised while still topmost: drop the flag as well.
        assert_eq!(board_layer(false, false, true), BoardLayer::Settle);
    }
}
