//! Vérification et installation des mises à jour (plugin updater Tauri 2).
//!
//! La clé publique minisign de `tauri.conf.json` est encore un placeholder :
//! les appels échouent proprement et le message le signale, sans paniquer. Le
//! branchement des métadonnées signées et de l'état détaillé (équivalent
//! `auto-update-service` Electron) est prévu en Phase 9.

use serde::Serialize;
use tauri::{AppHandle, Wry};
use tauri_plugin_updater::UpdaterExt;

/// État renvoyé aux renderers qui interrogent l'updater.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// `up_to_date`, `available`, `installed`, `disabled` ou `error`.
    pub state: String,
    pub current_version: String,
    pub available_version: Option<String>,
    pub message: String,
}

impl UpdateStatus {
    fn error(current_version: String, message: String) -> Self {
        Self {
            state: "error".to_string(),
            current_version,
            available_version: None,
            message,
        }
    }
}

fn current_version(app: &AppHandle<Wry>) -> String {
    app.package_info().version.to_string()
}

fn not_available(current: String) -> UpdateStatus {
    UpdateStatus {
        state: "up_to_date".to_string(),
        current_version: current,
        available_version: None,
        message: "Aucune mise à jour disponible.".to_string(),
    }
}

/// L'updater est explicitement **désactivé** tant que la clé de signature
/// n'est pas configurée (AUD-A6-005).
///
/// La clé publique minisign de `tauri.conf.json` est un placeholder et
/// `bundle.createUpdaterArtifacts` vaut `false` : activer l'updater sans clé
/// réelle et sans installateurs signés produirait un faux sentiment de
/// sécurité. L'activation (génération de clé, signature, publication
/// `latest.json`) est planifiée en Phase 9 et documentée dans
/// `docs/rust/overlay-release.md`.
pub const UPDATER_ENABLED_FOR_RELEASE: bool = false;

fn disabled(current: String) -> UpdateStatus {
    UpdateStatus {
        state: "disabled".to_string(),
        current_version: current,
        available_version: None,
        message: "Mises à jour automatiques désactivées (clé de signature \
                  non configurée)."
            .to_string(),
    }
}

/// Interroge l'endpoint updater ; renvoie l'état à afficher.
pub async fn check_for_update(app: &AppHandle<Wry>) -> UpdateStatus {
    let current = current_version(app);
    if !UPDATER_ENABLED_FOR_RELEASE {
        return disabled(current);
    }
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            return UpdateStatus::error(
                current,
                format!("Updater indisponible (clé minisign placeholder ?) : {error}"),
            );
        }
    };
    match updater.check().await {
        Ok(Some(update)) => UpdateStatus {
            state: "available".to_string(),
            current_version: current.clone(),
            available_version: Some(update.version.clone()),
            message: format!(
                "Mise à jour {} disponible (installée {current}).",
                update.version
            ),
        },
        Ok(None) => not_available(current),
        Err(error) => UpdateStatus::error(current, format!("Échec de la vérification : {error}")),
    }
}

/// Télécharge et installe la mise à jour disponible.
///
/// Sous Windows l'application est quittée par l'installeur à la fin ; le statut
/// renvoyé n'est donc qu'un accusé de réception (même contrat que le legacy).
pub async fn install_update(app: &AppHandle<Wry>) -> UpdateStatus {
    let current = current_version(app);
    if !UPDATER_ENABLED_FOR_RELEASE {
        return disabled(current);
    }
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(error) => {
            return UpdateStatus::error(
                current,
                format!("Updater indisponible (clé minisign placeholder ?) : {error}"),
            );
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();
            tracing::info!(version = %version, "mise à jour trouvée, téléchargement");
            match update
                .download_and_install(
                    |chunk_length, content_length| {
                        tracing::debug!(chunk_length, content_length, "téléchargement update");
                    },
                    || tracing::info!("téléchargement de la mise à jour terminé"),
                )
                .await
            {
                Ok(()) => UpdateStatus {
                    state: "installed".to_string(),
                    current_version: current,
                    available_version: Some(version),
                    message: "Mise à jour téléchargée ; l'application va redémarrer.".to_string(),
                },
                Err(error) => {
                    UpdateStatus::error(current, format!("Échec de l'installation : {error}"))
                }
            }
        }
        Ok(None) => not_available(current),
        Err(error) => UpdateStatus::error(current, format!("Échec de la vérification : {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_status_serializes_in_camel_case() {
        let status = UpdateStatus {
            state: "available".to_string(),
            current_version: "3.0.0".to_string(),
            available_version: Some("3.0.1".to_string()),
            message: "Mise à jour 3.0.1 disponible.".to_string(),
        };
        let value = serde_json::to_value(&status).expect("sérialisation");
        assert_eq!(value["state"], "available");
        assert_eq!(value["currentVersion"], "3.0.0");
        assert_eq!(value["availableVersion"], "3.0.1");
        assert_eq!(value["message"], "Mise à jour 3.0.1 disponible.");
    }

    #[test]
    fn update_status_absent_version_is_null_or_omitted() {
        let status = not_available("3.0.0".to_string());
        let value = serde_json::to_value(&status).expect("sérialisation");
        assert_eq!(value["state"], "up_to_date");
        assert!(value["availableVersion"].is_null());
        assert_eq!(value["currentVersion"], "3.0.0");
    }
}
