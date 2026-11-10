//! Ouverture du pool SQLite et application du contrat de persistance.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::config::DbConfig;
use crate::error::{DbError, DbResult};
use crate::repos::{
    GuildsRepository, IngestClientsRepository, MediaAssetsRepository, MemeBoardRepository,
    MetadataRepository, OverlayClientsRepository, PairingCodesRepository, PlaybackJobsRepository,
};

/// Version du schéma attendue dans `app_metadata.schema_version`.
pub const SCHEMA_VERSION: &str = "3.0.0";

/// Migrations embarquées dans le binaire (`crates/lc-db/migrations`).
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Base de données LiveChat : un pool SQLite et ses repositories.
///
/// `Db` est [`Clone`] : le clone partage le même pool (compteur de références
/// `sqlx`), il peut donc être distribué tel quel aux workers/tasks de la Phase 3.
#[derive(Clone, Debug)]
pub struct Db {
    pool: SqlitePool,
    config: DbConfig,
}

impl Db {
    /// Ouvre (et crée au besoin) une base SQLite à partir d'un chemin de
    /// fichier.
    ///
    /// - crée le dossier parent si `config.create_parent_dirs` ;
    /// - applique les pragmas du contrat (WAL, FK, busy_timeout, synchronous) ;
    /// - exécute les migrations embarquées puis vérifie `schema_version`.
    pub async fn open(path: impl AsRef<Path>, config: &DbConfig) -> DbResult<Self> {
        let path = path.as_ref();
        if config.create_parent_dirs {
            create_parent_dir(path)?;
        }
        let options = apply_pragmas(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(config.create_if_missing),
            config,
        );
        Self::from_options(options, config).await
    }

    /// Ouvre une base SQLite **en mémoire**.
    ///
    /// Une base mémoire est propre à chaque connexion SQLite : le pool est donc
    /// forcé à une seule connexion (`max_connections = min_connections = 1`),
    /// sinon les requêtes suivantes ne verraient ni les tables ni les données.
    /// Pratique pour les tests et les démonstrations ; les données disparaissent
    /// à la fermeture.
    pub async fn open_in_memory(config: &DbConfig) -> DbResult<Self> {
        let mut effective = config.clone();
        effective.max_connections = 1;
        effective.min_connections = 1;
        let options = apply_pragmas(
            SqliteConnectOptions::new()
                .in_memory(true)
                .create_if_missing(true),
            &effective,
        );
        Self::from_options(options, &effective).await
    }

    /// Ouvre une base à partir d'une URL de connexion.
    ///
    /// Accepte les schémas `sqlite:` de `sqlx` (`sqlite://data/livechat.db`,
    /// `sqlite::memory:`) et les URLs `file:` façon Prisma
    /// (`file:./data/livechat.db`, `file::memory:`). Pour `file:` :
    ///
    /// - `?mode=memory` ouvre une base en mémoire ;
    /// - `?mode=ro` ouvre une connexion **strictement en lecture seule** : le
    ///   dossier parent n'est pas créé, les migrations et la création du
    ///   fichier sont désactivées, et les pragmas d'écriture (`journal_mode`,
    ///   `synchronous`) ne sont pas appliqués. Toute écriture échoue alors avec
    ///   l'erreur SQLite `SQLITE_READONLY` (refus effectif, pas seulement
    ///   documentaire) ;
    /// - `?mode=rw` et `?mode=rwc` sont acceptés pour la compatibilité ; le
    ///   comportement reste piloté par `DbConfig` (comme [`Db::open`]).
    ///
    /// `?mode=ro` désactive `run_migrations` et `create_parent_dirs` pour cette
    /// connexion (les migrations écrivent) ; `check_schema_version` reste actif
    /// car il ne fait que lire.
    pub async fn connect(url: &str, config: &DbConfig) -> DbResult<Self> {
        let parsed = parse_connection_url(url)?;
        let mut effective = config.clone();
        if parsed.read_only {
            effective.create_if_missing = false;
            effective.create_parent_dirs = false;
            effective.run_migrations = false;
            tracing::debug!(
                url,
                "connexion SQLite en lecture seule : migrations et création désactivées"
            );
        }
        if effective.create_parent_dirs {
            if let Some(path) = parsed.file_path.as_deref() {
                create_parent_dir(path)?;
            }
        }
        let options = if parsed.read_only {
            apply_read_only_pragmas(parsed.options.create_if_missing(false), &effective)
        } else {
            apply_pragmas(
                parsed
                    .options
                    .create_if_missing(effective.create_if_missing),
                &effective,
            )
        };
        Self::from_options(options, &effective).await
    }

