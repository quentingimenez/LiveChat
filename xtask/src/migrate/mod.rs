//! Sous-commande `migrate-legacy` : migration one-shot de la base SQLite du
//! serveur legacy (Prisma 5.22) vers le schéma 3.0 de `docs/rust/schema-v3.sql`.
//!
//! Déroulé : validation des chemins, snapshot cohérent de la source (copie +
//! `VACUUM INTO`, signature et intégrité vérifiées), lecte strictement en
//! lecture seule du snapshot (intégrité, 7 tables, normalisation des dates),
//! écriture dans `<to>.tmp`, vérification complète du temporaire, contrôle que
//! la source n'a pas bougé depuis le snapshot, puis renommage atomique en
//! `<to>`. Un échec à n'importe quelle étape laisse la source intacte et
//! supprime le temporaire : aucun fichier final n'est créé.
//!
//! Récupération : si un crash a laissé une sauvegarde `<to>.old` sans `<to>`
//! (fenêtre entre les deux renommages de l'installation), elle est restaurée en
//! `<to>` au démarrage du migrateur ; sans `--force`, l'outil s'arrête ensuite
//! avec un message explicite pour que l'opérateur relance la migration.
//!
//! La base legacy n'est **jamais ouverte** : elle est copiée avec ses annexes
//! `-wal`/`-journal` dans un répertoire temporaire, la copie est contrôlée par
//! `PRAGMA integrity_check`, puis un snapshot autonome en est extrait par
//! `VACUUM INTO` ([`snapshot`]). La cible n'est écrite qu'après lecture complète
//! en mémoire : la migration est rejouable depuis la source d'origine.

mod dates;
mod ledger;
mod legacy;
mod read;
mod schema;
mod snapshot;
mod tables;
mod target;
#[cfg(test)]
mod tests;
mod verify;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};
use thiserror::Error;

use legacy::{Legacy, SourceMeta};
use read::LegacyData;
use snapshot::Snapshot;
use verify::VerifyReport;

/// Options validées de `migrate-legacy`, construites par la CLI (`main.rs`).
#[derive(Debug, Clone)]
pub struct Options {
    /// Base legacy à lire, jamais modifiée.
    pub from: PathBuf,
    /// Base cible 3.0 à installer.
    pub to: PathBuf,
    /// Valide et affiche le rapport sans rien écrire.
    pub dry_run: bool,
    /// Autorise le remplacement d'un fichier cible existant.
    pub force: bool,
}

