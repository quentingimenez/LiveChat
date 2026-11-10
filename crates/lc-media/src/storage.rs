//! Stockage content-addressed des médias normalisés.
//!
//! La racine est `MEDIA_STORAGE_DIR` (défaut `./data/media`) ; chaque source est
//! matérialisée par une base `<racine>/<source_hash>` puis une variante
//! `<racine>/<source_hash>.<extension>`. Les fonctions d'ici ne touchent jamais
//! la base de données : elles gèrent uniquement les fichiers.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use lc_domain::MediaKind;

use crate::config::StorageConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::source::{extension_from_mime, mime_from_extension};

/// Extension de repli quand le type MIME est inconnu.
pub const FALLBACK_EXTENSION: &str = "bin";

/// Fichier présent dans le stockage média.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredVariant {
    /// Chemin absolu du fichier.
    pub path: PathBuf,
    /// Taille en octets.
    pub size_bytes: u64,
    /// Date de dernière modification, en millisecondes epoch UTC.
    pub modified_ms: i64,
}

/// Racine de stockage des médias normalisés.
#[derive(Clone, Debug)]
pub struct MediaStorage {
    root: PathBuf,
}

impl MediaStorage {
    /// Construit le stockage depuis la configuration.
    pub fn new(config: &StorageConfig) -> Self {
        Self {
            root: config.directory.clone(),
        }
    }