    async fn from_options(options: SqliteConnectOptions, config: &DbConfig) -> DbResult<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections.max(1))
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout)
            .connect_with(options)
            .await?;

        let db = Self {
            pool,
            config: config.clone(),
        };

        if config.run_migrations {
            db.migrate().await?;
        }
        if config.check_schema_version {
            db.ensure_schema_version().await?;
        }
        Ok(db)
    }

    /// Exécute les migrations embarquées.
    ///
    /// Idempotent : `sqlx` ne rejoue que les migrations dont la version n'est
    /// pas déjà enregistrée dans `_sqlx_migrations`.
    pub async fn migrate(&self) -> DbResult<()> {
        tracing::debug!("application des migrations sqlx");
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    /// Vérifie que `app_metadata.schema_version` vaut [`SCHEMA_VERSION`].
    pub async fn ensure_schema_version(&self) -> DbResult<()> {
        let found = self.get_metadata("schema_version").await?;
        match found {
            Some(found) if found == SCHEMA_VERSION => Ok(()),
            Some(found) => Err(DbError::SchemaVersion {
                expected: SCHEMA_VERSION,
                found,
            }),
            None => Err(DbError::SchemaVersion {
                expected: SCHEMA_VERSION,
                found: "<absente>".to_owned(),
            }),
        }
    }

    /// Lit une entrée `app_metadata`.
    pub async fn get_metadata(&self, key: &str) -> DbResult<Option<String>> {
        self.metadata().get(key).await
    }

    /// Écrit (upsert) une entrée `app_metadata`.
    pub async fn set_metadata(&self, key: &str, value: &str) -> DbResult<()> {
        self.metadata().set(key, value).await
    }

    /// Configuration effective (utile au diagnostic).
    pub fn config(&self) -> &DbConfig {
        &self.config
    }

    /// Pool `sqlx` sous-jacent.
    ///
    /// Exposé pour les migrations/outils d'administration ; le code métier
    /// passe par les repositories.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Ferme le pool et attend la fin des requêtes en cours.
    pub async fn close(self) {
        self.pool.close().await;
    }

    /// Repository `app_metadata`.
    pub fn metadata(&self) -> MetadataRepository {
        MetadataRepository::new(self.pool.clone())
    }

    /// Repository `media_assets`.
    pub fn media_assets(&self) -> MediaAssetsRepository {
        MediaAssetsRepository::new(self.pool.clone())
    }

    /// Repository `meme_board_items`.
    pub fn meme_board(&self) -> MemeBoardRepository {
        MemeBoardRepository::new(self.pool.clone())
    }

    /// Repository `playback_jobs`.
    pub fn playback_jobs(&self) -> PlaybackJobsRepository {
        PlaybackJobsRepository::new(self.pool.clone())
    }

    /// Repository `guilds`.
    pub fn guilds(&self) -> GuildsRepository {
        GuildsRepository::new(self.pool.clone())
    }

    /// Repository `overlay_clients`.
    pub fn overlay_clients(&self) -> OverlayClientsRepository {
        OverlayClientsRepository::new(self.pool.clone())
    }

    /// Repository `ingest_clients`.
    pub fn ingest_clients(&self) -> IngestClientsRepository {
        IngestClientsRepository::new(self.pool.clone())
    }

    /// Repository `pairing_codes`.
    pub fn pairing_codes(&self) -> PairingCodesRepository {
        PairingCodesRepository::new(self.pool.clone())
    }
}

/// Applique les pragmas du contrat de persistance.
fn apply_pragmas(options: SqliteConnectOptions, config: &DbConfig) -> SqliteConnectOptions {
    options
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(config.busy_timeout)
}

/// Applique les pragmas d'une connexion strictement en lecture seule.
///
/// `journal_mode` et `synchronous` écrivent dans l'en-tête de la base : ils sont
/// volontairement omis (passer une base en WAL est une écriture). `foreign_keys`
/// et `busy_timeout` sont des réglages de connexion, sans effet sur le fichier.
fn apply_read_only_pragmas(
    options: SqliteConnectOptions,
    config: &DbConfig,
) -> SqliteConnectOptions {
    options
        .read_only(true)
        .foreign_keys(true)
        .busy_timeout(config.busy_timeout)
}

