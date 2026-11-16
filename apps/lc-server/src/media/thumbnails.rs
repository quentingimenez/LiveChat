//! Thumbnails of linked videos (YouTube, TikTok, X), fetched in the background.
//!
//! An ingestion never waits for its thumbnail: once the video is `READY`,
//! [`spawn_thumbnail_fetch`] downloads the platform's picture on its own task
//! and records it as the asset's cover (`media_assets.cover_asset_id`, see
//! [`assets::attach_cover`]). The board then shows it on the video's card, and
//! every guild with that video on its board gets a `board_changed` (`updated`)
//! so its open overlays reload the card.
//!
//! At startup, [`spawn_thumbnail_backfill`] gives the same picture to board
//! videos imported before this existed.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use lc_db::Db;
use lc_domain::entities::MediaAsset;
use lc_domain::ids::MediaAssetId;
use lc_domain::{Clock, GuildId, MediaKind};
use lc_media::config::MediaConfig;
use lc_media::resolve::ResolvedMedia;
use lc_media::source::{MediaSource, SourceKind};
use lc_media::storage::MediaStorage;
use lc_media::thumbnail::{discover_thumbnail_candidates, fetch_thumbnail};
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};
use sqlx::Row;

use crate::app::AppState;
use crate::http::admin::hub::AdminHub;
use crate::media::assets;
use crate::media::settings::media_config_for_state;

/// Most board videos given a thumbnail by one startup backfill.
const BACKFILL_LIMIT: i64 = 500;

/// What a thumbnail task needs, cheap to clone.
#[derive(Clone)]
pub struct ThumbnailContext {
    pub config: Arc<MediaConfig>,
    pub storage: MediaStorage,
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub hub: Option<Arc<dyn AdminHub>>,
}

/// Sources fetched (or being fetched) by this process: a video reused from
/// the cache many times, or whose platform has no picture, is tried once.
fn attempted() -> &'static Mutex<HashSet<String>> {
    static ATTEMPTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ATTEMPTED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn first_attempt(source_hash: &str) -> bool {
    match attempted().lock() {
        Ok(mut set) => set.insert(source_hash.to_owned()),
        Err(poisoned) => poisoned.into_inner().insert(source_hash.to_owned()),
    }
}

/// `true` when a video of this source gets a platform thumbnail.
pub fn wants_thumbnail(asset: &MediaAsset, source: &MediaSource) -> bool {
    asset.kind == MediaKind::Video
        && matches!(
            source.source_kind,
            SourceKind::YouTube | SourceKind::TikTok | SourceKind::Twitter
        )
}

/// Starts fetching the thumbnail of a `READY` video, unless it already has a
/// picture or was tried before. Returns at once.
pub fn spawn_thumbnail_fetch(
    context: &ThumbnailContext,
    asset: &MediaAsset,
    source: MediaSource,
    resolved: Option<ResolvedMedia>,
) {
    if !wants_thumbnail(asset, &source) || !first_attempt(&asset.source_hash) {
        return;
    }
    let context = context.clone();
    let asset_id = asset.id;
    tokio::spawn(async move {
        fetch_and_attach(&context, asset_id, &source, resolved.as_ref()).await;
    });
}

/// Downloads the thumbnail and records it. Best effort: failures are logged.
async fn fetch_and_attach(
    context: &ThumbnailContext,
    asset_id: MediaAssetId,
    source: &MediaSource,
    resolved: Option<&ResolvedMedia>,
) {
    let assets_repo = context.db.media_assets();
    let source_hash = match (
        assets_repo.get(asset_id).await,
        assets_repo.cover_of(asset_id).await,
    ) {
        (Ok(Some(asset)), Ok(None)) if asset.is_ready() => asset.source_hash,
        (Ok(_), Ok(_)) => return,
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!(error = %error, "lecture de la miniature d'une vidéo impossible");
            return;
        }
    };
    let candidates = discover_thumbnail_candidates(&context.config, source, resolved).await;
    if candidates.is_empty() {
        return;
    }
    // Stored next to the video file (`<hash>.cover.<ext>`).
    let Some(cover) =
        fetch_thumbnail(&context.config, &context.storage, &source_hash, &candidates).await
    else {
        tracing::debug!(source = %source.canonical_url, "aucune miniature récupérée");
        return;
    };
    // Read again: the video may have been pinned to a board (longer expiry)
    // or removed while the picture was downloading.
    let asset = match context.db.media_assets().get(asset_id).await {
        Ok(Some(asset)) if asset.is_ready() => asset,
        Ok(_) => {
            let _removed = std::fs::remove_file(&cover.storage_path);
            return;
        }
        Err(error) => {
            tracing::warn!(error = %error, "relecture de la vidéo impossible");
            return;
        }
    };
    let now = context.clock.now();
    let cover_id = match assets::attach_cover(&context.db, &asset, &cover, now).await {
        Ok(cover_id) => cover_id,
        Err(error) => {
            tracing::warn!(error = %error, "miniature de la vidéo non enregistrée");
            return;
        }
    };
    if let Err(error) = align_cover_expiry(&context.db, asset.id, cover_id).await {
        tracing::warn!(error = %error, "échéance de la miniature non alignée");
    }
    tracing::info!(
        asset_id = %asset.id.to_canonical(),
        source_kind = ?source.source_kind,
        "miniature de la vidéo enregistrée"
    );
    notify_boards(context, asset.id).await;
}

