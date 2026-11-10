//! Budgets de stockage : cache non persistant et meme board persistant.
//!
//! Le legacy distinguait deux budgets :
//!
//! - le cache (médias non épinglés) est évincé LRU quand une nouvelle ingestion
//!   dépasserait `MEDIA_CACHE_MAX_TOTAL_MB` ; les médias épinglés au meme board
//!   ou référencés par un job `PENDING`/`PLAYING` sont protégés ;
//! - le meme board ne s'évince jamais automatiquement : un ajout qui dépasserait
//!   `MEDIA_BOARD_MAX_TOTAL_MB` échoue en `BOARD_STORAGE_LIMIT_REACHED`.
//!
//! Les décisions pures (tri, plan d'éviction) viennent de
//! [`lc_media::cache`] ; ce module fournit les entrées d'éviction depuis SQLite
//! et applique le plan (suppression des fichiers puis des lignes).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use lc_db::{Db, DbError};
use lc_domain::enums::MediaStatus;
use lc_domain::ids::GuildId;
use lc_media::cache::{plan_cache_eviction, CacheEntry, CacheEvictionPlan};
use lc_media::config::MediaConfig;
use lc_media::error::{MediaError, MediaErrorCode};
use lc_media::storage::MediaStorage;

use crate::media::assets::cache_entry;
use crate::media::errors::{MediaServiceError, MediaServiceResult};

const BYTES_PER_MEGABYTE: u64 = 1024 * 1024;

/// Construit les entrées d'éviction du cache depuis SQLite.
///
/// `pinned` = présent dans le meme board (toutes guildes), `active` = référencé
/// par un job `PENDING`/`PLAYING`. Les repositories `lc-db` n'exposent pas de
/// requête « assets référencés » : on interroge directement le pool sur les
/// colonnes du schéma (documenté dans `docs/rust/server-ingest.md`).
pub async fn cache_entries(db: &Db) -> Result<Vec<CacheEntry>, DbError> {
    let assets = db
        .media_assets()
        .list_by_status(MediaStatus::Ready, 0)
        .await?;
    let pinned =
        query_asset_ids(db, "SELECT DISTINCT media_asset_id FROM meme_board_items").await?;
    let active = query_asset_ids(
        db,
        "SELECT DISTINCT media_asset_id FROM playback_jobs \
          WHERE media_asset_id IS NOT NULL AND status IN ('PENDING', 'PLAYING')",
    )
    .await?;

    Ok(assets
        .iter()
        .map(|asset| {
            let id = asset.id.to_canonical();
            cache_entry(asset, pinned.contains(&id), active.contains(&id))
        })
        .collect())
}

/// Exécute une requête d'identifiants d'assets et retourne l'ensemble.
async fn query_asset_ids(db: &Db, sql: &str) -> Result<HashSet<String>, DbError> {
    let ids = sqlx::query_scalar::<_, String>(sql)
        .fetch_all(db.pool())
        .await?;
    Ok(ids.into_iter().collect())
}

/// Applique le budget du cache avant de persister un nouveau média.
///
/// `required_bytes` est la taille du fichier normalisé, `exclude_source_hash`
/// l'empreinte du média en cours d'ingestion (jamais évincé). En cas de budget
/// insuffisant, renvoie `CACHE_STORAGE_LIMIT_REACHED` (422 côté route).
pub async fn enforce_cache_budget(
    db: &Db,
    storage: &MediaStorage,
    config: &MediaConfig,
    required_bytes: u64,
    exclude_source_hash: Option<&str>,
) -> MediaServiceResult<()> {
    let entries = cache_entries(db).await?;
    let plan = plan_cache_eviction(
        &entries,
        required_bytes,
        config.storage.cache_max_total_bytes,
        exclude_source_hash,
    );

    if !plan.has_capacity {
        return Err(MediaServiceError::Media(cache_limit_error(&plan)));
    }

    for key in &plan.evicted_keys {
        let Some(asset) = db.media_assets().get_by_source_hash(key).await? else {
            continue;
        };

        // Supprime les variantes `<root>/<hash>.*` puis, en dernier recours, le
        // chemin persisté (qui peut être hors racine si la configuration a
        // changé entre deux démarrages).
        storage.remove_variants(&asset.source_hash)?;
        if !asset.storage_path.is_empty() {
            storage.remove_path(std::path::Path::new(&asset.storage_path))?;
        }

        sqlx::query("DELETE FROM media_assets WHERE id = ?")
            .bind(asset.id.to_canonical())
            .execute(db.pool())
            .await
            .map_err(DbError::from)?;

        tracing::info!(
            source_hash = %asset.source_hash,
            size_bytes = asset.size_bytes,
            "asset média évincé du cache"
        );
    }

    Ok(())
}

