//! Hot server settings shared by every HTTP and WebSocket handler.
//!
//! The admin console persists a small JSON file **next to the SQLite database**
//! (`admin_runtime.json`) with the settings that may be changed without a
//! restart. It is loaded once when [`AppState`](crate::app::AppState) is built
//! and kept in memory behind a [`RwLock`]; a `PATCH` writes the file and swaps
//! the in-memory overrides, so the next request sees the new values.
//!
//! A missing key means "use the environment value": [`RuntimeOverrides`] stores
//! `Option<T>` per key and every accessor falls back to [`Config`]. `null` in a
//! `PATCH` therefore clears the override and brings the environment value back.
//!
//! The TikTok cookie stays a special case of the legacy contract: it is stored
//! in clear text in the same file and returned by the admin GET route. The
//! normalisation (optional `cookie:` prefix, trimmed spaces) is unchanged.
//!
//! The file is written atomically (`<file>.tmp` then `rename`). On Windows
//! `rename` fails when the destination already exists, so the destination is
//! removed just before the rename; if the process crashes in between, the file
//! is missing and every value falls back to the environment (never a partially
//! written file).

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Name of the runtime settings file, written next to the database.
pub const SETTINGS_FILE_NAME: &str = "admin_runtime.json";

/// Persisted overrides of the hot settings. `None` means "environment value".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeOverrides {
    /// `TIKTOK_COOKIE` override (clear text, legacy behaviour).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tiktok_cookie: Option<String>,
    /// `YOUTUBE_COOKIE` override (clear text, same secret handling as TikTok).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub youtube_cookie: Option<String>,
    /// `DEFAULT_DURATION` override, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_duration: Option<u64>,
    /// `MEDIA_CACHE_TTL_HOURS` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_cache_ttl_hours: Option<u64>,
    /// `MEDIA_CACHE_MAX_TOTAL_MB` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_cache_max_total_mb: Option<u64>,
    /// `MEDIA_BOARD_MAX_TOTAL_MB` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_board_max_total_mb: Option<u64>,
    /// `MEDIA_MAX_SIZE_MB` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_max_size_mb: Option<u64>,
    /// `MEDIA_VIDEO_MAX_HEIGHT` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_video_max_height: Option<u32>,
    /// `MIN_OVERLAY_VERSION` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_overlay_version: Option<String>,
    /// `TTS_ENABLED` override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts_enabled: Option<bool>,
}

/// Immutable server configuration plus the hot overrides loaded from disk.
#[derive(Debug)]
pub struct Runtime {
    config: Arc<Config>,
    overrides: RwLock<RuntimeOverrides>,
}

impl Runtime {
    /// Builds the runtime values, loading `admin_runtime.json` when present.
    ///
    /// The file is read synchronously: it is tiny and this only happens once,
    /// when the application state is built.
    pub fn new(config: Arc<Config>) -> Self {
        let overrides = load_overrides(&config);
        Self {
            config,
            overrides: RwLock::new(overrides),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, RuntimeOverrides> {
        match self.overrides.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write(&self) -> RwLockWriteGuard<'_, RuntimeOverrides> {
        match self.overrides.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Current overrides (a snapshot that can be edited before persisting).
    #[must_use]
    pub fn snapshot(&self) -> RuntimeOverrides {
        self.read().clone()
    }

    /// Replaces the in-memory overrides without touching the file.
    pub fn apply(&self, overrides: RuntimeOverrides) {
        *self.write() = overrides;
    }

    /// Effective `DEFAULT_DURATION`, in seconds.
    #[must_use]
    pub fn default_duration(&self) -> u64 {
        self.read()
            .default_duration
            .unwrap_or(self.config.default_duration)
    }

    /// Effective `MEDIA_CACHE_TTL_HOURS`.
    #[must_use]
    pub fn media_cache_ttl_hours(&self) -> u64 {
        self.read()
            .media_cache_ttl_hours
            .unwrap_or(self.config.media_cache_ttl_hours)
    }

    /// Effective `MEDIA_CACHE_MAX_TOTAL_MB`.
    #[must_use]
    pub fn media_cache_max_total_mb(&self) -> u64 {
        self.read()
            .media_cache_max_total_mb
            .unwrap_or(self.config.media_cache_max_total_mb)
    }

    /// Effective `MEDIA_BOARD_MAX_TOTAL_MB`.
    #[must_use]
    pub fn media_board_max_total_mb(&self) -> u64 {
        self.read()
            .media_board_max_total_mb
            .unwrap_or(self.config.media_board_max_total_mb)
    }

    /// Effective `MEDIA_MAX_SIZE_MB`.
    #[must_use]
    pub fn media_max_size_mb(&self) -> u64 {
        self.read()
            .media_max_size_mb
            .unwrap_or(self.config.media_max_size_mb)
    }

    /// Effective `MEDIA_VIDEO_MAX_HEIGHT`.
    #[must_use]
    pub fn media_video_max_height(&self) -> u32 {
        self.read()
            .media_video_max_height
            .unwrap_or(self.config.media_video_max_height)
    }

    /// Effective `MIN_OVERLAY_VERSION`.
    #[must_use]
    pub fn min_overlay_version(&self) -> String {
        self.read()
            .min_overlay_version
            .clone()
            .unwrap_or_else(|| self.config.min_overlay_version.clone())
    }

    /// Effective `TTS_ENABLED`.
    #[must_use]
    pub fn tts_enabled(&self) -> bool {
        self.read().tts_enabled.unwrap_or(self.config.tts_enabled)
    }

    /// Effective TikTok cookie (override, otherwise the environment value).
    #[must_use]
    pub fn tiktok_cookie(&self) -> String {
        self.read()
            .tiktok_cookie
            .clone()
            .unwrap_or_else(|| normalize_tiktok_cookie(self.config.tiktok_cookie.expose_secret()))
    }

    /// Effective YouTube cookie (override, otherwise the environment value).
    #[must_use]
    pub fn youtube_cookie(&self) -> String {
        self.read()
            .youtube_cookie
            .clone()
            .unwrap_or_else(|| normalize_youtube_cookie(self.config.youtube_cookie.expose_secret()))
    }

    /// Writes the given overrides to `admin_runtime.json` atomically.
    ///
    /// The caller decides when to [`Self::apply`] them to memory, so a failed
    /// write leaves the running server on its previous values.
    pub async fn persist(&self, overrides: &RuntimeOverrides) -> std::io::Result<()> {
        let path = settings_path(&self.config);
        let temporary_path = temporary_path(&path);

        if let Some(directory) = path.parent() {
            if !directory.as_os_str().is_empty() {
                tokio::fs::create_dir_all(directory).await?;
            }
        }

        let contents = serde_json::to_vec_pretty(overrides).map_err(std::io::Error::other)?;
        tokio::fs::write(&temporary_path, &contents).await?;

        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            tokio::fs::remove_file(&path).await?;
        }
        tokio::fs::rename(&temporary_path, &path).await
    }
}

/// Loads the overrides from disk, falling back to the environment on any error.
fn load_overrides(config: &Config) -> RuntimeOverrides {
    let path = settings_path(config);
    let contents = match std::fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RuntimeOverrides::default()
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "runtime settings file is unreadable, using environment values"
            );
            return RuntimeOverrides::default();
        }
    };

    match serde_json::from_slice::<RuntimeOverrides>(&contents) {
        Ok(mut overrides) => {
            if let Some(cookie) = overrides.tiktok_cookie.as_deref() {
                overrides.tiktok_cookie = Some(normalize_tiktok_cookie(cookie));
            }
            if let Some(cookie) = overrides.youtube_cookie.as_deref() {
                overrides.youtube_cookie = Some(normalize_youtube_cookie(cookie));
            }
            overrides
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "runtime settings file is malformed, using environment values"
            );
            RuntimeOverrides::default()
        }
    }
}

