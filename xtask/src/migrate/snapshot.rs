//! Snapshot cohérent de la source legacy, produit avant toute lecture
//! (AUD-A5-002 / AUD-COR-16).
//!
//! La source n'est jamais ouverte en écriture : ses fichiers (`main`, `-wal`,
//! `-journal`) sont copiés dans un répertoire temporaire, puis un snapshot
//! autonome est produit par `VACUUM INTO` à partir de cette copie privée. Une
//! connexion `read_only` ne permet pas `VACUUM INTO` (SQLite répond « attempt to
//! write a readonly database ») : c'est la copie, jamais la source, qui est
//! ouverte en lecture-écriture.
//!
//! La signature de la source (SHA-256 et taille du fichier principal, du `-wal`
//! et du `-journal`) est relevée avant et après la copie. Toute écriture
//! concurrente pendant la copie annule la tentative et recommence (3 essais
//! maximum) ; la signature est de nouveau contrôlée après le `VACUUM INTO` et
//! avant chacune des lectures. Le snapshot final est vérifié par
//! `PRAGMA integrity_check` et son empreinte SHA-256 est enregistrée dans la
//! cible. Le `-shm` n'entre volontairement pas dans la signature : c'est un
//! index mémoire recréé à l'ouverture et il change même sans écriture de
//! données (faux positifs).
//!
//! Le snapshot est consommé exclusivement par [`super::legacy::Legacy`] en
//! lecture seule ; il est supprimé à la destruction de [`Snapshot`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};
use tempfile::TempDir;

use super::legacy;
use super::{append_suffix, Error};

/// Nombre de tentatives de copie avant d'abandonner si la source bouge.
pub const MAX_ATTEMPTS: u32 = 3;

/// Empreinte d'un fichier annexe (`-wal` ou `-journal`) quand il existe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSig {
    /// Taille du fichier en octets.
    pub bytes: u64,
    /// Empreinte SHA256 hexadécimale du fichier.
    pub sha256: String,
}

/// Signature stable de la source : fichier principal et annexes présentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSignature {
    /// Taille du fichier principal en octets.
    pub main_bytes: u64,
    /// SHA256 hexadécimal du fichier principal.
    pub main_sha256: String,
    /// Signature du `-wal` s'il existe.
    pub wal: Option<FileSig>,
    /// Signature d'un éventuel `-journal` (rollback journal chaud).
    pub journal: Option<FileSig>,
}

impl SourceSignature {
    /// Résumé lisible pour les messages d'erreur.
    pub fn summary(&self) -> String {
        let wal = match &self.wal {
            Some(sig) => format!("présent ({} octets, sha256 {})", sig.bytes, sig.sha256),
            None => "absent".to_string(),
        };
        let journal = match &self.journal {
            Some(sig) => format!("présent ({} octets, sha256 {})", sig.bytes, sig.sha256),
            None => "absent".to_string(),
        };
        format!(
            "main {} octets sha256 {} ; WAL {} ; journal {}",
            self.main_bytes, self.main_sha256, wal, journal
        )
    }
}

/// Métadonnées du snapshot, enregistrées dans `app_metadata` de la cible.
#[derive(Debug, Clone)]
pub struct SnapshotMeta {
    /// SHA256 du fichier snapshot autonome.
    pub sha256: String,
    /// Taille du fichier snapshot en octets.
    pub bytes: u64,
    /// Horodatage de création du snapshot (epoch ms UTC).
    pub created_at_ms: i64,
    /// Version de schéma legacy détectée (`_prisma_migrations`), informative.
    pub schema_version: String,
    /// Signature de la source stabilisée pendant la création du snapshot.
    pub source: SourceSignature,
    /// Numéro de la tentative ayant réussi (1 = pas de reprise).
    pub attempts: u32,
}

/// Snapshot temporaire prêt à être lu en lecture seule.
pub struct Snapshot {
    // Le répertoire temporaire porte le snapshot : le supprimer à la destruction.
    _dir: TempDir,
    path: PathBuf,
    meta: SnapshotMeta,
}

impl Snapshot {
    /// Chemin du fichier snapshot autonome.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Métadonnées à enregistrer dans la cible.
    pub fn meta(&self) -> &SnapshotMeta {
        &self.meta
    }

