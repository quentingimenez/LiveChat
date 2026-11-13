//! Repository `pairing_codes` (codes d'appairage à usage unique).

use chrono::DateTime;
use lc_domain::entities::PairingCodeEntry;
use lc_domain::enums::PairingMode;
use lc_domain::ids::{GuildId, PairingCode as PairingCodeValue};
use sqlx::SqlitePool;

use crate::convert::{decode, from_ms, from_opt_ms, to_ms};
use crate::error::{conflict_on_unique, DbResult};

const TABLE: &str = "pairing_codes";

/// Ligne SQLite de `pairing_codes`.
#[derive(Debug, sqlx::FromRow)]
struct PairingCodeRow {
    code: String,
    guild_id: String,
    mode: String,
    grants_admin: i64,
    created_by_discord_user_id: String,
    author_name: Option<String>,
    author_image: Option<String>,
    expires_at: Option<i64>,
    used_at: Option<i64>,
    created_at: i64,
}

impl PairingCodeRow {
    fn into_entity(self) -> DbResult<PairingCodeEntry> {
        Ok(PairingCodeEntry {
            code: decode(TABLE, "code", &self.code, PairingCodeValue::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            mode: decode(TABLE, "mode", &self.mode, PairingMode::from_db_str)?,
            grants_admin: self.grants_admin != 0,
            created_by_discord_user_id: self.created_by_discord_user_id,
            author_name: self.author_name,
            author_image: self.author_image,
            expires_at: from_opt_ms(TABLE, "expires_at", self.expires_at)?,
            used_at: from_opt_ms(TABLE, "used_at", self.used_at)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Repository des codes d'appairage.
#[derive(Clone, Debug)]
pub struct PairingCodesRepository {
    pool: SqlitePool,
}

impl PairingCodesRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Crée un code d'appairage (clé primaire = code normalisé).
    ///
    /// Violation `UNIQUE` → [`lc_domain::DomainError::Conflict`].
    pub async fn create(&self, code: &PairingCodeEntry) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO pairing_codes ( \
                code, guild_id, mode, grants_admin, created_by_discord_user_id, author_name, \
                author_image, expires_at, used_at, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(code.code.as_str())
        .bind(code.guild_id.as_str())
        .bind(code.mode.as_db_str())
        .bind(i64::from(code.grants_admin))
        .bind(code.created_by_discord_user_id.as_str())
        .bind(code.author_name.as_deref())
        .bind(code.author_image.as_deref())
        .bind(code.expires_at.map(to_ms))
        .bind(code.used_at.map(to_ms))
        .bind(to_ms(code.created_at))
        .execute(&self.pool)
        .await
        .map_err(|error| conflict_on_unique(error, "ce code d'appairage existe déjà"))?;
        Ok(())
    }

    /// Lit un code d'appairage (consommé ou non).
    pub async fn get(&self, code: &PairingCodeValue) -> DbResult<Option<PairingCodeEntry>> {
        let row = sqlx::query_as::<_, PairingCodeRow>("SELECT * FROM pairing_codes WHERE code = ?")
            .bind(code.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(PairingCodeRow::into_entity).transpose()
    }

    /// Consomme un code **atomiquement**.
    ///
    /// La condition `used_at IS NULL AND (expires_at IS NULL OR expires_at >
    /// now)` est évaluée dans la même instruction que la mise à jour : deux
    /// consommateurs concurrents ne peuvent pas réussir tous les deux, et un
    /// code expiré n'est jamais consommé.
    ///
    /// Retourne `None` si le code est inconnu, déjà consommé ou expiré.
    pub async fn consume(
        &self,
        code: &PairingCodeValue,
        now: DateTime<chrono::Utc>,
    ) -> DbResult<Option<PairingCodeEntry>> {
        let row = sqlx::query_as::<_, PairingCodeRow>(
            "UPDATE pairing_codes SET used_at = ? \
             WHERE code = ? \
               AND used_at IS NULL \
               AND (expires_at IS NULL OR expires_at > ?) \
             RETURNING *",
        )
        .bind(to_ms(now))
        .bind(code.as_str())
        .bind(to_ms(now))
        .fetch_optional(&self.pool)
        .await?;
        row.map(PairingCodeRow::into_entity).transpose()
    }

    /// Révoque un code non encore consommé (pose `used_at` sans le consommer).
    ///
    /// Retourne `false` si le code n'existe pas ou s'il était déjà consommé.
    pub async fn revoke(
        &self,
        code: &PairingCodeValue,
        revoked_at: DateTime<chrono::Utc>,
    ) -> DbResult<bool> {
        let result =
            sqlx::query("UPDATE pairing_codes SET used_at = ? WHERE code = ? AND used_at IS NULL")
                .bind(to_ms(revoked_at))
                .bind(code.as_str())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Supprime les codes inutilisables : consommés/révoqués ou expirés.
    ///
    /// Retourne le nombre de codes supprimés.
    pub async fn delete_expired(&self, now: DateTime<chrono::Utc>) -> DbResult<u64> {
        let result = sqlx::query(
            "DELETE FROM pairing_codes \
             WHERE used_at IS NOT NULL \
                OR (expires_at IS NOT NULL AND expires_at <= ?)",
        )
        .bind(to_ms(now))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Liste tous les codes, du plus récent au plus ancien.
    pub async fn list(&self) -> DbResult<Vec<PairingCodeEntry>> {
        let rows = sqlx::query_as::<_, PairingCodeRow>(
            "SELECT * FROM pairing_codes ORDER BY created_at DESC, code ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(PairingCodeRow::into_entity).collect()
    }

    /// Liste les codes d'une guilde, du plus récent au plus ancien.
    pub async fn list_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<PairingCodeEntry>> {
        let rows = sqlx::query_as::<_, PairingCodeRow>(
            "SELECT * FROM pairing_codes WHERE guild_id = ? ORDER BY created_at DESC, code ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(PairingCodeRow::into_entity).collect()
    }
}
