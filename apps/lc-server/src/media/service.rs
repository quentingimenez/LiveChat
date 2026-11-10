//! Orchestrateur de l'ingestion média : pipeline `lc-media` → persistance DB.
//!
//! Le service reproduit `mediaIngestion.ts` : hit de cache par `source_hash`,
//! passage de l'asset en `PROCESSING`, exécution du pipeline (résolution,
//! téléchargement, probe, transcodage), vérification du budget du cache puis
//! persistance `READY`. Toute erreur du pipeline marque l'asset `FAILED` avec le
//! code legacy (`[CODE] message`) pour que l'overlay admin puisse l'afficher.
//!
//! Le service ne dépend pas du hub WebSocket : la lecture est déclenchée par un
//! hook injecté (voir [`crate::media::hook`], contrat Phase 4b).

use std::sync::{Arc, OnceLock};

use lc_db::Db;
use lc_domain::entities::MediaAsset;
use lc_domain::Clock;
use lc_media::config::MediaConfig;
use lc_media::error::{MediaError, MediaErrorCode};
use lc_media::ingest::MediaPipeline;
use lc_media::source::MediaSource;

use crate::app::AppState;
use crate::media::assets::{self, ReadyAsset};
use crate::media::errors::{MediaServiceError, MediaServiceResult};
use crate::media::quotas;
use crate::media::settings::media_config;

/// Pipeline partagé : un seul client HTTP et un seul sémaphore de concurrence
/// par processus. La configuration serveur est immuable après le démarrage.
static PIPELINE: OnceLock<Arc<MediaPipeline>> = OnceLock::new();

/// Service d'ingestion persistée.
pub struct MediaService {
    config: Arc<MediaConfig>,
    pipeline: Arc<MediaPipeline>,
    storage: lc_media::storage::MediaStorage,
    db: Db,
    clock: Arc<dyn Clock>,
}

impl MediaService {
    /// Construit un service avec un pipeline propre (tests, workers).
    pub fn new(config: MediaConfig, db: Db, clock: Arc<dyn Clock>) -> MediaServiceResult<Self> {
        let pipeline = Arc::new(MediaPipeline::new(config.clone())?);
        let storage = lc_media::storage::MediaStorage::new(&config.storage);
        Ok(Self {
            config: Arc::new(config),
            pipeline,
            storage,
            db,
            clock,
        })
    }

    /// Construit un service depuis l'état de l'application.
    ///
    /// `Unavailable` si aucune base n'est configurée : l'ingestion persistée
    /// nécessite SQLite.
    pub fn for_state(state: &AppState) -> MediaServiceResult<Self> {
        let db = state.db.clone().ok_or_else(|| {
            MediaServiceError::unavailable(
                "ingestion impossible : aucune base de données configurée (DATABASE_URL)",
            )
        })?;
        let config = media_config(&state.config);
        let pipeline = shared_pipeline(&config)?;
        let storage = lc_media::storage::MediaStorage::new(&config.storage);

        Ok(Self {
            config: Arc::new(config),
            pipeline,
            storage,
            db,
            clock: state.clock.clone(),
        })
    }

    /// Configuration média effective.
    pub fn config(&self) -> &MediaConfig {
        &self.config
    }

    /// Base de données utilisée pour la persistance.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Stockage content-addressed associé.
    pub fn storage(&self) -> &lc_media::storage::MediaStorage {
        &self.storage
    }

    /// Ingère une source et retourne l'asset `READY`.
    ///
    /// `media` est prioritaire sur `url` (sémantique legacy). `force_refresh`
    /// ignore le cache et ré-ingère la source. Un asset réutilisé est touché
    /// (expiration prolongée) mais n'est pas re-téléchargé.
    pub async fn ingest(
        &self,
        url: Option<&str>,
        media: Option<&str>,
        force_refresh: bool,
    ) -> MediaServiceResult<MediaAsset> {
        let raw = media
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| url.map(str::trim).filter(|value| !value.is_empty()))
            .ok_or_else(|| {
                MediaServiceError::Media(MediaError::new(
                    MediaErrorCode::UnsupportedSource,
                    "aucune URL ou média fourni à l'ingestion",
                ))
            })?;

        let source = MediaSource::from_raw(raw).ok_or_else(|| {
            MediaServiceError::Media(MediaError::new(
                MediaErrorCode::UnsupportedSource,
                format!("source média non supportée : {raw}"),
            ))
        })?;

        let now = self.clock.now();
        let ttl = assets::cache_ttl(&self.config);

        if !force_refresh {
            if let Some(cached) =
                assets::find_reusable(&self.db, &self.config, &source.source_hash, now).await?
            {
                tracing::debug!(
                    source_hash = %cached.source_hash,
                    "asset média réutilisé depuis le cache"
                );
                return Ok(cached);
            }
        }

        let asset = assets::begin_processing(&self.db, &source, now, ttl).await?;

        let ingested = match self.pipeline.ingest_url(raw).await {
            Ok(ingested) => ingested,
            Err(error) => {
                self.record_failure(asset.id, &error, now).await;
                return Err(MediaServiceError::Media(error));
            }
        };

        let ready = ReadyAsset::from_ingested(&ingested);