    /// Signature de la source qui a servi au snapshot.
    pub fn source_signature(&self) -> &SourceSignature {
        &self.meta.source
    }
}

/// Crée un snapshot cohérent de `source`.
///
/// La source est copiée, puis le snapshot est extrait par `VACUUM INTO` de la
/// copie privée. Si la source change pendant la copie (signature avant/après
/// différente) ou si la copie n'est pas intègre, la tentative est recommencée
/// sur une copie neuve ; après [`MAX_ATTEMPTS`] échecs, `SnapshotUnstable` est
/// renvoyé (aucun fichier de la migration n'a alors été écrit).
pub async fn create(source: &Path) -> Result<Snapshot, Error> {
    let mut details: Vec<String> = Vec::with_capacity(MAX_ATTEMPTS as usize);
    for attempt in 1..=MAX_ATTEMPTS {
        match attempt_snapshot(source, attempt).await? {
            Ok(snapshot) => return Ok(snapshot),
            Err(detail) => details.push(format!("tentative {attempt} : {detail}")),
        }
    }
    Err(Error::SnapshotUnstable {
        attempts: MAX_ATTEMPTS,
        detail: details.join(" ; "),
    })
}

/// Une tentative de snapshot : `Ok(Ok(_))` succès, `Ok(Err(_))` motif de reprise,
/// `Err(_)` erreur fatale (I/O, SQL, droits).
async fn attempt_snapshot(source: &Path, attempt: u32) -> Result<Result<Snapshot, String>, Error> {
    let dir = TempDir::new()?;
    let copy = dir.path().join("legacy-copy.sqlite");

    let before = capture_signature(source)?;
    copy_file(source, &copy, true)?;
    copy_file(
        &append_suffix(source, "-wal"),
        &append_suffix(&copy, "-wal"),
        false,
    )?;
    copy_file(
        &append_suffix(source, "-journal"),
        &append_suffix(&copy, "-journal"),
        false,
    )?;
    let after = capture_signature(source)?;
    if before != after {
        return Ok(Err(
            "la source a changé pendant la copie (serveur legacy encore actif ?)".to_string(),
        ));
    }

    let open = SqliteConnectOptions::new()
        .filename(&copy)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(10));
    let mut conn = SqliteConnection::connect_with(&open)
        .await
        .map_err(|source| Error::SnapshotOpen {
            path: copy.clone(),
            source,
        })?;

    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut conn)
        .await
        .map_err(Error::Sqlx)?;
    if integrity != "ok" {
        conn.close().await.map_err(Error::Sqlx)?;
        return Ok(Err(format!(
            "copie de la source incohérente (integrity_check = {integrity:?})"
        )));
    }

    let schema_version = read_legacy_schema_version(&mut conn)
        .await
        .unwrap_or_else(|_| "inconnue (lecture impossible)".to_string());

    let snapshot_path = dir.path().join("legacy-snapshot.sqlite");
    if snapshot_path.exists() {
        std::fs::remove_file(&snapshot_path)?;
    }
    let statement = format!("VACUUM INTO {}", sql_literal(&snapshot_path));
    sqlx::raw_sql(&statement)
        .execute(&mut conn)
        .await
        .map_err(|source| Error::SnapshotVacuum {
            path: snapshot_path.clone(),
            source,
        })?;
    conn.close().await.map_err(Error::Sqlx)?;

    // Libère la copie privée : seul le snapshot compact reste nécessaire.
    for suffix in ["", "-wal", "-journal"] {
        drop(std::fs::remove_file(append_suffix(&copy, suffix)));
    }

    let final_signature = capture_signature(source)?;
    if final_signature != after {
        return Ok(Err(
            "la source a changé pendant la production du snapshot".to_string()
        ));
    }

    verify_snapshot(&snapshot_path).await?;

    let bytes = std::fs::metadata(&snapshot_path)?.len();
    let sha256 = legacy::hash_file(&snapshot_path)?;
    let meta = SnapshotMeta {
        sha256,
        bytes,
        created_at_ms: Utc::now().timestamp_millis(),
        schema_version,
        source: final_signature,
        attempts: attempt,
    };
    Ok(Ok(Snapshot {
        _dir: dir,
        path: snapshot_path,
        meta,
    }))
}

