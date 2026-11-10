//! Persistance et cycle de vie des `MediaAsset`.
//!
//! Le legacy suivait un cycle explicite : ligne `PROCESSING` créée avant le
//! téléchargement, puis `READY` avec les métadonnées du fichier normalisé, ou
//! `FAILED` avec `"[CODE] message"` (tronqué à 500 caractères) et une expiration
//! courte. Ces fonctions reproduisent ce cycle au-dessus des repositories de
//! [`lc_db`].
//!
//! Deux mises à jour n'existent pas dans l'API `lc-db` (mise à jour complète de
//! l'asset prêt, suppression après éviction) : elles passent par des requêtes
//! SQL directes sur le pool, avec les mêmes colonnes que le schéma `lc-db`
//! (`crates/lc-db/migrations/0001_init.sql`). C'est documenté dans
//! `docs/rust/server-ingest.md`.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lc_db::Db;
use lc_domain::entities::MediaAsset;
use lc_domain::enums::{MediaKind, MediaStatus};
use lc_domain::ids::MediaAssetId;
use lc_domain::time::datetime_to_ms;
use lc_media::cache::CacheEntry;
use lc_media::config::MediaConfig;
use lc_media::error::MediaError;
use lc_media::ingest::IngestedMedia;
use lc_media::source::MediaSource;

use crate::media::errors::{MediaServiceError, MediaServiceResult};

/// Durée pendant laquelle un asset `FAILED`/`PROCESSING` abandonné est conservé
/// (`2 h` côté legacy avant purge du worker).
pub const STALE_PROCESSING_TTL: ChronoDuration = ChronoDuration::hours(2);

/// Durée de conservation d'un asset `FAILED` : elle laisse le temps au code
/// d'erreur d'être consulté sans hacher immédiatement le cache.
pub const FAILED_TTL: ChronoDuration = ChronoDuration::hours(1);

/// Durée d'épinglage d'un élément du meme board (100 ans, comme le legacy).
pub const PINNED_TTL_DAYS: i64 = 36_500;

/// Champ `error` maximal du legacy (`meda_assets.error`), tronqué proprement.
pub const MAX_ERROR_CHARS: usize = 500;

/// Données d'un média prêt à être persisté en `READY`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyAsset {
    /// Empreinte de la source résolue.
    pub source_hash: String,
    /// URL canonique de la source résolue.
    pub source_url: String,
    /// Type média normalisé.
    pub kind: MediaKind,
    /// Type MIME du fichier persisté.
    pub mime: String,
    /// Durée en secondes (médias temporels uniquement).
    pub duration_sec: Option<i64>,
    /// Largeur en pixels.
    pub width: Option<i64>,
    /// Hauteur en pixels.
    pub height: Option<i64>,
    /// Orientation verticale (vidéo portrait).
    pub is_vertical: bool,
    /// Chemin absolu du fichier normalisé.
    pub storage_path: String,
    /// Taille du fichier en octets.
    pub size_bytes: i64,
}

impl ReadyAsset {
    /// Extrait les champs persistables du résultat du pipeline.
    pub fn from_ingested(ingested: &IngestedMedia) -> Self {
        let normalized = &ingested.normalized;
        Self {
            source_hash: ingested.source.source_hash.clone(),
            source_url: ingested.source.canonical_url.clone(),
            kind: normalized.kind,
            mime: normalized.mime.clone(),
            duration_sec: normalized.duration_sec,
            width: normalized.width.map(i64::from),
            height: normalized.height.map(i64::from),
            is_vertical: normalized.is_vertical,
            storage_path: normalized.storage_path.to_string_lossy().into_owned(),
            size_bytes: i64::try_from(normalized.size_bytes).unwrap_or(i64::MAX),
        }
    }

    /// Taille en octets non signée (pour les budgets de cache).
    pub fn size_bytes_u64(&self) -> u64 {
        u64::try_from(self.size_bytes).unwrap_or(0)
    }
}

/// Conversion d'une durée de configuration en durée chrono, saturante.
pub fn cache_ttl(config: &MediaConfig) -> ChronoDuration {
    ChronoDuration::seconds(i64::try_from(config.storage.cache_ttl.as_secs()).unwrap_or(i64::MAX))
}

