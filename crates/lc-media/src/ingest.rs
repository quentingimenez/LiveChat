//! Orchestration du pipeline d'ingestion média.
//!
//! Le pipeline enchaîne : résolution de la source, téléchargement borné,
//! normalisation (probe + transcode) et publication dans le stockage
//! content-addressed. La concurrence est limitée par un sémaphore
//! (`MEDIA_MAX_CONCURRENT_INGESTIONS`, défaut 2) comme l'exige l'audit S10.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

use lc_domain::MediaKind;

use crate::config::MediaConfig;
use crate::download::{self, DownloadResult};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::guard;
use crate::resolve::{self, ResolvedMedia};
use crate::source::{self, kind_from_mime, mime_from_extension, MediaSource, SourceKind};
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
        let normalized = self.normalize_and_store(source, &download, false).await?;
        Ok(IngestedMedia {
            source: source.clone(),
            resolved,
            download,
            normalized,
        })
    }

    /// Ingests a local file already written to a temporary directory.
    ///
    /// The content SHA-256 is computed here;
    /// [`Self::ingest_local_file_with_hash`] lets a caller that already computed
    /// it while streaming avoid the extra read. The input file is never removed
    /// by the pipeline: cleanup remains the caller's responsibility.
    pub async fn ingest_local_file(
        &self,
        path: &Path,
        original_filename: &str,
        content_type_hint: Option<&str>,
    ) -> MediaResult<IngestedMedia> {
        let size_bytes = local_file_size(path).await?;
        if size_bytes > self.config.download.max_size_bytes {
            return Err(MediaError::new(
                MediaErrorCode::FileTooLarge,
                "le média dépasse la taille maximale autorisée",
            ));
        }
        let content_sha256 = hash_local_file(path).await?;
        self.ingest_local_file_with_hash(
            path,
            original_filename,
            content_type_hint,
            &content_sha256,
        )
        .await
    }

    /// Variant of [`Self::ingest_local_file`] with the SHA-256 digest already
    /// known (lowercase hex computed over the content).
    pub async fn ingest_local_file_with_hash(
        &self,
        path: &Path,
        original_filename: &str,
        content_type_hint: Option<&str>,
        content_sha256: &str,
    ) -> MediaResult<IngestedMedia> {
        let source = MediaSource::from_upload(original_filename, content_sha256);
        let _permit = self.limiter.acquire().await?;
        let _source_lock = lock_source_hash(&source.source_hash).await;

        let size_bytes = local_file_size(path).await?;
        if size_bytes > self.config.download.max_size_bytes {
            return Err(MediaError::new(
                MediaErrorCode::FileTooLarge,
                "le média dépasse la taille maximale autorisée",
            ));
        }
        if size_bytes == 0 {
            return Err(MediaError::new(
                MediaErrorCode::InvalidMedia,
                "le fichier téléversé est vide",
            ));
        }

        let header = read_local_header(path).await?;
        if local_file_is_svg(&header, original_filename, content_type_hint) {
            return Err(MediaError::new(
                MediaErrorCode::InvalidMedia,
                "image SVG non supportée : contenu actif, rasterisation requise",
            ));
        }
        let kind = download::sniff_kind(&header)
            .or_else(|| extension_kind(original_filename))
            .or_else(|| content_type_hint.and_then(kind_from_mime))
            .ok_or_else(|| {
                MediaError::new(
                    MediaErrorCode::UnsupportedSource,
                    "format de média non reconnu",
                )
            })?;
        let mime = local_content_type(&header, kind, original_filename, content_type_hint);
        let download = DownloadResult {
            path: path.to_path_buf(),
            kind: Some(kind),
            mime: Some(mime),
            size_bytes,
            source_url: source.canonical_url.clone(),
        };
        let normalized = self.normalize_and_store(&source, &download, true).await?;
        Ok(IngestedMedia {
            source,
            resolved: None,
            download,
            normalized,
        })
    }

    /// Normalizes and publishes an already available file (download or upload).
    ///
    /// Shared by `ingest_url` and `ingest_local_file`: size checks, canonical
    /// storage root, transcoding and stale-variant cleanup. Local uploads allow
    /// small raster images; remote sources keep the stricter guard. The input is
    /// never removed here.
    async fn normalize_and_store(
        &self,
        source: &MediaSource,
        download: &DownloadResult,
        local_upload: bool,
    ) -> MediaResult<NormalizedMedia> {
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
        let normalized = if local_upload {
            transcode::normalize_local_media(
                &download.path,
                download.mime.as_deref(),
                &output_base,
                &self.config,
            )
            .await?
        } else {
            transcode::normalize_downloaded_media(
                &download.path,
                download.mime.as_deref(),
                &output_base,
                &self.config,
            )
            .await?
        };
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
        Ok(normalized)
    }
}