    /// Construit le stockage depuis une racine explicite.
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Racine du stockage.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Crée la racine si nécessaire.
    pub fn ensure(&self) -> MediaResult<()> {
        fs::create_dir_all(&self.root).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "création du stockage média",
                &error,
            )
        })
    }

    /// Crée la racine puis vérifie qu'elle se canonicalise.
    ///
    /// Confinement *fail-closed* (AUD-A2-003/A4-014) : une racine absente ou non
    /// canonicalisable fait échouer l'ingestion au lieu de laisser le pipeline
    /// écrire/relire depuis un chemin arbitraire.
    pub fn ensure_canonical(&self) -> MediaResult<PathBuf> {
        self.ensure()?;
        self.canonical_root()
    }

    /// Canonicalise la racine de stockage (échec explicite si impossible).
    pub fn canonical_root(&self) -> MediaResult<PathBuf> {
        fs::canonicalize(&self.root).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "racine média non canonicalisable (confinement du stockage)",
                &error,
            )
        })
    }

    /// Base de stockage (sans extension) d'une source.
    pub fn output_base_path(&self, source_hash: &str) -> PathBuf {
        self.root.join(sanitize_hash(source_hash))
    }

    /// Chemin complet d'une variante à partir de son type MIME.
    pub fn output_path(&self, source_hash: &str, mime: &str) -> PathBuf {
        let extension = extension_from_mime(mime).unwrap_or(FALLBACK_EXTENSION);
        self.variant_path(source_hash, extension)
    }

    /// Chemin complet d'une variante à partir de son extension.
    pub fn variant_path(&self, source_hash: &str, extension: &str) -> PathBuf {
        let normalized = extension
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        let extension = if normalized.is_empty() {
            FALLBACK_EXTENSION.to_string()
        } else {
            normalized
        };
        self.output_base_path(source_hash).with_extension(extension)
    }

    /// Variantes existantes d'une source (préfixe `<hash>.`).
    pub fn variant_paths(&self, source_hash: &str) -> MediaResult<Vec<PathBuf>> {
        let prefix = format!("{}.", sanitize_hash(source_hash));
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(MediaError::from_io(
                    MediaErrorCode::DownloadFailed,
                    "lecture du stockage média",
                    &error,
                ));
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                MediaError::from_io(
                    MediaErrorCode::DownloadFailed,
                    "lecture du stockage média",
                    &error,
                )
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with(&prefix) {
                paths.push(entry.path());
            }
        }
        paths.sort();
        Ok(paths)
    }

    /// Résout une référence de stockage sous la racine média.
    ///
    /// Les lignes importées du legacy peuvent porter un `storage_path` relatif
    /// (`./data/media/<hash>.mp4`) : il doit être interprété relativement à la
    /// racine du stockage, **jamais** au répertoire courant du processus
    /// (AUD-V2-S-002). Les variantes sont plates sous la racine : on ne garde
    /// donc que le nom de fichier.
    pub fn resolve_reference(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            return path.to_path_buf();
        }
        match path.file_name() {
            Some(name) if !name.is_empty() => self.root.join(name),
            _ => self.root.clone(),
        }
    }

    /// Résout et confine un chemin de stockage sans suivre les liens.
    ///
    /// Retourne le chemin canonique à supprimer, ou `None` (après un `warn`)
    /// si la cible est un lien symbolique, sort de la racine ou n'existe pas :
    /// une suppression hors confinement est toujours refusée (AUD-V2-S-001/002).
    fn confined_path(&self, path: &Path) -> MediaResult<Option<PathBuf>> {
        let resolved = self.resolve_reference(path);
        if resolved.as_os_str().is_empty() {
            return Ok(None);
        }
        match fs::symlink_metadata(&resolved) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    tracing::warn!(
                        path = %resolved.display(),
                        "suppression média refusée : lien symbolique"
                    );
                    return Ok(None);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(MediaError::from_io(
                    MediaErrorCode::DownloadFailed,
                    "inspection du fichier média",
                    &error,
                ));
            }
        }
        let canonical = fs::canonicalize(&resolved).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "canonicalisation du fichier média",
                &error,
            )
        })?;
        let root = fs::canonicalize(&self.root).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "canonicalisation de la racine média",
                &error,
            )
        })?;
        if !canonical.starts_with(&root) {
            tracing::warn!(
                path = %canonical.display(),
                root = %root.display(),
                "suppression média refusée : chemin hors de la racine de stockage"
            );
            return Ok(None);
        }
        Ok(Some(canonical))
    }

    /// Supprime toutes les variantes existantes d'une source (sous confinement).
    pub fn remove_variants(&self, source_hash: &str) -> MediaResult<usize> {
        let mut removed = 0;
        for path in self.variant_paths(source_hash)? {
            if self.remove_path(&path)? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Supprime les variantes d'une source autres que `keep` (sous confinement).
    ///
    /// Appelée **après** la publication atomique de la nouvelle variante : les
    /// anciennes extensions d'un même `source_hash` ne doivent jamais être
    /// effacées avant que le remplaçant soit sur disque (AUD-A4-007).
    pub fn remove_other_variants(&self, source_hash: &str, keep: &Path) -> MediaResult<usize> {
        let keep = std::fs::canonicalize(keep).unwrap_or_else(|_| keep.to_path_buf());
        let mut removed = 0;
        for path in self.variant_paths(source_hash)? {
            let candidate = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if candidate == keep {
                continue;
            }
            if self.remove_path(&path)? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Supprime un fichier s'il existe, **uniquement sous la racine média**.
    pub fn remove_path(&self, path: &Path) -> MediaResult<bool> {
        let Some(confined) = self.confined_path(path)? else {
            return Ok(false);
        };
        match fs::remove_file(&confined) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "suppression d'un fichier média",
                &error,
            )),
        }
    }

    /// Publie atomiquement `staged` sous `final_path` (rename même volume).
    ///
    /// Le fichier temporaire est supprimé en cas d'échec ; `final_path` n'est
    /// jamais tronqué ni partiellement écrit (AUD-A4-007). Le `fsync` est
    /// best-effort : les systèmes de fichiers Windows refusent l'ouverture d'un
    /// répertoire en tant que fichier, ce n'est donc pas une erreur.
    pub fn commit_file(staged: &Path, final_path: &Path) -> MediaResult<()> {
        if let Some(parent) = final_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                let _removed = fs::remove_file(staged);
                return Err(MediaError::from_io(
                    MediaErrorCode::DownloadFailed,
                    "création du stockage",
                    &error,
                ));
            }
        }
        if let Ok(file) = fs::File::open(staged) {
            let _synced = file.sync_all();
        }
        if let Err(error) = fs::rename(staged, final_path) {
            let _removed = fs::remove_file(staged);
            return Err(MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "publication atomique du fichier média",
                &error,
            ));
        }
        if let Some(parent) = final_path.parent() {
            if let Ok(directory) = fs::File::open(parent) {
                let _synced = directory.sync_all();
            }
        }
        Ok(())
    }

    /// Taille d'un fichier en octets.
    pub fn file_size(path: &Path) -> MediaResult<u64> {
        let metadata = fs::metadata(path).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::InvalidMedia,
                "taille du fichier média",
                &error,
            )
        })?;
        Ok(metadata.len())
    }

    /// Liste les variantes stockées avec leur taille et date de modification.
    pub fn list_variants(&self) -> MediaResult<Vec<StoredVariant>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(MediaError::from_io(
                    MediaErrorCode::DownloadFailed,
                    "lecture du stockage média",
                    &error,
                ));
            }
        };
        let mut variants = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                MediaError::from_io(
                    MediaErrorCode::DownloadFailed,
                    "lecture du stockage média",
                    &error,
                )
            })?;
            // Ne jamais suivre un lien symbolique : la purge ne doit ni lister
            // ni supprimer la cible d'un lien (AUD-V2-S-001).
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() || !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            variants.push(StoredVariant {
                path: path.clone(),
                size_bytes: metadata.len(),
                modified_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64)
                    .unwrap_or_else(now_ms),
            });
        }
        variants.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(variants)
    }

    /// Type MIME attendu pour le fichier d'une variante.
    pub fn mime_for_path(path: &Path) -> Option<&'static str> {
        let extension = path.extension()?.to_str()?;
        mime_from_extension(extension)
    }

    /// Type de média attendu pour une extension.
    pub fn kind_for_extension(extension: &str) -> Option<MediaKind> {
        mime_from_extension(extension).and_then(crate::source::kind_from_mime)
    }
}

