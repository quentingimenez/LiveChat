//! Réglages runtime du panneau (`GET`/`PATCH /admin/api/runtime-settings`).
//!
//! Seul réglage concerné pour l'instant : `TIKTOK_COOKIE`. Le legacy modifiait le
//! fichier `.env` (`runtimeSettings.ts`) ; le serveur Rust persiste plutôt un
//! petit fichier JSON **à côté de la base de données** (`admin_runtime.json`),
//! avec écriture atomique (`<fichier>.tmp` puis `rename`). Choix validé par le
//! Lead : cela évite de réécrire `.env` (source de déploiement) et survit aux
//! redémarrages — « rechargé au boot » signifie que le prochain démarrage du
//! serveur relit ce fichier au premier accès, il n'y a pas de copie en mémoire.
//!
//! # Défauts
//!
//! - fichier absent ou illisible : la valeur de `Config::tiktok_cookie`
//!   (variable d'environnement `TIKTOK_COOKIE`) sert de valeur par défaut, après
//!   normalisation ;
//! - valeur normalisée comme le legacy : préfixe `cookie:` retiré (insensible à
//!   la casse), espaces de début/fin retirés ;
//! - une valeur vide est légitime : `GET` renvoie `tiktokCookie: ""`,
//!   `hasTikTokCookie: false`.
//!
//! # Chemin du fichier
//!
//! Déduit de `Config::database_url` (`sqlite:` ou `file:`, schéma Prisma
//! accepté) : le répertoire de la base, sinon le répertoire courant. Le nom est
//! [`SETTINGS_FILE_NAME`].

use std::path::{Path, PathBuf};

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::auth::AdminAccess;
use super::AdminError;
use crate::app::AppState;

/// Nom du fichier de réglages runtime, posé à côté de la base de données.
pub const SETTINGS_FILE_NAME: &str = "admin_runtime.json";

/// Contenu du fichier de réglages runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSettings {
    #[serde(default)]
    tiktok_cookie: String,
}

impl RuntimeSettings {
    /// Normalise le cookie (préfixe `cookie:` et espaces retirés).
    fn normalize(&mut self) {
        self.tiktok_cookie = normalize_tiktok_cookie(&self.tiktok_cookie);
    }
}

/// Handler `GET /admin/api/runtime-settings`.
pub(super) async fn get(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Response, AdminError> {
    let settings = load_settings(&state).await;
    let has_tiktok_cookie = !settings.tiktok_cookie.is_empty();

    Ok(Json(json!({
        "tiktokCookie": settings.tiktok_cookie,
        "hasTikTokCookie": has_tiktok_cookie,
    }))
    .into_response())
}

/// Handler `PATCH /admin/api/runtime-settings`.
///
/// Corps attendu : `{"tiktokCookie": "<chaîne>"}`. Tout autre type (y compris
/// l'absence du champ) vaut `400 invalid_payload`, comme le legacy.
pub(super) async fn patch(
    State(state): State<AppState>,
    _access: AdminAccess,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let cookie = payload
        .get("tiktokCookie")
        .and_then(Value::as_str)
        .ok_or_else(|| AdminError::bad_request("invalid_payload"))?;
    let cookie = normalize_tiktok_cookie(cookie);

    let mut settings = load_settings(&state).await;
    settings.tiktok_cookie = cookie.clone();
    save_settings(&state, &settings).await?;

    Ok(Json(json!({
        "updated": true,
        "tiktokCookie": cookie,
        "hasTikTokCookie": !cookie.is_empty(),
    }))
    .into_response())
}

/// Normalisation legacy de `TIKTOK_COOKIE`.
///
/// `normalizeTikTokCookie` du legacy ne retirait pas des espaces internes et
/// seulement un éventuel préfixe `cookie:` en tête.
fn normalize_tiktok_cookie(value: &str) -> String {
    let value = value.trim();
    let without_prefix = value
        .strip_prefix("cookie:")
        .or_else(|| value.strip_prefix("Cookie:"))
        .or_else(|| value.strip_prefix("COOKIE:"))
        .unwrap_or(value);

    without_prefix.trim().to_owned()
}

/// Charge les réglages : fichier JSON, sinon défaut d'environnement.
async fn load_settings(state: &AppState) -> RuntimeSettings {
    let path = settings_path(state);

    match tokio::fs::read(&path).await {
        Ok(contents) => match serde_json::from_slice::<RuntimeSettings>(&contents) {
            Ok(mut settings) => {
                settings.normalize();
                settings
            }
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "fichier de réglages admin illisible, utilisation de la valeur d'environnement"
                );
                default_settings(state)
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => default_settings(state),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "fichier de réglages admin inaccessible, utilisation de la valeur d'environnement"
            );
            default_settings(state)
        }
    }
}

fn default_settings(state: &AppState) -> RuntimeSettings {
    RuntimeSettings {
        tiktok_cookie: normalize_tiktok_cookie(state.config.tiktok_cookie.expose_secret()),
    }
}

