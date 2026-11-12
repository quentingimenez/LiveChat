//! Repository `discord_link_codes` (liaison d'une guilde indépendante à un
//! serveur Discord).

use chrono::DateTime;
use lc_domain::entities::DiscordLinkCode;
use lc_domain::ids::{GuildId, GuildMemberId, PairingCode as PairingCodeValue};
use sqlx::SqlitePool;

use crate::convert::{decode, decode_opt, from_bool, from_ms, from_opt_ms, to_ms};
use crate::error::{conflict_on_unique, DbResult};

const TABLE: &str = "discord_link_codes";

/// Ligne SQLite de `discord_link_codes`.
#[derive(Debug, sqlx::FromRow)]
struct DiscordLinkCodeRow {
    code: String,
    guild_id: String,
    created_by_member_id: Option<String>,
    expires_at: i64,
    used_at: Option<i64>,
    created_at: i64,
}

impl DiscordLinkCodeRow {
    fn into_entity(self) -> DbResult<DiscordLinkCode> {
        Ok(DiscordLinkCode {
            code: decode(TABLE, "code", &self.code, PairingCodeValue::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            created_by_member_id: decode_opt(
                TABLE,
                "created_by_member_id",
                self.created_by_member_id.as_deref(),
                GuildMemberId::parse,
            )?,
            expires_at: from_ms(TABLE, "expires_at", self.expires_at)?,
            used_at: from_opt_ms(TABLE, "used_at", self.used_at)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Guilde cible d'un code de liaison, lue dans la même transaction.
#[derive(Debug, sqlx::FromRow)]
struct GuildTargetRow {
    name: Option<String>,
    standalone: i64,
}

/// Résultat d'une tentative de consommation d'un code de liaison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkOutcome {
    /// Serveur Discord lié à la guilde cible.
    Linked {
        /// Guilde indépendante désormais cible du serveur.
        guild_id: GuildId,
        /// Nom lisible de la guilde (`None` s'il n'est pas renseigné).
        guild_name: Option<String>,
    },
    /// Code inconnu, déjà utilisé, expiré, ou visant une guilde inexistante ou
    /// non indépendante.
    InvalidCode,
    /// Le serveur Discord est déjà lié à une autre guilde indépendante ; le
    /// code n'a pas été consommé.
    AlreadyLinkedElsewhere,
}

/// Repository des codes de liaison Discord.
#[derive(Clone, Debug)]
pub struct DiscordLinkCodesRepository {
    pool: SqlitePool,
}

impl DiscordLinkCodesRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Remplace les codes **non utilisés** de la guilde par `link`.
    ///
    /// La suppression et l'insertion forment une seule transaction : un code
    /// déjà consommé n'est jamais supprimé, et un nouveau code n'est visible
    /// qu'une fois l'ancien retiré. Violation `UNIQUE` sur `code` →
    /// [`lc_domain::DomainError::Conflict`] : l'appelant régénère un code.
    pub async fn replace_for_guild(&self, link: &DiscordLinkCode) -> DbResult<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM discord_link_codes WHERE guild_id = ? AND used_at IS NULL")
            .bind(link.guild_id.as_str())
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO discord_link_codes ( \
                code, guild_id, created_by_member_id, expires_at, used_at, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(link.code.as_str())
        .bind(link.guild_id.as_str())
        .bind(link.created_by_member_id.map(GuildMemberId::to_canonical))
        .bind(to_ms(link.expires_at))
        .bind(link.used_at.map(to_ms))
        .bind(to_ms(link.created_at))
        .execute(&mut *transaction)
        .await
        .map_err(|error| conflict_on_unique(error, "ce code de liaison existe déjà"))?;
        transaction.commit().await?;
        Ok(())
    }

    /// Consomme un code et lie le serveur Discord à sa guilde cible.
    ///
    /// Toute la séquence (lecture du code, vérification de la cible, contrôle
    /// d'unicité du serveur, mise à jour et marquage du code) se fait dans une
    /// seule transaction : un code n'est marqué utilisé que si le lien est posé,
    /// et un serveur déjà lié ailleurs laisse le code réutilisable.
    pub async fn redeem(
        &self,
        code: &PairingCodeValue,
        discord_guild_id: &GuildId,
        now: DateTime<chrono::Utc>,
    ) -> DbResult<LinkOutcome> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<_, DiscordLinkCodeRow>(
            "SELECT * FROM discord_link_codes WHERE code = ?",
        )
        .bind(code.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(LinkOutcome::InvalidCode);
        };
        let link = row.into_entity()?;
        if link.used_at.is_some() || link.expires_at <= now {
            return Ok(LinkOutcome::InvalidCode);
        }

        let target =
            sqlx::query_as::<_, GuildTargetRow>("SELECT name, standalone FROM guilds WHERE id = ?")
                .bind(link.guild_id.as_str())
                .fetch_optional(&mut *transaction)
                .await?;
        let Some(target) = target else {
            return Ok(LinkOutcome::InvalidCode);
        };
        if !from_bool(target.standalone) {
            return Ok(LinkOutcome::InvalidCode);
        }

        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM guilds WHERE discord_guild_id = ?")
                .bind(discord_guild_id.as_str())
                .fetch_optional(&mut *transaction)
                .await?;
        if existing
            .as_deref()
            .is_some_and(|guild_id| guild_id != link.guild_id.as_str())
        {
            return Ok(LinkOutcome::AlreadyLinkedElsewhere);
        }

        sqlx::query("UPDATE guilds SET discord_guild_id = ? WHERE id = ?")
            .bind(discord_guild_id.as_str())
            .bind(link.guild_id.as_str())
            .execute(&mut *transaction)
            .await?;
        // Garde `used_at IS NULL` : deux `/lier` simultanés ne consomment
        // jamais le même code deux fois.
        let consumed = sqlx::query(
            "UPDATE discord_link_codes SET used_at = ? WHERE code = ? AND used_at IS NULL",
        )
        .bind(to_ms(now))
        .bind(code.as_str())
        .execute(&mut *transaction)
        .await?;
        if consumed.rows_affected() == 0 {
            return Ok(LinkOutcome::InvalidCode);
        }
        transaction.commit().await?;
        Ok(LinkOutcome::Linked {
            guild_id: link.guild_id,
            guild_name: target.name,
        })
    }

    /// Supprime tous les codes de liaison d'une guilde (purge).
    pub async fn delete_by_guild(&self, guild_id: &GuildId) -> DbResult<u64> {
        let result = sqlx::query("DELETE FROM discord_link_codes WHERE guild_id = ?")
            .bind(guild_id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