/// Cherche un asset réutilisable pour cette empreinte de source.
///
/// Conditions identiques au legacy : statut `READY`, non expiré et fichier
/// toujours présent sur disque. Un asset réutilisé voit son expiration repoussée
/// (`touchMediaAsset`).
pub async fn find_reusable(
    db: &Db,
    config: &MediaConfig,
    source_hash: &str,
    now: DateTime<Utc>,
) -> MediaServiceResult<Option<MediaAsset>> {
    let assets = db.media_assets();
    let Some(asset) = assets.get_by_source_hash(source_hash).await? else {
        return Ok(None);
    };

    if !asset.is_ready() || asset.is_expired_at(now) {
        return Ok(None);
    }

    if !tokio::fs::try_exists(&asset.storage_path)
        .await
        .unwrap_or(false)
    {
        return Ok(None);
    }

    // La nouvelle expiration ne raccourcit jamais celle déjà accordée.
    let next_expiry = (now + cache_ttl(config)).max(asset.expires_at);
    assets
        .touch_last_accessed(asset.id, now, next_expiry)
        .await?;

    Ok(assets.get(asset.id).await?)
}

/// Crée (ou réarme) la ligne `PROCESSING` d'une ingestion.
///
/// L'identifiant est conservé si une ligne existe déjà pour la même empreinte de
/// source ; la ligne est remise en `PROCESSING` avec une nouvelle expiration.
pub async fn begin_processing(
    db: &Db,
    source: &MediaSource,
    now: DateTime<Utc>,
    ttl: ChronoDuration,
) -> MediaServiceResult<MediaAsset> {
    let assets = db.media_assets();
    let expires_at = now + ttl;

    if let Some(existing) = assets.get_by_source_hash(&source.source_hash).await? {
        sqlx::query(
            "UPDATE media_assets \
                SET source_url = ?, status = 'PROCESSING', error = NULL, \
                    last_accessed_at = ?, expires_at = ? \
              WHERE id = ?",
        )
        .bind(source.canonical_url.as_str())
        .bind(datetime_to_ms(now))
        .bind(datetime_to_ms(expires_at))
        .bind(existing.id.to_canonical())
        .execute(db.pool())
        .await
        .map_err(lc_db::DbError::from)?;

        return Ok(assets.get(existing.id).await?.unwrap_or(existing));
    }

    let asset = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: source.source_hash.clone(),
        source_url: source.canonical_url.clone(),
        // Le legacy créait la ligne provisoire en `VIDEO` /
        // `application/octet-stream` avant correction par le pipeline.
        kind: MediaKind::Video,
        mime: "application/octet-stream".to_owned(),
        duration_sec: None,
        width: None,
        height: None,
        is_vertical: false,
        storage_path: String::new(),
        size_bytes: 0,
        status: MediaStatus::Processing,
        error: None,
        last_accessed_at: now,
        expires_at,
        created_at: now,
    };

    match assets.insert(&asset).await {
        Ok(()) => Ok(asset),
        // Course entre deux ingestions concurrentes de la même source : la
        // ligne gagnante est réutilisée.
        Err(error) if error.is_unique_violation() => {
            if let Some(existing) = assets.get_by_source_hash(&source.source_hash).await? {
                return Ok(existing);
            }
            Err(MediaServiceError::Database(error))
        }
        Err(error) => Err(MediaServiceError::Database(error)),
    }
}

