//! Orchestration du pipeline d'ingestion média.
//!
//! Le pipeline enchaîne : résolution de la source, téléchargement borné,
//! normalisation (probe + transcode) et publication dans le stockage
//! content-addressed. La concurrence est limitée par un sémaphore
//! (`MEDIA_MAX_CONCURRENT_INGESTIONS`, défaut 2) comme l'exige l'audit S10.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

use crate::config::MediaConfig;
use crate::download::{self, DownloadResult};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::guard;
use crate::resolve::{self, ResolvedMedia};
use crate::source::{self, MediaSource, SourceKind};
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

/// Verrous single-flight par hash de source : deux ingestions concurrentes du
/// même média sont sérialisées pour éviter des écritures concurrentes dans le
/// stockage content-addressed (audit A4-007).
static SOURCE_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

/// Acquiert le verrou single-flight d'un `source_hash`.
///
/// Les purges de stockage doivent prendre ce même verrou avant de supprimer le
/// fichier d'un asset `READY` expiré : une ingestion concurrente du même média
/// pourrait sinon publier puis voir ses fichiers supprimés (AUD-V2-S-004).
pub async fn lock_source_hash(source_hash: &str) -> OwnedMutexGuard<()> {
    let registry = SOURCE_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let lock = {
        let mut locks = registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Purge les verrous qui ne sont plus référencés que par la table.
        locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        locks
            .entry(source_hash.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    lock.lock_owned().await
}

/// Pipeline d'ingestion réutilisable (limiteur + configuration).
#[derive(Clone, Debug)]
pub struct MediaPipeline {
    limiter: IngestLimiter,
    config: MediaConfig,
}

impl MediaPipeline {
    /// Construit le pipeline.
    pub fn new(config: MediaConfig) -> MediaResult<Self> {
        Ok(Self {
            limiter: IngestLimiter::from_config(&config),
            config,
        })
    }

    /// Configuration utilisée par le pipeline.
    pub fn config(&self) -> &MediaConfig {
        &self.config
    }

    /// Limiteur de concurrence.
    pub fn limiter(&self) -> &IngestLimiter {
        &self.limiter
    }

    /// Ingère une URL brute (résolution de source incluse).
    pub async fn ingest_url(&self, raw: &str) -> MediaResult<IngestedMedia> {
        let source = source::resolve_source(&self.config, raw)
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
        match source.source_kind {
            SourceKind::LocalFile | SourceKind::Gtts | SourceKind::Unknown => {
                return Err(MediaError::new(
                    MediaErrorCode::UnsupportedSource,
                    format!(
                        "source média refusée par la politique : {}",
                        source.canonical_url
                    ),
                ));
            }
            SourceKind::YouTube
            | SourceKind::TikTok
            | SourceKind::Twitter
            | SourceKind::DirectMedia => {
                guard::validate_target(&source.canonical_url).await?;
            }
        }
        let resolved = match resolve::resolve_media(&self.config, source).await {
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
        let _source_lock = lock_source_hash(&source.source_hash).await;
        let temp_dir = tempfile::TempDir::new().map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "création du dossier temporaire d'ingestion",
                &error,
            )
        })?;
        let download =
            download::download_source(source, resolved.as_ref(), &self.config, temp_dir.path())
                .await?;
        if download.size_bytes > self.config.download.max_size_bytes {
            return Err(MediaError::new(
                MediaErrorCode::FileTooLarge,
                "le média dépasse la taille maximale autorisée",
            ));
        }
        let storage = MediaStorage::new(&self.config.storage);
        // Fail-closed : une racine média non canonicalisable (absente, lien
        // cassé...) doit faire échouer l'ingestion avant toute écriture.
        let canonical_root = storage.ensure_canonical()?;
        let storage = MediaStorage::from_root(canonical_root);
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
        // Les variantes obsolètes ne sont supprimées qu'après publication
        // atomique du nouveau fichier : un échec de transcodage ne détruit
        // jamais le média précédent (audit A4-007).
        match storage.remove_other_variants(&source.source_hash, &normalized.storage_path) {
            Ok(removed) => {
                if removed > 0 {
                    tracing::debug!(
                        source_hash = %source.source_hash,
                        removed,
                        "anciennes variantes média supprimées"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(
                    source_hash = %source.source_hash,
                    "nettoyage des anciennes variantes impossible : {error}"
                );
            }
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
    async fn pipeline_exposes_config_and_limiter() {
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
    }
}
