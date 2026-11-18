//! Board uploads of local files, sent in parts.
//!
//! A reverse proxy in front of the server may cap the request body (Cloudflare
//! refuses bodies over 100 MB on its free plan), so a file travels as several
//! requests:
//!
//! 1. `POST /overlay/media-board/uploads` (JSON: `filename`, `size`, optional
//!    `title`, `message`, `contentType`, `replaceItemId`, `playOnce`) opens an
//!    upload session and answers `201 {uploadId}`. The declared size is checked
//!    against `MEDIA_MAX_SIZE_MB` before any byte is sent (`413
//!    {error: payload_too_large, maxBytes}`), and a replaced item and the right
//!    to edit it are checked here. `playOnce` (right `TriggerPlayback`) keeps
//!    the media off the board: it is played once on the guild's overlays.
//! 2. `PUT /overlay/media-board/uploads/{uploadId}/parts?offset=N` appends a
//!    raw body of at most [`MAX_PART_BYTES`] at byte `N`, which must be the
//!    number of bytes already received. It answers `200 {received}`.
//! 3. `POST /overlay/media-board/uploads/{uploadId}/complete` checks that the
//!    declared size was received, registers an ingestion and answers `202
//!    {ingestionId}`; the temporary directory is moved into the detached
//!    [`super::media_ingestion`] task and removed when that task ends.
//! 4. `DELETE /overlay/media-board/uploads/{uploadId}` abandons a session.
//!
//! A session belongs to the overlay client that opened it. Parts are streamed
//! to a temporary file with an incremental SHA-256, so a failed part cannot be
//! sent again: any part error discards the session and the client starts over.
//! Sessions left idle for [`IDLE_TIMEOUT`] are discarded.
//!
//! The part route does not use `Bytes`/`Json`: `axum::body::Body` does not
//! apply the global 1 MiB `DefaultBodyLimit`, which stays active elsewhere. It
//! is exempt from the global timeout because a part can take longer than the
//! ordinary request budget on a slow link.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::Json;
use futures_util::StreamExt as _;
use lc_domain::{GuildId, Permission};
use lc_media::error::MediaErrorCode;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;
use uuid::Uuid;

use crate::app::AppState;
use crate::ingestions::StartIngestionError;
use crate::media::errors::MediaServiceError;
use crate::media::service::MediaService;

use super::auth::AuthedOverlayClient;
use super::guild::require;
use super::media_board::{
    ensure_can_edit, ensure_writable, guild_item, media_service_error, too_many_ingestions,
    IngestionFailure,
};
use super::media_ingestion::{spawn_ingestion, BoardTarget, IngestionInput};
use super::{db, internal_error, json_response, now};

/// Largest body accepted for one part (90 MiB, under Cloudflare's 100 MB).
pub const MAX_PART_BYTES: u64 = 90 * 1024 * 1024;
/// A session without any request for this long is discarded.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Open sessions allowed per overlay client.
pub const MAX_SESSIONS_PER_CLIENT: usize = 8;
/// Maximum filename length (Unicode scalar values).
const MAX_FILENAME_CHARS: usize = 255;
/// Prefix of temporary upload directories.
const UPLOAD_TEMP_PREFIX: &str = "lc-media-upload-";
/// Number of attempts to create a unique temporary directory.
const TEMP_CREATE_ATTEMPTS: usize = 8;

/// Body of `POST /overlay/media-board/uploads`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateUploadBody {
    /// Original filename (default title = filename without extension).
    #[serde(default)]
    filename: Option<String>,
    /// Total size of the file, in bytes.
    #[serde(default)]
    size: Option<u64>,
    /// Display title (≤ 240 characters, truncated like existing titles).
    #[serde(default)]
    title: Option<String>,
    /// Text shown for a play-once only (≤ 500 characters). It is never stored
    /// on the board: an add-to-board ignores it.
    #[serde(default)]
    message: Option<String>,
    /// Content type hint of the file.
    #[serde(default)]
    content_type: Option<String>,
    /// Board item whose media is replaced by the uploaded file.
    #[serde(default)]
    replace_item_id: Option<String>,
    /// `true` : le fichier est joué une fois sur les overlays de la guilde
    /// sans créer d'item de board (droit `TriggerPlayback` au lieu
    /// d'`AddMedia`).
    #[serde(default)]
    play_once: bool,
}

/// Query of `PUT /overlay/media-board/uploads/{uploadId}/parts`.
#[derive(Debug, Default, Deserialize)]
pub(super) struct PartQuery {
    /// Byte position of the part in the file.
    #[serde(default)]
    offset: Option<u64>,
}

