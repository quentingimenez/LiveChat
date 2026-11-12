//! Repository `guild_invites` (codes d'invitation porteurs d'un rôle).

use chrono::DateTime;
use lc_domain::entities::GuildInvite;
use lc_domain::enums::GuildRole;
use lc_domain::ids::{GuildId, GuildMemberId, PairingCode as PairingCodeValue};
use sqlx::SqlitePool;

use crate::convert::{decode, decode_opt, from_ms, from_opt_ms, to_ms, to_opt_ms};
use crate::error::{conflict_on_unique, DbResult};

const TABLE: &str = "guild_invites";

/// Ligne SQLite de `guild_invites`.
#[derive(Debug, sqlx::FromRow)]
struct GuildInviteRow {
    code: String,
    guild_id: String,
    role: String,
    member_id: Option<String>,
    max_uses: Option<i64>,
    use_count: i64,
    created_by_member_id: Option<String>,
    expires_at: Option<i64>,
    revoked_at: Option<i64>,
    created_at: i64,
}

impl GuildInviteRow {
    fn into_entity(self) -> DbResult<GuildInvite> {
        Ok(GuildInvite {
            code: decode(TABLE, "code", &self.code, PairingCodeValue::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            role: decode(TABLE, "role", &self.role, GuildRole::from_db_str)?,
            member_id: decode_opt(
                TABLE,
                "member_id",
                self.member_id.as_deref(),
                GuildMemberId::parse,
            )?,
            max_uses: self.max_uses,
            use_count: self.use_count,
            created_by_member_id: decode_opt(
                TABLE,
                "created_by_member_id",
                self.created_by_member_id.as_deref(),
                GuildMemberId::parse,
            )?,
            expires_at: from_opt_ms(TABLE, "expires_at", self.expires_at)?,
            revoked_at: from_opt_ms(TABLE, "revoked_at", self.revoked_at)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Repository des invitations.
#[derive(Clone, Debug)]
pub struct GuildInvitesRepository {
    pool: SqlitePool,
}

impl GuildInvitesRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Crée une invitation (clé primaire = code normalisé).
    ///
    /// Violation `UNIQUE` → [`lc_domain::DomainError::Conflict`] : l'appelant
    /// régénère un code.
    pub async fn create(&self, invite: &GuildInvite) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guild_invites ( \
                code, guild_id, role, member_id, max_uses, use_count, created_by_member_id, \
                expires_at, revoked_at, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(invite.code.as_str())
        .bind(invite.guild_id.as_str())
        .bind(invite.role.as_db_str())
        .bind(invite.member_id.map(GuildMemberId::to_canonical))
        .bind(invite.max_uses)
        .bind(invite.use_count)
        .bind(invite.created_by_member_id.map(GuildMemberId::to_canonical))
        .bind(to_opt_ms(invite.expires_at))
        .bind(to_opt_ms(invite.revoked_at))
        .bind(to_ms(invite.created_at))
        .execute(&self.pool)
        .await
        .map_err(|error| conflict_on_unique(error, "ce code d'invitation existe déjà"))?;
        Ok(())
    }

    /// Lit une invitation (utilisable ou non).
    pub async fn get(&self, code: &PairingCodeValue) -> DbResult<Option<GuildInvite>> {
        let row = sqlx::query_as::<_, GuildInviteRow>("SELECT * FROM guild_invites WHERE code = ?")
            .bind(code.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(GuildInviteRow::into_entity).transpose()
    }

    /// Consomme une utilisation de l'invitation **atomiquement**.
    ///
    /// Les conditions (non révoquée, non expirée, usages restants) sont
    /// évaluées dans la même instruction que l'incrément : deux consommateurs
    /// concurrents ne peuvent pas dépasser `max_uses`.
    ///
    /// Retourne l'invitation après incrément, ou `None` si elle est inconnue,
    /// révoquée, expirée ou épuisée.
    pub async fn consume(
        &self,
        code: &PairingCodeValue,
        now: DateTime<chrono::Utc>,
    ) -> DbResult<Option<GuildInvite>> {
        let row = sqlx::query_as::<_, GuildInviteRow>(
            "UPDATE guild_invites SET use_count = use_count + 1 \
             WHERE code = ? \
               AND revoked_at IS NULL \
               AND (expires_at IS NULL OR expires_at > ?) \
               AND (max_uses IS NULL OR use_count < max_uses) \
             RETURNING *",
        )
        .bind(code.as_str())
        .bind(to_ms(now))
        .fetch_optional(&self.pool)
        .await?;
        row.map(GuildInviteRow::into_entity).transpose()
    }

    /// Révoque une invitation de la guilde donnée.
    ///
    /// Retourne `false` si elle n'existe pas dans cette guilde ou si elle était
    /// déjà révoquée.
    pub async fn revoke(
        &self,
        guild_id: &GuildId,
        code: &PairingCodeValue,
        revoked_at: DateTime<chrono::Utc>,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE guild_invites SET revoked_at = ? \
             WHERE code = ? AND guild_id = ? AND revoked_at IS NULL",
        )
        .bind(to_ms(revoked_at))
        .bind(code.as_str())
        .bind(guild_id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Liste les invitations d'une guilde, de la plus récente à la plus
    /// ancienne.
    pub async fn list_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<GuildInvite>> {
        let rows = sqlx::query_as::<_, GuildInviteRow>(
            "SELECT * FROM guild_invites WHERE guild_id = ? ORDER BY created_at DESC, code ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(GuildInviteRow::into_entity).collect()
    }

    /// Supprime toutes les invitations d'une guilde (purge).
    pub async fn delete_by_guild(&self, guild_id: &GuildId) -> DbResult<u64> {
        let result = sqlx::query("DELETE FROM guild_invites WHERE guild_id = ?")
            .bind(guild_id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