/// Construit l'erreur legacy de dépassement du cache.
fn cache_limit_error(plan: &CacheEvictionPlan) -> MediaError {
    MediaError::new(
        MediaErrorCode::CacheStorageLimitReached,
        format!(
            "Cache storage limit reached ({used}/{max} MB). Unable to store {missing} MB more.",
            used = bytes_to_megabytes(plan.total_before_bytes),
            max = bytes_to_megabytes(plan.max_total_bytes),
            missing = bytes_to_megabytes(plan.missing_bytes()),
        ),
    )
}

/// Somme des tailles des médias du meme board d'une guilde.
pub async fn board_usage_bytes(db: &Db, guild_id: &GuildId) -> Result<i64, DbError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(assets.size_bytes), 0) \
           FROM meme_board_items AS items \
           JOIN media_assets AS assets ON assets.id = items.media_asset_id \
          WHERE items.guild_id = ?",
    )
    .bind(guild_id.as_str())
    .fetch_one(db.pool())
    .await
    .map_err(DbError::from)
}

/// Vérifie que le meme board de la guilde peut accueillir un média.
pub async fn ensure_board_capacity(
    db: &Db,
    config: &MediaConfig,
    guild_id: &GuildId,
    requested_bytes: i64,
) -> MediaServiceResult<()> {
    let current = board_usage_bytes(db, guild_id).await?;
    let max = i64::try_from(config.storage.board_max_total_bytes).unwrap_or(i64::MAX);

    if requested_bytes > 0 && current.saturating_add(requested_bytes) > max {
        return Err(MediaServiceError::Media(board_limit_error(
            current,
            max,
            requested_bytes,
        )));
    }

    Ok(())
}

/// Construit l'erreur legacy de dépassement du meme board.
fn board_limit_error(current: i64, max: i64, requested: i64) -> MediaError {
    MediaError::new(
        MediaErrorCode::BoardStorageLimitReached,
        format!(
            "Persistent meme board storage limit reached ({current}/{max} MB). \
             Requested media: {requested} MB. Remove older board media first.",
            current = bytes_to_megabytes(u64::try_from(current).unwrap_or(0)),
            max = bytes_to_megabytes(u64::try_from(max).unwrap_or(0)),
            requested = bytes_to_megabytes(u64::try_from(requested).unwrap_or(0)),
        ),
    )
}

/// Conversion octets → mégaoctets arrondis (messages d'erreur legacy).
fn bytes_to_megabytes(bytes: u64) -> u64 {
    bytes / BYTES_PER_MEGABYTE
}

/// Récupère les assets prêts dont l'expiration est dépassée ou dont le fichier
/// n'existe plus (worker de purge Phase 5b, exposé pour les tests).
pub async fn purge_candidates(
    db: &Db,
    config: &MediaConfig,
    now: DateTime<Utc>,
) -> MediaServiceResult<Vec<String>> {
    let entries = cache_entries(db).await?;
    let ttl_ms = i64::try_from(config.storage.cache_ttl.as_millis()).unwrap_or(i64::MAX);
    Ok(lc_media::cache::purge_candidates(
        &entries,
        lc_domain::time::datetime_to_ms(now),
        ttl_ms,
    ))
}

/// Évince explicitement une liste d'empreintes (utilisé par les tests et la
/// purge de Phase 5b).
pub async fn evict_source_hashes(
    db: &Db,
    storage: &MediaStorage,
    source_hashes: &[String],
) -> MediaServiceResult<usize> {
    let mut evicted = 0;
    for hash in source_hashes {
        let Some(asset) = db.media_assets().get_by_source_hash(hash).await? else {
            continue;
        };
        storage.remove_variants(&asset.source_hash)?;
        if !asset.storage_path.is_empty() {
            storage.remove_path(std::path::Path::new(&asset.storage_path))?;
        }
        sqlx::query("DELETE FROM media_assets WHERE id = ?")
            .bind(asset.id.to_canonical())
            .execute(db.pool())
            .await
            .map_err(DbError::from)?;
        evicted += 1;
    }
    Ok(evicted)
}