/// Size of a local file (media error when unreadable).
async fn local_file_size(path: &Path) -> MediaResult<u64> {
    let metadata = tokio::fs::metadata(path).await.map_err(|error| {
        MediaError::from_io(
            MediaErrorCode::DownloadFailed,
            "inspection du fichier téléversé",
            &error,
        )
    })?;
    if !metadata.is_file() {
        return Err(MediaError::new(
            MediaErrorCode::InvalidMedia,
            "le chemin téléversé n'est pas un fichier",
        ));
    }
    Ok(metadata.len())
}

/// Streams a local file to compute its SHA-256 digest.
async fn hash_local_file(path: &Path) -> MediaResult<String> {
    let mut file = tokio::fs::File::open(path).await.map_err(|error| {
        MediaError::from_io(
            MediaErrorCode::DownloadFailed,
            "ouverture du fichier téléversé",
            &error,
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await.map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "lecture du fichier téléversé",
                &error,
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Reads the leading bytes used for type detection.
async fn read_local_header(path: &Path) -> MediaResult<Vec<u8>> {
    let mut file = tokio::fs::File::open(path).await.map_err(|error| {
        MediaError::from_io(
            MediaErrorCode::DownloadFailed,
            "ouverture du fichier téléversé",
            &error,
        )
    })?;
    let mut header = vec![0_u8; 32];
    let read = file.read(&mut header).await.map_err(|error| {
        MediaError::from_io(
            MediaErrorCode::DownloadFailed,
            "lecture de l'en-tête du fichier téléversé",
            &error,
        )
    })?;
    header.truncate(read);
    Ok(header)
}

/// Detects an SVG by signature, extension or MIME hint.
///
/// Recognized bytes win: a PNG named `.svg` remains a raster image.
fn local_file_is_svg(
    header: &[u8],
    original_filename: &str,
    content_type_hint: Option<&str>,
) -> bool {
    if download::sniff_kind(header).is_some() {
        return false;
    }
    let extension_is_svg =
        extension(original_filename).is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
    let hint_is_svg = content_type_hint.is_some_and(|hint| {
        hint.split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("image/svg+xml")
    });
    if extension_is_svg || hint_is_svg {
        return true;
    }
    let head = String::from_utf8_lossy(header)
        .trim_start()
        .to_ascii_lowercase();
    head.starts_with("<svg")
        || head.starts_with("<!doctype svg")
        || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// Reliable MIME type inferred from bytes, falling back to MIME hint then extension.
fn local_content_type(
    header: &[u8],
    kind: MediaKind,
    original_filename: &str,
    content_type_hint: Option<&str>,
) -> String {
    if let Some(mime) = sniff_image_mime(header) {
        return mime.to_string();
    }
    if let Some(hint) = content_type_hint.filter(|hint| kind_from_mime(hint) == Some(kind)) {
        return hint.to_string();
    }
    if let Some(mime) = extension(original_filename)
        .and_then(mime_from_extension)
        .filter(|mime| kind_from_mime(mime) == Some(kind))
    {
        return mime.to_string();
    }
    match kind {
        MediaKind::Image => "image/png",
        MediaKind::Audio => "audio/mpeg",
        MediaKind::Video => "video/mp4",
    }
    .to_string()
}

/// Exact MIME type of an image recognized by its magic bytes.
fn sniff_image_mime(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if header.starts_with(b"GIF8") {
        return Some("image/gif");
    }
    if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if header.starts_with(b"BM") {
        return Some("image/bmp");
    }
    None
}

/// Filename extension without the dot, when present.
fn extension(filename: &str) -> Option<&str> {
    filename
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
}

/// Media kind inferred from the original filename extension.
fn extension_kind(filename: &str) -> Option<MediaKind> {
    extension(filename)
        .and_then(mime_from_extension)
        .and_then(kind_from_mime)
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

    fn png_bytes(min_len: usize) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.resize(min_len.max(bytes.len()), 0);
        bytes
    }

    /// Valid 1×1 PNG (68 bytes), below the remote-source minimum size.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xB5,
        0x1C, 0x0C, 0x02, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x64,
        0xF8, 0x0F, 0x00, 0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xE3, 0x66, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[tokio::test]
    async fn local_file_accepts_a_valid_tiny_png() {
        let directory = tempfile::TempDir::new().unwrap();
        let mut config = MediaConfig::default();
        config.storage.directory = directory.path().join("media");
        config.tools.ffprobe = directory.path().join("missing-ffprobe");
        let pipeline = MediaPipeline::new(config).unwrap();
        let path = directory.path().join("upload-tiny.bin");
        tokio::fs::write(&path, TINY_PNG).await.unwrap();

        let ingested = pipeline
            .ingest_local_file(&path, "tiny.png", None)
            .await
            .expect("tiny PNG accepted for a local upload");
        assert_eq!(ingested.normalized.kind, MediaKind::Image);
        assert_eq!(ingested.normalized.mime, "image/png");
        assert_eq!(ingested.normalized.size_bytes, TINY_PNG.len() as u64);
        assert!(ingested.normalized.storage_path.exists());
    }

    #[tokio::test]
    async fn local_file_ingestion_sniffs_png_and_rejects_svg() {
        let directory = tempfile::TempDir::new().unwrap();
        let mut config = MediaConfig::default();
        config.storage.directory = directory.path().join("media");
        config.tools.ffprobe = directory.path().join("missing-ffprobe");
        config.tools.ffmpeg = directory.path().join("missing-ffmpeg");
        let pipeline = MediaPipeline::new(config).unwrap();

        let bytes = png_bytes(2048);
        let expected_hash = format!("{:x}", Sha256::digest(&bytes));
        let png_path = directory.path().join("upload.bin");
        tokio::fs::write(&png_path, &bytes).await.unwrap();

        let ingested = pipeline
            .ingest_local_file(&png_path, "photo.png", None)
            .await
            .expect("PNG local ingéré");
        assert_eq!(
            ingested.source.source_hash,
            format!("upload:sha256:{expected_hash}")
        );
        assert_eq!(ingested.source.canonical_url, "upload:photo.png");
        assert_eq!(ingested.source.source_kind, SourceKind::LocalFile);
        assert_eq!(ingested.normalized.kind, MediaKind::Image);
        assert_eq!(ingested.normalized.mime, "image/png");
        assert_eq!(ingested.normalized.size_bytes, 2048);
        assert!(ingested.normalized.storage_path.exists());

        let svg_path = directory.path().join("upload-svg.bin");
        tokio::fs::write(
            &svg_path,
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
        )
        .await
        .unwrap();
        let error = pipeline
            .ingest_local_file(&svg_path, "x.svg", Some("image/svg+xml"))
            .await
            .expect_err("SVG refusé");
        assert_eq!(error.code(), MediaErrorCode::InvalidMedia);
    }
}
