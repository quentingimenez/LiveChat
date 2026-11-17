//! Detached background ingestion used by the media-board write routes (link,
//! upload, edit).
//!
//! The handlers validate the request and register an ingestion in
//! [`crate::ingestions::IngestionRegistry`]; this module owns the Tokio task that
//! performs the download/transcode, inserts the item through
//! [`super::media_board::complete_board_item`], updates the registry and notifies
//! the requesting overlay client (`media.ingestion`) plus the guild
//! (`board.changed`).
//!
//! The task holds no reference to the HTTP connection: dropping the client
//! request (proxy timeout, closed tab) never cancels the work.

use std::path::PathBuf;

use lc_domain::{MediaBoardItemId, MediaKind};
use lc_media::edit::EditRecipe;
use lc_protocol::{
    BoardChange, BoardChangedPayload, MediaIngestionError, MediaIngestionPayload, ServerMessage,
};

use crate::app::AppState;
use crate::media::service::MediaService;

use super::auth::AuthedOverlayClient;
use super::media_board::{
    complete_board_item, media_service_failure, replace_board_item_media, IngestionFailure,
    ItemTitle,
};
use super::media_upload::{upload_service_failure, UploadTempDir};

/// Where an ingested media goes on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BoardTarget {
    /// A new board item (or the existing one already holding this media).
    NewItem,
    /// The media of this existing item is replaced; the item keeps its id,
    /// title, message, author, date, favorites and picture.
    Replace(MediaBoardItemId),
}

/// The ingestion sources accepted by the board.
pub(super) enum IngestionInput {
    /// Remote source from `POST /overlay/media-board/items`.
    Link {
        /// Source URL confirmed by the synchronous validation.
        url: String,
        /// Ignore the media cache and ingest again.
        force_refresh: bool,
    },
    /// Raw file fully received through `/overlay/media-board/uploads`.
    Upload {
        /// Temporary directory owned by the task and removed when it ends.
        temp: UploadTempDir,
        /// Original filename (already validated).
        filename: String,
        /// Content type hint sent by the client, if any.
        content_type: Option<String>,
        /// SHA-256 of the received bytes (upload cache key).
        content_sha256: String,
    },
    /// Edit of a stored media from `POST /overlay/media-board/items/{id}/edit`.
    Edit {
        /// Stored file of the edited asset (already confined to the storage).
        source_path: PathBuf,
        /// Kind of the edited asset.
        kind: MediaKind,
        /// Recipe, already validated against the asset's stored duration.
        recipe: Box<EditRecipe>,
        /// Name of the result, without extension (shown as its source).
        filename_stem: String,
    },
}

/// Spawns the detached ingestion task.
///
/// The caller has already reserved an entry in `state.ingestions`; the task is
/// responsible for completing it in every path. A panic inside the task is
/// reported as `internal_error`: otherwise the entry would stay `processing`
/// and hold one of the client's slots until the server restarts.
pub(super) fn spawn_ingestion(
    state: AppState,
    auth: AuthedOverlayClient,
    ingestion_id: String,
    input: IngestionInput,
    target: BoardTarget,
    title: Option<String>,
    message: Option<String>,
) {
    let task = tokio::spawn(run_ingestion(
        state.clone(),
        auth.clone(),
        ingestion_id.clone(),
        input,
        target,
        title,
        message,
    ));
    tokio::spawn(async move {
        if let Err(join_error) = task.await {
            tracing::error!(
                error = %join_error,
                ingestion_id = %ingestion_id,
                "tâche d'ingestion média interrompue"
            );
            finish_failure(
                &state,
                &auth,
                &ingestion_id,
                IngestionFailure {
                    code: "internal_error",
                    message: None,
                },
            );
        }
    });
}

