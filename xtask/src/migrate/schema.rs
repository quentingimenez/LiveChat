//! DDL du schéma cible 3.0, repris de la migration embarquée `lc-db`.
//!
//! [`lc_db::MIGRATION_SQL`] (`crates/lc-db/migrations/0001_init.sql`) est la
//! source unique du schéma ; `docs/rust/schema-v3.sql` en est la copie humaine
//! et un test de `lc-db` garantit leur égalité octet pour octet. Le migrateur
//! ne duplique donc plus le DDL : une évolution de schéma se fait dans la
//! migration, la doc suit, et le binaire `xtask` la reprend automatiquement.

/// Contenu intégral de la migration initiale v3, exécuté sur la base cible.
pub const SCHEMA_V3_SQL: &str = lc_db::MIGRATION_SQL;