/// Gives the cover the expiry of its video when the video got a later one
/// (pinned to a board) while the picture was downloading.
async fn align_cover_expiry(
    db: &Db,
    asset_id: MediaAssetId,
    cover_id: MediaAssetId,
) -> Result<(), lc_db::DbError> {
    sqlx::query(
        "UPDATE media_assets \
            SET expires_at = (SELECT expires_at FROM media_assets WHERE id = ?) \
          WHERE id = ? \
            AND expires_at < (SELECT expires_at FROM media_assets WHERE id = ?)",
    )
    .bind(asset_id.to_canonical())
    .bind(cover_id.to_canonical())
    .bind(asset_id.to_canonical())
    .execute(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;
    Ok(())
}

/// Tells each board showing this video that its card changed.
async fn notify_boards(context: &ThumbnailContext, asset_id: MediaAssetId) {
    let Some(hub) = context.hub.as_ref() else {
        return;
    };
    let rows =
        match sqlx::query("SELECT id, guild_id FROM media_board_items WHERE media_asset_id = ?")
            .bind(asset_id.to_canonical())
            .fetch_all(context.db.pool())
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "boards de la vidéo illisibles");
                return;
            }
        };
    for row in rows {
        let (Ok(item_id), Ok(guild_id)) = (
            row.try_get::<String, _>("id"),
            row.try_get::<String, _>("guild_id"),
        ) else {
            continue;
        };
        let Ok(guild_id) = GuildId::parse(&guild_id) else {
            continue;
        };
        hub.broadcast_to_guild(
            &guild_id,
            ServerMessage::BoardChanged(BoardChangedPayload {
                change: BoardChange::Updated,
                item_id,
            }),
        );
    }
}

/// Board videos from a platform that have no picture yet.
async fn videos_without_thumbnail(db: &Db) -> Result<Vec<(MediaAssetId, String)>, lc_db::DbError> {
    let rows = sqlx::query(
        "SELECT DISTINCT a.id, a.source_url FROM media_assets a \
           JOIN media_board_items b ON b.media_asset_id = a.id \
          WHERE a.kind = 'VIDEO' AND a.status = 'READY' AND a.cover_asset_id IS NULL \
          LIMIT ?",
    )
    .bind(BACKFILL_LIMIT)
    .fetch_all(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;
    let mut videos = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id").map_err(lc_db::DbError::from)?;
        let source_url: String = row.try_get("source_url").map_err(lc_db::DbError::from)?;
        if let Ok(id) = id.parse::<MediaAssetId>() {
            videos.push((id, source_url));
        }
    }
    Ok(videos)
}

/// Builds the context of the thumbnail tasks from the application state.
pub fn context_for_state(state: &AppState) -> Option<ThumbnailContext> {
    let db = state.db.clone()?;
    let config = media_config_for_state(state);
    let storage = MediaStorage::new(&config.storage);
    Some(ThumbnailContext {
        config: Arc::new(config),
        storage,
        db,
        clock: Arc::clone(&state.clock),
        hub: Some(Arc::clone(&state.admin_hub)),
    })
}

/// Gives a thumbnail to the board videos that have none, one after the other,
/// on a background task. YouTube and TikTok only: an X post needs its full
/// resolution to find its poster.
pub fn spawn_thumbnail_backfill(state: &AppState) {
    let Some(context) = context_for_state(state) else {
        return;
    };
    tokio::spawn(async move {
        let videos = match videos_without_thumbnail(&context.db).await {
            Ok(videos) => videos,
            Err(error) => {
                tracing::warn!(error = %error, "vidéos sans miniature illisibles");
                return;
            }
        };
        for (asset_id, source_url) in videos {
            let Some(source) = MediaSource::from_raw(&source_url) else {
                continue;
            };
            if !matches!(source.source_kind, SourceKind::YouTube | SourceKind::TikTok)
                || !first_attempt(&source.source_hash)
            {
                continue;
            }
            fetch_and_attach(&context, asset_id, &source, None).await;
        }
    });
}