/// Taille d'un asset déjà persisté, en octets non signés.
pub fn asset_size_bytes(asset: &lc_domain::entities::MediaAsset) -> u64 {
    u64::try_from(asset.size_bytes).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use chrono::Duration;
    use lc_db::DbConfig;
    use lc_domain::entities::{MediaAsset, PlaybackJob};
    use lc_domain::enums::{JobStatus, MediaKind};
    use lc_domain::ids::{GuildId, MediaAssetId, PlaybackJobId};
    use lc_domain::time::datetime_to_ms;
    use lc_media::storage::MediaStorage;
    use uuid::Uuid;

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Db,
        storage: MediaStorage,
        config: MediaConfig,
        now: DateTime<Utc>,
    }

    async fn fixture(cache_max_bytes: u64, board_max_bytes: u64) -> Fixture {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        let mut config = MediaConfig::default();
        config.storage.directory = dir.path().join("media");
        config.storage.cache_max_total_bytes = cache_max_bytes;
        config.storage.board_max_total_bytes = board_max_bytes;
        let storage = MediaStorage::new(&config.storage);
        storage.ensure().expect("racine de stockage");
        Fixture {
            _dir: dir,
            db,
            storage,
            config,
            now: Utc::now(),
        }
    }

    async fn insert_asset(
        db: &Db,
        dir: &std::path::Path,
        hash: &str,
        size: i64,
        last_accessed_at: DateTime<Utc>,
    ) -> MediaAsset {
        let path = dir.join(format!("{hash}.mp4"));
        tokio::fs::write(&path, vec![0_u8; usize::try_from(size).unwrap()])
            .await
            .expect("fichier média");
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: hash.to_owned(),
            source_url: format!("https://example.com/{hash}"),
            kind: MediaKind::Video,
            mime: "video/mp4".into(),
            duration_sec: Some(5),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: size,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at,
            expires_at: last_accessed_at + Duration::hours(1),
            created_at: last_accessed_at - Duration::minutes(1),
        };
        db.media_assets()
            .insert(&asset)
            .await
            .expect("asset inséré");
        asset
    }

    #[tokio::test]
    async fn evicts_least_recently_used_asset_to_make_room() {
        let fixture = fixture(10, 1_000).await;
        let dir = fixture.config.storage.directory.clone();
        let older = fixture.now - Duration::hours(2);
        let recent = fixture.now - Duration::minutes(5);

        let first = insert_asset(&fixture.db, &dir, "hash-old", 6, older).await;
        let second = insert_asset(&fixture.db, &dir, "hash-new", 6, recent).await;

        // 6 + 6 > 10 : le plus ancien doit être évincé.
        enforce_cache_budget(&fixture.db, &fixture.storage, &fixture.config, 0, None)
            .await
            .expect("budget respecté");

        assert!(fixture
            .db
            .media_assets()
            .get(first.id)
            .await
            .expect("lecture")
            .is_none());
        assert!(fixture
            .db
            .media_assets()
            .get(second.id)
            .await
            .expect("lecture")
            .is_some());
    }

    #[tokio::test]
    async fn active_assets_are_not_evicted_and_budget_fails() {
        let fixture = fixture(10, 1_000).await;
        let dir = fixture.config.storage.directory.clone();
        let asset = insert_asset(&fixture.db, &dir, "hash-active", 6, fixture.now).await;

        // Un job PENDING référence l'asset : il est protégé de l'éviction mais
        // reste compté dans le total du cache non persistant.
        let guild = GuildId::parse("123456789012345678").expect("guilde");
        let job = PlaybackJob {
            id: PlaybackJobId::new_v4(),
            guild_id: guild,
            media_asset_id: Some(asset.id),
            text: None,
            show_text: false,
            author_name: None,
            author_image: None,
            duration_sec: 5,
            priority: 0,
            status: JobStatus::Pending,
            resumes_after_job_id: None,
            resume_offset_sec: 0,
            remaining_ms_snapshot: None,
            last_playback_state_at: None,
            submission_date: fixture.now,
            execution_date: fixture.now,
            scheduled_at: fixture.now,
            started_at: None,
            finished_at: None,
        };
        fixture
            .db
            .playback_jobs()
            .create(&job)
            .await
            .expect("job inséré");

        let error = enforce_cache_budget(&fixture.db, &fixture.storage, &fixture.config, 6, None)
            .await
            .expect_err("budget insuffisant : l'asset actif ne peut pas être évincé");

        let media = error.media_error().expect("erreur média");
        assert_eq!(media.code(), MediaErrorCode::CacheStorageLimitReached);
        assert!(media.message().contains("Cache storage limit reached"));
        assert!(fixture
            .db
            .media_assets()
            .get(asset.id)
            .await
            .expect("lecture")
            .is_some());
    }

    #[tokio::test]
    async fn board_capacity_is_enforced_per_guild() {
        let fixture = fixture(1_000, 10).await;
        let dir = fixture.config.storage.directory.clone();
        let asset = insert_asset(&fixture.db, &dir, "hash-board", 20, fixture.now).await;
        let guild = GuildId::parse("123456789012345678").expect("guilde");

        // Guilde vide : tout passe.
        ensure_board_capacity(&fixture.db, &fixture.config, &guild, 5)
            .await
            .expect("guilde vide");

        // Un élément de 20 octets occupe déjà plus que le budget de 10 octets.
        sqlx::query(
            "INSERT INTO meme_board_items (id, guild_id, media_asset_id, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(guild.as_str())
        .bind(asset.id.to_canonical())
        .bind(datetime_to_ms(fixture.now))
        .execute(fixture.db.pool())
        .await
        .expect("élément de board");

        let error = ensure_board_capacity(&fixture.db, &fixture.config, &guild, 5)
            .await
            .expect_err("budget board insuffisant");
        let media = error.media_error().expect("erreur média");
        assert_eq!(media.code(), MediaErrorCode::BoardStorageLimitReached);
        assert!(media
            .message()
            .contains("Persistent meme board storage limit reached"));
    }
}
