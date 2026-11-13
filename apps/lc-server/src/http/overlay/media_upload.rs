//! `POST /overlay/media-board/upload` — add a board media from a local file.
//!
//! The body is the raw file (no multipart, no JSON). It is streamed to a
//! temporary directory in the same space as pipeline downloads, with a
//! `MEDIA_MAX_SIZE_MB` cap and an incremental SHA-256. The temporary file is
//! removed by an RAII guard in every case (success, error, handler drop).
//!
//! The route does not use `Bytes`/`Json`: `axum::body::Body` does not apply the
//! global 1 MiB `DefaultBodyLimit`, which stays active elsewhere. The route is
//! exempt from the global timeout like the other synchronous ingestions.

use std::io;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt as _;
use lc_media::error::MediaErrorCode;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;
use uuid::Uuid;

use crate::app::AppState;
use crate::media::errors::MediaServiceError;
use crate::media::service::MediaService;

use super::auth::AuthedOverlayClient;
use super::media_board::{complete_board_item, ensure_writable, media_service_error};
use super::{db, internal_error, json_response, now};

/// Maximum filename length (Unicode scalar values).
const MAX_FILENAME_CHARS: usize = 255;
/// Prefix of temporary upload directories.
const UPLOAD_TEMP_PREFIX: &str = "lc-media-upload-";
/// Number of attempts to create a unique temporary directory.
const TEMP_CREATE_ATTEMPTS: usize = 8;

/// Route query parameters (`filename` is required).
#[derive(Debug, Default, Deserialize)]
pub(super) struct UploadQuery {
    /// Original filename (default title = filename without extension).
    #[serde(default)]
    filename: Option<String>,
    /// Display title (≤ 240 characters, truncated like existing titles).
    #[serde(default)]
    title: Option<String>,
    /// Display message (≤ 500 characters).
    #[serde(default)]
    message: Option<String>,
}

/// Handler for `POST /overlay/media-board/upload`.
pub async fn upload_item(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    query: Result<Query<UploadQuery>, QueryRejection>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    // Session guard BEFORE any body read (403 read_only_session).
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }

    let query = match query {
        Ok(Query(query)) => query,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "query invalide sur POST /overlay/media-board/upload"
            );
            return invalid_payload("invalid query parameters");
        }
    };
    let filename = match validate_filename(query.filename.as_deref()) {
        Ok(filename) => filename,
        Err(response) => return response,
    };

    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let service = match MediaService::for_state(&state) {
        Ok(service) => service,
        Err(service_error) => return media_service_error(&service_error),
    };
    let max_size_bytes = service.config().download.max_size_bytes;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let temp = match UploadTempDir::create().await {
        Ok(temp) => temp,
        Err(error) => {
            tracing::error!(error = %error, "création du dossier temporaire d'upload impossible");
            return internal_error();
        }
    };
    let temp_path = temp.file_path();
    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::error!(error = %error, "création du fichier temporaire d'upload impossible");
            return internal_error();
        }
    };

    let mut stream = body.into_data_stream();
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                tracing::debug!(error = %error, "corps d'upload interrompu");
                return invalid_payload("request body could not be read");
            }
        };
        total = total.saturating_add(chunk.len() as u64);
        if total > max_size_bytes {
            drop(file);
            return payload_too_large();
        }
        if let Err(error) = file.write_all(&chunk).await {
            tracing::error!(error = %error, "écriture du fichier temporaire d'upload impossible");
            return internal_error();
        }
        hasher.update(&chunk);
    }
    if let Err(error) = file.flush().await {
        tracing::error!(error = %error, "finalisation du fichier temporaire d'upload impossible");
        return internal_error();
    }
    drop(file);

    if total == 0 {
        return invalid_payload("file is empty");
    }
    let content_sha256 = format!("{:x}", hasher.finalize());
    let title = query
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| Some(default_title(&filename)));
    let message = query
        .message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let timestamp = now(&state);

    let asset = match service
        .ingest_file(
            &temp_path,
            &filename,
            content_type.as_deref(),
            &content_sha256,
        )
        .await
    {
        Ok(asset) => asset,
        Err(service_error) => return upload_service_error(&service_error),
    };

    complete_board_item(&state, &auth, db, asset, title, message, timestamp).await
}

/// Validates the original filename without ever using it as a path.
fn validate_filename(raw: Option<&str>) -> Result<String, Response> {
    let Some(raw) = raw else {
        return Err(invalid_payload("filename is required"));
    };
    let filename = raw.trim();
    if filename.is_empty() {
        return Err(invalid_payload("filename is required"));
    }
    if filename.chars().count() > MAX_FILENAME_CHARS {
        return Err(invalid_payload("filename is too long (max 255 characters)"));
    }
    if filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
        || filename.chars().any(char::is_control)
    {
        return Err(invalid_payload("filename is invalid"));
    }
    Ok(filename.to_owned())
}

/// Default title: filename without its last extension.
fn default_title(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or(filename)
        .to_owned()
}

/// Maps upload ingestion errors.
///
/// An unsupported type is distinguishable (`UNSUPPORTED_SOURCE`) → `415`; other
/// media errors keep the board mapping (`422`, `500`).
fn upload_service_error(service_error: &MediaServiceError) -> Response {
    if service_error.media_code() == Some(MediaErrorCode::UnsupportedSource) {
        return json_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            serde_json::json!({"error": "unsupported_media"}),
        );
    }
    media_service_error(service_error)
}

/// `400 invalid_payload` with a message.
fn invalid_payload(message: &'static str) -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({"error": "invalid_payload", "message": message}),
    )
}

/// `413 payload_too_large`.
fn payload_too_large() -> Response {
    json_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        serde_json::json!({"error": "payload_too_large"}),
    )
}

/// Temporary upload directory, removed on drop.
struct UploadTempDir {
    path: PathBuf,
}

impl UploadTempDir {
    /// Creates a unique directory under the system temporary directory
    /// (TMP/TEMP), the same space as `lc-media` downloads.
    async fn create() -> io::Result<Self> {
        let base = std::env::temp_dir();
        for _ in 0..TEMP_CREATE_ATTEMPTS {
            let path = base.join(format!("{UPLOAD_TEMP_PREFIX}{}", Uuid::new_v4()));
            match tokio::fs::create_dir(&path).await {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "aucun nom de dossier temporaire d'upload disponible",
        ))
    }

    /// Path of the temporary file written while streaming.
    fn file_path(&self) -> PathBuf {
        self.path.join("upload.bin")
    }
}

impl Drop for UploadTempDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %error,
                    "nettoyage du dossier temporaire d'upload impossible"
                );
            }
        }
    }
}