/// Crée le dossier parent d'un fichier de base (sans effet pour un nom nu).
fn create_parent_dir(path: &Path) -> DbResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// Options extraites d'une URL de connexion.
#[derive(Debug)]
struct ParsedUrl {
    options: SqliteConnectOptions,
    /// Chemin de fichier (pour la création du dossier parent et les messages).
    file_path: Option<PathBuf>,
    /// `true` si l'URL demande une connexion strictement en lecture seule
    /// (`?mode=ro`).
    read_only: bool,
}

/// Parse une URL de connexion en options `sqlx`.
///
/// `file:` est le format Prisma repris tel quel : `file:./prisma/dev.db`,
/// `file::memory:` ou `file:/tmp/lc.db?mode=memory`. Les paramètres de requête
/// inconnus (ex. `connection_limit`) sont ignorés ; seul `mode` est interprété
/// (`memory`, `ro`, `rw`, `rwc`).
fn parse_connection_url(url: &str) -> DbResult<ParsedUrl> {
    if let Some(rest) = url.strip_prefix("file:") {
        let (path, query) = match rest.split_once('?') {
            Some((path, query)) => (path, query),
            None => (rest, ""),
        };
        if path.is_empty() {
            return Err(DbError::InvalidUrl(url.to_owned()));
        }

        let mut options = SqliteConnectOptions::new().filename(path);
        let mut file_path = Some(PathBuf::from(path));
        let mut read_only = false;
        for pair in query.split('&').filter(|part| !part.is_empty()) {
            let (key, value) = match pair.split_once('=') {
                Some((key, value)) => (key, value),
                None => (pair, ""),
            };
            match (key, value) {
                ("mode", "memory") => {
                    options = options.in_memory(true);
                    file_path = None;
                }
                ("mode", "ro") => read_only = true,
                ("mode", "rwc" | "rw") => {}
                ("mode", unknown) => {
                    return Err(DbError::InvalidUrl(format!(
                        "mode SQLite inconnu `{unknown}` dans `{url}`"
                    )));
                }
                _ => {
                    tracing::debug!(parametre = key, "paramètre d'URL SQLite ignoré");
                }
            }
        }
        Ok(ParsedUrl {
            options,
            file_path,
            read_only,
        })
    } else {
        Ok(ParsedUrl {
            options: SqliteConnectOptions::from_str(url)?,
            file_path: None,
            read_only: requests_read_only(url),
        })
    }
}

/// `true` si l'URL contient un paramètre de requête `mode=ro`.
///
/// Utilisé pour les URLs `sqlite:` analysées par `sqlx` (qui ne permettent pas
/// de relire `read_only` depuis les options) : la demande de lecture seule est
/// donc détectée dans la chaîne pour que [`Db::connect`] n'applique pas les
/// pragmas d'écriture.
fn requests_read_only(url: &str) -> bool {
    url.split_once('?').is_some_and(|(_, query)| {
        query.split('&').any(|part| {
            let (key, value) = match part.split_once('=') {
                Some(pair) => pair,
                None => (part, ""),
            };
            key.eq_ignore_ascii_case("mode") && value.eq_ignore_ascii_case("ro")
        })
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_connection_url, DbConfig, SCHEMA_VERSION};
    use crate::error::DbError;

    #[test]
    fn url_file_relative() {
        let parsed = parse_connection_url("file:./data/livechat.db").unwrap();
        assert_eq!(
            parsed.file_path.as_deref(),
            Some(std::path::Path::new("./data/livechat.db"))
        );
    }

    #[test]
    fn url_file_memory() {
        let parsed = parse_connection_url("file::memory:?mode=memory").unwrap();
        assert!(parsed.file_path.is_none());
        assert!(!parsed.read_only);
    }

    #[test]
    fn url_mode_ro_demande_lecture_seule() {
        let parsed = parse_connection_url("file:./db.sqlite?mode=ro").unwrap();
        assert!(parsed.read_only);
        assert_eq!(
            parsed.file_path.as_deref(),
            Some(std::path::Path::new("./db.sqlite"))
        );

        let parsed = parse_connection_url("sqlite://./db.sqlite?mode=ro").unwrap();
        assert!(parsed.read_only);

        let parsed = parse_connection_url("file:./db.sqlite?mode=rw").unwrap();
        assert!(!parsed.read_only);
    }

    #[test]
    fn url_mode_inconnu_rejete() {
        let error = parse_connection_url("file:./db.sqlite?mode=bidon").unwrap_err();
        assert!(matches!(error, DbError::InvalidUrl(_)));
    }

    #[test]
    fn version_de_schema_du_contrat() {
        assert_eq!(SCHEMA_VERSION, "3.0.0");
        assert_eq!(DbConfig::default().max_connections, 5);
    }
}
