//! Worker de purge du stockage média (AUD-A4-008/A4-014).
//!
//! Les audits ont relevé deux manques : les durées `FAILED_TTL` /
//! `STALE_PROCESSING_TTL` et `quotas::purge_candidates`/`evict_source_hashes`
//! n'avaient aucun appelant, et les fichiers de transit laissés par un arrêt
//! brutal (`.tmp-*`) n'étaient jamais nettoyés.
//!
//! [`purge_once`] applique une passe complète et [`spawn_purge_worker`] la
//! rejoue toutes les [`PURGE_INTERVAL`] (au démarrage puis en arrière-plan). La
//! suppression est toujours protégée par une grâce ([`ORPHAN_GRACE`]) pour ne
//! jamais toucher un fichier en cours d'écriture par une ingestion concurrente.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lc_db::Db;
use lc_domain::enums::MediaStatus;
use lc_domain::time::datetime_to_ms;
use lc_media::ingest::lock_source_hash;
use lc_media::storage::{is_staged_file_name, MediaStorage};
use time::{Duration as TimeDuration, UtcDateTime};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::app::AppState;
use crate::media::assets;
use crate::media::confinement;
use crate::media::errors::MediaServiceResult;

/// Intervalle entre deux passes de purge (10 minutes, comme les workers legacy).
pub const PURGE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Grâce avant de supprimer un fichier inconnu de la base ou un fichier de
/// transit : un fichier peut être écrit juste avant sa persistance en base, ou
/// être en cours d'écriture par une ingestion concurrente.
pub const ORPHAN_GRACE: TimeDuration = TimeDuration::hours(1);

/// Compteurs d'une passe de purge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PurgeReport {
    /// Assets `FAILED` périmés supprimés.
    pub failed_removed: usize,
    /// Assets `PROCESSING` abandonnés supprimés.
    pub processing_removed: usize,
    /// Assets `READY` expirés évincés.
    pub expired_evicted: usize,
    /// Fichiers de transit (`.tmp-*`) périmés supprimés.
    pub staged_files_removed: usize,
    /// Fichiers orphelins (absents de la base) supprimés.
    pub orphan_files_removed: usize,
}

impl PurgeReport {
    /// Vrai si aucun élément n'a été supprimé.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Nombre total d'éléments supprimés.
    pub fn total(&self) -> usize {
        self.failed_removed
            + self.processing_removed
            + self.expired_evicted
            + self.staged_files_removed
            + self.orphan_files_removed
    }
}

/// Une passe de purge : lignes périmées, assets expirés, puis fichiers du
/// stockage (transit et orphelins).
pub async fn purge_once(
    db: &Db,
    storage: &MediaStorage,
    now: UtcDateTime,
) -> MediaServiceResult<PurgeReport> {
    // Toute suppression de fichier exige une racine canonicalisable : si elle
    // ne l'est pas, on ne supprime rien plutôt que de comparer des chemins
    // incomparables (AUD-V2-S-002). Le worker la canonicalise déjà au démarrage.
    let root = match std::fs::canonicalize(storage.root()) {
        Ok(root) => root,
        Err(error) => {
            warn!(
                root = %storage.root().display(),
                %error,
                "purge média ignorée : racine non canonicalisable, aucune suppression"
            );
            return Ok(PurgeReport::default());
        }
    };

    // Deux lignes d'assets peuvent référencer le même fichier (AUD-V2-S-004) :
    // on ne supprime le fichier qu'après la disparition de la dernière ligne.
    let mut references = reference_counts(db, &root).await?;

    // 1. Assets `FAILED` périmés (jamais épinglés par le board ni actifs).
    let failed_removed = purge_stale_status(
        db,
        &root,
        &mut references,
        MediaStatus::Failed,
        now - assets::FAILED_TTL,
    )
    .await?;

    // 2. Assets `PROCESSING` abandonnés (crash du serveur pendant l'ingestion).
    let processing_removed = purge_stale_status(
        db,
        &root,
        &mut references,
        MediaStatus::Processing,
        now - assets::STALE_PROCESSING_TTL,
    )
    .await?;

    // 3. Assets `READY` expirés (hors épinglage et hors lecture active).
    let expired_evicted = purge_expired_ready(db, &root, &mut references, now).await?;

    // 4. Fichiers du stockage : transit périmé puis orphelins.
    let (staged_files_removed, orphan_files_removed) =
        purge_storage_files(db, storage, &root, now).await?;

    Ok(PurgeReport {
        failed_removed,
        processing_removed,
        expired_evicted,
        staged_files_removed,
        orphan_files_removed,
    })
}