/// An upload being received.
struct UploadSession {
    target: BoardTarget,
    /// The media is played once instead of reaching the board.
    play_once: bool,
    filename: String,
    content_type: Option<String>,
    title: Option<String>,
    /// Caption of a play-once; `None` for anything that joins the board.
    play_text: Option<String>,
    /// Declared total size.
    size: u64,
    /// Bytes written so far.
    received: u64,
    hasher: Sha256,
    file: tokio::fs::File,
    temp: UploadTempDir,
}

/// A registered session. The inner mutex serializes the parts; it holds
/// `None` once the session was completed or failed.
struct SessionSlot {
    guild_id: GuildId,
    client_id: Uuid,
    touched: StdMutex<Instant>,
    session: tokio::sync::Mutex<Option<UploadSession>>,
}

impl SessionSlot {
    fn touch(&self) {
        *self
            .touched
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Instant::now();
    }

    fn idle_since(&self, now: Instant) -> Duration {
        let touched = *self
            .touched
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        now.saturating_duration_since(touched)
    }
}

/// Open upload sessions, shared by the handlers through [`AppState`].
#[derive(Default)]
pub struct UploadSessions {
    slots: StdMutex<HashMap<Uuid, Arc<SessionSlot>>>,
}

impl std::fmt::Debug for UploadSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UploadSessions")
            .field("open", &self.len())
            .finish()
    }
}

