//! Hot server settings exposed to the console.
//!
//! `GET /admin/api/runtime-settings` returns the modifiable values with their
//! source (`runtime` when `admin_runtime.json` overrides the environment) and
//! the environment default, plus the read-only settings whose value never
//! changes while the process runs.
//!
//! `PATCH` accepts any subset of the modifiable keys. A `null` value removes the
//! override and brings the environment value back. Values are applied to the
//! in-memory [`Runtime`](crate::runtime::Runtime) before the response, so the
//! next request (overview, WebSocket handshake, ingestion, speech…) uses them
//! without a restart.
//!
//! The legacy TikTok cookie stays writable through `PATCH` and is still
//! persisted in clear text in `admin_runtime.json`, but it is never returned:
//! the payload only exposes `hasTikTokCookie`, and the value is normalised like
//! the legacy `normalizeTikTokCookie`.
//!
//! # Persistence
//!
//! `admin_runtime.json` lives next to the SQLite database (see
//! [`crate::runtime::settings_path`]). Writes are atomic (`<file>.tmp` then
//! `rename`); a failed write leaves the process on its previous values.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::auth::AdminAccess;
use super::AdminError;
use crate::app::AppState;
use crate::runtime::normalize_tiktok_cookie;

/// Name of the runtime settings file, kept here for the admin tests.
#[cfg(test)]
pub use crate::runtime::SETTINGS_FILE_NAME;

/// `GET /admin/api/runtime-settings`.
pub(super) async fn get(
    State(state): State<AppState>,
    _access: AdminAccess,
) -> Result<Response, AdminError> {
    Ok(Json(settings_payload(&state)).into_response())
}

/// `PATCH /admin/api/runtime-settings`.
pub(super) async fn patch(
    State(state): State<AppState>,
    _access: AdminAccess,
    payload: Json<Value>,
) -> Result<Response, AdminError> {
    let object = payload
        .as_object()
        .filter(|object| !object.is_empty())
        .ok_or_else(invalid_payload)?;

    let mut overrides = state.runtime().snapshot();
    for (key, value) in object {
        match key.as_str() {
            "tiktokCookie" => {
                let cookie = value.as_str().ok_or_else(invalid_payload)?;
                overrides.tiktok_cookie = Some(normalize_tiktok_cookie(cookie));
            }
            "defaultDuration" => overrides.default_duration = nullable_u64(value)?,
            "mediaCacheTtlHours" => overrides.media_cache_ttl_hours = nullable_u64(value)?,
            "mediaCacheMaxTotalMb" => overrides.media_cache_max_total_mb = nullable_u64(value)?,
            "mediaBoardMaxTotalMb" => overrides.media_board_max_total_mb = nullable_u64(value)?,
            "mediaMaxSizeMb" => overrides.media_max_size_mb = nullable_u64(value)?,
            "mediaVideoMaxHeight" => overrides.media_video_max_height = nullable_u32(value)?,
            "minOverlayVersion" => overrides.min_overlay_version = nullable_version(value)?,
            "ttsEnabled" => overrides.tts_enabled = nullable_bool(value)?,
            unknown => {
                tracing::debug!(key = unknown, "unknown runtime setting in admin PATCH");
                return Err(invalid_payload());
            }
        }
    }

    state.runtime().persist(&overrides).await.map_err(|error| {
        tracing::error!(error = %error, "writing the admin runtime settings failed");
        AdminError::Internal
    })?;
    state.runtime().apply(overrides);

    let mut body = settings_payload(&state);
    if let Value::Object(object) = &mut body {
        object.insert("updated".to_owned(), Value::Bool(true));
    }
    Ok(Json(body).into_response())
}

/// `400 invalid_payload`.
fn invalid_payload() -> AdminError {
    AdminError::bad_request("invalid_payload")
}

/// `null` resets to the environment value; otherwise a non-negative integer.
fn nullable_u64(value: &Value) -> Result<Option<u64>, AdminError> {
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => number.as_u64().map(Some).ok_or_else(invalid_payload),
        _ => Err(invalid_payload()),
    }
}

