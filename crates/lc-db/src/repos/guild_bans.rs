//! Repository `guild_bans` (people banned from a guild).

use lc_domain::entities::GuildBan;
use lc_domain::ids::{GuildBanId, GuildId, GuildMemberId};
use sqlx::SqlitePool;
use time::UtcDateTime;

use crate::convert::{decode, decode_opt, from_ms, to_ms};
use crate::error::{conflict_on_unique, is_unique_violation, DbResult};

const TABLE: &str = "guild_bans";

/// SQLite row of `guild_bans`.
#[derive(Debug, sqlx::FromRow)]
struct GuildBanRow {
    id: String,
    guild_id: String,
    member_id: Option<String>,
    discord_user_id: Option<String>,
    display_name: String,
    avatar: Option<String>,
    banned_by: Option<String>,
    created_at: i64,
}

impl GuildBanRow {
    fn into_entity(self) -> DbResult<GuildBan> {
        Ok(GuildBan {
            id: decode(TABLE, "id", &self.id, GuildBanId::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            member_id: decode_opt(
                TABLE,
                "member_id",
                self.member_id.as_deref(),
                GuildMemberId::parse,
            )?,
            discord_user_id: self.discord_user_id,
            display_name: self.display_name,
            avatar: self.avatar,
            banned_by: self.banned_by,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Repository of guild bans.
#[derive(Clone, Debug)]
pub struct GuildBansRepository {
    pool: SqlitePool,
}

impl GuildBansRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Records a ban. A second ban of the same Discord account in the guild is
    /// a [`lc_domain::DomainError::Conflict`].
    pub async fn create(&self, ban: &GuildBan) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guild_bans ( \
                id, guild_id, member_id, discord_user_id, display_name, avatar, \
                banned_by, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ban.id.to_canonical())
        .bind(ban.guild_id.as_str())
        .bind(ban.member_id.map(GuildMemberId::to_canonical))
        .bind(ban.discord_user_id.as_deref())
        .bind(ban.display_name.as_str())
        .bind(ban.avatar.as_deref())
        .bind(ban.banned_by.as_deref())
        .bind(to_ms(ban.created_at))
        .execute(&self.pool)
        .await
        .map_err(|error| conflict_on_unique(error, "discord account already banned"))?;
        Ok(())
    }

    /// Creates a ban and attaches its device fingerprints in one transaction.
    ///
    /// Returns `None` when the Discord account is already banned (the caller
    /// then attaches the fingerprints to the existing ban). On any other error
    /// nothing is committed: a ban always carries the fingerprints collected
    /// with it, never a ban without them because the second insert failed.
    pub async fn create_with_devices(
        &self,
        ban: &GuildBan,
        device_hashes: &[String],
        now: UtcDateTime,
    ) -> DbResult<Option<GuildBan>> {
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO guild_bans ( \
                id, guild_id, member_id, discord_user_id, display_name, avatar, \
                banned_by, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ban.id.to_canonical())
        .bind(ban.guild_id.as_str())
        .bind(ban.member_id.map(GuildMemberId::to_canonical))
        .bind(ban.discord_user_id.as_deref())
        .bind(ban.display_name.as_str())
        .bind(ban.avatar.as_deref())
        .bind(ban.banned_by.as_deref())
        .bind(to_ms(ban.created_at))
        .execute(&mut *transaction)
        .await;
        if let Err(error) = inserted {
            transaction.rollback().await?;
            return if is_unique_violation(&error) {
                Ok(None)
            } else {
                Err(error.into())
            };
        }
        for device_hash in device_hashes {
            let recorded = sqlx::query(
                "INSERT OR IGNORE INTO guild_ban_devices \
                    (ban_id, guild_id, device_hash, created_at) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(ban.id.to_canonical())
            .bind(ban.guild_id.as_str())
            .bind(device_hash.as_str())
            .bind(to_ms(now))
            .execute(&mut *transaction)
            .await;
            if let Err(error) = recorded {
                transaction.rollback().await?;
                return Err(error.into());
            }
        }
        transaction.commit().await?;
        Ok(Some(ban.clone()))
    }

    /// Bans of a guild, most recent first.
    pub async fn list_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<GuildBan>> {
        let rows = sqlx::query_as::<_, GuildBanRow>(
            "SELECT * FROM guild_bans WHERE guild_id = ? ORDER BY created_at DESC, id ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(GuildBanRow::into_entity).collect()
    }

    /// `true` when the Discord account is banned from the guild.
    pub async fn is_discord_user_banned(
        &self,
        guild_id: &GuildId,
        discord_user_id: &str,
    ) -> DbResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM guild_bans WHERE guild_id = ? AND discord_user_id = ?",
        )
        .bind(guild_id.as_str())
        .bind(discord_user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    /// `true` when this member — by its id or by the Discord account linked to
    /// it — is banned from the guild.
    ///
    /// A ban recorded from the overlay carries both identities; a ban imported
    /// from Discord may carry the account alone (no member row), so checking
    /// the id alone would miss it.
    pub async fn is_member_banned(
        &self,
        guild_id: &GuildId,
        member_id: GuildMemberId,
        discord_user_id: Option<&str>,
    ) -> DbResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM guild_bans \
             WHERE guild_id = ? \
               AND (member_id = ? OR (? IS NOT NULL AND discord_user_id = ?))",
        )
        .bind(guild_id.as_str())
        .bind(member_id.to_canonical())
        .bind(discord_user_id)
        .bind(discord_user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    /// The ban that already covers this member, by its id or its linked
    /// Discord account.
    ///
    /// Used when a ban is recorded twice (the second `create` conflicts on the
    /// account): the existing row is the one that must carry the member's
    /// devices.
    pub async fn find_for_member(
        &self,
        guild_id: &GuildId,
        member_id: GuildMemberId,
        discord_user_id: Option<&str>,
    ) -> DbResult<Option<GuildBan>> {
        let row = sqlx::query_as::<_, GuildBanRow>(
            "SELECT * FROM guild_bans \
             WHERE guild_id = ? \
               AND (member_id = ? OR (? IS NOT NULL AND discord_user_id = ?)) \
             ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(guild_id.as_str())
        .bind(member_id.to_canonical())
        .bind(discord_user_id)
        .bind(discord_user_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(GuildBanRow::into_entity).transpose()
    }

    /// Lifts a ban. Returns `false` when it does not exist in this guild.
    pub async fn delete(&self, guild_id: &GuildId, id: GuildBanId) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM guild_bans WHERE id = ? AND guild_id = ?")
            .bind(id.to_canonical())
            .bind(guild_id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Deletes every ban of a guild (purge). Returns the number of rows.
    pub async fn delete_by_guild(&self, guild_id: &GuildId) -> DbResult<u64> {
        let result = sqlx::query("DELETE FROM guild_bans WHERE guild_id = ?")
            .bind(guild_id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
