//! Cover of a video (YouTube, TikTok, X, or an upload), made in the background.
//!
//! An ingestion never waits for its cover: once the video is `READY`,
//! [`spawn_cover_fetch`] works on its own task. It tries the platform's
//! thumbnail first ([`platform_cover`]), then a frame of the video itself
//! (`lc_media::poster`) when the source has none — an upload, a direct link, a
//! platform that answers without a picture. Many uploads open on a black frame
//! (fade-in, title card, encoder warm-up): taking the first frame is what made
//! black cards, so the frame is chosen among several moments of the video.
//!
//! The picture is recorded as the asset's cover (`media_assets.cover_asset_id`,
//! see [`assets::attach_cover`]). The board then shows it on the video's card,
//! and every guild with that video on its board gets a `board_changed`
//! (`updated`) so its open overlays reload the card.
//!
//! At startup, [`spawn_cover_backfill`] gives the same picture to board videos
//! that have none, and makes it again when the cover row survived but its file
//! did not (an older re-ingestion could delete the file, and the row then
//! pointed at nothing).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use lc_db::Db;
use lc_domain::entities::MediaAsset;
use lc_domain::ids::MediaAssetId;
use lc_domain::{Clock, GuildId, MediaKind};
use lc_media::config::MediaConfig;
use lc_media::poster::{extract_poster, PosterSource};
use lc_media::resolve::ResolvedMedia;
use lc_media::source::{MediaSource, SourceKind};
use lc_media::storage::MediaStorage;
use lc_media::thumbnail::{discover_thumbnail_candidates, fetch_thumbnail};
use lc_media::transcode::NormalizedCover;
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};
use sqlx::Row;

use crate::app::AppState;
use crate::http::admin::hub::AdminHub;
use crate::media::assets;
use crate::media::errors::MediaServiceResult;
use crate::media::settings::media_config_for_state;

/// Most board videos given a cover by one startup backfill.
const BACKFILL_LIMIT: i64 = 500;

/// What a cover task needs, cheap to clone.
#[derive(Clone)]
pub struct ThumbnailContext {
    pub config: Arc<MediaConfig>,
    pub storage: MediaStorage,
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub hub: Option<Arc<dyn AdminHub>>,
}

/// Sources worked on (or being worked on) by this process: a video reused from
/// the cache many times, or whose source has no picture, is tried once.
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

/// `true` when an asset gets a cover: every video, from its platform when it
/// has one, else from its own frames.
pub fn wants_cover(asset: &MediaAsset) -> bool {
    asset.kind == MediaKind::Video
}

/// Starts working on the cover of a `READY` video, unless it already has a
/// picture or was tried before. Returns at once.
///
/// `source` is `None` for an upload: there is no platform to ask, only the
/// video's own frames.
pub fn spawn_cover_fetch(
    context: &ThumbnailContext,
    asset: &MediaAsset,
    source: Option<MediaSource>,
    resolved: Option<ResolvedMedia>,
) {
    if !wants_cover(asset) || !first_attempt(&asset.source_hash) {
        return;
    }
    let context = context.clone();
    let asset_id = asset.id;
    tokio::spawn(async move {
        fetch_and_attach(&context, asset_id, source.as_ref(), resolved.as_ref()).await;
    });
}