/// Erreurs de la migration. Chaque message porte le contexte nécessaire
/// (table, colonne, ligne, clé primaire) pour corriger la base legacy.
#[derive(Debug, Error)]
pub enum Error {
    #[error("base legacy introuvable : {0}")]
    LegacyNotFound(PathBuf),
    #[error("le chemin source n'est pas un fichier : {0}")]
    SourceIsNotFile(PathBuf),
    #[error("ouverture de la base legacy {path} impossible : {source}\n  piste : {hint}")]
    LegacyOpen {
        path: PathBuf,
        source: sqlx::Error,
        hint: String,
    },
    #[error("PRAGMA integrity_check de la base legacy = {0:?} (attendu \"ok\")")]
    LegacyIntegrity(String),
    #[error("ouverture du snapshot {path} impossible : {source}")]
    SnapshotOpen { path: PathBuf, source: sqlx::Error },
    #[error("production du snapshot {path} impossible (VACUUM INTO) : {source}")]
    SnapshotVacuum { path: PathBuf, source: sqlx::Error },
    #[error("snapshot {path} incohérent : PRAGMA integrity_check = {detail:?} (attendu \"ok\")")]
    SnapshotIntegrity { path: PathBuf, detail: String },
    #[error(
        "source instable : {attempts} tentative(s) de snapshot ont échoué ({detail})\n  \
         piste : arrêter le serveur legacy avant de migrer (aucune cible n'a été écrite)"
    )]
    SnapshotUnstable { attempts: u32, detail: String },
    #[error(
        "la source a changé pendant la migration :\n  avant : {before}\n  après : {after}\n  \
         piste : arrêter le serveur legacy puis relancer ; la cible n'a pas été installée"
    )]
    SourceChanged { before: String, after: String },
    #[error("table legacy absente : {0}")]
    MissingLegacyTable(&'static str),
    #[error("table legacy {table} illisible : {source}")]
    LegacyQuery {
        table: &'static str,
        source: sqlx::Error,
    },
    #[error("schéma inattendu pour la table legacy {table} : {source}")]
    LegacySchema {
        table: &'static str,
        source: sqlx::Error,
    },
    #[error("lecture de {table}.{column} impossible (ligne {row}, clé {row_id:?}) : {source}")]
    ColumnRead {
        table: &'static str,
        column: &'static str,
        row: i64,
        row_id: String,
        source: sqlx::Error,
    },
    #[error("valeur NULL inattendue dans {table}.{column} (ligne {row}, clé {row_id:?})")]
    UnexpectedNull {
        table: &'static str,
        column: &'static str,
        row: i64,
        row_id: String,
    },
    #[error(
        "date invalide dans {table}.{column} (ligne {row}, clé {row_id:?}) : {value} — {detail}"
    )]
    InvalidDate {
        table: &'static str,
        column: &'static str,
        row: i64,
        row_id: String,
        value: String,
        detail: String,
    },
    #[error(
        "booléen invalide dans {table}.{column} (ligne {row}, clé {row_id:?}) : {value} (attendu 0 ou 1)"
    )]
    InvalidBoolean {
        table: &'static str,
        column: &'static str,
        row: i64,
        row_id: String,
        value: i64,
    },
    #[error(
        "valeur énumérée invalide dans {table}.{column} (ligne {row}, clé {row_id:?}) : {value:?} (attendu : {allowed})"
    )]
    InvalidEnum {
        table: &'static str,
        column: &'static str,
        row: i64,
        row_id: String,
        value: String,
        allowed: String,
    },
    #[error("clés étrangères orphelines dans la base legacy : {detail}")]
    OrphanForeignKeys { detail: String },
    #[error("cible déjà existante : {0} (utiliser --force pour la remplacer)")]
    TargetExists(PathBuf),
    #[error(
        "sauvegarde orpheline restaurée : {backup} -> {target}\n  \
         un crash a laissé `<to>.old` sans `<to>` ; la sauvegarde a été remise en place. \
         Relancer avec --force pour migrer par-dessus, ou supprimer {target} pour annuler"
    )]
    BackupRestored { target: PathBuf, backup: PathBuf },
    #[error("la source et la cible désignent le même fichier : {0}")]
    SameSourceAndTarget(PathBuf),
    #[error("fichier temporaire déjà existant : {0} (le supprimer ou utiliser --force)")]
    TmpExists(PathBuf),
    #[error("le chemin cible est un dossier : {0}")]
    TargetIsDirectory(PathBuf),
    #[error("dossier parent de la cible absent : {0}")]
    TargetParentMissing(PathBuf),
    #[error("création du fichier cible temporaire {path} impossible : {source}")]
    TargetInit { path: PathBuf, source: sqlx::Error },
    #[error("écriture des métadonnées d'import ({key}) impossible : {source}")]
    TargetMetadata {
        key: &'static str,
        source: sqlx::Error,
    },
    #[error("ouverture du fichier cible temporaire {path} impossible : {source}")]
    TargetOpen { path: PathBuf, source: sqlx::Error },
    #[error("écriture de la table cible {table} impossible : {source}")]
    TargetWrite {
        table: &'static str,
        source: sqlx::Error,
    },
    #[error("écriture du registre sqlx _sqlx_migrations dans la cible impossible : {0}")]
    LedgerWrite(sqlx::Error),
    #[error("lecture du registre sqlx _sqlx_migrations dans la cible impossible : {0}")]
    LedgerVerify(sqlx::Error),
    #[error("registre sqlx (_sqlx_migrations) incohérent : {0}")]
    LedgerInvalid(String),
    #[error("vérification post-migration échouée : {0}")]
    Verify(String),
    #[error(
        "échantillon incohérent pour {table} (clé {key}) :\n  legacy = {legacy}\n  cible  = {target}"
    )]
    SampleMismatch {
        table: &'static str,
        key: String,
        legacy: String,
        target: String,
    },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Exécute `migrate-legacy` de bout en bout.
