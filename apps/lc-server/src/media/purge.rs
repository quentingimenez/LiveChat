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

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lc_db::Db;
use lc_domain::time::datetime_to_ms;
use lc_media::storage::{is_staged_file_name, MediaStorage};
use sqlx::Row as _;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::app::AppState;
use crate::media::assets;
use crate::media::errors::MediaServiceResult;

/// Intervalle entre deux passes de purge (10 minutes, comme les workers legacy).
pub const PURGE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Grâce avant de supprimer un fichier inconnu de la base ou un fichier de
/// transit : un fichier peut être écrit juste avant sa persistance en base, ou
/// être en cours d'écriture par une ingestion concurrente.
pub const ORPHAN_GRACE: ChronoDuration = ChronoDuration::hours(1);

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
    now: DateTime<Utc>,
) -> MediaServiceResult<PurgeReport> {
    // 1. Assets `FAILED` périmés (jamais épinglés par le board ni actifs).
    let failed_removed = purge_stale_status(
        db,
        storage,
        "FAILED",
        datetime_to_ms(now - assets::FAILED_TTL),
    )
    .await?;

    // 2. Assets `PROCESSING` abandonnés (crash du serveur pendant l'ingestion).
    let processing_removed = purge_stale_status(
        db,
        storage,
        "PROCESSING",
        datetime_to_ms(now - assets::STALE_PROCESSING_TTL),
    )
    .await?;

    // 3. Assets `READY` expirés (hors épinglage et hors lecture active).
    let expired_evicted = purge_expired_ready(db, storage, datetime_to_ms(now)).await?;

    // 4. Fichiers du stockage : transit périmé puis orphelins.
    let (staged_files_removed, orphan_files_removed) =
        purge_storage_files(db, storage, now).await?;

    Ok(PurgeReport {
        failed_removed,
        processing_removed,
        expired_evicted,
        staged_files_removed,
        orphan_files_removed,
    })
}

/// Supprime les lignes d'un statut terminal dont `last_accessed_at` est antérieur
/// à `cutoff_ms`, en excluant le board épinglé et les jobs actifs.
async fn purge_stale_status(
    db: &Db,
    storage: &MediaStorage,
    status: &str,
    cutoff_ms: i64,
) -> MediaServiceResult<usize> {
    let rows = sqlx::query(
        "SELECT id, storage_path FROM media_assets \
          WHERE status = ? AND last_accessed_at <= ? \
            AND id NOT IN (SELECT media_asset_id FROM meme_board_items \
                            WHERE media_asset_id IS NOT NULL) \
            AND id NOT IN (SELECT media_asset_id FROM playback_jobs \
                            WHERE media_asset_id IS NOT NULL \
                              AND status IN ('PENDING', 'PLAYING'))",
    )
    .bind(status)
    .bind(cutoff_ms)
    .fetch_all(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;

    let mut removed = 0;
    for row in rows {
        let id: String = row.try_get("id").map_err(lc_db::DbError::from)?;
        let storage_path: String = row.try_get("storage_path").map_err(lc_db::DbError::from)?;
        if !storage_path.trim().is_empty() {
            // Le fichier peut déjà avoir été remplacé/supprimé : best effort.
            storage.remove_path(Path::new(&storage_path))?;
        }
        let affected = sqlx::query("DELETE FROM media_assets WHERE id = ?")
            .bind(&id)
            .execute(db.pool())
            .await
            .map_err(lc_db::DbError::from)?
            .rows_affected();
        if affected > 0 {
            removed += 1;
        }
    }

    Ok(removed)
}

/// Évince les assets `READY` expirés (fichier + variantes + ligne).
async fn purge_expired_ready(
    db: &Db,
    storage: &MediaStorage,
    now_ms: i64,
) -> MediaServiceResult<usize> {
    let rows = sqlx::query(
        "SELECT id, source_hash, storage_path FROM media_assets \
          WHERE status = 'READY' AND expires_at <= ? \
            AND id NOT IN (SELECT media_asset_id FROM meme_board_items \
                            WHERE media_asset_id IS NOT NULL) \
            AND id NOT IN (SELECT media_asset_id FROM playback_jobs \
                            WHERE media_asset_id IS NOT NULL \
                              AND status IN ('PENDING', 'PLAYING'))",
    )
    .bind(now_ms)
    .fetch_all(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;

    let mut removed = 0;
    for row in rows {
        let id: String = row.try_get("id").map_err(lc_db::DbError::from)?;
        let source_hash: String = row.try_get("source_hash").map_err(lc_db::DbError::from)?;
        let storage_path: String = row.try_get("storage_path").map_err(lc_db::DbError::from)?;

        // Course possible avec une ingestion qui vient de réarmer la ligne :
        // le DELETE conditionnel ne supprime que l'état expiré observé.
        let affected = sqlx::query(
            "DELETE FROM media_assets \
              WHERE id = ? AND status = 'READY' AND expires_at <= ?",
        )
        .bind(&id)
        .bind(now_ms)
        .execute(db.pool())
        .await
        .map_err(lc_db::DbError::from)?
        .rows_affected();
        if affected == 0 {
            continue;
        }

        storage.remove_variants(&source_hash)?;
        if !storage_path.trim().is_empty() {
            storage.remove_path(Path::new(&storage_path))?;
        }
        removed += 1;
    }

    Ok(removed)
}

/// Supprime les fichiers de transit et orphelins du stockage, avec grâce.
async fn purge_storage_files(
    db: &Db,
    storage: &MediaStorage,
    now: DateTime<Utc>,
) -> MediaServiceResult<(usize, usize)> {
    let known_rows = sqlx::query("SELECT storage_path FROM media_assets WHERE storage_path != ''")
        .fetch_all(db.pool())
        .await
        .map_err(lc_db::DbError::from)?;
    let mut known: HashSet<PathBuf> = HashSet::with_capacity(known_rows.len());
    for row in &known_rows {
        let storage_path: String = row.try_get("storage_path").map_err(lc_db::DbError::from)?;
        // Les lignes importées du legacy (ou écrites avant canonicalisation de
        // la racine) peuvent porter un chemin relatif, avec une casse
        // différente ou sans préfixe verbatim `\\?\`. La comparaison avec les
        // entrées de `read_dir` (qui héritent du préfixe de la racine
        // canonicalisée) doit donc se faire sur des chemins normalisés, sous
        // peine de supprimer un fichier référencé comme « orphelin ».
        known.insert(normalized_path(Path::new(&storage_path)));
    }

    let grace_ms = ORPHAN_GRACE.num_milliseconds();
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
            if storage.remove_path(&variant.path)? {
                staged += 1;
            }
            continue;
        }
        let variant_path = normalized_path(&variant.path);
        if !known.contains(&variant_path) && storage.remove_path(&variant_path)? {
            orphans += 1;
        }
    }

    Ok((staged, orphans))
}

