//! Automatic updates (Tauri 2 updater plugin).
//!
//! The overlay reads `latest.json` from the GitHub release marked *Latest*
//! right at start, every time the board opens, and every 6 hours. A newer
//! version is downloaded and its minisign signature verified in the background,
//! then kept in memory. Release builds then install it on their own once the
//! overlay has stayed free for a minute (no media, no queued play, no upload,
//! board hidden), and install it when LiveChat quits if that never happened.
//! The `autoUpdate` device setting turns that zero-click path off: the board
//! then announces the download and offers the installation in a banner, and
//! the quit install is skipped. Manual installs stay available (banner and
//! Settings), and the NSIS installer runs in passive mode and relaunches
//! LiveChat; the quit install skips the relaunch, so the update applies at the
//! next start.
//!
//! Debug builds skip the start checks, the 6-hour checks and the automatic
//! install; opening the board and the Settings button still check.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_updater::{Update, UpdaterExt};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// How long the overlay must stay free before installing on its own.
const AUTO_INSTALL_GRACE: Duration = Duration::from_secs(60);
/// How often the automatic installer looks at the overlay state.
const AUTO_INSTALL_POLL: Duration = Duration::from_secs(5);

/// Updater state shown to the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    Idle,
    Checking,
    Downloading,
    UpToDate,
    Ready,
    Error,
}

/// `settings.update`, `overlay_check_updates` and `overlay_install_update` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub state: UpdateState,
    pub current_version: String,
    pub available_version: Option<String>,
    pub message: String,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Checking,
    Downloading {
        version: String,
    },
    UpToDate,
    Ready {
        update: Box<Update>,
        bytes: Vec<u8>,
    },
    Failed {
        message: String,
    },
}

/// Managed state: the current phase, with the verified package once ready.
#[derive(Default)]
pub struct UpdaterState {
    phase: Mutex<Phase>,
}

impl UpdaterState {
    fn phase(&self) -> MutexGuard<'_, Phase> {
        self.phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn status(&self) -> UpdateStatus {
        let phase = self.phase();
        match &*phase {
            Phase::Idle => describe(UpdateState::Idle, None, None),
            Phase::Checking => describe(UpdateState::Checking, None, None),
            Phase::Downloading { version } => {
                describe(UpdateState::Downloading, Some(version), None)
            }
            Phase::UpToDate => describe(UpdateState::UpToDate, None, None),
            Phase::Ready { update, .. } => {
                describe(UpdateState::Ready, Some(&update.version), None)
            }
            Phase::Failed { message } => describe(UpdateState::Error, None, Some(message)),
        }
    }

    /// Version waiting to be installed, if any.
    pub fn ready_version(&self) -> Option<String> {
        match &*self.phase() {
            Phase::Ready { update, .. } => Some(update.version.clone()),
            _ => None,
        }
    }

    fn set(&self, phase: Phase) {
        *self.phase() = phase;
    }
}

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Builds the status shown for a state (pure, for tests).
fn describe(state: UpdateState, version: Option<&str>, error: Option<&str>) -> UpdateStatus {
    let version_text = version.unwrap_or("?");
    let message = match state {
        UpdateState::Idle => "Les mises à jour sont vérifiées automatiquement.".to_string(),
        UpdateState::Checking => "Recherche d'une mise à jour…".to_string(),
        UpdateState::Downloading => format!("Téléchargement de la version {version_text}…"),
        UpdateState::UpToDate => "LiveChat est à jour.".to_string(),
        UpdateState::Ready => format!("La version {version_text} est prête à être installée."),
        UpdateState::Error => format!(
            "Échec de la mise à jour : {}",
            error.unwrap_or("erreur inconnue")
        ),
    };
    UpdateStatus {
        state,
        current_version: current_version(),
        available_version: version.map(str::to_string),
        message,
    }
}

/// Tells the board (Settings › Mise à jour, and the banner) that the updater
/// state changed.
fn publish(app: &AppHandle<Wry>) {
    if let Err(error) = crate::events::emit_device_settings(app) {
        tracing::debug!(%error, "échec d'émission overlay://device-settings (updater)");
    }
}

/// Checks right away, then every 6 hours, and installs the update on its own
/// once the overlay is free (release builds only).
pub fn start(app: &AppHandle<Wry>) {
    if cfg!(debug_assertions) {
        tracing::info!(
            "build de debug : pas de vérification périodique ni d'installation automatique des mises à jour"
        );
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            log_check(&check_and_download(&handle).await);
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        auto_install(&handle).await;
    });
}

/// Checks without waiting for the result (board opening).
pub fn check_in_background(app: &AppHandle<Wry>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        log_check(&check_and_download(&handle).await);
    });
}