impl UploadSessions {
    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, Arc<SessionSlot>>> {
        self.slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Number of open sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether no session is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Discards sessions idle for at least [`IDLE_TIMEOUT`] at `now`. A session
    /// receiving a part is never idle.
    pub fn purge_idle(&self, now: Instant) {
        self.lock().retain(|_, slot| {
            slot.session.try_lock().is_err() || slot.idle_since(now) < IDLE_TIMEOUT
        });
    }

    /// Registers a session; `None` once the client has
    /// [`MAX_SESSIONS_PER_CLIENT`] open sessions.
    fn insert(&self, guild_id: GuildId, client_id: Uuid, session: UploadSession) -> Option<Uuid> {
        self.purge_idle(Instant::now());
        let mut slots = self.lock();
        let open = slots
            .values()
            .filter(|slot| slot.client_id == client_id)
            .count();
        if open >= MAX_SESSIONS_PER_CLIENT {
            return None;
        }
        let upload_id = Uuid::new_v4();
        slots.insert(
            upload_id,
            Arc::new(SessionSlot {
                guild_id,
                client_id,
                touched: StdMutex::new(Instant::now()),
                session: tokio::sync::Mutex::new(Some(session)),
            }),
        );
        Some(upload_id)
    }

    /// The session if it belongs to this client.
    fn get(&self, upload_id: Uuid, auth: &AuthedOverlayClient) -> Option<Arc<SessionSlot>> {
        self.purge_idle(Instant::now());
        self.lock()
            .get(&upload_id)
            .filter(|slot| {
                slot.client_id == auth.client.id.as_uuid() && &slot.guild_id == auth.guild_id()
            })
            .cloned()
    }

    fn remove(&self, upload_id: Uuid) {
        self.lock().remove(&upload_id);
    }
}

/// Handler for `POST /overlay/media-board/uploads`.
pub async fn create_upload(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    body: Result<Json<CreateUploadBody>, JsonRejection>,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let body = match body {
        Ok(Json(body)) => body,
        Err(rejection) => {
            tracing::debug!(
                error = %rejection.body_text(),
                "corps invalide sur POST /overlay/media-board/uploads"
            );
            return invalid_payload("invalid upload description");
        }
    };
    // « Jouer une fois » déclenche une lecture au lieu d'ajouter au board :
    // c'est le droit d'un déclenchement manuel (`TriggerPlayback`), pas
    // `AddMedia` (403 comme les autres refus).
    if body.play_once {
        if let Err(response) = require(&state, &auth, Permission::TriggerPlayback) {
            return response;
        }
    }
    let filename = match validate_filename(body.filename.as_deref()) {
        Ok(filename) => filename,
        Err(response) => return response,
    };
    let size = match body.size {
        Some(0) => return invalid_payload("file is empty"),
        Some(size) => size,
        None => return invalid_payload("size is required"),
    };

    // A missing database is reported at once (503): the ingestion cannot
    // persist the media without it.
    let db = match db(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let service = match MediaService::for_state(&state) {
        Ok(service) => service,
        Err(service_error) => return media_service_error(&service_error),
    };
    let max_size_bytes = service.config().download.max_size_bytes;
    if size > max_size_bytes {
        return payload_too_large(max_size_bytes);
    }
    // A play-once never touches the board, so it has nothing to replace.
    let replace_item_id = body
        .replace_item_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if body.play_once && replace_item_id.is_some() {
        return invalid_payload("playOnce cannot replace a board item");
    }
    // A replacement is checked before any byte is sent: the item must belong
    // to the guild (404) and the caller must be allowed to edit it (403).
    let target = match replace_item_id {
        Some(item_id_raw) => {
            let item = match guild_item(
                db,
                &auth,
                item_id_raw,
                "overlay_media_upload.replace_lookup",
            )
            .await
            {
                Ok(item) => item,
                Err(response) => return response,
            };
            if let Err(response) = ensure_can_edit(&state, &auth, item.id).await {
                return response;
            }
            BoardTarget::Replace(item.id)
        }
        None => BoardTarget::NewItem,
    };

    let temp = match UploadTempDir::create().await {
        Ok(temp) => temp,
        Err(error) => {
            tracing::error!(error = %error, "création du dossier temporaire d'upload impossible");
            return internal_error();
        }
    };
    let file = match tokio::fs::File::create(temp.file_path()).await {
        Ok(file) => file,
        Err(error) => {
            tracing::error!(error = %error, "création du fichier temporaire d'upload impossible");
            return internal_error();
        }
    };
    let session = UploadSession {
        target,
        play_once: body.play_once,
        title: non_blank(body.title.as_deref()).or_else(|| Some(default_title(&filename))),
        // The text only belongs to a play-once; a board add keeps none.
        play_text: if body.play_once {
            non_blank(body.message.as_deref())
        } else {
            None
        },
        content_type: non_blank(body.content_type.as_deref()),
        filename,
        size,
        received: 0,
        hasher: Sha256::new(),
        file,
        temp,
    };
    let Some(upload_id) =
        state
            .uploads
            .insert(auth.guild_id().clone(), auth.client.id.as_uuid(), session)
    else {
        return json_response(
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!({ "error": "too_many_uploads" }),
        );
    };
    json_response(
        StatusCode::CREATED,
        serde_json::json!({ "uploadId": upload_id }),
    )
}

/// Handler for `PUT /overlay/media-board/uploads/{uploadId}/parts`.
pub async fn put_part(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(upload_id): Path<String>,
    query: Result<Query<PartQuery>, QueryRejection>,
    body: Body,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let Some(offset) = query.ok().and_then(|Query(query)| query.offset) else {
        return invalid_payload("offset is required");
    };
    let Some((upload_id, slot)) = session_slot(&state, &auth, &upload_id) else {
        return upload_not_found();
    };
    let Ok(mut guard) = slot.session.try_lock() else {
        return upload_conflict("upload_busy", None);
    };
    let Some(session) = guard.as_mut() else {
        return upload_not_found();
    };
    if offset != session.received {
        return upload_conflict("upload_offset_mismatch", Some(session.received));
    }
    slot.touch();

    match receive_part(session, body).await {
        Ok(()) => {
            let received = session.received;
            slot.touch();
            json_response(StatusCode::OK, serde_json::json!({ "received": received }))
        }
        Err(response) => {
            // The hash cannot go back: the session is over.
            *guard = None;
            state.uploads.remove(upload_id);
            response
        }
    }
}

/// Streams one part to the temporary file.
async fn receive_part(session: &mut UploadSession, body: Body) -> Result<(), Response> {
    let mut stream = body.into_data_stream();
    let mut part_bytes = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            tracing::debug!(error = %error, "partie d'upload interrompue");
            invalid_payload("request body could not be read")
        })?;
        let length = chunk.len() as u64;
        part_bytes = part_bytes.saturating_add(length);
        if part_bytes > MAX_PART_BYTES {
            return Err(json_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                serde_json::json!({ "error": "part_too_large", "maxBytes": MAX_PART_BYTES }),
            ));
        }
        if session.received.saturating_add(length) > session.size {
            return Err(invalid_payload("more bytes than the declared size"));
        }
        session.file.write_all(&chunk).await.map_err(|error| {
            tracing::error!(error = %error, "écriture du fichier temporaire d'upload impossible");
            internal_error()
        })?;
        session.hasher.update(&chunk);
        session.received += length;
    }
    session.file.flush().await.map_err(|error| {
        tracing::error!(error = %error, "écriture du fichier temporaire d'upload impossible");
        internal_error()
    })
}