/// Écrit les réglages de façon atomique : `<fichier>.tmp` puis `rename`.
///
/// Sur Windows, `rename` échoue si la destination existe : on supprime donc le
/// fichier final juste avant le `rename`. La fenêtre est minuscule et sans
/// conséquence : en cas de crash, le fichier absent fait retomber sur la valeur
/// d'environnement.
async fn save_settings(state: &AppState, settings: &RuntimeSettings) -> Result<(), AdminError> {
    let path = settings_path(state);
    let temporary_path = temporary_path(&path);

    if let Some(directory) = path.parent() {
        if !directory.as_os_str().is_empty() {
            tokio::fs::create_dir_all(directory)
                .await
                .map_err(|error| {
                    tracing::error!(
                        path = %directory.display(),
                        error = %error,
                        "création du dossier des réglages admin impossible"
                    );
                    AdminError::Internal
                })?;
        }
    }

    let contents = serde_json::to_vec_pretty(settings).map_err(|error| {
        tracing::error!(error = %error, "sérialisation des réglages admin impossible");
        AdminError::Internal
    })?;

    tokio::fs::write(&temporary_path, &contents)
        .await
        .map_err(|error| {
            tracing::error!(
                path = %temporary_path.display(),
                error = %error,
                "écriture du fichier temporaire des réglages admin impossible"
            );
            AdminError::Internal
        })?;

    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        tokio::fs::remove_file(&path).await.map_err(|error| {
            tracing::error!(
                path = %path.display(),
                error = %error,
                "suppression de l'ancien fichier de réglages admin impossible"
            );
            AdminError::Internal
        })?;
    }

    tokio::fs::rename(&temporary_path, &path)
        .await
        .map_err(|error| {
            tracing::error!(
                from = %temporary_path.display(),
                to = %path.display(),
                error = %error,
                "renommage du fichier de réglages admin impossible"
            );
            AdminError::Internal
        })?;

    Ok(())
}

/// `admin_runtime.json` dans le répertoire de la base, sinon répertoire courant.
pub fn settings_path(state: &AppState) -> PathBuf {
    database_directory(state.config.database_url.as_deref()).join(SETTINGS_FILE_NAME)
}

/// Répertoire de la base déduit d'une URL `sqlite:` / `file:` (Prisma accepté).
fn database_directory(database_url: Option<&str>) -> PathBuf {
    let Some(database_url) = database_url else {
        return PathBuf::from(".");
    };

    let trimmed = database_url.trim();
    if trimmed.is_empty() || trimmed == ":memory:" || trimmed.contains("mode=memory") {
        return PathBuf::from(".");
    }

    let path = strip_database_prefix(trimmed);
    let path = path.split('?').next().unwrap_or(path).trim();

    if path.is_empty() || path == ":memory:" {
        return PathBuf::from(".");
    }

    let path = Path::new(path);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Retire les préfixes d'URL de base connus.
fn strip_database_prefix(value: &str) -> &str {
    // `sqlite://C:/…` et `sqlite:/chemin` sont acceptés par `Db::connect`.
    for prefix in ["sqlite://", "sqlite:", "file://", "file:"] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return rest;
        }
    }

    value
}

/// Chemin du fichier temporaire : `admin_runtime.json.tmp`.
fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn normalisation_du_cookie() {
        assert_eq!(normalize_tiktok_cookie("  cookie:abc  "), "abc");
        assert_eq!(normalize_tiktok_cookie("COOKIE: abc"), "abc");
        assert_eq!(normalize_tiktok_cookie("Cookie:abc"), "abc");
        assert_eq!(normalize_tiktok_cookie("abc"), "abc");
        assert_eq!(normalize_tiktok_cookie(""), "");
        assert_eq!(normalize_tiktok_cookie("   "), "");
        // Le préfixe n'est retiré qu'en tête, comme le legacy.
        assert_eq!(normalize_tiktok_cookie("abc:cookie:def"), "abc:cookie:def");
    }

    #[test]
    fn repertoire_de_base_deduit_de_lurl() {
        assert_eq!(
            database_directory(Some("sqlite:./data/livechat.sqlite")),
            PathBuf::from("./data")
        );
        assert_eq!(
            database_directory(Some("sqlite://./data/livechat.sqlite?mode=rwc")),
            PathBuf::from("./data")
        );
        assert_eq!(
            database_directory(Some("file:./data/prisma.sqlite")),
            PathBuf::from("./data")
        );
        assert_eq!(
            database_directory(Some("sqlite:livechat.sqlite")),
            PathBuf::from(".")
        );
        assert_eq!(
            database_directory(Some("sqlite::memory:")),
            PathBuf::from(".")
        );
        assert_eq!(database_directory(None), PathBuf::from("."));
    }

    #[test]
    fn chemin_temporaire_suffixe() {
        assert_eq!(
            temporary_path(Path::new("data/admin_runtime.json")),
            PathBuf::from("data/admin_runtime.json.tmp")
        );
    }
}