/// Copie un fichier ; `required = false` ignore l'absence (annexes optionnelles).
fn copy_file(from: &Path, to: &Path, required: bool) -> Result<(), Error> {
    if !required && !from.is_file() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    Ok(())
}

/// Relève la signature de la source : fichier principal + `-wal` + `-journal`.
pub fn capture_signature(source: &Path) -> Result<SourceSignature, Error> {
    let metadata = std::fs::metadata(source)?;
    let journal = file_sig(&append_suffix(source, "-journal"))?;
    let wal = file_sig(&append_suffix(source, "-wal"))?;
    Ok(SourceSignature {
        main_bytes: metadata.len(),
        main_sha256: legacy::hash_file(source)?,
        wal,
        journal,
    })
}

fn file_sig(path: &Path) -> Result<Option<FileSig>, Error> {
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(FileSig {
        bytes: std::fs::metadata(path)?.len(),
        sha256: legacy::hash_file(path)?,
    }))
}

/// Ouvre le snapshot en lecture seule et exige `PRAGMA integrity_check = ok`.
async fn verify_snapshot(path: &Path) -> Result<(), Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(10));
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .map_err(|source| Error::SnapshotOpen {
            path: path.to_path_buf(),
            source,
        })?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&mut conn)
        .await
        .map_err(Error::Sqlx)?;
    conn.close().await.map_err(Error::Sqlx)?;
    if integrity != "ok" {
        return Err(Error::SnapshotIntegrity {
            path: path.to_path_buf(),
            detail: integrity,
        });
    }
    Ok(())
}

/// Lit la version de schéma legacy à partir de `_prisma_migrations`.
///
/// La version applicative legacy n'est pas stockée dans un `user_version` :
/// Prisma conserve l'historique de ses migrations. La valeur enregistrée ici
/// est informative (nombre de migrations appliquées + dernière) et n'entre pas
/// dans les contrôles bloquants.
async fn read_legacy_schema_version(conn: &mut SqliteConnection) -> Result<String, sqlx::Error> {
    let table: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_prisma_migrations'",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if table.is_none() {
        return Ok("inconnue (pas de table _prisma_migrations)".to_string());
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM \"_prisma_migrations\" \
         WHERE finished_at IS NOT NULL AND rolled_back_at IS NULL",
    )
    .fetch_one(&mut *conn)
    .await?;
    let last: Option<String> = sqlx::query_scalar(
        "SELECT migration_name FROM \"_prisma_migrations\" \
         WHERE finished_at IS NOT NULL AND rolled_back_at IS NULL \
         ORDER BY finished_at DESC, id DESC LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match last {
        Some(name) => format!("{count} migration(s) appliquée(s), dernière={name}"),
        None => format!("{count} migration(s) appliquée(s)"),
    })
}

/// Échappe un chemin pour l'insérer comme littéral SQL (`'` doublé).
fn sql_literal(path: &Path) -> String {
    let rendered = path.display().to_string().replace('\'', "''");
    format!("'{rendered}'")
}

/// Crochet de test exécuté juste après la création du snapshot, avant les
/// lectures : permet de simuler une source modifiée en cours de migration et
/// de vérifier que l'installation est refusée.
#[cfg(test)]
static AFTER_SNAPSHOT_HOOK: std::sync::Mutex<Option<fn(&Path)>> = std::sync::Mutex::new(None);

/// Enregistre (ou retire) le crochet de test.
#[cfg(test)]
pub(crate) fn set_after_snapshot_hook(hook: Option<fn(&Path)>) {
    if let Ok(mut slot) = AFTER_SNAPSHOT_HOOK.lock() {
        *slot = hook;
    }
}

/// Exécute le crochet de test s'il est enregistré.
#[cfg(test)]
pub(crate) fn run_after_snapshot_hook(source: &Path) {
    if let Ok(slot) = AFTER_SNAPSHOT_HOOK.lock() {
        if let Some(hook) = *slot {
            hook(source);
        }
    }
}
