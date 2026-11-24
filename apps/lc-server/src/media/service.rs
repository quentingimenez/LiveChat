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

use std::path::Path;
use std::sync::{Arc, OnceLock};

use lc_db::Db;
use lc_domain::entities::MediaAsset;
use lc_domain::{Clock, MediaKind};
use lc_media::config::MediaConfig;
use lc_media::edit::EditRecipe;
use lc_media::error::{MediaError, MediaErrorCode};
use lc_media::ingest::MediaPipeline;
use lc_media::source::MediaSource;

use crate::app::AppState;
use crate::media::assets::{self, ReadyAsset};
use crate::media::errors::{MediaServiceError, MediaServiceResult};
use crate::media::quotas;
use crate::media::settings::media_config_for_state;
use crate::media::thumbnails::{spawn_cover_fetch, spawn_cover_refresh, ThumbnailContext};

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
    /// Hub told when a video gets its thumbnail (`None`: nobody to tell).
    hub: Option<Arc<dyn crate::http::admin::hub::AdminHub>>,
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
            hub: None,
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
        let config = media_config_for_state(state);
        let pipeline = shared_pipeline(&config)?;
        let storage = lc_media::storage::MediaStorage::new(&config.storage);

        Ok(Self {
            config: Arc::new(config),
            pipeline,
            storage,
            db,
            clock: state.clock.clone(),
            hub: Some(Arc::clone(&state.admin_hub)),
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
    /// (expiration prolongée) mais n'est pas re-téléchargé. Un lien court TikTok
    /// est résolu avant la lecture du cache : le lien et son URL canonique
    /// désignent le même asset (voir [`Self::resolve_ingest_source`]).
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

        let source = self.resolve_ingest_source(raw).await?;

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
                // A video cached before covers existed gets one now.
                spawn_cover_fetch(&self.thumbnail_context(), &cached, Some(source), None);
                return Ok(cached);
            }
        }

        let asset = assets::begin_processing(&self.db, &source, now, ttl).await?;

        let ingested = match self.pipeline.ingest_url(raw).await {
            Ok(ingested) => ingested,
            Err(error) => {
                self.record_failure(asset.id, &error, now).await;
                self.discard_partial_files(&source.source_hash);
                return Err(MediaServiceError::Media(error));
            }
        };

        let ready = ReadyAsset::from_ingested(&ingested);

        // Vérifie le budget AVANT de persister le fichier normalisé en base,
        // comme le legacy (en cas d'échec, l'asset reste FAILED). L'asset écarté
        // du budget est celui de la ligne écrite, pas l'empreinte résolue par le
        // pipeline : les deux peuvent différer (lien court non suivi).
        if let Err(error) = quotas::enforce_cache_budget(
            &self.db,
            &self.storage,
            &self.config,
            ready.size_bytes_u64(),
            Some(asset.source_hash.as_str()),
        )
        .await
        {
            if let Some(media_error) = error.media_error() {
                self.record_failure(asset.id, media_error, now).await;
            }
            self.discard_partial_files(&source.source_hash);
            return Err(error);
        }

        let ready_asset = assets::mark_ready(&self.db, asset.id, &ready, now, ttl).await?;
        self.attach_cover(&ready_asset, &ingested).await;
        // The cover never delays the media: the platform's thumbnail or a
        // frame of the video is made on its own task once the media is ready.
        // A cache bypass also replaces the picture, not only the media.
        if force_refresh {
            spawn_cover_refresh(
                &self.thumbnail_context(),
                &ready_asset,
                Some(ingested.source.clone()),
                ingested.resolved.clone(),
            );
        } else {
            spawn_cover_fetch(
                &self.thumbnail_context(),
                &ready_asset,
                Some(ingested.source.clone()),
                ingested.resolved.clone(),
            );
        }
        Ok(ready_asset)
    }

    /// Source écrite en base pour une URL brute d'ingestion.
    ///
    /// Un lien court TikTok est suivi avant toute écriture : la ligne créée
    /// doit porter l'empreinte de l'URL canonique que le pipeline résoudra.
    /// Sans cela, le lien court et son URL longue créaient deux assets pour le
    /// même média — puis `mark_ready` recopiait l'empreinte résolue sur la
    /// ligne du lien court, heurtait la contrainte `UNIQUE` et `/ingest`
    /// répondait 500. La résolution est best-effort : un lien non suivi garde
    /// son identité brute, que le pipeline résoudra de son côté.
    async fn resolve_ingest_source(&self, raw: &str) -> MediaServiceResult<MediaSource> {
        let fallback = MediaSource::from_raw(raw).ok_or_else(|| {
            MediaServiceError::Media(MediaError::new(
                MediaErrorCode::UnsupportedSource,
                format!("source média non supportée : {raw}"),
            ))
        })?;

        if !lc_media::source::is_tiktok_short_url(raw) {
            return Ok(fallback);
        }

        match lc_media::source::resolve_source(&self.config, raw).await {
            Ok(Some(resolved)) => Ok(resolved),
            // `resolve_short_url` rend l'URL d'origine quand elle n'a pas pu
            // être suivie : l'identité brute reste alors celle de la ligne.
            Ok(None) => Ok(fallback),
            Err(error) => {
                tracing::debug!(
                    source = %raw,
                    "lien court non résolu, identité brute conservée : {error}"
                );
                Ok(fallback)
            }
        }
    }

    /// What the background thumbnail tasks need.
    fn thumbnail_context(&self) -> ThumbnailContext {
        ThumbnailContext {
            config: Arc::clone(&self.config),
            storage: self.storage.clone(),
            db: self.db.clone(),
            clock: Arc::clone(&self.clock),
            hub: self.hub.clone(),
        }
    }

    /// Ingests a locally uploaded file and returns the `READY` asset.
    ///
    /// The cache/deduplication key is `upload:sha256:<hex>`, where `<hex>` is
    /// the content SHA-256 computed by the caller while streaming the body. A
    /// reusable `READY` asset for that key is returned without running the
    /// pipeline again. The input file remains owned by the caller.
    pub async fn ingest_file(
        &self,
        path: &Path,
        original_filename: &str,
        content_type_hint: Option<&str>,
        content_sha256: &str,
    ) -> MediaServiceResult<MediaAsset> {
        self.ingest_file_with_cache(
            path,
            original_filename,
            content_type_hint,
            content_sha256,
            false,
        )
        .await
    }

    /// Like [`Self::ingest_file`], but `force_refresh` re-runs the pipeline
    /// instead of reusing the cached asset with the same content hash.
    pub async fn ingest_file_with_cache(
        &self,
        path: &Path,
        original_filename: &str,
        content_type_hint: Option<&str>,
        content_sha256: &str,
        force_refresh: bool,
    ) -> MediaServiceResult<MediaAsset> {
        self.ingest_local(
            path,
            original_filename,
            content_type_hint,
            content_sha256,
            false,
            force_refresh,
        )
        .await
    }

    /// Returns a reusable asset for an upload content hash without touching a
    /// file. Used when the client proved it already owns the bytes.
    pub async fn cached_upload(&self, source_hash: &str) -> MediaServiceResult<Option<MediaAsset>> {
        let now = self.clock.now();
        assets::find_reusable(&self.db, &self.config, source_hash, now).await
    }

    /// Applies an edit recipe to a stored media file and ingests the result
    /// as a new asset (key `upload:sha256:<hex>` of the edited file, source
    /// `upload:<filename_stem>.<ext>`).
    ///
    /// The edited file lives in a temporary directory removed when this
    /// function returns. It is ingested without loudness normalization: the
    /// source already went through it.
    pub async fn ingest_edit(
        &self,
        source_path: &Path,
        kind: MediaKind,
        recipe: &EditRecipe,
        filename_stem: &str,
    ) -> MediaServiceResult<MediaAsset> {
        let rendered = self
            .pipeline
            .render_edit(source_path, kind, recipe)
            .await
            .map_err(MediaServiceError::Media)?;
        let filename = format!("{filename_stem}.{}", rendered.plan.output_extension());
        self.ingest_local(
            &rendered.path,
            &filename,
            Some(rendered.plan.output_mime()),
            &rendered.content_sha256,
            true,
            false,
        )
        .await
    }

    /// Shared body of [`Self::ingest_file`] and [`Self::ingest_edit`].
    async fn ingest_local(
        &self,
        path: &Path,
        original_filename: &str,
        content_type_hint: Option<&str>,
        content_sha256: &str,
        edited: bool,
        force_refresh: bool,
    ) -> MediaServiceResult<MediaAsset> {
        let source = MediaSource::from_upload(original_filename, content_sha256);
        let now = self.clock.now();
        let ttl = assets::cache_ttl(&self.config);

        if !force_refresh {
            if let Some(cached) =
                assets::find_reusable(&self.db, &self.config, &source.source_hash, now).await?
            {
                tracing::debug!(
                    source_hash = %cached.source_hash,
                    "asset média téléversé réutilisé depuis le cache"
                );
                // An upload cached before covers existed gets one now: there is
                // no platform to ask, so the cover comes from the video itself.
                spawn_cover_fetch(&self.thumbnail_context(), &cached, None, None);
                return Ok(cached);
            }
        }

        let asset = assets::begin_processing(&self.db, &source, now, ttl).await?;

        let pipeline_result = if edited {
            self.pipeline
                .ingest_edited_file_with_hash(
                    path,
                    original_filename,
                    content_type_hint,
                    content_sha256,
                )
                .await
        } else {
            self.pipeline
                .ingest_local_file_with_hash(
                    path,
                    original_filename,
                    content_type_hint,
                    content_sha256,
                )
                .await
        };
        let ingested = match pipeline_result {
            Ok(ingested) => ingested,
            Err(error) => {
                self.record_failure(asset.id, &error, now).await;
                self.discard_partial_files(&source.source_hash);
                return Err(MediaServiceError::Media(error));
            }
        };

        let ready = ReadyAsset::from_ingested(&ingested);
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
            self.discard_partial_files(&source.source_hash);
            return Err(error);
        }

        let ready_asset = assets::mark_ready(&self.db, asset.id, &ready, now, ttl).await?;
        self.attach_cover(&ready_asset, &ingested).await;
        // A video upload gets a frame of itself as its card picture: its first
        // frame is often black.
        spawn_cover_fetch(&self.thumbnail_context(), &ready_asset, None, None);
        Ok(ready_asset)
    }

    /// Records the embedded cover of an audio file. Best effort: without it
    /// the audio simply has no picture.
    async fn attach_cover(&self, audio: &MediaAsset, ingested: &lc_media::ingest::IngestedMedia) {
        let Some(cover) = ingested.normalized.cover.as_ref() else {
            return;
        };
        if let Err(error) = assets::attach_cover(&self.db, audio, cover, self.clock.now()).await {
            tracing::warn!(
                asset_id = %audio.id.to_canonical(),
                error = %error,
                "pochette de l'audio non enregistrée"
            );
        }
    }

    /// Marque l'asset en échec et journalise si même l'écriture échoue.
    async fn record_failure(
        &self,
        asset_id: lc_domain::ids::MediaAssetId,
        error: &MediaError,
        now: time::UtcDateTime,
    ) {
        if let Err(failure) = assets::mark_failed(&self.db, asset_id, error, now).await {
            tracing::warn!(
                asset_id = %asset_id.to_canonical(),
                error = %failure,
                "impossible de marquer l'asset média en échec"
            );
        }
    }

    /// Supprime les fichiers de stockage d'une source dont l'ingestion a échoué.
    ///
    /// AUD-A2-007 : avec `forceRefresh`, les variantes précédentes sont
    /// remplacées ; en cas d'échec du pipeline, le fichier partiel (ou l'ancien
    /// fichier devenu orphelin) ne doit pas survivre à une ligne `FAILED`, sinon
    /// le disque se remplit sans référence en base. Best-effort : un échec de
    /// nettoyage est journalisé mais ne masque jamais l'erreur d'ingestion.
    fn discard_partial_files(&self, source_hash: &str) {
        match self.storage.remove_variants(source_hash) {
            Ok(0) => {}
            Ok(removed) => {
                tracing::debug!(
                    source_hash,
                    removed,
                    "variantes de la source en échec supprimées du stockage"
                );
            }
            Err(error) => {
                tracing::warn!(
                    source_hash,
                    error = %error,
                    "nettoyage des variantes de la source en échec impossible"
                );
            }
        }
    }
}