/// Clé canonique d'une référence de stockage, utilisée pour dédupliquer les
/// références entre lignes d'assets (AUD-V2-S-004).
fn reference_key(root: &Path, reference: &Path) -> PathBuf {
    let resolved = confinement::resolve_reference(root, reference);
    std::fs::canonicalize(&resolved).unwrap_or(resolved)
}

/// Compte les références de chaque fichier du stockage avant la passe.
async fn reference_counts(db: &Db, root: &Path) -> MediaServiceResult<HashMap<PathBuf, usize>> {
    let storage_paths = db.media_assets().storage_paths().await?;
    let mut counts: HashMap<PathBuf, usize> = HashMap::with_capacity(storage_paths.len());
    for storage_path in storage_paths {
        if storage_path.trim().is_empty() {
            continue;
        }
        let key = reference_key(root, Path::new(&storage_path));
        *counts.entry(key).or_insert(0) += 1;
    }
    Ok(counts)
}

/// Décrémente la référence d'un fichier après suppression de sa ligne ; vrai si
/// c'était la dernière référence (le fichier peut alors être supprimé).
fn release_reference(counts: &mut HashMap<PathBuf, usize>, root: &Path, reference: &Path) -> bool {
    let key = reference_key(root, reference);
    match counts.get_mut(&key) {
        Some(count) if *count > 0 => {
            *count -= 1;
            *count == 0
        }
        // Référence inconnue (ligne insérée pendant la passe) : prudence, on ne
        // supprime pas un fichier potentiellement encore référencé.
        _ => false,
    }
}

/// Supprime le fichier d'une référence s'il était le dernier, en restant
/// confiné sous la racine média (AUD-V2-S-001/002).
fn remove_reference_if_last(
    root: &Path,
    counts: &mut HashMap<PathBuf, usize>,
    reference: &Path,
) -> bool {
    if !release_reference(counts, root, reference) {
        return false;
    }
    match confinement::confined_removal_path(root, reference) {
        Some(path) => std::fs::remove_file(&path).is_ok(),
        None => false,
    }
}

/// Supprime les lignes d'un statut terminal dont `last_accessed_at` est antérieur
/// à `stale_before`, en excluant le board épinglé, la pochette et les jobs actifs.
///
/// La sélection et la suppression vivent dans `lc-db` : le worker ne connaît
/// plus le SQL des exclusions, testé par plan dans le repository. Le DELETE
/// rejoue les exclusions board/pochette/job actif : un asset épinglé après la
/// sélection n'est pas supprimé (`affected == false`).
async fn purge_stale_status(
    db: &Db,
    root: &Path,
    references: &mut HashMap<PathBuf, usize>,
    status: MediaStatus,
    stale_before: UtcDateTime,
) -> MediaServiceResult<usize> {
    let candidates = db
        .media_assets()
        .list_stale_purge_candidates(status, stale_before)
        .await?;

    let mut removed = 0;
    for candidate in candidates {
        let affected = db
            .media_assets()
            .delete_purge_candidate(&candidate.id)
            .await?;
        if !affected {
            continue;
        }
        if !candidate.storage_path.trim().is_empty() {
            // Fichier supprimé seulement si la ligne était sa dernière
            // référence, et uniquement sous la racine média.
            let _deleted =
                remove_reference_if_last(root, references, Path::new(&candidate.storage_path));
        }
        removed += 1;
    }

    Ok(removed)
}