/// Runs one ingestion and publishes its result.
async fn run_ingestion(
    state: AppState,
    auth: AuthedOverlayClient,
    ingestion_id: String,
    input: IngestionInput,
    target: BoardTarget,
    title: Option<String>,
    message: Option<String>,
) {
    let service = match MediaService::for_state(&state) {
        Ok(service) => service,
        Err(service_error) => {
            finish_failure(
                &state,
                &auth,
                &ingestion_id,
                media_service_failure(&service_error),
            );
            return;
        }
    };

    let ingested = match input {
        IngestionInput::Link { url, force_refresh } => service
            .ingest(Some(&url), None, force_refresh)
            .await
            .map_err(|service_error| media_service_failure(&service_error)),
        IngestionInput::Upload {
            temp,
            filename,
            content_type,
            content_sha256,
        } => {
            let path = temp.file_path();
            let result = service
                .ingest_file(&path, &filename, content_type.as_deref(), &content_sha256)
                .await;
            // The temporary directory is owned by the task and always removed
            // once the ingest attempt is over (success or failure).
            drop(temp);
            result.map_err(|service_error| upload_service_failure(&service_error))
        }
        IngestionInput::Edit {
            source_path,
            kind,
            recipe,
            filename_stem,
        } => service
            .ingest_edit(&source_path, kind, &recipe, &filename_stem)
            .await
            .map_err(|service_error| media_service_failure(&service_error)),
    };

    // The item is dated when it reaches the board, not when it was requested:
    // a long conversion must not land below the media added in the meantime.
    let completed = match (ingested, target) {
        (Ok(asset), BoardTarget::NewItem) => {
            complete_board_item(
                &state,
                &auth,
                service.db(),
                asset,
                // Titre vide → titre source de l'asset (vrai titre du lien).
                ItemTitle::Requested(title),
                message,
                state.clock.now(),
            )
            .await
        }
        (Ok(asset), BoardTarget::Replace(item_id)) => {
            replace_board_item_media(
                &state,
                &auth,
                service.db(),
                item_id,
                asset,
                state.clock.now(),
            )
            .await
        }
        (Err(failure), _) => Err(failure),
    };

    match completed {
        Ok(completed) => finish_success(&state, &auth, &ingestion_id, completed),
        Err(failure) => finish_failure(&state, &auth, &ingestion_id, failure),
    }
}

/// Updates the registry, sends `media.ingestion`, and broadcasts
/// `board.changed`: `added` for a new item, `updated` for a replaced media.
fn finish_success(
    state: &AppState,
    auth: &AuthedOverlayClient,
    ingestion_id: &str,
    completed: super::media_board::CompletedBoardItem,
) {
    let item_id = completed.item.id.to_canonical();
    state.ingestions.complete_success(
        ingestion_id,
        completed.created,
        item_id.clone(),
        state.clock.now(),
    );

    let payload = MediaIngestionPayload {
        ingestion_id: ingestion_id.to_owned(),
        ok: true,
        created: Some(completed.created),
        item_id: Some(item_id.clone()),
        error: None,
    };
    state.admin_hub.send_to_client(
        &auth.client.id.as_uuid(),
        ServerMessage::MediaIngestion(payload),
    );

    let change = if completed.created {
        Some(BoardChange::Added)
    } else if completed.replaced {
        Some(BoardChange::Updated)
    } else {
        None
    };
    if let Some(change) = change {
        state.admin_hub.broadcast_to_guild(
            auth.guild_id(),
            ServerMessage::BoardChanged(BoardChangedPayload { change, item_id }),
        );
    }
}

/// Updates the registry and sends the failed `media.ingestion` payload.
fn finish_failure(
    state: &AppState,
    auth: &AuthedOverlayClient,
    ingestion_id: &str,
    failure: IngestionFailure,
) {
    state.ingestions.complete_failure(
        ingestion_id,
        crate::ingestions::IngestionErrorInfo {
            code: failure.code.to_owned(),
            message: failure.message.clone(),
        },
        state.clock.now(),
    );

    let payload = MediaIngestionPayload {
        ingestion_id: ingestion_id.to_owned(),
        ok: false,
        created: None,
        item_id: None,
        error: Some(MediaIngestionError {
            code: failure.code.to_owned(),
            message: failure.message,
        }),
    };
    state.admin_hub.send_to_client(
        &auth.client.id.as_uuid(),
        ServerMessage::MediaIngestion(payload),
    );
}