/// Retourne le pipeline partagé par processus pour un état d'application.
///
/// Utilisé par les routes qui n'instancient pas de [`MediaService`] complet
/// (board média) : sans cela, chaque requête construisait un nouveau client HTTP
/// et un nouveau sémaphore, contournant les budgets de cache et le compteur de
/// concurrence globaux (AUD-A2-006/A4-007).
pub fn shared_pipeline_for_state(state: &AppState) -> MediaServiceResult<Arc<MediaPipeline>> {
    shared_pipeline(&media_config_for_state(state))
}

/// Retourne le pipeline partagé, en l'initialisant au premier usage.
fn shared_pipeline(config: &MediaConfig) -> MediaServiceResult<Arc<MediaPipeline>> {
    if let Some(pipeline) = PIPELINE.get() {
        if pipeline.config() == config {
            return Ok(Arc::clone(pipeline));
        }
        // Different configuration (notably parallel tests with distinct storage
        // roots): never reuse a pipeline whose frozen configuration would write
        // to the wrong directory.
        tracing::debug!("configuration média différente du pipeline partagé : pipeline dédié");
        return Ok(Arc::new(MediaPipeline::new(config.clone())?));
    }

    let built = Arc::new(MediaPipeline::new(config.clone())?);
    // Si un autre thread a gagné la course, on garde son instance et on jette la
    // nôtre : les deux sont équivalentes (même configuration).
    if PIPELINE.set(Arc::clone(&built)).is_err() {
        tracing::debug!("pipeline média partagé déjà initialisé par un autre thread");
    }
    match PIPELINE.get() {
        Some(shared) if shared.config() == config => Ok(Arc::clone(shared)),
        _ => Ok(built),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use lc_db::DbConfig;
    use lc_domain::entities::MediaAsset;
    use lc_domain::enums::{MediaKind, MediaStatus};
    use lc_domain::ids::MediaAssetId;
    use lc_domain::SystemClock;
    use lc_media::error::MediaErrorCode;
    use lc_media::source::source_hash;
    use time::{Duration, UtcDateTime};

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
        let now = UtcDateTime::now();
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: hash.to_owned(),
            source_url: source_url.to_owned(),
            source_title: None,
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
    async fn force_refresh_bypasses_the_upload_cache() {
        let fixture = fixture().await;
        let service = MediaService::new(
            fixture.config.clone(),
            fixture.db.clone(),
            Arc::new(SystemClock),
        )
        .expect("service");

        let digest = "a".repeat(64);
        let cache_key = format!("upload:sha256:{digest}");
        let cached_path = fixture._dir.path().join("cached.bin");
        tokio::fs::write(&cached_path, b"cached")
            .await
            .expect("fichier en cache");
        let asset =
            insert_ready_asset(&fixture, "upload:cached.bin", &cache_key, &cached_path).await;

        let missing = fixture._dir.path().join("missing.bin");
        let reused = service
            .ingest_file_with_cache(&missing, "missing.bin", Some("video/mp4"), &digest, false)
            .await
            .expect("cache hit");
        assert_eq!(reused.id, asset.id);

        let error = service
            .ingest_file_with_cache(&missing, "missing.bin", Some("video/mp4"), &digest, true)
            .await
            .expect_err("le contournement aurait dû relancer le pipeline");
        assert!(
            error.media_error().is_some(),
            "erreur média attendue : {error:?}"
        );
        let stored = fixture
            .db
            .media_assets()
            .get_by_source_hash(&cache_key)
            .await
            .expect("lecture")
            .expect("asset existant");
        assert_eq!(stored.id, asset.id);
        assert_eq!(stored.status, MediaStatus::Failed);
    }

    /// Hors lien court, l'identité écrite est celle de `MediaSource` : la
    /// résolution n'est tentée que pour `vm.tiktok.com` et consorts. Sinon une
    /// source locale (`file:`, `gtts:`) ou une URL canonique passerait par la
    /// garde d'ingestion, qui les refuse.
    #[tokio::test]
    async fn resolve_ingest_source_only_follows_short_links() {
        let fixture = fixture().await;
        let service = MediaService::new(
            fixture.config.clone(),
            fixture.db.clone(),
            Arc::new(SystemClock),
        )
        .expect("service");

        for raw in [
            "https://www.tiktok.com/@alexandro.dance/video/7706300590749535495?_r=1&_t=ZG-9Beyix5zRNQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42",
            "file:/chemin/video.mp4",
        ] {
            let source = service
                .resolve_ingest_source(raw)
                .await
                .expect("source d'ingestion");
            assert_eq!(
                source,
                MediaSource::from_raw(raw).expect("source brute"),
                "identité brute conservée pour {raw}"
            );
        }
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
    async fn force_refresh_failure_removes_partial_files() {
        // AUD-A2-007 / COR-15-5 : un rafraîchissement forcé qui échoue ne doit
        // pas laisser de variantes orphelines sur disque.
        let fixture = fixture().await;
        let service = MediaService::new(
            fixture.config.clone(),
            fixture.db.clone(),
            Arc::new(SystemClock),
        )
        .expect("service");

        let raw = "file:/chemin/inexistant/lc-server-test-cleanup.mp4";
        let source = lc_media::source::MediaSource::from_raw(raw).expect("source");
        let variant = fixture
            .config
            .storage
            .directory
            .join(format!("{}.mp4", source.source_hash));
        tokio::fs::create_dir_all(&fixture.config.storage.directory)
            .await
            .expect("répertoire");
        tokio::fs::write(&variant, b"partiel")
            .await
            .expect("fichier partiel");

        let result = service.ingest(Some(raw), None, true).await;
        assert!(result.is_err());
        assert!(
            !tokio::fs::try_exists(&variant).await.expect("stat"),
            "la variante partielle aurait dû être supprimée"
        );
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