/// Makes the cover of the video and records it. Best effort: failures are
/// logged, the card keeps its glyph.
async fn fetch_and_attach(
    context: &ThumbnailContext,
    asset_id: MediaAssetId,
    source: Option<&MediaSource>,
    resolved: Option<&ResolvedMedia>,
) {
    let assets_repo = context.db.media_assets();
    let (asset, existing_cover) = match (
        assets_repo.get(asset_id).await,
        assets_repo.cover_of(asset_id).await,
    ) {
        (Ok(Some(asset)), Ok(cover)) if asset.is_ready() => (asset, cover),
        (Ok(_), Ok(_)) => return,
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!(error = %error, "lecture de la vidéo impossible");
            return;
        }
    };
    // A cover row without its file is a broken card (an older build deleted the
    // file, or a storage restore lost it): make the picture again instead of
    // trusting the row.
    if let Some(cover_id) = existing_cover {
        match assets_repo.get(cover_id).await {
            Ok(Some(cover))
                if cover_file_present(context, cover.is_ready(), Some(&cover.storage_path)) =>
            {
                return;
            }
            Ok(_) => {
                tracing::info!(
                    asset_id = %asset.id.to_canonical(),
                    "image de la vidéo absente du stockage : nouvelle tentative"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "lecture de l'image de la vidéo impossible");
                return;
            }
        }
    }
    let (cover, origin) = match platform_cover(context, &asset, source, resolved).await {
        Some(cover) => (cover, "platform"),
        None => match poster_cover(context, &asset).await {
            Some(cover) => (cover, "frame"),
            None => {
                tracing::debug!(
                    asset_id = %asset.id.to_canonical(),
                    "aucune image pour la vidéo"
                );
                return;
            }
        },
    };
    // Read again: the video may have been pinned to a board (longer expiry)
    // or removed while the picture was being made.
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
    store_cover(context, &asset, &cover, origin).await;
}

/// Records a cover that already exists for `asset` and tells the boards.
///
/// The background tasks above make their own picture, then record it here; the
/// route that takes the editor's frame
/// ([`super::super::http::overlay::media_cover`]) does the same, so both paths
/// share the expiry alignment and the board notification.
pub(crate) async fn store_cover(
    context: &ThumbnailContext,
    asset: &MediaAsset,
    cover: &NormalizedCover,
    origin: &'static str,
) -> bool {
    let now = context.clock.now();
    let cover_id = match assets::attach_cover(&context.db, asset, cover, now).await {
        Ok(cover_id) => cover_id,
        Err(error) => {
            tracing::warn!(error = %error, "image de la vidéo non enregistrée");
            return false;
        }
    };
    if let Err(error) = align_cover_expiry(&context.db, asset.id, cover_id).await {
        tracing::warn!(error = %error, "échéance de l'image non alignée");
    }
    tracing::info!(
        asset_id = %asset.id.to_canonical(),
        origin,
        "image de la vidéo enregistrée"
    );
    notify_boards(context, asset.id).await;
    true
}

/// The picture published by the source's platform, when there is one.
///
/// X needs the resolved media to find its poster; without it the platform has
/// no candidate and the video's own frames are used instead.
async fn platform_cover(
    context: &ThumbnailContext,
    asset: &MediaAsset,
    source: Option<&MediaSource>,
    resolved: Option<&ResolvedMedia>,
) -> Option<NormalizedCover> {
    let source = source?;
    if !matches!(
        source.source_kind,
        SourceKind::YouTube | SourceKind::TikTok | SourceKind::Twitter
    ) {
        return None;
    }
    let candidates = discover_thumbnail_candidates(&context.config, source, resolved).await;
    if candidates.is_empty() {
        return None;
    }
    fetch_thumbnail(
        &context.config,
        &context.storage,
        &asset.source_hash,
        &candidates,
    )
    .await
}

/// A frame of the video itself, chosen among several moments: the first frame
/// of an upload is often black.
async fn poster_cover(context: &ThumbnailContext, asset: &MediaAsset) -> Option<NormalizedCover> {
    if asset.storage_path.trim().is_empty() {
        return None;
    }
    let input = context
        .storage
        .resolve_reference(Path::new(&asset.storage_path));
    if !input.is_file() {
        return None;
    }
    let source = PosterSource {
        input: &input,
        width: optional_side(asset.width),
        height: optional_side(asset.height),
        duration_sec: asset.duration_sec.map(|seconds| seconds as f64),
    };
    extract_poster(
        context.config.as_ref(),
        &context.storage,
        &asset.source_hash,
        &source,
    )
    .await
}

/// An asset dimension as the poster sampler expects it.
fn optional_side(value: Option<i64>) -> Option<u32> {
    value.and_then(|value| u32::try_from(value).ok())
}

