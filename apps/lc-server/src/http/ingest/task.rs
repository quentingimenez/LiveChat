//! Background processing of the `/ingest` and `/ingest/file` submissions.
//!
//! The handlers authenticate, validate and (for a file) receive the request,
//! then hand the media over to [`accept`]: it registers the ingestion in
//! [`crate::ingestions::IngestionRegistry`] under the ingest client's id,
//! answers `202 {accepted, ingestionId, guildId}` and runs the download,
//! transcode and job (or board item) creation in a detached Tokio task.
//!
//! The task holds no reference to the HTTP connection. A client whose own
//! timeout is shorter than the work (an iOS Shortcut gives up after about a
//! minute, a TikTok comment card takes about as long to render) no longer
//! cancels it by closing the connection. The client reads the result through
//! `GET /ingest/ingestions/{id}` ([`super::status`]).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use lc_domain::entities::IngestClient;

use crate::app::AppState;
use crate::ingestions::{IngestionErrorInfo, IngestionSource, StartIngestionError};
use crate::media::errors::MediaServiceError;
use crate::media::MediaService;

use super::file::TempUploadDir;
use super::simple_error;
use super::submit::{complete_ingest, Completed, IngestOutcome};

/// Media submitted by an ingest client.
pub(super) enum IngestSource {
    /// A link (`url`) or a direct media URL (`media`) from `POST /ingest`.
    Link {
        /// Shared page URL.
        url: Option<String>,
        /// Direct media URL, preferred over `url` when both are set.
        media: Option<String>,
        /// Ignore the media cache and ingest again.
        force_refresh: bool,
    },
    /// A raw file fully received by `POST /ingest/file`.
    File {
        /// Temporary directory owned by the task, removed when it ends.
        temp: TempUploadDir,
        /// Original filename (already validated).
        filename: String,
        /// Content type sent by the client, if any.
        content_type: Option<String>,
        /// SHA-256 of the received bytes (upload cache key).
        content_hash: String,
    },
}

impl IngestSource {
    /// Source recorded with the registry entry (admin console « Média »).
    fn registry_source(&self) -> IngestionSource {
        match self {
            Self::Link { url, media, .. } => {
                IngestionSource::link(media.as_deref().or(url.as_deref()).unwrap_or_default())
            }
            Self::File { filename, .. } => IngestionSource::upload(filename),
        }
    }
}

/// Everything the background task needs once the request is validated.
pub(super) struct IngestTask {
    /// Authenticated ingest client.
    pub(super) client: IngestClient,
    /// Media to ingest.
    pub(super) source: IngestSource,
    /// Text already built for playback (rich payload included).
    pub(super) job_text: Option<String>,
    /// Board title override (raw-file uploads only).
    pub(super) title: Option<String>,
    /// Whether the overlay displays `job_text`.
    pub(super) show_text: bool,
    /// Live author display name (member profile, client fallback).
    pub(super) author_name: String,
    /// Live author avatar (member profile, client fallback).
    pub(super) author_image: Option<String>,
    /// Requested display duration in seconds.
    pub(super) duration_sec: Option<i64>,
    /// Store the media on the board instead of creating a playback job.
    pub(super) save_to_board: bool,
}

/// Registers the ingestion, starts its task and answers `202`.
///
/// `429 too_many_ingestions` once the client already has
/// [`crate::ingestions::MAX_PROCESSING_PER_CLIENT`] ingestions running.
pub(super) fn accept(state: &AppState, service: MediaService, task: IngestTask) -> Response {
    let guild_id = task.client.guild_id.clone();
    let ingestion_id = match state.ingestions.start(
        guild_id.clone(),
        task.client.id.as_uuid(),
        task.source.registry_source(),
        state.clock.now(),
    ) {
        Ok(ingestion_id) => ingestion_id,
        Err(StartIngestionError::TooManyProcessing) => {
            return simple_error(StatusCode::TOO_MANY_REQUESTS, "too_many_ingestions");
        }
    };
    spawn(state.clone(), service, ingestion_id.clone(), task);
    let payload = json!({
        "accepted": true,
        "ingestionId": ingestion_id,
        "guildId": guild_id.as_str(),
    });
    (StatusCode::ACCEPTED, Json(payload)).into_response()
}