fn log_check(status: &UpdateStatus) {
    tracing::info!(state = ?status.state, message = %status.message, "vérification des mises à jour");
}

/// Delays the automatic install until the overlay has stayed free long enough.
#[derive(Debug, Default)]
struct AutoInstallWindow {
    since: Option<Instant>,
}

impl AutoInstallWindow {
    /// Arms on the first eligible observation, starts over when automatic
    /// installs are off or the overlay becomes busy again, and reports `true`
    /// once the grace delay elapsed.
    fn ready(&mut self, enabled: bool, eligible: bool, now: Instant) -> bool {
        if !enabled || !eligible {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        now.duration_since(since) >= AUTO_INSTALL_GRACE
    }
}

/// Whether installing now would disturb nothing: an update is ready, the board
/// is hidden, no media plays or waits, no upload runs, and the user is not
/// placing the overlay zone.
fn auto_install_eligible(app: &AppHandle<Wry>) -> bool {
    if app.state::<UpdaterState>().ready_version().is_none() {
        return false;
    }
    if crate::windows::board_visible(app) || crate::placement::is_active() {
        return false;
    }
    if app
        .state::<crate::net::upload::UploadRegistry>()
        .has_active()
    {
        return false;
    }
    app.state::<crate::state::AppState>().playback_idle()
}

/// Installs the ready update once the overlay has been free for the grace
/// delay. Nothing is downloaded here: only the package already verified by
/// `check_and_download` is installed.
async fn auto_install(app: &AppHandle<Wry>) {
    let mut window = AutoInstallWindow::default();
    loop {
        tokio::time::sleep(AUTO_INSTALL_POLL).await;
        let enabled = app
            .state::<crate::state::AppState>()
            .config_snapshot()
            .auto_update;
        if window.ready(enabled, auto_install_eligible(app), Instant::now()) {
            match install(app) {
                Ok(()) => tracing::info!("mise à jour lancée automatiquement"),
                // The failed phase makes the window empty again; a later check
                // can download the update anew and retry.
                Err(error) => tracing::warn!(%error, "installation automatique impossible"),
            }
        }
    }
}

/// Checks for a newer version and downloads it. Does nothing while a check
/// runs or an update is already waiting to be installed.
pub async fn check_and_download(app: &AppHandle<Wry>) -> UpdateStatus {
    let state = app.state::<UpdaterState>();
    {
        let mut phase = state.phase();
        if matches!(
            *phase,
            Phase::Checking | Phase::Downloading { .. } | Phase::Ready { .. }
        ) {
            drop(phase);
            return state.status();
        }
        *phase = Phase::Checking;
    }
    publish(app);

    let next = match find_update(app).await {
        Ok(None) => Phase::UpToDate,
        Ok(Some(update)) => {
            state.set(Phase::Downloading {
                version: update.version.clone(),
            });
            publish(app);
            match update.download(|_, _| {}, || {}).await {
                Ok(bytes) => {
                    tracing::info!(version = %update.version, "mise à jour téléchargée et vérifiée");
                    Phase::Ready {
                        update: Box::new(update),
                        bytes,
                    }
                }
                Err(error) => Phase::Failed {
                    message: error.to_string(),
                },
            }
        }
        Err(message) => Phase::Failed { message },
    };
    state.set(next);
    publish(app);
    state.status()
}

async fn find_update(app: &AppHandle<Wry>) -> Result<Option<Update>, String> {
    let updater = app.updater().map_err(|error| error.to_string())?;
    updater.check().await.map_err(|error| error.to_string())
}

/// Installs the downloaded update and relaunches LiveChat: the NSIS installer
/// runs in passive mode. On Windows this does not return when it succeeds.
pub fn install(app: &AppHandle<Wry>) -> Result<(), String> {
    install_with(app, true)
}

/// Whether quitting should install the ready update: only when automatic
/// installs are on (pure, for tests).
fn quit_install_planned(auto_update: bool, ready: bool) -> bool {
    auto_update && ready
}

/// Installs the downloaded update when LiveChat is quitting, without relaunching
/// it: the update applies at the next start instead of being lost. A device
/// that turned automatic installs off keeps the update for a manual install.
pub fn install_on_quit(app: &AppHandle<Wry>) -> Result<(), String> {
    let auto_update = app
        .state::<crate::state::AppState>()
        .config_snapshot()
        .auto_update;
    let ready = app.state::<UpdaterState>().ready_version().is_some();
    if !quit_install_planned(auto_update, ready) {
        return Ok(());
    }
    install_with(app, false)
}

/// Installs the ready update. `restart` relaunches LiveChat once the installer
/// ran; quitting for good passes `false`.
fn install_with(app: &AppHandle<Wry>, restart: bool) -> Result<(), String> {
    let state = app.state::<UpdaterState>();
    let (update, bytes) = match std::mem::take(&mut *state.phase()) {
        Phase::Ready { update, bytes } => (update, bytes),
        other => {
            state.set(other);
            return Err("Aucune mise à jour prête à installer.".to_string());
        }
    };
    tracing::info!(version = %update.version, restart, "installation de la mise à jour");
    // The plugin cleans the tray up and exits the process once the installer
    // is launched: Windows then closes the sockets and frees the shortcuts.
    let update = (*update).restart_after_install(restart);
    if let Err(error) = update.install(&bytes) {
        let message = error.to_string();
        tracing::warn!(%message, "échec du lancement de l'installeur");
        state.set(Phase::Failed {
            message: message.clone(),
        });
        publish(app);
        return Err(message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_in_camel_case_with_snake_case_states() {
        let status = describe(UpdateState::Ready, Some("3.2.0"), None);
        let value = serde_json::to_value(&status).expect("sérialisation");
        assert_eq!(value["state"], "ready");
        assert_eq!(value["currentVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["availableVersion"], "3.2.0");
        assert_eq!(
            value["message"],
            "La version 3.2.0 est prête à être installée."
        );
        let value = serde_json::to_value(describe(UpdateState::UpToDate, None, None))
            .expect("sérialisation");
        assert_eq!(value["state"], "up_to_date");
        assert!(value["availableVersion"].is_null());
    }

    #[test]
    fn auto_install_waits_for_the_grace_delay() {
        let mut window = AutoInstallWindow::default();
        let start = Instant::now();
        assert!(!window.ready(true, true, start));
        assert!(!window.ready(
            true,
            true,
            start + AUTO_INSTALL_GRACE - Duration::from_millis(1)
        ));
        assert!(window.ready(true, true, start + AUTO_INSTALL_GRACE));
    }

    #[test]
    fn auto_install_starts_over_when_the_overlay_becomes_busy() {
        let mut window = AutoInstallWindow::default();
        let start = Instant::now();
        assert!(!window.ready(true, true, start));
        assert!(!window.ready(true, false, start + Duration::from_secs(20)));
        let free_again = start + Duration::from_secs(25);
        assert!(!window.ready(true, true, free_again));
        assert!(!window.ready(
            true,
            true,
            free_again + AUTO_INSTALL_GRACE - Duration::from_millis(1)
        ));
        assert!(window.ready(true, true, free_again + AUTO_INSTALL_GRACE));
    }

    #[test]
    fn auto_install_does_not_arm_without_a_ready_update() {
        let mut window = AutoInstallWindow::default();
        let start = Instant::now();
        assert!(!window.ready(true, false, start));
        assert!(!window.ready(true, false, start + AUTO_INSTALL_GRACE));
        assert!(!window.ready(true, true, start + AUTO_INSTALL_GRACE));
        assert!(window.ready(true, true, start + 2 * AUTO_INSTALL_GRACE));
    }

    #[test]
    fn turning_automatic_installs_off_disarms_the_window() {
        let mut window = AutoInstallWindow::default();
        let start = Instant::now();
        assert!(!window.ready(false, true, start));
        assert!(!window.ready(false, true, start + 2 * AUTO_INSTALL_GRACE));
        // Back on: the grace delay starts from the first enabled observation,
        // not from the observation made while the setting was off.
        let enabled_again = start + 3 * AUTO_INSTALL_GRACE;
        assert!(!window.ready(true, true, enabled_again));
        assert!(!window.ready(
            true,
            true,
            enabled_again + AUTO_INSTALL_GRACE - Duration::from_millis(1)
        ));
        assert!(window.ready(true, true, enabled_again + AUTO_INSTALL_GRACE));
    }

    #[test]
    fn quitting_only_installs_when_automatic_installs_are_on() {
        assert!(quit_install_planned(true, true));
        assert!(!quit_install_planned(false, true));
        assert!(!quit_install_planned(true, false));
        assert!(!quit_install_planned(false, false));
    }

    #[test]
    fn errors_carry_the_cause() {
        let status = describe(UpdateState::Error, None, Some("signature invalide"));
        assert_eq!(
            status.message,
            "Échec de la mise à jour : signature invalide"
        );
    }

    #[test]
    fn a_new_state_starts_idle_with_nothing_to_install() {
        let state = UpdaterState::default();
        assert_eq!(state.status().state, UpdateState::Idle);
        assert_eq!(state.ready_version(), None);
    }
}
