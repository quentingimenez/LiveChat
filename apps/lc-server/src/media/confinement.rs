//! Confinement du répertoire de stockage média.
//!
//! L'audit (AUD-A2-003/A4-014) a montré qu'une racine média absente ou non
//! canonicalisable faisait retomber le handler de téléchargement dans un mode
//! *fail-open* : un chemin arbitraire stocké en base pouvait être servi.
//!
//! La garde ci-dessous canonicalise la racine **au démarrage** (échec explicite
//! si elle n'est pas exploitable) et fournit la racine canonique à réutiliser
//! pour les vérifications de confinement des requêtes.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _};

/// Crée puis canonicalise le répertoire de stockage média.
///
/// Le répertoire est créé s'il manque (comme le faisait le legacy au premier
/// téléchargement) ; en revanche un chemin qui n'est pas un répertoire ou une
/// canonicalisation impossible font **échouer le démarrage** : mieux vaut un
/// serveur qui refuse de démarrer qu'un serveur qui sert des fichiers hors
/// racine pendant que le confinement n'est pas vérifiable.
pub fn ensure_storage_root(directory: &Path) -> anyhow::Result<PathBuf> {
    if directory.as_os_str().is_empty() {
        bail!("MEDIA_STORAGE_DIR vide : répertoire de stockage média introuvable");
    }

    std::fs::create_dir_all(directory).with_context(|| {
        format!(
            "création du répertoire de stockage média impossible : {}",
            directory.display()
        )
    })?;

    let canonical = std::fs::canonicalize(directory).with_context(|| {
        format!(
            "canonicalisation de la racine média impossible : {}",
            directory.display()
        )
    })?;

    if !canonical.is_dir() {
        bail!(
            "la racine média n'est pas un répertoire : {}",
            canonical.display()
        );
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn creates_and_canonicalizes_missing_directory() {
        let parent = tempfile::tempdir().expect("dossier temporaire");
        let target = parent.path().join("media").join("nested");

        let canonical = ensure_storage_root(&target).expect("racine créée");

        assert!(canonical.is_absolute());
        assert_eq!(canonical, std::fs::canonicalize(&target).unwrap());
        // Idempotent.
        assert_eq!(ensure_storage_root(&target).unwrap(), canonical);
    }

    #[test]
    fn refuses_empty_or_file_paths() {
        assert!(ensure_storage_root(Path::new("")).is_err());

        let parent = tempfile::tempdir().expect("dossier temporaire");
        let file = parent.path().join("not-a-directory");
        std::fs::write(&file, b"contenu").expect("fichier");
        assert!(ensure_storage_root(&file).is_err());
    }
}