/// Gives the cover the expiry of its video when the video got a later one
/// (pinned to a board) while the picture was being made.
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

/// `true` when a cover row, when there is one, still points at a stored file.
///
/// The row alone proves nothing: a re-ingestion of an older build deleted
/// `<hash>.cover.*` while keeping the row, so the card asks for a file that is
/// no longer there.
fn cover_file_present(
    context: &ThumbnailContext,
    cover_ready: bool,
    cover_path: Option<&str>,
) -> bool {
    if !cover_ready {
        return false;
    }
    let Some(path) = cover_path.filter(|path| !path.trim().is_empty()) else {
        return false;
    };
    context.storage.resolve_reference(Path::new(path)).is_file()
}

/// Board videos that need a picture, with the source they came from: no cover
/// row yet, or a cover row whose file is gone.
async fn videos_without_cover(
    db: &Db,
) -> Result<Vec<(MediaAssetId, String, String, bool, Option<String>)>, lc_db::DbError> {
    let rows = sqlx::query(
        "SELECT DISTINCT a.id, a.source_url, a.source_hash, \
                COALESCE(c.status = 'READY', 0) AS cover_ready, c.storage_path AS cover_path \
           FROM media_assets a \
           JOIN media_board_items b ON b.media_asset_id = a.id \
           LEFT JOIN media_assets c ON c.id = a.cover_asset_id \
          WHERE a.kind = 'VIDEO' AND a.status = 'READY' \
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
        let source_hash: String = row.try_get("source_hash").map_err(lc_db::DbError::from)?;
        let cover_ready: i64 = row.try_get("cover_ready").map_err(lc_db::DbError::from)?;
        let cover_path: Option<String> = row.try_get("cover_path").map_err(lc_db::DbError::from)?;
        if let Ok(id) = id.parse::<MediaAssetId>() {
            videos.push((id, source_url, source_hash, cover_ready != 0, cover_path));
        }
    }
    Ok(videos)
}

/// Drops the audio covers whose picture file is gone.
///
/// An audio's picture is the cover embedded in the uploaded file; unlike a
/// video's frame it cannot be made again from the stored media. Keeping the row
/// only makes the overlay ask for a file the server cannot serve (the card
/// falls back to its audio mark), so the row is removed: the audio and its card
/// stop advertising a picture they lost. The audio's own file must still be
/// there: an unreachable storage root must not empty the board's pictures.
pub(crate) async fn drop_lost_audio_covers(
    context: &ThumbnailContext,
) -> MediaServiceResult<usize> {
    let rows = sqlx::query(
        "SELECT c.id AS cover_id, c.storage_path AS cover_path, c.status AS cover_status, \
                p.storage_path AS audio_path \
           FROM media_assets c \
           JOIN media_assets p ON p.cover_asset_id = c.id \
           JOIN media_board_items b ON b.media_asset_id = p.id \
          WHERE p.kind = 'AUDIO' AND p.status = 'READY'",
    )
    .fetch_all(context.db.pool())
    .await
    .map_err(lc_db::DbError::from)?;

    let mut dropped = 0;
    for row in rows {
        let cover_id: String = row.try_get("cover_id").map_err(lc_db::DbError::from)?;
        let cover_path: String = row.try_get("cover_path").map_err(lc_db::DbError::from)?;
        let cover_status: String = row.try_get("cover_status").map_err(lc_db::DbError::from)?;
        let audio_path: String = row.try_get("audio_path").map_err(lc_db::DbError::from)?;
        let audio_present = context
            .storage
            .resolve_reference(Path::new(&audio_path))
            .is_file();
        if !audio_present || cover_file_present(context, cover_status == "READY", Some(&cover_path))
        {
            continue;
        }
        let result = sqlx::query(
            "DELETE FROM media_assets \
              WHERE id = ? AND id NOT IN (SELECT media_asset_id FROM media_board_items)",
        )
        .bind(&cover_id)
        .execute(context.db.pool())
        .await
        .map_err(lc_db::DbError::from)?;
        dropped += usize::try_from(result.rows_affected()).unwrap_or(usize::MAX);
    }
    Ok(dropped)
}