pub async fn execute(options: &Options) -> Result<(), Error> {
    let started = Instant::now();
    println!(
        "migrate-legacy : {} -> {}",
        options.from.display(),
        options.to.display()
    );
    recover_orphan_backup(options)?;
    validate_paths(options)?;

    let source = legacy::source_meta(&options.from)?;
    println!(
        "  source  : {} ({} octet(s), sha256 {})",
        source.path.display(),
        source.bytes,
        source.sha256
    );
    if source.wal.exists {
        println!(
            "  WAL     : présent ({} octet(s), -shm {}) — la base legacy doit être au repos",
            source.wal.bytes,
            if source.wal.shm_exists {
                "présent"
            } else {
                "absent"
            }
        );
    } else {
        println!("  WAL     : absent");
    }

    // Snapshot cohérent : la source n'est plus lue directement (AUD-A5-002).
    let snapshot = snapshot::create(&options.from).await?;
    println!(
        "  snapshot: {} octet(s), sha256 {}, schéma legacy : {} (copie {} tentative(s))",
        snapshot.meta().bytes,
        snapshot.meta().sha256,
        snapshot.meta().schema_version,
        snapshot.meta().attempts
    );
    #[cfg(test)]
    snapshot::run_after_snapshot_hook(&options.from);

    let legacy = Legacy::open(snapshot.path()).await?;
    let outcome = run(options, &legacy, &source, &snapshot, started).await;
    legacy.close().await;
    outcome
}

/// Corps de la migration, une fois la source ouverte.
async fn run(
    options: &Options,
    legacy: &Legacy,
    source: &SourceMeta,
    snapshot: &Snapshot,
    started: Instant,
) -> Result<(), Error> {
    let integrity = legacy.integrity_check().await?;
    if integrity != "ok" {
        return Err(Error::LegacyIntegrity(integrity));
    }
    let journal = legacy.journal_mode().await?;
    println!("  intégrité : ok ; journal legacy : {journal}");

    let data = legacy.read_tables().await?;
    println!(
        "  lecture  : {} ligne(s) ; dates : {}",
        data.total_rows(),
        data.dates
    );
    print_warnings(&data.warnings);

    if options.dry_run {
        print_dry_run(options, &data, source, started.elapsed()).await;
        return Ok(());
    }

    let tmp = target::tmp_path(&options.to);
    if tmp.exists() {
        if options.force {
            std::fs::remove_file(&tmp)?;
        } else {
            return Err(Error::TmpExists(tmp));
        }
    }

    let verified = match target::write(&tmp, &data, source, snapshot.meta()).await {
        Ok(()) => verify_target(&tmp, legacy, &data).await,
        Err(err) => Err(err),
    };
    let report = match verified {
        Ok(report) => report,
        Err(err) => {
            drop(std::fs::remove_file(&tmp));
            return Err(err);
        }
    };

    println!(
        "  cible    : {} ({} octet(s)) vérifiée",
        tmp.display(),
        std::fs::metadata(&tmp)?.len()
    );
    report.print();

    // Dernier contrôle avant l'installation : la source ne doit pas avoir bougé
    // depuis le snapshot, sinon la migration serait partielle (AUD-A5-002).
    let current = snapshot::capture_signature(&options.from)?;
    if &current != snapshot.source_signature() {
        drop(std::fs::remove_file(&tmp));
        return Err(Error::SourceChanged {
            before: snapshot.source_signature().summary(),
            after: current.summary(),
        });
    }

    target::install(&tmp, &options.to, options.force)?;

    println!(
        "  installé : {} ({} octet(s))",
        options.to.display(),
        std::fs::metadata(&options.to)?.len()
    );
    println!(
        "migration terminée en {:.2} s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Récupère une sauvegarde `<to>.old` orpheline laissée par un crash entre les
/// deux renommages de [`target::install`] (AUD-A5-003).
///
/// - `<to>` absent + `<to>.old` présent → la sauvegarde est restaurée en `<to>`
///   (rename atomique) ; sans `--force`, on s'arrête avec un message explicite
///   pour que l'opérateur relance la migration en connaissance de cause ;
/// - `<to>` présent + `<to>.old` présent → sauvegarde orpheline seulement
///   signalée, jamais supprimée automatiquement (elle peut être la dernière
///   copie saine de la cible) ;
/// - `--dry-run` ne renomme rien : il annonce seulement la restauration qui
///   aurait lieu.
fn recover_orphan_backup(options: &Options) -> Result<(), Error> {
    // La validation de la source (erreurs `LegacyNotFound`/`SourceIsNotFile`)
    // appartient à `validate_paths` : ne rien toucher tant qu'elle n'est pas
    // lisible, pour que le diagnostic reste celui de la source.
    if !options.from.is_file() {
        return Ok(());
    }
    let backup = append_suffix(&options.to, ".old");
    if !backup.exists() {
        return Ok(());
    }
    if options.to.exists() {
        println!(
            "  .old    : sauvegarde orpheline conservée ({}) — la cible existe déjà",
            backup.display()
        );
        return Ok(());
    }
    if options.dry_run {
        println!(
            "  .old    : cible absente — la sauvegarde {} serait restaurée au prochain \
             lancement réel",
            backup.display()
        );
        return Ok(());
    }

    println!(
        "  .old    : cible absente — restauration de la sauvegarde {} en {}",
        backup.display(),
        options.to.display()
    );
    std::fs::rename(&backup, &options.to)?;
    if !options.force {
        return Err(Error::BackupRestored {
            target: options.to.clone(),
            backup,
        });
    }
    println!("  .old    : sauvegarde restaurée ; migration --force poursuivie");
    Ok(())
}

/// Valide les chemins avant toute lecture : échec rapide, sans effet de bord.
fn validate_paths(options: &Options) -> Result<(), Error> {
    let from = &options.from;
    if !from.exists() {
        return Err(Error::LegacyNotFound(from.clone()));
    }
    if !from.is_file() {
        return Err(Error::SourceIsNotFile(from.clone()));
    }

    let to = &options.to;
    if to.exists() && to.is_dir() {
        return Err(Error::TargetIsDirectory(to.clone()));
    }
    let Some(file_name) = to.file_name() else {
        return Err(Error::TargetIsDirectory(to.clone()));
    };
    let parent = to
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(Error::TargetParentMissing(parent.to_path_buf()));
    }

    let from_canonical = std::fs::canonicalize(from)?;
    let to_canonical = if to.exists() {
        std::fs::canonicalize(to)?
    } else {
        std::fs::canonicalize(parent)?.join(file_name)
    };
    if from_canonical == to_canonical {
        return Err(Error::SameSourceAndTarget(from.clone()));
    }

    if to.exists() && !options.force {
        return Err(Error::TargetExists(to.clone()));
    }
    Ok(())
}

/// Ouvre le temporaire en lecture seule et exécute tous les contrôles.
async fn verify_target(
    tmp: &Path,
    legacy: &Legacy,
    data: &LegacyData,
) -> Result<VerifyReport, Error> {
    let options = SqliteConnectOptions::new()
        .filename(tmp)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(10));
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .map_err(|source| Error::TargetOpen {
            path: tmp.to_path_buf(),
            source,
        })?;
    let report = verify::verify(&mut conn, legacy, data).await?;
    conn.close().await.map_err(|source| Error::TargetOpen {
        path: tmp.to_path_buf(),
        source,
    })?;
    Ok(report)
}

