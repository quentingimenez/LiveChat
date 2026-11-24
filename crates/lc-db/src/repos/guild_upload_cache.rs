//! Repository `guild_upload_cache` (empreintes de fichiers déjà importés par
//! une guilde, pour éviter de renvoyer les octets).

use lc_domain::ids::GuildId;
use sqlx::SqlitePool;
use time::UtcDateTime;

use crate::convert::to_ms;
use crate::error::DbResult;

/// Empreintes d'uploads connues d'une guilde.
#[derive(Clone, Debug)]
pub struct GuildUploadCacheRepository {
    pool: SqlitePool,
}

impl GuildUploadCacheRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Enregistre qu'une guilde a importé ce contenu.
    ///
    /// Idempotent : un réimport du même fichier rafraîchit seulement la date.
    pub async fn record(
        &self,
        guild_id: &GuildId,
        source_hash: &str,
        now: UtcDateTime,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guild_upload_cache (guild_id, source_hash, created_at) \
             VALUES (?, ?, ?) \
             ON CONFLICT(guild_id, source_hash) DO UPDATE SET created_at = excluded.created_at",
        )
        .bind(guild_id.as_str())
        .bind(source_hash)
        .bind(to_ms(now))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `true` si cette guilde a déjà importé ce contenu.
    pub async fn contains(&self, guild_id: &GuildId, source_hash: &str) -> DbResult<bool> {
        let present: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM guild_upload_cache \
             WHERE guild_id = ? AND source_hash = ?)",
        )
        .bind(guild_id.as_str())
        .bind(source_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(present != 0)
    }
}