/// Handler for `POST /overlay/media-board/uploads/{uploadId}/complete`.
pub async fn complete_upload(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(upload_id): Path<String>,
) -> Response {
    if let Err(response) = ensure_writable(&auth) {
        return response;
    }
    let Some((upload_id, slot)) = session_slot(&state, &auth, &upload_id) else {
        return upload_not_found();
    };
    let Ok(mut guard) = slot.session.try_lock() else {
        return upload_conflict("upload_busy", None);
    };
    let Some(session) = guard.take() else {
        return upload_not_found();
    };
    state.uploads.remove(upload_id);
    if session.received != session.size {
        return invalid_payload("the file was not fully received");
    }
    let UploadSession {
        target,
        play_once,
        filename,
        content_type,
        title,
        play_text,
        hasher,
        file,
        temp,
        ..
    } = session;
    drop(file);
    // A play-once keeps the media off the board: the ingestion turns it into
    // a playback job instead of an item.
    let target = if play_once {
        BoardTarget::PlayOnce
    } else {
        target
    };

    let ingestion_id = match state.ingestions.start(
        auth.guild_id().clone(),
        auth.client.id.as_uuid(),
        now(&state),
    ) {
        Ok(ingestion_id) => ingestion_id,
        Err(StartIngestionError::TooManyProcessing) => return too_many_ingestions(),
    };
    // The task owns the temporary directory and removes it when it finishes.
    spawn_ingestion(
        state,
        auth,
        ingestion_id.clone(),
        IngestionInput::Upload {
            temp,
            filename,
            content_type,
            content_sha256: format!("{:x}", hasher.finalize()),
        },
        target,
        title,
        play_text,
    );
    json_response(
        StatusCode::ACCEPTED,
        serde_json::json!({ "ingestionId": ingestion_id }),
    )
}

/// Handler for `DELETE /overlay/media-board/uploads/{uploadId}`.
///
/// A part still being received keeps its own reference and ends normally; the
/// session is gone for every later request.
pub async fn cancel_upload(
    State(state): State<AppState>,
    auth: AuthedOverlayClient,
    Path(upload_id): Path<String>,
) -> Response {
    let Some((upload_id, _slot)) = session_slot(&state, &auth, &upload_id) else {
        return upload_not_found();
    };
    state.uploads.remove(upload_id);
    StatusCode::NO_CONTENT.into_response()
}

/// Looks up a session of the caller from the raw path segment.
fn session_slot(
    state: &AppState,
    auth: &AuthedOverlayClient,
    raw: &str,
) -> Option<(Uuid, Arc<SessionSlot>)> {
    let upload_id = Uuid::parse_str(raw.trim()).ok()?;
    state
        .uploads
        .get(upload_id, auth)
        .map(|slot| (upload_id, slot))
}

fn non_blank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
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

/// Maps upload ingestion errors for the asynchronous `media.ingestion` report.
///
/// An unsupported type is distinguishable (`UNSUPPORTED_SOURCE`) →
/// `unsupported_media`; other media errors keep the board vocabulary
/// (`media_ingestion_failed`, `internal_error`).
pub(super) fn upload_service_failure(service_error: &MediaServiceError) -> IngestionFailure {
    if service_error.media_code() == Some(MediaErrorCode::UnsupportedSource) {
        return IngestionFailure {
            code: "unsupported_media",
            message: None,
        };
    }
    super::media_board::media_service_failure(service_error)
}

/// `400 invalid_payload` with a message.
fn invalid_payload(message: &'static str) -> Response {
    json_response(
        StatusCode::BAD_REQUEST,
        serde_json::json!({"error": "invalid_payload", "message": message}),
    )
}

/// `413 payload_too_large` with the server's limit.
fn payload_too_large(max_bytes: u64) -> Response {
    json_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        serde_json::json!({"error": "payload_too_large", "maxBytes": max_bytes}),
    )
}

/// `404 upload_not_found`.
fn upload_not_found() -> Response {
    json_response(
        StatusCode::NOT_FOUND,
        serde_json::json!({"error": "upload_not_found"}),
    )
}

/// `409` with the bytes already received when known.
fn upload_conflict(code: &'static str, received: Option<u64>) -> Response {
    let body = match received {
        Some(received) => serde_json::json!({ "error": code, "received": received }),
        None => serde_json::json!({ "error": code }),
    };
    json_response(StatusCode::CONFLICT, body)
}

/// Temporary upload directory, removed on drop.
///
/// Owned by the upload session until the whole file is received, then moved
/// into the detached ingestion task.
pub(super) struct UploadTempDir {
    path: PathBuf,
}

impl UploadTempDir {
    /// Creates a unique directory under the system temporary directory
    /// (TMP/TEMP), the same space as `lc-media` downloads.
    pub(super) async fn create() -> io::Result<Self> {
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
    pub(super) fn file_path(&self) -> PathBuf {
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