/// Normalise un chemin pour la comparaison des ensembles : `canonicalize`
/// résout le préfixe verbatim `\\?\`, les liens et la casse ; en cas d'échec
/// (fichier déjà supprimé, droits), on retombe sur le chemin brut.
fn normalized_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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

    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use lc_db::DbConfig;
    use lc_domain::entities::{MediaAsset, MemeBoardItem};
    use lc_domain::enums::{MediaKind, MediaStatus};
    use lc_domain::ids::{GuildId, MediaAssetId, MemeBoardItemId};
    use lc_media::config::MediaConfig;
    use lc_media::storage::MediaStorage;

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Db,
        storage: MediaStorage,
        now: DateTime<Utc>,
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
            now: Utc::now(),
        }
    }

    fn asset(
        status: MediaStatus,
        source_hash: &str,
        storage_path: &StdPath,
        last_accessed_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) -> MediaAsset {
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: source_hash.to_owned(),
            source_url: format!("https://example.com/{source_hash}.mp4"),
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
            fixture.now - ChronoDuration::hours(3),
            fixture.now,
            fixture.now - ChronoDuration::hours(4),
        );
        let processing = asset(
            MediaStatus::Processing,
            "processing-hash",
            StdPath::new(""),
            fixture.now - ChronoDuration::hours(3),
            fixture.now,
            fixture.now - ChronoDuration::hours(4),
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
            fixture.now - ChronoDuration::hours(2),
            fixture.now - ChronoDuration::hours(1),
            fixture.now - ChronoDuration::hours(3),
        );
        let expired = asset(
            MediaStatus::Ready,
            "expired-hash",
            &expired_file,
            fixture.now - ChronoDuration::hours(2),
            fixture.now - ChronoDuration::hours(1),
            fixture.now - ChronoDuration::hours(3),
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

        let item = MemeBoardItem {
            id: MemeBoardItemId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
            media_asset_id: pinned.id,
            title: None,
            message: None,
            created_by_discord_user_id: None,
            created_by_name: None,
            created_at: fixture.now,
        };
        fixture
            .db
            .meme_board()
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

        let later = fixture.now + ORPHAN_GRACE + ChronoDuration::minutes(1);
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
            fixture.now - ChronoDuration::hours(3),
            fixture.now + ChronoDuration::days(30),
            fixture.now - ChronoDuration::hours(4),
        );
        fixture
            .db
            .media_assets()
            .insert(&row)
            .await
            .expect("asset READY");

        let later = fixture.now + ORPHAN_GRACE + ChronoDuration::minutes(1);
        let report = purge_once(&fixture.db, &fixture.storage, later)
            .await
            .expect("purge différée");

        assert_eq!(report.orphan_files_removed, 0);
        assert!(
            file.exists(),
            "un fichier référencé ne doit pas être supprimé"
        );
    }
}
