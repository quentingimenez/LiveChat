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
//!
//! A `playOnce` ingestion ([`BoardTarget::PlayOnce`]) skips the board entirely:
//! the ready asset becomes a playback job for the guild's overlays (the same
//! creation as `http/ingest/submit.rs` without `saveToBoard`), the registry
//! keeps the `jobId` and only the requesting client is notified — no
//! `board.changed`, since no item was added.

use std::path::PathBuf;

use lc_domain::entities::{MediaAsset, PlaybackJob};
use lc_domain::enums::JobSource;
use lc_domain::{MediaBoardItemId, MediaKind};
use lc_media::edit::EditRecipe;
use lc_protocol::{
    BoardChange, BoardChangedPayload, MediaIngestionError, MediaIngestionPayload, ServerMessage,
    MAX_MEDIA_TEXT_CHARS,
};

use crate::app::AppState;
use crate::media::hook;
use crate::media::jobs::{self, JobSpec};
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
    /// No board item: the media is played once on the guild's overlays.
    /// `display_duration_sec` is the display time requested with the play
    /// (1..=600 s); it is ignored unless the ready asset is an image.
    PlayOnce { display_duration_sec: Option<i64> },
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
        /// Re-run the pipeline instead of reusing the cached asset.
        force_refresh: bool,
    },
    /// An upload already known to the guild: no bytes are received, the cached
    /// asset is attached to the target.
    Cached {
        /// Upload cache key (`upload:sha256:<hex>`) proven by the lookup.
        source_hash: String,
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
/// `message` is the text carried by the ingestion: the caption of a
/// [`BoardTarget::PlayOnce`] play, or the hand-written board message copied by
/// an edit's new item ([`BoardTarget::NewItem`]). Uploads and links pass `None`
/// for a board add: a text sent with a media is never stored on the board.
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
            force_refresh,
        } => {
            let path = temp.file_path();
            let result = service
                .ingest_file_with_cache(
                    &path,
                    &filename,
                    content_type.as_deref(),
                    &content_sha256,
                    force_refresh,
                )
                .await;
            // The temporary directory is owned by the task and always removed
            // once the ingest attempt is over (success or failure).
            drop(temp);
            if let Ok(asset) = &result {
                // The bytes were received: this guild may now import the same
                // content without uploading them again. A failure here must not
                // fail the ingestion, the media is already stored.
                if let Err(error) = service
                    .db()
                    .guild_upload_cache()
                    .record(
                        auth.guild_id(),
                        asset.source_hash.as_str(),
                        state.clock.now(),
                    )
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        source_hash = %asset.source_hash,
                        "enregistrement de l'empreinte d'upload impossible"
                    );
                }
            }
            result.map_err(|service_error| upload_service_failure(&service_error))
        }
        IngestionInput::Cached { source_hash } => match service.cached_upload(&source_hash).await {
            Ok(Some(asset)) => Ok(asset),
            Ok(None) => Err(IngestionFailure {
                code: "cache_miss",
                message: None,
            }),
            Err(service_error) => Err(upload_service_failure(&service_error)),
        },
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
        (Ok(asset), BoardTarget::NewItem) => complete_board_item(
            &state,
            &auth,
            service.db(),
            asset,
            // An empty title falls back to the asset's source title.
            ItemTitle::Requested(title),
            message,
            state.clock.now(),
        )
        .await
        .map(CompletedIngestion::BoardItem),
        (Ok(asset), BoardTarget::Replace(item_id)) => replace_board_item_media(
            &state,
            &auth,
            service.db(),
            item_id,
            asset,
            state.clock.now(),
        )
        .await
        .map(CompletedIngestion::BoardItem),
        (
            Ok(asset),
            BoardTarget::PlayOnce {
                display_duration_sec,
            },
        ) => complete_play_once(
            &state,
            &auth,
            service.db(),
            asset,
            message,
            display_duration_sec,
            state.clock.now(),
        )
        .await
        .map(CompletedIngestion::PlaybackJob),
        (Err(failure), _) => Err(failure),
    };

    match completed {
        Ok(CompletedIngestion::BoardItem(completed)) => {
            finish_success(&state, &auth, &ingestion_id, completed)
        }
        Ok(CompletedIngestion::PlaybackJob(job)) => {
            finish_play_once(&state, &auth, &ingestion_id, job)
        }
        Err(failure) => finish_failure(&state, &auth, &ingestion_id, failure),
    }
}

