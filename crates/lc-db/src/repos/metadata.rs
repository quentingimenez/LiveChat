//! Repository `app_metadata` (clé/valeur TEXT).

use sqlx::SqlitePool;

use crate::error::DbResult;

/// Accès au magasin clé/valeur `app_metadata`.
///
/// Utilisé pour `schema_version` (contrat v3) et les petits drapeaux de
/// déploiement (ex. date de dernière migration de données Phase 4).
#[derive(Clone, Debug)]
pub struct MetadataRepository {
    pool: SqlitePool,
}

impl MetadataRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Lit une entrée ; `None` si la clé n'existe pas.
    pub async fn get(&self, key: &str) -> DbResult<Option<String>> {
        let value = sqlx::query_scalar::<_, String>("SELECT value FROM app_metadata WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(value)
    }

    /// Écrit une entrée (insert ou remplacement).
    pub async fn set(&self, key: &str, value: &str) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO app_metadata (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
