//! Stockage content-addressed des médias normalisés.
//!
//! La racine est `MEDIA_STORAGE_DIR` (défaut `./data/media`) ; chaque source est
//! matérialisée par une base `<racine>/<source_hash>` puis une variante
//! `<racine>/<source_hash>.<extension>`. Les fonctions d'ici ne touchent jamais
//! la base de données : elles gèrent uniquement les fichiers.

use std::fs;
use std::path::{Path, PathBuf};
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

    /// Supprime toutes les variantes existantes d'une source.
    pub fn remove_variants(&self, source_hash: &str) -> MediaResult<usize> {
        let mut removed = 0;
        for path in self.variant_paths(source_hash)? {
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Supprime un fichier s'il existe.
    pub fn remove_path(&self, path: &Path) -> MediaResult<bool> {
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "suppression d'un fichier média",
                &error,
            )),
        }
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
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
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
}