/// Évince les assets `READY` expirés (ligne puis fichier s'il n'est plus
/// référencé).
async fn purge_expired_ready(
    db: &Db,
    root: &Path,
    references: &mut HashMap<PathBuf, usize>,
    now: UtcDateTime,
) -> MediaServiceResult<usize> {
    let candidates = db
        .media_assets()
        .list_expired_ready_purge_candidates(now)
        .await?;

    let mut removed = 0;
    for candidate in candidates {
        // Sérialise avec une ingestion concurrente du même média : sans ce
        // verrou, une ingestion pourrait publier des fichiers entre le DELETE
        // et la suppression du fichier (AUD-V2-S-004).
        let _source_lock = lock_source_hash(&candidate.source_hash).await;

        // Course possible avec une ingestion qui vient de réarmer la ligne ou
        // un item de board qui vient de l'épingler : le DELETE conditionnel ne
        // supprime que l'état expiré et non référencé observé.
        let affected = db
            .media_assets()
            .delete_expired_ready_candidate(&candidate.id, now)
            .await?;
        if !affected {
            continue;
        }

        // Ne jamais supprimer les variantes du hash par principe : une autre
        // ligne peut référencer le même fichier. On ne retire que le fichier
        // de cette ligne, s'il était la dernière référence.
        if !candidate.storage_path.trim().is_empty() {
            let _deleted =
                remove_reference_if_last(root, references, Path::new(&candidate.storage_path));
        }
        removed += 1;
    }

    Ok(removed)
}

/// Supprime les fichiers de transit et orphelins du stockage, avec grâce.
async fn purge_storage_files(
    db: &Db,
    storage: &MediaStorage,
    root: &Path,
    now: UtcDateTime,
) -> MediaServiceResult<(usize, usize)> {
    let storage_paths = db.media_assets().storage_paths().await?;
    let mut known: HashSet<PathBuf> = HashSet::with_capacity(storage_paths.len());
    for storage_path in storage_paths {
        if storage_path.trim().is_empty() {
            continue;
        }
        // Les lignes relatives héritées du legacy sont résolues sous la racine
        // média (jamais le cwd) puis canonicalisées pour être comparables aux
        // entrées de `read_dir` (AUD-V2-S-002).
        known.insert(reference_key(root, Path::new(&storage_path)));
    }

    let grace_ms = lc_domain::time::duration_to_ms(ORPHAN_GRACE);
    let now_ms = datetime_to_ms(now);

    let mut staged = 0;
    let mut orphans = 0;
    for variant in storage.list_variants()? {
        let old_enough = now_ms.saturating_sub(variant.modified_ms) >= grace_ms;
        if !old_enough {
            continue;
        }
        let name = variant
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if is_staged_file_name(name) {
            if remove_or_skip(storage, &variant.path) {
                staged += 1;
            }
            continue;
        }
        let variant_path =
            std::fs::canonicalize(&variant.path).unwrap_or_else(|_| variant.path.to_path_buf());
        if !known.contains(&variant_path) && remove_or_skip(storage, &variant.path) {
            orphans += 1;
        }
    }

    Ok((staged, orphans))
}

/// Removes a stored file during a cleanup sweep. A file that cannot be
/// removed is logged and skipped: on Windows a file still open (an edit or a
/// stream reading it, an antivirus scan) refuses deletion, and one such file
/// must not stop the sweep. The file is retried by the next orphan sweep.
///
/// Returns `true` when the file was removed.
pub(crate) fn remove_or_skip(storage: &MediaStorage, path: &Path) -> bool {
    match storage.remove_path(path) {
        Ok(removed) => removed,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "media file not removed, left for the next cleanup"
            );
            false
        }
    }
}