/// `null` resets to the environment value; otherwise a `u32` integer.
fn nullable_u32(value: &Value) -> Result<Option<u32>, AdminError> {
    match nullable_u64(value)? {
        Some(number) => u32::try_from(number)
            .map(Some)
            .map_err(|_| invalid_payload()),
        None => Ok(None),
    }
}

/// `null` resets to the environment value; otherwise a boolean.
fn nullable_bool(value: &Value) -> Result<Option<bool>, AdminError> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(boolean) => Ok(Some(*boolean)),
        _ => Err(invalid_payload()),
    }
}

/// `null` resets to the environment value; otherwise a semver-like version.
fn nullable_version(value: &Value) -> Result<Option<String>, AdminError> {
    match value {
        Value::Null => Ok(None),
        Value::String(version) => {
            let version = version.trim();
            if lc_protocol::parse_simple_version(version).is_none() {
                return Err(invalid_payload());
            }
            Ok(Some(version.to_owned()))
        }
        _ => Err(invalid_payload()),
    }
}

/// JSON body shared by `GET` and the `PATCH` response.
fn settings_payload(state: &AppState) -> Value {
    let overrides = state.runtime().snapshot();
    let config = &state.config;
    let tiktok_cookie = state.runtime().tiktok_cookie();

    let setting = |value: Value, runtime: bool, default: Value| {
        json!({
            "value": value,
            "source": if runtime { "runtime" } else { "env" },
            "default": default,
        })
    };

    json!({
        "hasTikTokCookie": !tiktok_cookie.is_empty(),
        "settings": {
            "defaultDuration": setting(
                json!(state.runtime().default_duration()),
                overrides.default_duration.is_some(),
                json!(config.default_duration),
            ),
            "mediaCacheTtlHours": setting(
                json!(state.runtime().media_cache_ttl_hours()),
                overrides.media_cache_ttl_hours.is_some(),
                json!(config.media_cache_ttl_hours),
            ),
            "mediaCacheMaxTotalMb": setting(
                json!(state.runtime().media_cache_max_total_mb()),
                overrides.media_cache_max_total_mb.is_some(),
                json!(config.media_cache_max_total_mb),
            ),
            "mediaBoardMaxTotalMb": setting(
                json!(state.runtime().media_board_max_total_mb()),
                overrides.media_board_max_total_mb.is_some(),
                json!(config.media_board_max_total_mb),
            ),
            "mediaMaxSizeMb": setting(
                json!(state.runtime().media_max_size_mb()),
                overrides.media_max_size_mb.is_some(),
                json!(config.media_max_size_mb),
            ),
            "mediaVideoMaxHeight": setting(
                json!(state.runtime().media_video_max_height()),
                overrides.media_video_max_height.is_some(),
                json!(config.media_video_max_height),
            ),
            "minOverlayVersion": setting(
                json!(state.runtime().min_overlay_version()),
                overrides.min_overlay_version.is_some(),
                json!(config.min_overlay_version),
            ),
            "ttsEnabled": setting(
                json!(state.runtime().tts_enabled()),
                overrides.tts_enabled.is_some(),
                json!(config.tts_enabled),
            ),
        },
        "readOnly": read_only_payload(state),
    })
}

/// Read-only settings: environment values and presence flags, never secrets.
fn read_only_payload(state: &AppState) -> Value {
    let config = &state.config;
    json!({
        "port": config.port,
        "apiUrl": config.api_url.as_str(),
        "mediaStorageDir": config.media_storage_dir.display().to_string(),
        "databaseConfigured": state.db.is_some(),
        "discordTokenSet": config.discord_token.as_ref().is_some_and(|secret| !secret.is_empty()),
        "discordClientSecretSet": config.discord_client_secret.as_ref().is_some_and(|secret| !secret.is_empty()),
        "ingestApiTokenSet": !config.ingest_api_token.is_empty(),
        "adminSecretSet": !config.admin_ui_token.is_empty(),
        "adminDiscordUserIds": config.admin_discord_user_ids.len(),
        "adminUiLocalOnly": config.admin_ui_local_only,
        "adminTrustForwardedFor": config.admin_trust_forwarded_for,
    })
}