/// Rapport du mode `--dry-run` : aucune écriture, seulement des comptes et une
/// comparaison du registre sqlx attendu avec la cible existante (si `--force`).
async fn print_dry_run(
    options: &Options,
    data: &LegacyData,
    source: &SourceMeta,
    elapsed: Duration,
) {
    println!("  mode     : --dry-run, aucune écriture");
    println!("  tables legacy :");
    for (table, count) in data.counts() {
        println!("    - {table:<18} {count:>8}");
    }
    println!("  dates     : {}", data.dates);
    println!(
        "  registre sqlx : {}",
        ledger::compare_existing(&options.to).await
    );
    println!(
        "  conclusion : source lisible et normalisable ; la migration réelle écrirait {} puis \
         l'installerait en {}",
        target::tmp_path(&options.to).display(),
        options.to.display()
    );
    println!(
        "  durée de lecture/validation : {:.2} s",
        elapsed.as_secs_f64()
    );
    let _ = source;
}

/// Affiche les avertissements non bloquants agrégés pendant la lecture.
fn print_warnings(warnings: &BTreeMap<String, u64>) {
    if warnings.is_empty() {
        return;
    }
    println!("  avertissements :");
    for (message, count) in warnings {
        println!("    - {message} (×{count})");
    }
}

/// Concatène une chaîne à un chemin (`<path><suffix>`), sans dépendre du type
/// de fichier : utilisé pour `-wal`, `-shm`, `.tmp` et `.old`.
pub(crate) fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}