/// Séquence des fichiers de transit (`<stem>.tmp-<seq>.<ext>`).
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Chemin de transit d'une variante finale, dans le même répertoire.
///
/// L'extension est conservée (ffmpeg choisit le conteneur d'après elle) et le
/// préfixe `.tmp-` permet au worker de purge (AUD-A4-008) de reconnaître les
/// fichiers de transit laissés par un arrêt brutal.
pub fn staged_path(final_path: &Path) -> PathBuf {
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let extension = final_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or(FALLBACK_EXTENSION);
    let stem = final_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("media");
    let name = format!("{stem}.tmp-{sequence:08x}.{extension}");
    match final_path.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

/// Indique si un nom de fichier correspond à un fichier de transit
/// (`<stem>.tmp-<seq:08x>.<ext>`) laissé par une ingestion interrompue.
pub fn is_staged_file_name(name: &str) -> bool {
    let Some(extension_index) = name.rfind('.') else {
        return false;
    };
    let stem = &name[..extension_index];
    let Some(marker_index) = stem.rfind(".tmp-") else {
        return false;
    };
    let sequence = &stem[marker_index + ".tmp-".len()..];
    sequence.len() == 8
        && sequence
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

/// Nettoie un hash pour en faire un nom de fichier sûr (aucun séparateur).
pub fn sanitize_hash(hash: &str) -> String {
    let mut sanitized = String::with_capacity(hash.len());
    for character in hash.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            sanitized.push(character.to_ascii_lowercase());
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn builds_content_addressed_paths() {
        let storage = MediaStorage::from_root("data/media");
        assert_eq!(
            storage.output_base_path("abc123"),
            PathBuf::from("data/media").join("abc123")
        );
        assert_eq!(
            storage.variant_path("abc123", ".MP4"),
            PathBuf::from("data/media").join("abc123.mp4")
        );
        assert_eq!(
            storage.output_path("abc123", "audio/mpeg"),
            PathBuf::from("data/media").join("abc123.mp3")
        );
        assert_eq!(
            storage.output_path("abc123", "application/octet-stream"),
            PathBuf::from("data/media").join("abc123.bin")
        );
    }

    #[test]
    fn sanitizes_hash_for_file_names() {
        assert_eq!(sanitize_hash("AbC-12_3"), "abc-12_3");
        assert_eq!(sanitize_hash("../evil"), "___evil");
        assert_eq!(sanitize_hash("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_hash(""), "unknown");
        assert_eq!(sanitize_hash("é"), "_");
    }

    #[test]
    fn ensure_and_variant_lifecycle() {
        let directory = tempfile::TempDir::new().unwrap();
        let storage = MediaStorage::from_root(directory.path().join("media"));
        storage.ensure().unwrap();
        assert!(storage.root().is_dir());
        let mp4 = storage.variant_path("hash1", "mp4");
        let mp3 = storage.variant_path("hash1", "mp3");
        fs::write(&mp4, b"video").unwrap();
        fs::write(&mp3, b"audio").unwrap();
        fs::write(storage.variant_path("hash2", "png"), b"image").unwrap();
        let variants = storage.variant_paths("hash1").unwrap();
        assert_eq!(variants.len(), 2);
        assert!(variants.contains(&mp4));
        assert_eq!(MediaStorage::file_size(&mp4).unwrap(), 5);
        assert_eq!(storage.remove_variants("hash1").unwrap(), 2);
        assert!(storage.variant_paths("hash1").unwrap().is_empty());
        assert_eq!(storage.remove_variants("inconnu").unwrap(), 0);
        assert!(storage.remove_path(&mp4).unwrap().eq(&false));
        assert!(!storage.remove_path(&mp3).unwrap());
        let listed = storage.list_variants().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].size_bytes, 5);
        assert!(listed[0].modified_ms > 0);
    }

    #[test]
    fn root_creation_and_empty_lists() {
        let directory = tempfile::TempDir::new().unwrap();
        let storage = MediaStorage::from_root(directory.path().join("absent"));
        assert!(storage.variant_paths("hash").unwrap().is_empty());
        assert!(storage.list_variants().unwrap().is_empty());
        storage.ensure().unwrap();
        assert!(storage.list_variants().unwrap().is_empty());
    }

    #[test]
    fn maps_extensions_to_mime_and_kind() {
        assert_eq!(
            MediaStorage::mime_for_path(Path::new("a/b.mp4")),
            Some("video/mp4")
        );
        assert_eq!(
            MediaStorage::kind_for_extension("mp3"),
            Some(MediaKind::Audio)
        );
        assert_eq!(MediaStorage::kind_for_extension("inconnu"), None);
        assert_eq!(
            MediaStorage::mime_for_path(Path::new("sans_extension")),
            None
        );
    }

    #[test]
    fn canonical_root_is_fail_closed() {
        let directory = tempfile::TempDir::new().unwrap();
        let absent = MediaStorage::from_root(directory.path().join("absent"));
        assert!(absent.canonical_root().is_err());
        let canonical = absent.ensure_canonical().expect("racine créée");
        assert!(canonical.is_absolute());
        assert_eq!(absent.canonical_root().unwrap(), canonical);
    }

    #[test]
    fn staged_path_preserves_extension_and_stays_sibling() {
        let final_path = Path::new("data/media/hash.mp4");
        let staged = staged_path(final_path);
        assert_eq!(staged.parent(), final_path.parent());
        assert_eq!(staged.extension().and_then(|ext| ext.to_str()), Some("mp4"));
        let name = staged.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("hash.tmp-"), "nom de transit : {name}");
        assert_ne!(staged_path(final_path), staged_path(final_path));
    }

    #[test]
    fn staged_file_names_are_recognized() {
        assert!(is_staged_file_name("hash.tmp-0000000a.mp4"));
        assert!(is_staged_file_name("7f8e9a.tmp-00000001.mp3"));
        assert!(!is_staged_file_name("hash.mp4"));
        assert!(!is_staged_file_name("hash.tmp-1.mp4"));
        assert!(!is_staged_file_name("hash.tmp-0000000a"));
        assert!(!is_staged_file_name("hash.tmp-zzzzzzzz.mp4"));
    }

    #[test]
    fn commit_file_publishes_atomically_and_removes_failed_staging() {
        let directory = tempfile::TempDir::new().unwrap();
        let staged = directory.path().join("hash.tmp-0000.mp4");
        let final_path = directory.path().join("hash.mp4");
        fs::write(&staged, b"contenu normalise").unwrap();

        MediaStorage::commit_file(&staged, &final_path).unwrap();
        assert!(!staged.exists());
        assert_eq!(fs::read(&final_path).unwrap(), b"contenu normalise");

        // Échec : destination dans un répertoire inexistant impossible à créer
        // (un fichier occupe le chemin parent) → le transit est nettoyé.
        let blocker = directory.path().join("blocker");
        fs::write(&blocker, b"fichier").unwrap();
        let staged = directory.path().join("autre.tmp-0001.mp4");
        fs::write(&staged, b"x").unwrap();
        let invalid_final = blocker.join("nested").join("hash.mp4");
        assert!(MediaStorage::commit_file(&staged, &invalid_final).is_err());
        assert!(!staged.exists());
    }

    #[test]
    fn remove_other_variants_keeps_the_published_file() {
        let directory = tempfile::TempDir::new().unwrap();
        let storage = MediaStorage::from_root(directory.path().join("media"));
        storage.ensure().unwrap();
        let keep = storage.variant_path("hash", "mp4");
        let stale = storage.variant_path("hash", "mp3");
        fs::write(&keep, b"video").unwrap();
        fs::write(&stale, b"audio").unwrap();

        assert_eq!(storage.remove_other_variants("hash", &keep).unwrap(), 1);
        assert!(keep.exists());
        assert!(!stale.exists());
    }

    #[cfg(windows)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[test]
    fn remove_path_refuses_paths_outside_storage_root() {
        let directory = tempfile::TempDir::new().unwrap();
        let storage = MediaStorage::from_root(directory.path().join("media"));
        storage.ensure().unwrap();
        let outside = directory.path().join("outside.mp4");
        fs::write(&outside, b"secret").unwrap();

        assert!(!storage.remove_path(&outside).unwrap());
        assert!(outside.exists());
    }

    #[test]
    fn remove_path_refuses_symbolic_links_and_keeps_the_target() {
        let directory = tempfile::TempDir::new().unwrap();
        let storage = MediaStorage::from_root(directory.path().join("media"));
        storage.ensure().unwrap();
        let target = directory.path().join("target.mp4");
        fs::write(&target, b"secret").unwrap();
        let link = storage.root().join("innocent.mp4");
        if create_symlink(&target, &link).is_err() {
            // Création de lien symbolique refusée (privilège Windows) : le
            // scénario n'est pas reproductible sur cette machine.
            return;
        }

        assert!(!storage.remove_path(&link).unwrap());
        assert!(link.exists());
        assert!(target.exists());
    }

    #[test]
    fn list_variants_ignores_symbolic_links() {
        let directory = tempfile::TempDir::new().unwrap();
        let storage = MediaStorage::from_root(directory.path().join("media"));
        storage.ensure().unwrap();
        let target = directory.path().join("target.mp4");
        fs::write(&target, b"secret").unwrap();
        let link = storage.root().join("innocent.mp4");
        if create_symlink(&target, &link).is_err() {
            return;
        }

        assert!(storage.list_variants().unwrap().is_empty());
    }

    #[test]
    fn relative_references_are_resolved_against_the_storage_root() {
        let directory = tempfile::TempDir::new().unwrap();
        let root = directory.path().join("media");
        let storage = MediaStorage::from_root(&root);

        assert_eq!(
            storage.resolve_reference(Path::new("./data/media/hash.mp4")),
            root.join("hash.mp4")
        );
        assert_eq!(
            storage.resolve_reference(Path::new("hash.mp4")),
            root.join("hash.mp4")
        );
        let absolute = std::env::temp_dir().join("hash.mp4");
        assert_eq!(storage.resolve_reference(&absolute), absolute);
    }
}
