//! Orchestration du pipeline d'ingestion média.
//!
//! Le pipeline enchaîne : résolution de la source, téléchargement borné,
//! normalisation (probe + transcode) et publication dans le stockage
//! content-addressed. La concurrence est limitée par un sémaphore
//! (`MEDIA_MAX_CONCURRENT_INGESTIONS`, défaut 2) comme l'exige l'audit S10.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::config::MediaConfig;
use crate::download::{self, DownloadResult};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::http;
use crate::resolve::{self, ResolvedMedia};
use crate::source::{self, MediaSource};
use crate::storage::MediaStorage;
use crate::transcode::{self, NormalizedMedia};

/// Résultat complet d'une ingestion réussie.
#[derive(Clone, Debug)]
pub struct IngestedMedia {
    /// Source normalisée (URL canonique + hash).
    pub source: MediaSource,
    /// Métadonnées de résolution, si la source a pu être résolue.
    pub resolved: Option<ResolvedMedia>,
    /// Fichier effectivement téléchargé (dans un répertoire temporaire).
    pub download: DownloadResult,
    /// Média normalisé persisté dans le stockage média.
    pub normalized: NormalizedMedia,
}

/// Limiteur de concurrence du pipeline d'ingestion.
#[derive(Clone, Debug)]
pub struct IngestLimiter {
    semaphore: Arc<Semaphore>,
}

impl IngestLimiter {
    /// Crée un limiteur autorisant `max_concurrent` ingestions simultanées.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }

    /// Construit le limiteur depuis la configuration média.
    pub fn from_config(config: &MediaConfig) -> Self {
        Self::new(config.download.max_concurrent_ingestions)
    }

    /// Nombre de permits immédiatement disponibles.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Acquiert un permit (attend si la limite est atteinte).
    pub async fn acquire(&self) -> MediaResult<OwnedSemaphorePermit> {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| {
                MediaError::new(
                    MediaErrorCode::DownloadFailed,
                    format!("limiteur d'ingestion indisponible : {error}"),
                )
            })
    }
}

/// Pipeline d'ingestion réutilisable (client HTTP + limiteur + configuration).
#[derive(Clone, Debug)]
pub struct MediaPipeline {
    client: reqwest::Client,
    limiter: IngestLimiter,
    config: MediaConfig,
}

impl MediaPipeline {
    /// Construit le pipeline (client HTTP reqwest configuré).
    pub fn new(config: MediaConfig) -> MediaResult<Self> {
        let client = http::build_client(&config.download)?;
        Ok(Self {
            client,
            limiter: IngestLimiter::from_config(&config),
            config,
        })
    }

    /// Construit le pipeline à partir d'éléments déjà initialisés (tests, serveur).
    pub fn from_parts(config: MediaConfig, client: reqwest::Client) -> Self {
        Self {
            limiter: IngestLimiter::from_config(&config),
            client,
            config,
        }
    }

    /// Configuration utilisée par le pipeline.
    pub fn config(&self) -> &MediaConfig {
        &self.config
    }

    /// Client HTTP partagé.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Limiteur de concurrence.
    pub fn limiter(&self) -> &IngestLimiter {
        &self.limiter
    }

    /// Ingère une URL brute (résolution de source incluse).
    pub async fn ingest_url(&self, raw: &str) -> MediaResult<IngestedMedia> {
        let source = source::resolve_source(&self.client, raw, &self.config)
            .await?
            .ok_or_else(|| {
                MediaError::new(
                    MediaErrorCode::UnsupportedSource,
                    "aucune source média fournie",
                )
            })?;
        self.ingest_source(&source).await
    }

    /// Ingère une source déjà normalisée.
    pub async fn ingest_source(&self, source: &MediaSource) -> MediaResult<IngestedMedia> {
        let resolved = match resolve::resolve_media(&self.client, source, &self.config).await {
            Ok(resolved) => Some(resolved),
            Err(error) => {
                tracing::debug!(
                    source = %source.canonical_url,
                    "résolution de source ignorée : {error}"
                );
                None
            }
        };
        self.ingest_resolved(source, resolved).await
    }

    /// Ingère une source avec des métadonnées de résolution déjà connues.
    pub async fn ingest_resolved(
        &self,
        source: &MediaSource,
        resolved: Option<ResolvedMedia>,
    ) -> MediaResult<IngestedMedia> {
        let _permit = self.limiter.acquire().await?;
        let temp_dir = tempfile::TempDir::new().map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "création du dossier temporaire d'ingestion",
                &error,
            )
        })?;
        let download = download::download_source(
            &self.client,
            source,
            resolved.as_ref(),
            &self.config,
            temp_dir.path(),
        )
        .await?;
        if download.size_bytes > self.config.download.max_size_bytes {
            return Err(MediaError::new(
                MediaErrorCode::FileTooLarge,
                "le média dépasse la taille maximale autorisée",
            ));
        }
        let storage = MediaStorage::new(&self.config.storage);
        storage.ensure()?;
        let _removed = storage.remove_variants(&source.source_hash);
        let output_base = storage.output_base_path(&source.source_hash);
        let normalized = transcode::normalize_downloaded_media(
            &download.path,
            download.mime.as_deref(),
            &output_base,
            &self.config,
        )
        .await?;
        if normalized.size_bytes > self.config.download.max_size_bytes {
            let _removed = storage.remove_path(&normalized.storage_path);
            return Err(MediaError::new(
                MediaErrorCode::FileTooLarge,
                "le média normalisé dépasse la taille maximale autorisée",
            ));
        }
        Ok(IngestedMedia {
            source: source.clone(),
            resolved,
            download,
            normalized,
        })
    }
}

/// Ingère une URL avec un pipeline jetable (pratique pour les tests et scripts).
pub async fn ingest_url(raw: &str, config: &MediaConfig) -> MediaResult<IngestedMedia> {
    let pipeline = MediaPipeline::new(config.clone())?;
    pipeline.ingest_url(raw).await
}

/// Chemin persisté d'un média ingéré.
pub fn stored_path(media: &IngestedMedia) -> PathBuf {
    media.normalized.storage_path.clone()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn limiter_bounds_concurrency() {
        let limiter = IngestLimiter::new(2);
        assert_eq!(limiter.available_permits(), 2);
        let config = MediaConfig::default();
        assert_eq!(
            IngestLimiter::from_config(&config).available_permits(),
            config.download.max_concurrent_ingestions
        );
        assert_eq!(IngestLimiter::new(0).available_permits(), 1);
    }

    #[tokio::test]
    async fn limiter_grants_and_releases_permits() {
        let limiter = IngestLimiter::new(1);
        let permit = limiter.acquire().await.unwrap();
        assert_eq!(limiter.available_permits(), 0);
        drop(permit);
        assert_eq!(limiter.available_permits(), 1);
    }

    #[tokio::test]
    async fn pipeline_exposes_config_and_client() {
        let config = MediaConfig::default();
        let pipeline = MediaPipeline::new(config.clone()).unwrap();
        assert_eq!(
            pipeline.config().storage.directory,
            config.storage.directory
        );
        assert_eq!(
            pipeline.limiter().available_permits(),
            config.download.max_concurrent_ingestions
        );
        let _ = pipeline.client();
    }
}
