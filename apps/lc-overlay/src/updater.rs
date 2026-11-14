//! Automatic updates (Tauri 2 updater plugin).
//!
//! The overlay reads `latest.json` from the GitHub release marked *Latest*
//! right at start, every time the board opens, and every 6 hours. A newer
//! version is downloaded and its minisign signature verified in the background,
//! then kept in memory. Installing stays a user action (tray or Settings) so an
//! update never cuts a live stream: the NSIS installer then runs in passive mode
//! and relaunches LiveChat.
//!
//! Debug builds skip the start and 6-hour checks; opening the board and the
//! Settings button still check.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_updater::{Update, UpdaterExt};

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

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

/// Tells the board and the tray that the updater state changed.
fn publish(app: &AppHandle<Wry>) {
    if let Err(error) = crate::events::emit_device_settings(app) {
        tracing::debug!(%error, "échec d'émission overlay://device-settings (updater)");
    }
    crate::tray::refresh_tray(app);
}

/// Checks right away, then every 6 hours (release builds only).
pub fn start(app: &AppHandle<Wry>) {
    if cfg!(debug_assertions) {
        tracing::info!("build de debug : pas de vérification périodique des mises à jour");
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            log_check(&check_and_download(&handle).await);
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
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

/// Installs the downloaded update: the NSIS installer runs in passive mode and
/// relaunches LiveChat. On Windows this does not return when it succeeds.
pub fn install(app: &AppHandle<Wry>) -> Result<(), String> {
    let state = app.state::<UpdaterState>();
    let (update, bytes) = match std::mem::take(&mut *state.phase()) {
        Phase::Ready { update, bytes } => (update, bytes),
        other => {
            state.set(other);
            return Err("Aucune mise à jour prête à installer.".to_string());
        }
    };
    tracing::info!(version = %update.version, "installation de la mise à jour");
    // The plugin cleans the tray up and exits the process once the installer
    // is launched: Windows then closes the sockets and frees the shortcuts.
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