/// Spawns the detached task, and a watcher that fails the entry on a panic:
/// otherwise it would stay `processing` and hold one of the client's slots
/// until the server restarts.
fn spawn(state: AppState, service: MediaService, ingestion_id: String, task: IngestTask) {
    let handle = tokio::spawn(run(state.clone(), service, ingestion_id.clone(), task));
    tokio::spawn(async move {
        if let Err(join_error) = handle.await {
            tracing::error!(
                error = %join_error,
                ingestion_id = %ingestion_id,
                "tâche d'ingestion interrompue"
            );
            state
                .ingestions
                .complete_failure(&ingestion_id, internal_failure(), state.clock.now());
        }
    });
}

/// Processes one submission and records its outcome in the registry.
async fn run(state: AppState, service: MediaService, ingestion_id: String, task: IngestTask) {
    let result = process(&state, &service, task).await;
    let now = state.clock.now();
    match result {
        Ok(Completed::Board { created, item_id }) => {
            state
                .ingestions
                .complete_success(&ingestion_id, created, item_id, now);
        }
        Ok(Completed::Job { job_id, .. }) => {
            state.ingestions.complete_job(&ingestion_id, job_id, now);
        }
        Err(error) => {
            state
                .ingestions
                .complete_failure(&ingestion_id, failure_info(&error), now);
        }
    }
}

/// Ingests the media, then creates the board item or the playback job.
async fn process(
    state: &AppState,
    service: &MediaService,
    task: IngestTask,
) -> Result<Completed, MediaServiceError> {
    let Some(db) = state.db.as_ref() else {
        return Err(MediaServiceError::unavailable(
            "ingestion sans base de données",
        ));
    };
    let asset = match &task.source {
        IngestSource::Link {
            url,
            media,
            force_refresh,
        } => {
            service
                .ingest(url.as_deref(), media.as_deref(), *force_refresh)
                .await?
        }
        IngestSource::File {
            temp,
            filename,
            content_type,
            content_hash,
        } => {
            service
                .ingest_file(
                    &temp.file_path(),
                    filename,
                    content_type.as_deref(),
                    content_hash,
                )
                .await?
        }
    };
    // The item and the job are dated when the media is ready, not when it
    // was requested.
    let outcome = IngestOutcome {
        asset: Some(&asset),
        service: Some(service),
        job_text: task.job_text.as_deref(),
        title: task.title.as_deref(),
        show_text: task.show_text,
        author_name: &task.author_name,
        author_image: task.author_image.as_deref(),
        duration_sec: task.duration_sec,
        save_to_board: task.save_to_board,
    };
    complete_ingest(state, db, &task.client, outcome, state.clock.now()).await
}

/// Failure recorded for a media-service error.
///
/// A media error keeps the shape of the synchronous `422`: the machine code
/// `media_ingestion_failed`, the stable media code and the fixed sentence of
/// that code. The tool output stays in the logs.
fn failure_info(error: &MediaServiceError) -> IngestionErrorInfo {
    if let Some(media_error) = error.media_error() {
        let code = media_error.code();
        tracing::warn!(
            code = code.as_str(),
            detail = media_error.message(),
            raw = media_error.raw_message().unwrap_or(""),
            "ingestion média en échec"
        );
        return IngestionErrorInfo {
            code: "media_ingestion_failed".to_owned(),
            message: Some(code.user_message_fr().to_owned()),
            media_code: Some(code.as_str().to_owned()),
        };
    }
    match error {
        MediaServiceError::Unavailable(message) => {
            tracing::error!(detail = message, "service d'ingestion indisponible");
            IngestionErrorInfo {
                code: "ingest_unavailable".to_owned(),
                message: None,
                media_code: None,
            }
        }
        other => {
            tracing::error!(error = %other, "erreur interne du service d'ingestion");
            internal_failure()
        }
    }
}

/// `internal_error` failure (panic, database error).
fn internal_failure() -> IngestionErrorInfo {
    IngestionErrorInfo {
        code: "internal_error".to_owned(),
        message: None,
        media_code: None,
    }
}