/// Normalises the legacy TikTok cookie value.
///
/// A leading `cookie:` prefix (case-insensitive) and surrounding spaces are
/// removed, as the legacy `normalizeTikTokCookie` did.
#[must_use]
pub fn normalize_tiktok_cookie(value: &str) -> String {
    normalize_source_cookie(value)
}

/// Normalises the YouTube cookie value with the same legacy rule.
#[must_use]
pub fn normalize_youtube_cookie(value: &str) -> String {
    normalize_source_cookie(value)
}

fn normalize_source_cookie(value: &str) -> String {
    let value = value.trim();
    let without_prefix = value
        .strip_prefix("cookie:")
        .or_else(|| value.strip_prefix("Cookie:"))
        .or_else(|| value.strip_prefix("COOKIE:"))
        .unwrap_or(value);
    without_prefix.trim().to_owned()
}

/// Path of `admin_runtime.json`: the database directory, otherwise the current
/// directory.
#[must_use]
pub fn settings_path(config: &Config) -> PathBuf {
    database_directory(config.database_url.as_deref()).join(SETTINGS_FILE_NAME)
}

/// Database directory inferred from a `sqlite:` / `file:` URL (Prisma accepted).
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

/// Removes the known database URL schemes.
fn strip_database_prefix(value: &str) -> &str {
    for prefix in ["sqlite://", "sqlite:", "file://", "file:"] {
        if let Some(rest) = value.strip_prefix(prefix) {
            return rest;
        }
    }
    value
}

/// Temporary file path: `admin_runtime.json.tmp`.
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
    fn tiktok_cookie_normalisation_matches_the_legacy_rule() {
        assert_eq!(normalize_tiktok_cookie("  cookie:abc  "), "abc");
        assert_eq!(normalize_tiktok_cookie("COOKIE: abc"), "abc");
        assert_eq!(normalize_tiktok_cookie("Cookie:abc"), "abc");
        assert_eq!(normalize_tiktok_cookie("abc"), "abc");
        assert_eq!(normalize_tiktok_cookie(""), "");
        assert_eq!(normalize_tiktok_cookie("   "), "");
        // Only a leading prefix is removed, like the legacy implementation.
        assert_eq!(normalize_tiktok_cookie("abc:cookie:def"), "abc:cookie:def");
    }

    #[test]
    fn youtube_cookie_normalisation_reuses_the_same_rule() {
        assert_eq!(normalize_youtube_cookie("  cookie:abc  "), "abc");
        assert_eq!(normalize_youtube_cookie("COOKIE: abc"), "abc");
        assert_eq!(normalize_youtube_cookie("Cookie:abc"), "abc");
        assert_eq!(normalize_youtube_cookie("abc"), "abc");
        assert_eq!(normalize_youtube_cookie(""), "");
        assert_eq!(normalize_youtube_cookie("   "), "");
        assert_eq!(normalize_youtube_cookie("abc:cookie:def"), "abc:cookie:def");
    }

    #[test]
    fn database_directory_is_inferred_from_the_url() {
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
    fn temporary_path_has_the_expected_suffix() {
        assert_eq!(
            temporary_path(Path::new("data/admin_runtime.json")),
            PathBuf::from("data/admin_runtime.json.tmp")
        );
    }
}