/// Builds the context of the cover tasks from the application state.
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

/// Gives a cover to the board videos that have none, one after the other, on a
/// background task: the platform's picture when there is one, else a frame of
/// the video. Audio covers whose file is gone are dropped first.
pub fn spawn_cover_backfill(state: &AppState) {
    let Some(context) = context_for_state(state) else {
        return;
    };
    tokio::spawn(async move {
        match drop_lost_audio_covers(&context).await {
            Ok(0) => {}
            Ok(dropped) => {
                tracing::info!(dropped, "pochettes audio absentes du stockage détachées")
            }
            Err(error) => {
                tracing::warn!(error = %error, "nettoyage des pochettes audio impossible");
            }
        }
        backfill_covers(&context).await;
    });
}

/// One backfill pass over the board videos that need a picture. An upload is
/// covered like a link: there is no platform to ask, so its own frames are
/// used.
///
/// The pass does not go through `first_attempt`: it is the repair path, and a
/// source tried earlier by an ingestion may still have no file on disk.
pub(crate) async fn backfill_covers(context: &ThumbnailContext) {
    let videos = match videos_without_cover(&context.db).await {
        Ok(videos) => videos,
        Err(error) => {
            tracing::warn!(error = %error, "vidéos sans image illisibles");
            return;
        }
    };
    for (asset_id, source_url, _source_hash, cover_ready, cover_path) in videos {
        if cover_file_present(context, cover_ready, cover_path.as_deref()) {
            continue;
        }
        let source = MediaSource::from_raw(&source_url);
        fetch_and_attach(context, asset_id, source.as_ref(), None).await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path as StdPath;

    use lc_db::DbConfig;
    use lc_domain::clock::SystemClock;
    use lc_domain::entities::MediaBoardItem;
    use lc_domain::enums::MediaStatus;
    use lc_domain::ids::{GuildId, MediaBoardItemId};
    use time::{Duration as TimeDuration, UtcDateTime};

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        config: MediaConfig,
        storage: MediaStorage,
        db: Db,
        now: UtcDateTime,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        let mut config = MediaConfig::default();
        config.storage.directory = dir.path().join("media");
        let storage = MediaStorage::new(&config.storage);
        storage.ensure().expect("racine de stockage");
        Fixture {
            _dir: dir,
            config,
            storage,
            db,
            now: UtcDateTime::now(),
        }
    }

    /// A `READY` video as `ingest_local` leaves it: the upload's source has no
    /// platform, only the file itself.
    fn uploaded_video(source_hash: &str, storage_path: &StdPath, now: UtcDateTime) -> MediaAsset {
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: source_hash.to_owned(),
            source_url: "clip.mp4".to_owned(),
            source_title: None,
            kind: MediaKind::Video,
            mime: "video/mp4".into(),
            duration_sec: Some(4),
            width: Some(320),
            height: Some(240),
            is_vertical: false,
            storage_path: storage_path.to_string_lossy().into_owned(),
            size_bytes: 4,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + TimeDuration::hours(1),
            created_at: now,
        }
    }

    async fn on_board(fixture: &Fixture, asset: &MediaAsset) {
        on_board_with_artwork(fixture, asset, None).await;
    }

    /// Puts a media on the board, with the artwork its card shows.
    async fn on_board_with_artwork(
        fixture: &Fixture,
        asset: &MediaAsset,
        artwork: Option<MediaAssetId>,
    ) -> MediaBoardItem {
        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
            media_asset_id: asset.id,
            title: None,
            message: None,
            created_by_discord_user_id: None,
            created_by_name: None,
            created_at: fixture.now,
            artwork_asset_id: artwork,
            artwork_on_overlay: false,
            display_duration_sec: None,
            display_scale_percent: None,
        };
        fixture
            .db
            .media_board()
            .create(&item)
            .await
            .expect("item board");
        item
    }

    #[test]
    fn une_video_televersee_recoit_une_image() {
        // An upload has no platform to ask: its cover can only come from its
        // own frames. Requiring a platform source is what left every upload
        // with the black first frame as its card.
        let asset = uploaded_video(
            "upload:sha256:clip",
            Path::new("clip.mp4"),
            UtcDateTime::now(),
        );
        assert!(wants_cover(&asset));
    }

    #[test]
    fn une_image_et_un_son_ne_recoivent_pas_d_image_de_video() {
        let now = UtcDateTime::now();
        let mut audio = uploaded_video("upload:sha256:song", Path::new("song.mp3"), now);
        audio.kind = MediaKind::Audio;
        assert!(!wants_cover(&audio));
        let mut image = uploaded_video("upload:sha256:photo", Path::new("photo.jpg"), now);
        image.kind = MediaKind::Image;
        assert!(!wants_cover(&image));
    }

    /// The context a background task would get, without the admin hub.
    fn context(fixture: &Fixture) -> ThumbnailContext {
        ThumbnailContext {
            config: Arc::new(fixture.config.clone()),
            storage: fixture.storage.clone(),
            db: fixture.db.clone(),
            clock: Arc::new(SystemClock),
            hub: None,
        }
    }

    #[tokio::test]
    async fn une_video_televersee_sans_image_est_reprise_par_le_rattrapage() {
        let fixture = fixture().await;
        let file = fixture.storage.root().join("clip.mp4");
        std::fs::write(&file, b"video").expect("fichier");
        let video = uploaded_video("upload:sha256:clip", &file, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&video)
            .await
            .expect("vidéo");
        on_board(&fixture, &video).await;

        let picked = videos_without_cover(&fixture.db).await.expect("requête");
        assert_eq!(picked.len(), 1, "la vidéo téléversée doit être reprise");
        assert_eq!(picked[0].0, video.id);
        assert_eq!(picked[0].2, "upload:sha256:clip");
        assert!(!picked[0].3, "aucune image enregistrée pour l'instant");
    }

    #[tokio::test]
    async fn une_video_dont_l_image_a_disparu_est_reprise_par_le_rattrapage() {
        // The row survived but its file did not: the card asks for a picture
        // that is not there. The backfill must see the video again instead of
        // trusting the row.
        let fixture = fixture().await;
        let file = fixture.storage.root().join("clip.mp4");
        std::fs::write(&file, b"video").expect("fichier");
        let video = uploaded_video("upload:sha256:clip", &file, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&video)
            .await
            .expect("vidéo");
        on_board(&fixture, &video).await;
        let missing = fixture.storage.root().join("disparue.cover.jpg");
        let cover = uploaded_video("upload:sha256:clip#cover", &missing, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&cover)
            .await
            .expect("image");
        fixture
            .db
            .media_assets()
            .set_cover(video.id, Some(cover.id))
            .await
            .expect("liaison");

        let picked = videos_without_cover(&fixture.db).await.expect("requête");
        assert_eq!(picked.len(), 1, "la vidéo doit être examinée de nouveau");
        assert!(picked[0].3, "l'image est bien enregistrée en base");
        let context = context(&fixture);
        assert!(
            !cover_file_present(&context, picked[0].3, picked[0].4.as_deref()),
            "mais son fichier a disparu du stockage"
        );
    }

    /// An audio as `ingest_local` leaves it: its picture is the cover embedded
    /// in the uploaded file, which no stored frame can replace.
    fn uploaded_audio(source_hash: &str, storage_path: &StdPath, now: UtcDateTime) -> MediaAsset {
        let mut asset = uploaded_video(source_hash, storage_path, now);
        asset.kind = MediaKind::Audio;
        asset.mime = "audio/mpeg".into();
        asset.duration_sec = Some(12);
        asset.width = None;
        asset.height = None;
        asset
    }

    /// The `IMAGE` row `attach_cover` writes for an audio.
    fn uploaded_cover(source_hash: &str, storage_path: &StdPath, now: UtcDateTime) -> MediaAsset {
        let mut asset = uploaded_video(source_hash, storage_path, now);
        asset.kind = MediaKind::Image;
        asset.mime = "image/jpeg".into();
        asset.duration_sec = None;
        asset
    }

    /// An audio on the board whose card shows a cover file that is, or is not,
    /// on disk: what a lost picture looks like after a re-ingestion.
    async fn audio_with_cover(
        fixture: &Fixture,
        cover_on_disk: bool,
    ) -> (MediaAsset, MediaAsset, MediaBoardItem) {
        let audio_file = fixture.storage.root().join("song.mp3");
        std::fs::write(&audio_file, b"audio").expect("fichier audio");
        let audio = uploaded_audio("upload:sha256:song", &audio_file, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&audio)
            .await
            .expect("audio");
        let cover_path = fixture.storage.root().join("song.cover.jpg");
        if cover_on_disk {
            std::fs::write(&cover_path, b"image").expect("fichier pochette");
        }
        let cover = uploaded_cover("upload:sha256:song#cover", &cover_path, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&cover)
            .await
            .expect("pochette");
        fixture
            .db
            .media_assets()
            .set_cover(audio.id, Some(cover.id))
            .await
            .expect("liaison");
        let item = on_board_with_artwork(fixture, &audio, Some(cover.id)).await;
        (audio, cover, item)
    }

    #[tokio::test]
    async fn une_pochette_audio_disparue_est_detachee_du_board() {
        // The row survived a re-ingestion but its file did not: the card asks
        // for a picture the server cannot serve, and an audio has no stored
        // frame to remake it from. The dead reference is dropped so the card
        // shows its audio mark instead.
        let fixture = fixture().await;
        let (audio, cover, item) = audio_with_cover(&fixture, false).await;

        let dropped = drop_lost_audio_covers(&context(&fixture))
            .await
            .expect("nettoyage");

        assert_eq!(dropped, 1, "la pochette introuvable est retirée");
        assert_eq!(
            fixture
                .db
                .media_assets()
                .get(cover.id)
                .await
                .expect("lecture"),
            None
        );
        assert_eq!(
            fixture
                .db
                .media_assets()
                .cover_of(audio.id)
                .await
                .expect("lecture"),
            None
        );
        let stored = fixture
            .db
            .media_board()
            .get(item.id)
            .await
            .expect("lecture")
            .expect("item");
        assert_eq!(
            stored.artwork_asset_id, None,
            "la carte n'annonce plus une image absente"
        );
    }

    #[tokio::test]
    async fn une_pochette_audio_presente_est_gardee() {
        let fixture = fixture().await;
        let (audio, cover, item) = audio_with_cover(&fixture, true).await;

        let dropped = drop_lost_audio_covers(&context(&fixture))
            .await
            .expect("nettoyage");

        assert_eq!(dropped, 0, "une pochette servie reste en place");
        assert!(fixture
            .db
            .media_assets()
            .get(cover.id)
            .await
            .expect("lecture")
            .is_some());
        assert_eq!(
            fixture
                .db
                .media_assets()
                .cover_of(audio.id)
                .await
                .expect("lecture"),
            Some(cover.id)
        );
        let stored = fixture
            .db
            .media_board()
            .get(item.id)
            .await
            .expect("lecture")
            .expect("item");
        assert_eq!(stored.artwork_asset_id, Some(cover.id));
    }

    #[tokio::test]
    async fn un_stockage_absent_ne_detache_pas_les_pochettes() {
        // The audio's own file is gone too: the storage root is probably not
        // mounted, and the covers must not be deleted on that basis.
        let fixture = fixture().await;
        let (audio, cover, _item) = audio_with_cover(&fixture, false).await;
        std::fs::remove_file(fixture.storage.root().join("song.mp3")).expect("retrait");
        let context = context(&fixture);

        let dropped = drop_lost_audio_covers(&context).await.expect("nettoyage");

        assert_eq!(dropped, 0);
        assert_eq!(
            fixture
                .db
                .media_assets()
                .cover_of(audio.id)
                .await
                .expect("lecture"),
            Some(cover.id)
        );
    }

    /// A video that opens on a black frame, like many uploads.
    fn black_intro(config: &MediaConfig, path: &StdPath) {
        let status = std::process::Command::new(&config.tools.ffmpeg)
            .args([
                "-y",
                "-hide_banner",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:s=320x240:d=1.5",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2.5:size=320x240:rate=30",
                "-filter_complex",
                "[0:v][1:v]concat=n=2:v=1:a=0[v]",
                "-map",
                "[v]",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(path)
            .status()
            .expect("ffmpeg démarré");
        assert!(status.success(), "échantillon vidéo non produit");
    }

    async fn wait_for_cover(db: &Db, asset_id: MediaAssetId) -> MediaAssetId {
        for _ in 0..400 {
            if let Some(cover_id) = db.media_assets().cover_of(asset_id).await.expect("lecture") {
                return cover_id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("aucune image produite pour la vidéo");
    }

    #[tokio::test]
    #[ignore = "nécessite ffmpeg installé"]
    async fn une_video_televersee_recoit_une_frame_de_elle_meme() {
        let fixture = fixture().await;
        let input = fixture.storage.root().join("upload.mp4");
        black_intro(&fixture.config, &input);
        let video = uploaded_video("upload:sha256:frame", &input, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&video)
            .await
            .expect("vidéo");
        let context = ThumbnailContext {
            config: Arc::new(fixture.config.clone()),
            storage: fixture.storage.clone(),
            db: fixture.db.clone(),
            clock: Arc::new(SystemClock),
            hub: None,
        };

        spawn_cover_fetch(&context, &video, None, None);
        let cover_id = wait_for_cover(&fixture.db, video.id).await;

        let cover = fixture
            .db
            .media_assets()
            .get(cover_id)
            .await
            .expect("lecture")
            .expect("image enregistrée");
        assert_eq!(cover.kind, MediaKind::Image);
        assert_eq!(cover.mime, "image/jpeg");
        assert_eq!((cover.width, cover.height), (Some(320), Some(240)));
        assert!(
            Path::new(&cover.storage_path).is_file(),
            "le fichier de l'image existe"
        );
        assert!(cover.source_hash.starts_with("upload:sha256:frame#cover"));
    }

    #[tokio::test]
    #[ignore = "nécessite ffmpeg installé"]
    async fn le_rattrapage_donne_une_image_a_une_video_televersee() {
        let fixture = fixture().await;
        let input = fixture.storage.root().join("rattrapage.mp4");
        black_intro(&fixture.config, &input);
        let video = uploaded_video("upload:sha256:backfill", &input, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&video)
            .await
            .expect("vidéo");
        on_board(&fixture, &video).await;
        let context = ThumbnailContext {
            config: Arc::new(fixture.config.clone()),
            storage: fixture.storage.clone(),
            db: fixture.db.clone(),
            clock: Arc::new(SystemClock),
            hub: None,
        };

        // One pass covers the upload: no platform filter keeps it out.
        backfill_covers(&context).await;

        let cover_id = fixture
            .db
            .media_assets()
            .cover_of(video.id)
            .await
            .expect("lecture")
            .expect("image du rattrapage");
        let cover = fixture
            .db
            .media_assets()
            .get(cover_id)
            .await
            .expect("lecture")
            .expect("image enregistrée");
        assert_eq!(cover.kind, MediaKind::Image);
    }

    #[tokio::test]
    #[ignore = "nécessite ffmpeg installé"]
    async fn le_rattrapage_regenere_une_image_disparue() {
        // An older build deleted `<hash>.cover.jpg` when the same link was
        // ingested again, but kept the cover row. The backfill is the only
        // place that can notice and make the picture again.
        let fixture = fixture().await;
        let input = fixture.storage.root().join("regeneration.mp4");
        black_intro(&fixture.config, &input);
        let video = uploaded_video("upload:sha256:regeneration", &input, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&video)
            .await
            .expect("vidéo");
        on_board(&fixture, &video).await;
        let missing = fixture.storage.root().join("disparue.cover.jpg");
        let cover = uploaded_video("upload:sha256:regeneration#cover", &missing, fixture.now);
        fixture
            .db
            .media_assets()
            .insert(&cover)
            .await
            .expect("image");
        fixture
            .db
            .media_assets()
            .set_cover(video.id, Some(cover.id))
            .await
            .expect("liaison");
        let context = context(&fixture);
        assert!(
            !cover_file_present(&context, true, Some(&cover.storage_path)),
            "le fichier de l'image a bien disparu"
        );

        backfill_covers(&context).await;

        let repaired = fixture
            .db
            .media_assets()
            .get(cover.id)
            .await
            .expect("lecture")
            .expect("la ligne d'image est réutilisée");
        assert_eq!(repaired.id, cover.id, "pas de nouvelle ligne d'image");
        assert!(
            Path::new(&repaired.storage_path).is_file(),
            "le fichier de l'image est recréé"
        );
        assert_ne!(
            repaired.storage_path, cover.storage_path,
            "l'image pointe désormais sur le fichier recréé"
        );
    }

    /// A video imported from a link, whose platform has no picture to give
    /// (here a plain `.mp4` URL): its card must still show the video itself
    /// rather than the fallback glyph.
    #[tokio::test]
    #[ignore = "nécessite ffmpeg installé"]
    async fn une_video_importee_par_un_lien_recoit_une_frame() {
        let fixture = fixture().await;
        let input = fixture.storage.root().join("lien.mp4");
        black_intro(&fixture.config, &input);
        let source =
            MediaSource::from_raw("https://cdn.example.test/clip.mp4").expect("source du lien");
        let mut video = uploaded_video(&source.source_hash, &input, fixture.now);
        video.source_url = source.canonical_url.clone();
        fixture
            .db
            .media_assets()
            .insert(&video)
            .await
            .expect("vidéo");
        let context = ThumbnailContext {
            config: Arc::new(fixture.config.clone()),
            storage: fixture.storage.clone(),
            db: fixture.db.clone(),
            clock: Arc::new(SystemClock),
            hub: None,
        };

        spawn_cover_fetch(&context, &video, Some(source), None);
        let cover_id = wait_for_cover(&fixture.db, video.id).await;

        let cover = fixture
            .db
            .media_assets()
            .get(cover_id)
            .await
            .expect("lecture")
            .expect("image enregistrée");
        assert_eq!(cover.kind, MediaKind::Image);
        assert_eq!(cover.mime, "image/jpeg");
        assert_eq!((cover.width, cover.height), (Some(320), Some(240)));
        assert!(Path::new(&cover.storage_path).is_file(), "fichier présent");
    }

    /// A platform link whose thumbnail cannot be obtained (no resolution to
    /// read one from, and nothing to scrape): the frame of the stored video is
    /// the fallback, and asking for it must not require the platform.
    #[tokio::test]
    #[ignore = "nécessite ffmpeg installé"]
    async fn une_video_de_plateforme_sans_vignette_recoit_une_frame() {
        let fixture = fixture().await;
        let input = fixture.storage.root().join("plateforme.mp4");
        black_intro(&fixture.config, &input);
        let source =
            MediaSource::from_raw("https://x.com/quelqu_un/status/1234567890").expect("lien");
        let mut video = uploaded_video(&source.source_hash, &input, fixture.now);
        video.source_url = source.canonical_url.clone();
        fixture
            .db
            .media_assets()
            .insert(&video)
            .await
            .expect("vidéo");
        let context = ThumbnailContext {
            config: Arc::new(fixture.config.clone()),
            storage: fixture.storage.clone(),
            db: fixture.db.clone(),
            clock: Arc::new(SystemClock),
            hub: None,
        };

        spawn_cover_fetch(&context, &video, Some(source), None);
        let cover_id = wait_for_cover(&fixture.db, video.id).await;

        let cover = fixture
            .db
            .media_assets()
            .get(cover_id)
            .await
            .expect("lecture")
            .expect("image enregistrée");
        assert_eq!(cover.kind, MediaKind::Image);
        assert_eq!(cover.mime, "image/jpeg");
    }
}
