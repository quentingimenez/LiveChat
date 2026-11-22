//! Repository `guild_ban_devices` (appareils rattachés à un bannissement).
//!
//! Seules des empreintes hachées côté serveur sont stockées ; l'identifiant
//! d'appareil brut ne doit jamais atteindre la base ni les journaux.

use lc_domain::ids::{GuildBanId, GuildId};
use sqlx::SqlitePool;
use time::UtcDateTime;

use crate::convert::to_ms;
use crate::error::DbResult;

/// Repository des appareils bannis.
#[derive(Clone, Debug)]
pub struct GuildBanDevicesRepository {
    pool: SqlitePool,
}

impl GuildBanDevicesRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Records (idempotently) the device fingerprints attached to a ban.
    ///
    /// `device_hashes` are already server-side fingerprints. A fingerprint
    /// already attached to another ban of the guild is left untouched
    /// (`UNIQUE (guild_id, device_hash)`). Returns the number of new rows.
    pub async fn record(
        &self,
        ban_id: GuildBanId,
        guild_id: &GuildId,
        device_hashes: &[String],
        now: UtcDateTime,
    ) -> DbResult<u64> {
        let mut recorded = 0;
        for device_hash in device_hashes {
            let result = sqlx::query(
                "INSERT OR IGNORE INTO guild_ban_devices \
                    (ban_id, guild_id, device_hash, created_at) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(ban_id.to_canonical())
            .bind(guild_id.as_str())
            .bind(device_hash.as_str())
            .bind(to_ms(now))
            .execute(&self.pool)
            .await?;
            recorded += result.rows_affected();
        }
        Ok(recorded)
    }

    /// `true` when the device fingerprint is banned from the guild.
    pub async fn is_device_banned(&self, guild_id: &GuildId, device_hash: &str) -> DbResult<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM guild_ban_devices WHERE guild_id = ? AND device_hash = ?",
        )
        .bind(guild_id.as_str())
        .bind(device_hash)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    /// Number of fingerprints attached to a ban (diagnostics and tests).
    pub async fn count_by_ban(&self, ban_id: GuildBanId) -> DbResult<i64> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM guild_ban_devices WHERE ban_id = ?")
                .bind(ban_id.to_canonical())
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
    }
}