/// Démarre le worker de purge en arrière-plan (une passe immédiate puis toutes
/// les [`PURGE_INTERVAL`]).
///
/// Retourne `None` si aucune base n'est configurée : sans base, il n'y a ni
/// lignes à purger ni assets à confronter au stockage.
pub fn spawn_purge_worker(state: &AppState) -> Option<JoinHandle<()>> {
    let db = state.db.clone()?;
    let config = Arc::clone(&state.config);
    let clock = Arc::clone(&state.clock);

    let handle = tokio::spawn(async move {
        let root = match std::fs::canonicalize(&config.media_storage_dir) {
            Ok(root) => root,
            Err(error) => {
                warn!(
                    directory = %config.media_storage_dir.display(),
                    error = %error,
                    "worker de purge média désactivé : racine non canonicalisable"
                );
                return;
            }
        };
        let storage = MediaStorage::from_root(root);

        loop {
            let now = clock.now();
            match purge_once(&db, &storage, now).await {
                Ok(report) if report.is_empty() => {}
                Ok(report) => info!(
                    failed = report.failed_removed,
                    processing = report.processing_removed,
                    expired = report.expired_evicted,
                    staged_files = report.staged_files_removed,
                    orphan_files = report.orphan_files_removed,
                    "purge média terminée"
                ),
                Err(error) => warn!(error = %error, "purge média incomplète"),
            }
            tokio::time::sleep(PURGE_INTERVAL).await;
        }
    });

    Some(handle)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::Path as StdPath;

    use lc_db::DbConfig;
    use lc_domain::entities::{MediaAsset, MediaBoardItem, PlaybackJob};
    use lc_domain::enums::{JobSource, JobStatus, MediaKind, MediaStatus};
    use lc_domain::ids::{GuildId, MediaAssetId, MediaBoardItemId, PlaybackJobId};
    use lc_media::config::MediaConfig;
    use lc_media::storage::MediaStorage;
    use time::{Duration as TimeDuration, UtcDateTime};

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Db,
        storage: MediaStorage,
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
            db,
            storage,
            now: UtcDateTime::now(),
        }
    }

    fn asset(
        status: MediaStatus,
        source_hash: &str,
        storage_path: &StdPath,
        last_accessed_at: UtcDateTime,
        expires_at: UtcDateTime,
        created_at: UtcDateTime,
    ) -> MediaAsset {
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: source_hash.to_owned(),
            source_url: format!("https://example.com/{source_hash}.mp4"),
            source_title: None,
            kind: MediaKind::Video,
            mime: "video/mp4".into(),
            duration_sec: Some(10),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: storage_path.to_string_lossy().into_owned(),
            size_bytes: 4,
            status,
            error: None,
            last_accessed_at,
            expires_at,
            created_at,
        }
    }

    #[tokio::test]
    async fn removes_stale_failed_and_processing_rows() {
        let fixture = fixture().await;
        let file = fixture.storage.root().join("stale.mp4");
        std::fs::write(&file, b"media").expect("fichier");

        let failed = asset(
            MediaStatus::Failed,
            "failed-hash",
            &file,
            fixture.now - TimeDuration::hours(3),
            fixture.now,
            fixture.now - TimeDuration::hours(4),
        );
        let processing = asset(
            MediaStatus::Processing,
            "processing-hash",
            StdPath::new(""),
            fixture.now - TimeDuration::hours(3),
            fixture.now,
            fixture.now - TimeDuration::hours(4),
        );
        fixture
            .db
            .media_assets()
            .insert(&failed)
            .await
            .expect("asset FAILED");
        fixture
            .db
            .media_assets()
            .insert(&processing)
            .await
            .expect("asset PROCESSING");

        let report = purge_once(&fixture.db, &fixture.storage, fixture.now)
            .await
            .expect("purge");

        assert_eq!(report.failed_removed, 1);
        assert_eq!(report.processing_removed, 1);
        assert!(!file.exists());
        assert!(fixture
            .db
            .media_assets()
            .get(failed.id)
            .await
            .expect("lecture")
            .is_none());
        assert!(fixture
            .db
            .media_assets()
            .get(processing.id)
            .await
            .expect("lecture")
            .is_none());
    }

    #[tokio::test]
    async fn keeps_pinned_board_assets_and_evicts_expired_ready() {
        let fixture = fixture().await;
        let pinned_file = fixture.storage.root().join("pinned.mp4");
        let expired_file = fixture.storage.root().join("expired.mp4");
        std::fs::write(&pinned_file, b"pinned").expect("fichier");
        std::fs::write(&expired_file, b"expired").expect("fichier");

        let pinned = asset(
            MediaStatus::Ready,
            "pinned-hash",
            &pinned_file,
            fixture.now - TimeDuration::hours(2),
            fixture.now - TimeDuration::hours(1),
            fixture.now - TimeDuration::hours(3),
        );
        let expired = asset(
            MediaStatus::Ready,
            "expired-hash",
            &expired_file,
            fixture.now - TimeDuration::hours(2),
            fixture.now - TimeDuration::hours(1),
            fixture.now - TimeDuration::hours(3),
        );
        fixture
            .db
            .media_assets()
            .insert(&pinned)
            .await
            .expect("asset épinglé");
        fixture
            .db
            .media_assets()
            .insert(&expired)
            .await
            .expect("asset expiré");

        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
            media_asset_id: pinned.id,
            title: None,
            message: None,
            created_by_discord_user_id: None,
            created_by_name: None,
            created_at: fixture.now,
            artwork_asset_id: None,
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

        let report = purge_once(&fixture.db, &fixture.storage, fixture.now)
            .await
            .expect("purge");

        assert_eq!(report.expired_evicted, 1);
        assert!(pinned_file.exists());
        assert!(!expired_file.exists());
        assert!(fixture
            .db
            .media_assets()
            .get(pinned.id)
            .await
            .expect("lecture")
            .is_some());
        assert!(fixture
            .db
            .media_assets()
            .get(expired.id)
            .await
            .expect("lecture")
            .is_none());
    }

    #[tokio::test]
    async fn keeps_the_thumbnail_of_a_board_video() {
        let fixture = fixture().await;
        let video_file = fixture.storage.root().join("video.mp4");
        let cover_file = fixture.storage.root().join("video.cover.jpg");
        std::fs::write(&video_file, b"video").expect("fichier");
        std::fs::write(&cover_file, b"cover").expect("fichier");
        let expired = fixture.now - TimeDuration::hours(1);
        let stale = fixture.now - TimeDuration::days(30);
        let video = asset(
            MediaStatus::Ready,
            "video-hash",
            &video_file,
            stale,
            expired,
            stale,
        );
        let cover = MediaAsset {
            kind: MediaKind::Image,
            mime: "image/jpeg".into(),
            duration_sec: None,
            ..asset(
                MediaStatus::Ready,
                "video-hash#cover",
                &cover_file,
                stale,
                expired,
                stale,
            )
        };
        let assets = fixture.db.media_assets();
        assets.insert(&video).await.expect("vidéo");
        assets.insert(&cover).await.expect("miniature");
        assets
            .set_cover(video.id, Some(cover.id))
            .await
            .expect("liaison");
        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
            media_asset_id: video.id,
            title: None,
            message: None,
            created_by_discord_user_id: None,
            created_by_name: None,
            created_at: fixture.now,
            artwork_asset_id: None,
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

        let report = purge_once(&fixture.db, &fixture.storage, fixture.now)
            .await
            .expect("purge");

        assert_eq!(report.expired_evicted, 0);
        assert!(cover_file.exists());
        assert!(assets.get(cover.id).await.expect("lecture").is_some());
        // The cover is never the item's picture.
        let stored = fixture
            .db
            .media_board()
            .get(item.id)
            .await
            .expect("lecture")
            .expect("item");
        assert_eq!(stored.artwork_asset_id, None);
    }

    /// Un asset expiré épinglé comme média du board, artwork, pochette, ou lu
    /// par un job actif, ne doit jamais être évincé : les exclusions board,
    /// artwork, pochette et job sont désormais portées par `lc-db`.
    #[tokio::test]
    async fn keeps_board_artwork_cover_and_active_job_assets() {
        let fixture = fixture().await;
        let board_file = fixture.storage.root().join("board.mp4");
        let artwork_file = fixture.storage.root().join("artwork.jpg");
        let cover_file = fixture.storage.root().join("board.cover.jpg");
        let job_file = fixture.storage.root().join("job.mp4");
        std::fs::write(&board_file, b"board").expect("fichier");
        std::fs::write(&artwork_file, b"artwork").expect("fichier");
        std::fs::write(&cover_file, b"cover").expect("fichier");
        std::fs::write(&job_file, b"job").expect("fichier");

        let stale = fixture.now - TimeDuration::days(30);
        let expired = fixture.now - TimeDuration::hours(1);
        let board = asset(
            MediaStatus::Ready,
            "board-hash",
            &board_file,
            stale,
            fixture.now + TimeDuration::days(30),
            stale,
        );
        let artwork = MediaAsset {
            kind: MediaKind::Image,
            mime: "image/jpeg".into(),
            duration_sec: None,
            ..asset(
                MediaStatus::Ready,
                "artwork-hash",
                &artwork_file,
                stale,
                expired,
                stale,
            )
        };
        let cover = MediaAsset {
            kind: MediaKind::Image,
            mime: "image/jpeg".into(),
            duration_sec: None,
            ..asset(
                MediaStatus::Ready,
                "cover-hash",
                &cover_file,
                stale,
                expired,
                stale,
            )
        };
        let playing = asset(
            MediaStatus::Ready,
            "playing-hash",
            &job_file,
            stale,
            expired,
            stale,
        );
        let assets = fixture.db.media_assets();
        for row in [&board, &artwork, &cover, &playing] {
            assets.insert(row).await.expect("asset");
        }

        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
            media_asset_id: board.id,
            title: None,
            message: None,
            created_by_discord_user_id: None,
            created_by_name: None,
            created_at: fixture.now,
            artwork_asset_id: Some(artwork.id),
            artwork_on_overlay: true,
            display_duration_sec: None,
            display_scale_percent: None,
        };
        fixture
            .db
            .media_board()
            .create(&item)
            .await
            .expect("item board");
        assets
            .set_cover(board.id, Some(cover.id))
            .await
            .expect("pochette du board");

        let job = PlaybackJob {
            id: PlaybackJobId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
            media_asset_id: Some(playing.id),
            text: None,
            show_text: false,
            author_name: None,
            author_image: None,
            duration_sec: 10,
            source: JobSource::Board,
            status: JobStatus::Playing,
            resume_offset_sec: 0,
            last_playback_state_at: None,
            submission_date: fixture.now,
            execution_date: fixture.now,
            scheduled_at: fixture.now,
            started_at: Some(fixture.now),
            finished_at: None,
        };
        fixture
            .db
            .playback_jobs()
            .create(&job)
            .await
            .expect("job actif");

        let report = purge_once(&fixture.db, &fixture.storage, fixture.now)
            .await
            .expect("purge");

        assert_eq!(report.expired_evicted, 0);
        assert!(artwork_file.exists());
        assert!(cover_file.exists());
        assert!(job_file.exists());
        assert!(assets.get(artwork.id).await.expect("lecture").is_some());
        assert!(assets.get(cover.id).await.expect("lecture").is_some());
        assert!(assets.get(playing.id).await.expect("lecture").is_some());
    }

    #[tokio::test]
    async fn removes_staged_and_orphan_files_after_grace_only() {
        let fixture = fixture().await;
        let orphan = fixture.storage.root().join("orphan.mp4");
        let staged = fixture.storage.root().join("orphan.tmp-00000000.mp4");
        std::fs::write(&orphan, b"orphelin").expect("fichier");
        std::fs::write(&staged, b"transit").expect("fichier");

        let immediate = purge_once(&fixture.db, &fixture.storage, fixture.now)
            .await
            .expect("purge immédiate");
        assert_eq!(immediate.staged_files_removed, 0);
        assert_eq!(immediate.orphan_files_removed, 0);
        assert!(orphan.exists());
        assert!(staged.exists());

        let later = fixture.now + ORPHAN_GRACE + TimeDuration::minutes(1);
        let report = purge_once(&fixture.db, &fixture.storage, later)
            .await
            .expect("purge différée");
        assert_eq!(report.staged_files_removed, 1);
        assert_eq!(report.orphan_files_removed, 1);
        assert!(!orphan.exists());
        assert!(!staged.exists());
    }

    /// Régression : un `storage_path` non canonicalisé (chemin importé du
    /// legacy, sans préfixe verbatim `\\?\`) ne doit pas être confondu avec un
    /// orphelin — la purge supprimait sinon le fichier d'un asset référencé.
    #[tokio::test]
    async fn keeps_referenced_files_when_storage_path_is_not_canonical() {
        let fixture = fixture().await;
        let file = fixture.storage.root().join("legacy-import.mp4");
        std::fs::write(&file, b"media").expect("fichier");
        let canonical = std::fs::canonicalize(&file).expect("canonicalisation");
        let mut raw = canonical.to_string_lossy().into_owned();
        if let Some(stripped) = raw.strip_prefix("\\\\?\\") {
            raw = stripped.to_owned();
        }

        let row = asset(
            MediaStatus::Ready,
            "legacy-import-hash",
            StdPath::new(&raw),
            fixture.now - TimeDuration::hours(3),
            fixture.now + TimeDuration::days(30),
            fixture.now - TimeDuration::hours(4),
        );
        fixture
            .db
            .media_assets()
            .insert(&row)
            .await
            .expect("asset READY");

        let later = fixture.now + ORPHAN_GRACE + TimeDuration::minutes(1);
        let report = purge_once(&fixture.db, &fixture.storage, later)
            .await
            .expect("purge différée");

        assert_eq!(report.orphan_files_removed, 0);
        assert!(
            file.exists(),
            "un fichier référencé ne doit pas être supprimé"
        );
    }

    #[cfg(windows)]
    fn create_symlink(target: &StdPath, link: &StdPath) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(unix)]
    fn create_symlink(target: &StdPath, link: &StdPath) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    /// Régression AUD-V2-S-002 : les `storage_path` relatifs hérités du legacy
    /// sont résolus contre la racine média, jamais contre le répertoire courant
    /// du processus ; un fichier référencé reste conservé, un fichier périmé de
    /// la même forme est bien supprimé via la racine.
    #[tokio::test]
    async fn resolves_relative_storage_paths_against_the_storage_root() {
        let fixture = fixture().await;
        let referenced = fixture.storage.root().join("relative-referenced.mp4");
        let stale = fixture.storage.root().join("relative-stale.mp4");
        std::fs::write(&referenced, b"referenced").expect("fichier");
        std::fs::write(&stale, b"stale").expect("fichier");

        let ready = asset(
            MediaStatus::Ready,
            "relative-referenced-hash",
            StdPath::new("./data/media/relative-referenced.mp4"),
            fixture.now - TimeDuration::hours(3),
            fixture.now + TimeDuration::days(30),
            fixture.now - TimeDuration::hours(4),
        );
        let failed = asset(
            MediaStatus::Failed,
            "relative-stale-hash",
            StdPath::new("data\\media\\relative-stale.mp4"),
            fixture.now - TimeDuration::hours(3),
            fixture.now,
            fixture.now - TimeDuration::hours(4),
        );
        fixture
            .db
            .media_assets()
            .insert(&ready)
            .await
            .expect("asset relatif référencé");
        fixture
            .db
            .media_assets()
            .insert(&failed)
            .await
            .expect("asset relatif périmé");

        let later = fixture.now + ORPHAN_GRACE + TimeDuration::minutes(1);
        let report = purge_once(&fixture.db, &fixture.storage, later)
            .await
            .expect("purge");

        assert_eq!(report.failed_removed, 1);
        assert_eq!(report.orphan_files_removed, 0);
        assert!(
            referenced.exists(),
            "un chemin relatif référencé ne doit pas être traité comme orphelin"
        );
        assert!(
            !stale.exists(),
            "le fichier d'une ligne périmée doit être supprimé via la racine"
        );
    }

    /// Régression AUD-V2-S-002 : un `storage_path` absolu hors racine ne doit
    /// jamais être supprimé, même quand la ligne est purgée.
    #[tokio::test]
    async fn never_removes_files_outside_the_storage_root() {
        let fixture = fixture().await;
        let outside = tempfile::tempdir().expect("hors racine");
        let secret = outside.path().join("secret.mp4");
        std::fs::write(&secret, b"secret").expect("fichier hors racine");

        let row = asset(
            MediaStatus::Ready,
            "outside-hash",
            &secret,
            fixture.now - TimeDuration::hours(2),
            fixture.now - TimeDuration::hours(1),
            fixture.now - TimeDuration::hours(3),
        );
        fixture
            .db
            .media_assets()
            .insert(&row)
            .await
            .expect("asset hors racine");

        let report = purge_once(&fixture.db, &fixture.storage, fixture.now)
            .await
            .expect("purge");

        assert_eq!(report.expired_evicted, 1);
        assert!(
            secret.exists(),
            "un fichier hors racine ne doit jamais être supprimé"
        );
    }

    /// Régression AUD-V2-S-004 : deux assets partageant le même fichier ; la
    /// purge d'une ligne ne doit pas casser la seconde.
    #[tokio::test]
    async fn keeps_files_shared_by_two_assets() {
        let fixture = fixture().await;
        let shared = fixture.storage.root().join("shared.mp4");
        std::fs::write(&shared, b"shared").expect("fichier partagé");

        let expired = asset(
            MediaStatus::Ready,
            "shared-expired-hash",
            &shared,
            fixture.now - TimeDuration::hours(2),
            fixture.now - TimeDuration::hours(1),
            fixture.now - TimeDuration::hours(3),
        );
        let active = asset(
            MediaStatus::Ready,
            "shared-active-hash",
            &shared,
            fixture.now,
            fixture.now + TimeDuration::days(30),
            fixture.now - TimeDuration::hours(1),
        );
        fixture
            .db
            .media_assets()
            .insert(&expired)
            .await
            .expect("asset expiré");
        fixture
            .db
            .media_assets()
            .insert(&active)
            .await
            .expect("asset actif");

        let report = purge_once(&fixture.db, &fixture.storage, fixture.now)
            .await
            .expect("purge");

        assert_eq!(report.expired_evicted, 1);
        assert!(
            shared.exists(),
            "le fichier est encore référencé par l'autre asset"
        );
        assert!(fixture
            .db
            .media_assets()
            .get(active.id)
            .await
            .expect("lecture")
            .is_some());
    }

    /// Régression AUD-V2-S-001 : un lien symbolique dans la racine ne doit pas
    /// faire supprimer sa cible hors racine.
    #[tokio::test]
    async fn never_follows_symbolic_links_outside_the_storage_root() {
        let fixture = fixture().await;
        let outside = tempfile::tempdir().expect("hors racine");
        let target = outside.path().join("secret.mp4");
        std::fs::write(&target, b"secret").expect("cible hors racine");
        let link = fixture.storage.root().join("innocent.mp4");
        if create_symlink(&target, &link).is_err() {
            // Création de lien symbolique refusée (privilège Windows) : le
            // scénario n'est pas reproductible sur cette machine.
            return;
        }

        let later = fixture.now + ORPHAN_GRACE + TimeDuration::minutes(1);
        let report = purge_once(&fixture.db, &fixture.storage, later)
            .await
            .expect("purge");

        assert_eq!(report.orphan_files_removed, 0);
        assert!(link.exists(), "le lien lui-même ne doit pas être supprimé");
        assert!(
            target.exists(),
            "la cible hors racine ne doit jamais être supprimée"
        );
    }
}