        // Vérifie le budget AVANT de persister le fichier normalisé en base,
        // comme le legacy (en cas d'échec, l'asset reste FAILED).
        if let Err(error) = quotas::enforce_cache_budget(
            &self.db,
            &self.storage,
            &self.config,
            ready.size_bytes_u64(),
            Some(ready.source_hash.as_str()),
        )
        .await
        {
            if let Some(media_error) = error.media_error() {
                self.record_failure(asset.id, media_error, now).await;
            }
            return Err(error);
        }

        assets::mark_ready(&self.db, asset.id, &ready, now, ttl).await
    }

    /// Marque l'asset en échec et journalise si même l'écriture échoue.
    async fn record_failure(
        &self,
        asset_id: lc_domain::ids::MediaAssetId,
        error: &MediaError,
        now: chrono::DateTime<chrono::Utc>,
    ) {
        if let Err(failure) = assets::mark_failed(&self.db, asset_id, error, now).await {
            tracing::warn!(
                asset_id = %asset_id.to_canonical(),
                error = %failure,
                "impossible de marquer l'asset média en échec"
            );
        }
    }
}

/// Retourne le pipeline partagé, en l'initialisant au premier usage.
fn shared_pipeline(config: &MediaConfig) -> MediaServiceResult<Arc<MediaPipeline>> {
    if let Some(pipeline) = PIPELINE.get() {
        return Ok(Arc::clone(pipeline));
    }

    let built = Arc::new(MediaPipeline::new(config.clone())?);
    // Si un autre thread a gagné la course, on garde son instance et on jette la
    // nôtre : les deux sont équivalentes (même configuration).
    if PIPELINE.set(Arc::clone(&built)).is_err() {
        tracing::debug!("pipeline média partagé déjà initialisé par un autre thread");
    }
    Ok(PIPELINE.get().map_or(built, Arc::clone))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use chrono::{Duration, Utc};
    use lc_db::DbConfig;
    use lc_domain::entities::MediaAsset;
    use lc_domain::enums::{MediaKind, MediaStatus};
    use lc_domain::ids::MediaAssetId;
    use lc_domain::SystemClock;
    use lc_media::error::MediaErrorCode;
    use lc_media::source::source_hash;

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        db: Db,
        config: MediaConfig,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let mut config = MediaConfig::default();
        config.storage.directory = dir.path().join("media");
        Fixture {
            _dir: dir,
            db: Db::open_in_memory(&DbConfig::default())
                .await
                .expect("base de test"),
            config,
        }
    }

    async fn insert_ready_asset(
        fixture: &Fixture,
        source_url: &str,
        hash: &str,
        storage_path: &std::path::Path,
    ) -> MediaAsset {
        let now = Utc::now();
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: hash.to_owned(),
            source_url: source_url.to_owned(),
            kind: MediaKind::Video,
            mime: "video/mp4".into(),
            duration_sec: Some(5),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: storage_path.to_string_lossy().into_owned(),
            size_bytes: 4,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + Duration::hours(1),
            created_at: now,
        };
        fixture
            .db
            .media_assets()
            .insert(&asset)
            .await
            .expect("asset inséré");
        asset
    }

    #[tokio::test]
    async fn cache_hit_does_not_call_the_pipeline() {
        let fixture = fixture().await;
        let service = MediaService::new(
            fixture.config.clone(),
            fixture.db.clone(),
            Arc::new(SystemClock),
        )
        .expect("service");

        let url = "https://example.com/video-cache";
        let path = fixture._dir.path().join("cached.mp4");
        tokio::fs::write(&path, b"data")
            .await
            .expect("fichier de cache");
        let asset = insert_ready_asset(&fixture, url, &source_hash(url), &path).await;

        let ingested = service
            .ingest(Some(url), None, false)
            .await
            .expect("cache hit");
        assert_eq!(ingested.id, asset.id);
        assert_eq!(ingested.status, MediaStatus::Ready);
    }

    #[tokio::test]
    async fn pipeline_failure_marks_the_asset_failed() {
        let fixture = fixture().await;
        let service = MediaService::new(
            fixture.config.clone(),
            fixture.db.clone(),
            Arc::new(SystemClock),
        )
        .expect("service");

        let raw = "file:/chemin/inexistant/lc-server-test.mp4";
        let result = service.ingest(None, Some(raw), true).await;
        assert!(result.is_err());

        let stored = fixture
            .db
            .media_assets()
            .get_by_source_hash(&source_hash(&lc_media::source::canonicalize_source_url(
                raw,
            )))
            .await
            .expect("lecture")
            .expect("asset créé");
        assert_eq!(stored.status, MediaStatus::Failed);
        let error = stored.error.expect("message d'erreur");
        assert!(error.starts_with('['), "erreur formatée legacy : {error}");
    }

    #[tokio::test]
    async fn missing_source_is_rejected() {
        let fixture = fixture().await;
        let service = MediaService::new(
            fixture.config.clone(),
            fixture.db.clone(),
            Arc::new(SystemClock),
        )
        .expect("service");

        let error = service
            .ingest(None, None, false)
            .await
            .expect_err("aucune source");
        let media = error.media_error().expect("erreur média");
        assert_eq!(media.code(), MediaErrorCode::UnsupportedSource);
    }
}