/// Passe un asset en `READY` avec les métadonnées du fichier normalisé.
pub async fn mark_ready(
    db: &Db,
    asset_id: MediaAssetId,
    ready: &ReadyAsset,
    now: DateTime<Utc>,
    ttl: ChronoDuration,
) -> MediaServiceResult<MediaAsset> {
    sqlx::query(
        "UPDATE media_assets \
            SET source_hash = ?, source_url = ?, kind = ?, mime = ?, duration_sec = ?, \
                width = ?, height = ?, is_vertical = ?, storage_path = ?, size_bytes = ?, \
                status = 'READY', error = NULL, last_accessed_at = ?, expires_at = ? \
          WHERE id = ?",
    )
    .bind(ready.source_hash.as_str())
    .bind(ready.source_url.as_str())
    .bind(ready.kind.as_db_str())
    .bind(ready.mime.as_str())
    .bind(ready.duration_sec)
    .bind(ready.width)
    .bind(ready.height)
    .bind(i64::from(ready.is_vertical))
    .bind(ready.storage_path.as_str())
    .bind(ready.size_bytes)
    .bind(datetime_to_ms(now))
    .bind(datetime_to_ms(now + ttl))
    .bind(asset_id.to_canonical())
    .execute(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;

    let assets = db.media_assets();
    assets.get(asset_id).await?.ok_or_else(|| {
        MediaServiceError::Database(lc_db::DbError::Domain(lc_domain::DomainError::not_found(
            "media_assets",
            asset_id.to_string(),
        )))
    })
}

/// Épingle un asset `READY` (expiration à 100 ans) pour sa présence au board.
///
/// Parité legacy `ensurePinnedExpiry` : seules les échéances plus courtes sont
/// repoussées, un asset déjà épinglé n'est pas réécrit inutilement. La mise à
/// jour est conditionnée à `READY` pour ne jamais épingler un asset en échec.
pub async fn pin(
    db: &Db,
    asset_id: MediaAssetId,
    now: DateTime<Utc>,
) -> MediaServiceResult<MediaAsset> {
    let expiry = pinned_expiry(now);
    sqlx::query(
        "UPDATE media_assets \
            SET last_accessed_at = ?, expires_at = ? \
          WHERE id = ? AND status = 'READY' AND expires_at < ?",
    )
    .bind(datetime_to_ms(now))
    .bind(datetime_to_ms(expiry))
    .bind(asset_id.to_canonical())
    .bind(datetime_to_ms(expiry))
    .execute(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;

    let assets = db.media_assets();
    assets.get(asset_id).await?.ok_or_else(|| {
        MediaServiceError::Database(lc_db::DbError::Domain(lc_domain::DomainError::not_found(
            "media_assets",
            asset_id.to_string(),
        )))
    })
}

/// Passe un asset en `FAILED` avec `"[CODE] message"` et une expiration courte.
pub async fn mark_failed(
    db: &Db,
    asset_id: MediaAssetId,
    error: &MediaError,
    now: DateTime<Utc>,
) -> MediaServiceResult<()> {
    let detail = failure_detail(error);

    sqlx::query(
        "UPDATE media_assets \
            SET status = 'FAILED', error = ?, last_accessed_at = ?, expires_at = ? \
          WHERE id = ?",
    )
    .bind(detail)
    .bind(datetime_to_ms(now))
    .bind(datetime_to_ms(now + FAILED_TTL))
    .bind(asset_id.to_canonical())
    .execute(db.pool())
    .await
    .map_err(lc_db::DbError::from)?;

    Ok(())
}

/// Texte persisté dans `media_assets.error` : `"[CODE] message"` tronqué à 500
/// caractères (comme le legacy).
pub fn failure_detail(error: &MediaError) -> String {
    let raw = format!("[{}] {}", error.code().as_str(), error.message());
    if raw.chars().count() <= MAX_ERROR_CHARS {
        return raw;
    }

    raw.chars().take(MAX_ERROR_CHARS).collect()
}

/// Construit l'entrée de cache correspondant à un asset.
pub fn cache_entry(asset: &MediaAsset, pinned: bool, active: bool) -> CacheEntry {
    CacheEntry {
        key: asset.source_hash.clone(),
        size_bytes: u64::try_from(asset.size_bytes).unwrap_or(0),
        last_accessed_ms: datetime_to_ms(asset.last_accessed_at),
        created_at_ms: datetime_to_ms(asset.created_at),
        pinned,
        active,
        stale: false,
    }
}

/// Échéance d'épinglage d'un élément du meme board (100 ans).
pub fn pinned_expiry(now: DateTime<Utc>) -> DateTime<Utc> {
    now + ChronoDuration::days(PINNED_TTL_DAYS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;

    use chrono::{Duration, SubsecRound};
    use lc_db::DbConfig;
    use lc_domain::ids::MediaAssetId;
    use lc_media::download::DownloadResult;
    use lc_media::error::MediaErrorCode;
    use lc_media::resolve::ResolvedMedia;
    use lc_media::source::SourceKind;
    use lc_media::transcode::NormalizedMedia;

    use super::*;

    async fn database() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        (dir, db)
    }

    fn config() -> MediaConfig {
        MediaConfig::default()
    }

    /// Horodatage de test aligné sur la précision du schéma (millisecondes) :
    /// SQLite stocke les dates en millisecondes, comparer avec des nanosecondes
    /// fait échouer les assertions d'aller-retour.
    fn test_now() -> DateTime<Utc> {
        Utc::now().trunc_subsecs(3)
    }

    fn source(url: &str) -> MediaSource {
        MediaSource::from_raw(url).expect("source valide")
    }

    fn ingested(url: &str, file: PathBuf) -> IngestedMedia {
        let source = source(url);
        IngestedMedia {
            source: source.clone(),
            resolved: Some(ResolvedMedia {
                source_kind: SourceKind::DirectMedia,
                canonical_url: source.canonical_url.clone(),
                source_hash: source.source_hash,
                title: Some("titre".into()),
                author: None,
                duration_sec: Some(42.0),
                width: Some(1280),
                height: Some(720),
                is_vertical: false,
                kind: Some(MediaKind::Video),
                tweet_card: None,
                media_url: None,
                mime: Some("video/mp4".into()),
            }),
            download: DownloadResult {
                path: file.clone(),
                kind: Some(MediaKind::Video),
                mime: Some("video/mp4".into()),
                size_bytes: 1_234,
                source_url: url.to_owned(),
            },
            normalized: NormalizedMedia {
                kind: MediaKind::Video,
                mime: "video/mp4".into(),
                duration_sec: Some(42),
                width: Some(1280),
                height: Some(720),
                is_vertical: false,
                storage_path: file,
                size_bytes: 1_234,
            },
        }
    }

    #[tokio::test]
    async fn processing_then_ready_roundtrip() {
        let (_dir, db) = database().await;
        let source = source("https://example.com/video.mp4");
        let now = test_now();

        let processing = begin_processing(&db, &source, now, cache_ttl(&config()))
            .await
            .expect("ligne PROCESSING");
        assert_eq!(processing.status, MediaStatus::Processing);
        assert_eq!(processing.source_hash, source.source_hash);
        assert_eq!(processing.kind, MediaKind::Video);

        let file = PathBuf::from("/tmp/video.mp4");
        let ready = ReadyAsset::from_ingested(&ingested("https://example.com/video.mp4", file));
        let asset = mark_ready(&db, processing.id, &ready, now, cache_ttl(&config()))
            .await
            .expect("ligne READY");

        assert_eq!(asset.status, MediaStatus::Ready);
        assert_eq!(asset.kind, MediaKind::Video);
        assert_eq!(asset.mime, "video/mp4");
        assert_eq!(asset.duration_sec, Some(42));
        assert_eq!(asset.width, Some(1280));
        assert_eq!(asset.height, Some(720));
        assert_eq!(asset.size_bytes, 1_234);
        assert_eq!(asset.storage_path, "/tmp/video.mp4");
        assert!(asset.error.is_none());
        assert!(asset.is_ready());
    }

    #[tokio::test]
    async fn begin_processing_reuses_existing_row() {
        let (_dir, db) = database().await;
        let source = source("https://example.com/reuse.mp4");
        let now = test_now();

        let first = begin_processing(&db, &source, now, ChronoDuration::hours(1))
            .await
            .expect("première ligne");
        let second = begin_processing(&db, &source, now, ChronoDuration::hours(2))
            .await
            .expect("seconde ligne");

        assert_eq!(first.id, second.id);
        assert_eq!(second.status, MediaStatus::Processing);
        assert_eq!(second.expires_at, now + ChronoDuration::hours(2));
    }

    #[tokio::test]
    async fn failed_assets_carry_code_and_message() {
        let (_dir, db) = database().await;
        let source = source("https://example.com/broken.mp4");
        let now = test_now();
        let asset = begin_processing(&db, &source, now, ChronoDuration::hours(1))
            .await
            .expect("ligne PROCESSING");

        let error = MediaError::new(
            MediaErrorCode::DownloadFailed,
            "échec du téléchargement de la source",
        );
        mark_failed(&db, asset.id, &error, now)
            .await
            .expect("marquage FAILED");

        let stored = db
            .media_assets()
            .get(asset.id)
            .await
            .expect("lecture")
            .expect("asset présent");
        assert_eq!(stored.status, MediaStatus::Failed);
        assert_eq!(
            stored.error.as_deref(),
            Some("[DOWNLOAD_FAILED] échec du téléchargement de la source")
        );
        assert_eq!(stored.expires_at, now + FAILED_TTL);
    }

    #[tokio::test]
    async fn failure_detail_is_truncated_to_500_chars() {
        let long = "é".repeat(900);
        let error = MediaError::new(MediaErrorCode::InvalidMedia, long);
        let detail = failure_detail(&error);

        assert!(detail.starts_with("[INVALID_MEDIA] é"));
        assert_eq!(detail.chars().count(), MAX_ERROR_CHARS);
        assert!(!detail.contains('\u{0}'));
    }

    #[tokio::test]
    async fn find_reusable_requires_ready_file_and_freshness() {
        let (dir, db) = database().await;
        let now = test_now();
        let path = dir.path().join("cached.mp4");
        tokio::fs::write(&path, b"media")
            .await
            .expect("écriture fichier");
        let source = source("https://example.com/cached.mp4");

        let asset = begin_processing(&db, &source, now, ChronoDuration::hours(1))
            .await
            .expect("ligne");
        let ready = ReadyAsset {
            source_hash: source.source_hash.clone(),
            source_url: source.canonical_url.clone(),
            kind: MediaKind::Video,
            mime: "video/mp4".into(),
            duration_sec: Some(10),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: 5,
        };
        mark_ready(&db, asset.id, &ready, now, ChronoDuration::hours(1))
            .await
            .expect("READY");

        let reused = find_reusable(&db, &config(), &source.source_hash, now)
            .await
            .expect("recherche")
            .expect("asset réutilisable");
        assert_eq!(reused.id, asset.id);

        // Fichier absent : pas de réutilisation.
        tokio::fs::remove_file(&path).await.expect("suppression");
        assert!(find_reusable(&db, &config(), &source.source_hash, now)
            .await
            .expect("recherche")
            .is_none());

        // Fichier présent mais asset expiré : pas de réutilisation. La première
        // recherche a repoussé l'expiration à `now + cache_ttl`, on va au-delà.
        tokio::fs::write(&path, b"media").await.expect("écriture");
        let later = now + cache_ttl(&config()) + ChronoDuration::hours(1);
        assert!(find_reusable(&db, &config(), &source.source_hash, later)
            .await
            .expect("recherche")
            .is_none());
    }

    #[test]
    fn cache_entry_matches_asset_fields() {
        let now = test_now();
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "hash".into(),
            source_url: "https://example.com".into(),
            kind: MediaKind::Image,
            mime: "image/jpeg".into(),
            duration_sec: None,
            width: None,
            height: None,
            is_vertical: false,
            storage_path: String::new(),
            size_bytes: 512,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + Duration::hours(1),
            created_at: now - Duration::minutes(2),
        };

        let entry = cache_entry(&asset, true, false);
        assert_eq!(entry.key, "hash");
        assert_eq!(entry.size_bytes, 512);
        assert!(entry.pinned);
        assert!(!entry.active);
        assert!(!entry.is_evictable());
    }

    #[test]
    fn pinned_expiry_is_far_future() {
        let now = test_now();
        assert_eq!(
            pinned_expiry(now),
            now + ChronoDuration::days(PINNED_TTL_DAYS)
        );
    }
}
