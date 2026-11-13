//! Persistance LiveChat : pool `sqlx`/SQLite, migrations et repositories typés.
//!
//! ## Vue d'ensemble
//!
//! - [`Db`] ouvre un [`SqlitePool`](sqlx::SqlitePool) avec les pragmas imposés
//!   (WAL, `foreign_keys = ON`, `busy_timeout = 5000`, `synchronous = NORMAL`),
//!   applique les migrations embarquées de `crates/lc-db/migrations/`
//!   (`0001`..`0005`), puis vérifie `app_metadata.schema_version`. Une URL
//!   `file:…?mode=ro` ouvre une connexion
//!   strictement en lecture seule (migrations désactivées, aucun passage WAL).
//! - Chaque table possède un repository exposé par [`Db`] (`db.media_assets()`,
//!   `db.playback_jobs()`, …) qui renvoie des entités `lc-domain`.
//! - Toutes les requêtes sont **runtime** (`sqlx::query`/`query_as` + `FromRow`) :
//!   aucun `query!` compile-time, donc aucun besoin de `DATABASE_URL` pour
//!   builder le workspace.
//!
//! ```no_run
//! use lc_db::{Db, DbConfig};
//!
//! # async fn run() -> Result<(), lc_db::DbError> {
//! let db = Db::open("data/livechat.db", &DbConfig::default()).await?;
//! let guild = lc_domain::GuildId::parse("123456789012345678").expect("guild id valide");
//! let jobs = db.playback_jobs().list_pending_by_guild(&guild).await?;
//! # let _ = jobs;
//! # Ok(())
//! # }
//! ```
//!
//! ## Conventions
//!
//! - Les dates sont des `INTEGER` epoch **millisecondes UTC** en base et des
//!   [`chrono::DateTime<Utc>`] dans le domaine ; les fonctions de repository qui
//!   ont besoin du temps courant le reçoivent en paramètre (jamais d'appel caché
//!   à `Utc::now()`), ce qui rend les tests et le scheduler déterministes.
//! - Les booléens sont stockés `INTEGER 0/1` (contrainte `CHECK`) et exposés
//!   `bool`.
//! - Les enums métier sont stockés en `TEXT` MAJUSCULE (`'READY'`, `'PENDING'`…)
//!   et décodés via [`lc_domain::MediaStatus::from_db_str`] & co.

#![forbid(unsafe_code)]

mod config;
mod convert;
mod db;
mod error;
mod repos;
mod retry;

pub use config::DbConfig;
pub use db::{Db, SCHEMA_VERSION};
pub use error::{DbError, DbResult};
pub use retry::with_busy_retry;

/// DDL de la migration **initiale**, embarqué à la compilation.
///
/// C'est la source unique du schéma initial : `xtask` l'exécute pour créer la
/// base cible d'une migration legacy, puis seede le registre sqlx avec la seule
/// version 1. Les migrations suivantes (`0002`..`0005`) sont appliquées par
/// `sqlx` au premier démarrage du serveur.
///
/// `docs/rust/schema-v3.sql` est la copie humaine du schéma complet
/// (0001..0005) ; un test de `lc-db` vérifie que la concaténation des migrations
/// reste octet pour octet identique à ce contrat.
pub const MIGRATION_SQL: &str = include_str!("../migrations/0001_init.sql");

pub use repos::{
    DiscordLinkCodesRepository, GuildInvitesRepository, GuildMembersRepository, GuildsRepository,
    IngestClientsRepository, LinkOutcome, MediaAssetsRepository, MediaBoardRepository,
    MemberChange, MetadataRepository, OverlayClientsRepository, PairingCodesRepository,
    PlaybackJobsRepository,
};