/// Business outcome of a completed ingestion.
#[derive(Debug)]
enum CompletedIngestion {
    /// The media reached the board (new, deduplicated or replaced item).
    BoardItem(super::media_board::CompletedBoardItem),
    /// The media became a playback job without any board item (play once).
    PlaybackJob(PlaybackJob),
}

/// Author shown with a play-once job: the overlay member's live profile, else
/// the client's default author (same rule as a board playback request).
fn play_once_author(auth: &AuthedOverlayClient) -> (Option<String>, Option<String>) {
    match &auth.member {
        Some(member) => (Some(member.display_name.clone()), member.avatar.clone()),
        None => (
            auth.client.default_author_name.clone(),
            auth.client.default_author_image.clone(),
        ),
    }
}

/// Creates the playback job of a play-once ingestion (no board item).
///
/// Mirrors `http/ingest/submit.rs` when `saveToBoard` is false: the ready asset
/// becomes a `PENDING` job for the guild — no quota check, no pinning — the
/// message is shown with it, the overlay member is the author, and the realtime
/// hook is notified so the playback engine picks the job up at once.
///
/// `display_duration_sec` is the display time chosen with the play. It only
/// applies to an image: a video or an audio keeps its own length (and
/// [`jobs::resolve_duration`] ignores it for them anyway).
async fn complete_play_once(
    state: &AppState,
    auth: &AuthedOverlayClient,
    db: &lc_db::Db,
    asset: MediaAsset,
    message: Option<String>,
    display_duration_sec: Option<i64>,
    now: time::UtcDateTime,
) -> Result<PlaybackJob, IngestionFailure> {
    let (author_name, author_image) = play_once_author(auth);
    let text = message
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.chars().take(MAX_MEDIA_TEXT_CHARS).collect::<String>());
    let duration_sec = display_duration_sec.filter(|_| asset.kind == MediaKind::Image);
    let spec = JobSpec {
        guild_id: auth.guild_id().clone(),
        media: Some(&asset),
        text: text.as_deref(),
        show_text: text.is_some(),
        author_name: author_name.as_deref(),
        author_image: author_image.as_deref(),
        duration_sec,
        source: JobSource::Overlay,
        source_label: Some("overlay"),
    };
    match jobs::create_for_ingest(db, state.runtime().default_duration(), spec, now).await {
        Ok(job) => {
            hook::notify_ingested(&job).await;
            Ok(job)
        }
        Err(service_error) => Err(media_service_failure(&service_error)),
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
        job_id: None,
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

/// Updates the registry and sends the `media.ingestion` payload of a
/// play-once, carrying the created job instead of a board item.
///
/// Nothing was added to the board, so no `board.changed` is broadcast.
fn finish_play_once(
    state: &AppState,
    auth: &AuthedOverlayClient,
    ingestion_id: &str,
    job: PlaybackJob,
) {
    let job_id = job.id.to_canonical();
    state
        .ingestions
        .complete_job(ingestion_id, job_id.clone(), state.clock.now());

    let payload = MediaIngestionPayload {
        ingestion_id: ingestion_id.to_owned(),
        ok: true,
        created: Some(false),
        item_id: None,
        job_id: Some(job_id),
        error: None,
    };
    state.admin_hub.send_to_client(
        &auth.client.id.as_uuid(),
        ServerMessage::MediaIngestion(payload),
    );
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
        job_id: None,
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
