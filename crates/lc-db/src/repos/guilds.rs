//! Repository `guilds` (réglages et verrou `busy_until`).

use chrono::DateTime;
use lc_domain::entities::{Guild, GuildAccess};
use lc_domain::ids::GuildId;
use sqlx::SqlitePool;

use crate::convert::{decode, from_bool, from_opt_ms, to_bool, to_ms, to_opt_ms};
use crate::error::{conflict_on_unique, DbResult};

const TABLE: &str = "guilds";

/// Colonnes lues pour un [`GuildAccess`].
const ACCESS_COLUMNS: &str =
    "id, name, standalone, discord_guild_id, guests_can_view_board, created_at";

/// Projection SQLite de `guilds` pour le rattachement (migration 0003).
#[derive(Debug, sqlx::FromRow)]
struct GuildAccessRow {
    id: String,
    name: Option<String>,
    standalone: i64,
    discord_guild_id: Option<String>,
    guests_can_view_board: i64,
    created_at: Option<i64>,
}

impl GuildAccessRow {
    fn into_entity(self) -> DbResult<GuildAccess> {
        Ok(GuildAccess {
            id: decode(TABLE, "id", &self.id, GuildId::parse)?,
            name: self.name,
            standalone: from_bool(self.standalone),
            discord_guild_id: self.discord_guild_id,
            guests_can_view_board: from_bool(self.guests_can_view_board),
            created_at: from_opt_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Ligne SQLite de `guilds`.
#[derive(Debug, sqlx::FromRow)]
struct GuildRow {
    id: String,
    name: Option<String>,
    busy_until: Option<i64>,
    default_media_time: Option<i64>,
    max_media_time: Option<i64>,
    display_media_full: i64,
}

impl GuildRow {
    fn into_entity(self) -> DbResult<Guild> {
        Ok(Guild {
            id: decode(TABLE, "id", &self.id, GuildId::parse)?,
            name: self.name,
            busy_until: from_opt_ms(TABLE, "busy_until", self.busy_until)?,
            default_media_time: self.default_media_time,
            max_media_time: self.max_media_time,
            display_media_full: from_bool(self.display_media_full),
        })
    }
}

/// Repository des guildes.
#[derive(Clone, Debug)]
pub struct GuildsRepository {
    pool: SqlitePool,
}

impl GuildsRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Crée ou remplace tous les réglages d'une guilde.
    ///
    /// Reproduit le `prisma.guild.upsert` du serveur legacy : les lignes
    /// `guilds` sont créées à la volée, jamais préalablement provisionnées.
    /// Pour modifier un sous-ensemble de colonnes sans écraser le reste,
    /// utiliser [`GuildsRepository::set_busy_until`] ou
    /// [`GuildsRepository::update_settings`].
    pub async fn upsert(&self, guild: &Guild) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guilds ( \
                id, name, busy_until, default_media_time, max_media_time, display_media_full \
             ) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                name = excluded.name, \
                busy_until = excluded.busy_until, \
                default_media_time = excluded.default_media_time, \
                max_media_time = excluded.max_media_time, \
                display_media_full = excluded.display_media_full",
        )
        .bind(guild.id.as_str())
        .bind(guild.name.as_deref())
        .bind(to_opt_ms(guild.busy_until))
        .bind(guild.default_media_time)
        .bind(guild.max_media_time)
        .bind(to_bool(guild.display_media_full))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Lit les réglages d'une guilde ; `None` si la guilde n'a jamais été vue.
    pub async fn get(&self, id: &GuildId) -> DbResult<Option<Guild>> {
        let row = sqlx::query_as::<_, GuildRow>("SELECT * FROM guilds WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(GuildRow::into_entity).transpose()
    }

    /// Pose (ou lève avec `None`) le verrou `busy_until`.
    ///
    /// Crée la ligne de guilde si elle n'existe pas encore. Le verrou est
    /// consulté par `PlaybackJobsRepository::claim_next` : aucun job de la
    /// guilde ne peut démarrer tant que `busy_until > now`.
    pub async fn set_busy_until(
        &self,
        id: &GuildId,
        busy_until: Option<DateTime<chrono::Utc>>,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guilds (id, busy_until) VALUES (?, ?) \
             ON CONFLICT(id) DO UPDATE SET busy_until = excluded.busy_until",
        )
        .bind(id.as_str())
        .bind(to_opt_ms(busy_until))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Met à jour les réglages de lecture (durées et affichage plein écran).
    ///
    /// Crée la ligne de guilde si elle n'existe pas encore ; `name` et
    /// `busy_until` ne sont pas touchés.
    pub async fn update_settings(
        &self,
        id: &GuildId,
        default_media_time: Option<i64>,
        max_media_time: Option<i64>,
        display_media_full: bool,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guilds (id, default_media_time, max_media_time, display_media_full) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                default_media_time = excluded.default_media_time, \
                max_media_time = excluded.max_media_time, \
                display_media_full = excluded.display_media_full",
        )
        .bind(id.as_str())
        .bind(default_media_time)
        .bind(max_media_time)
        .bind(to_bool(display_media_full))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Crée une guilde indépendante de Discord (`standalone = 1`).
    ///
    /// Violation de clé primaire → [`lc_domain::DomainError::Conflict`] :
    /// l'appelant régénère un identifiant ([`GuildId::generate`]).
    pub async fn create_standalone(
        &self,
        id: &GuildId,
        name: &str,
        created_at: DateTime<chrono::Utc>,
    ) -> DbResult<()> {
        sqlx::query("INSERT INTO guilds (id, name, standalone, created_at) VALUES (?, ?, 1, ?)")
            .bind(id.as_str())
            .bind(name)
            .bind(to_ms(created_at))
            .execute(&self.pool)
            .await
            .map_err(|error| conflict_on_unique(error, "cet identifiant de guilde existe déjà"))?;
        Ok(())
    }

    /// Lit le rattachement d'une guilde (indépendante, liaison Discord,
    /// visibilité du board pour les invités).
    pub async fn access(&self, id: &GuildId) -> DbResult<Option<GuildAccess>> {
        let row = sqlx::query_as::<_, GuildAccessRow>(&format!(
            "SELECT {ACCESS_COLUMNS} FROM guilds WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(GuildAccessRow::into_entity).transpose()
    }

    /// Liste le rattachement de toutes les guildes, triées par identifiant.
    pub async fn list_access(&self) -> DbResult<Vec<GuildAccess>> {
        let rows = sqlx::query_as::<_, GuildAccessRow>(&format!(
            "SELECT {ACCESS_COLUMNS} FROM guilds ORDER BY id ASC"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(GuildAccessRow::into_entity).collect()
    }

    /// Résout la guilde LiveChat d'un serveur Discord.
    ///
    /// Une guilde indépendante liée à ce serveur (`discord_guild_id`) prime ;
    /// sinon c'est la guilde legacy dont l'identifiant est le snowflake, qu'elle
    /// ait déjà une ligne ou non (les flux Discord la créent à la volée).
    pub async fn resolve_discord(&self, discord_guild_id: &GuildId) -> DbResult<GuildId> {
        let linked: Option<String> =
            sqlx::query_scalar("SELECT id FROM guilds WHERE discord_guild_id = ?")
                .bind(discord_guild_id.as_str())
                .fetch_optional(&self.pool)
                .await?;
        match linked {
            Some(id) => decode(TABLE, "id", &id, GuildId::parse),
            None => Ok(discord_guild_id.clone()),
        }
    }

    /// Lie (ou délie avec `None`) une guilde indépendante à un serveur Discord.
    ///
    /// Retourne `false` si la guilde n'existe pas ou n'est pas indépendante.
    /// Un serveur Discord déjà lié à une autre guilde →
    /// [`lc_domain::DomainError::Conflict`].
    pub async fn link_discord(
        &self,
        id: &GuildId,
        discord_guild_id: Option<&GuildId>,
    ) -> DbResult<bool> {
        let result =
            sqlx::query("UPDATE guilds SET discord_guild_id = ? WHERE id = ? AND standalone = 1")
                .bind(discord_guild_id.map(GuildId::as_str))
                .bind(id.as_str())
                .execute(&self.pool)
                .await
                .map_err(|error| {
                    conflict_on_unique(error, "ce serveur Discord est déjà lié à une autre guilde")
                })?;
        Ok(result.rows_affected() > 0)
    }

    /// Renomme une guilde. Retourne `false` si elle n'existe pas.
    pub async fn set_name(&self, id: &GuildId, name: &str) -> DbResult<bool> {
        let result = sqlx::query("UPDATE guilds SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Autorise ou non les invités à consulter le meme board.
    ///
    /// Crée la ligne de guilde si elle n'existe pas encore.
    pub async fn set_guests_can_view_board(&self, id: &GuildId, allowed: bool) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO guilds (id, guests_can_view_board) VALUES (?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                guests_can_view_board = excluded.guests_can_view_board",
        )
        .bind(id.as_str())
        .bind(to_bool(allowed))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Liste toutes les guildes connues, triées par identifiant.
    pub async fn list(&self) -> DbResult<Vec<Guild>> {
        let rows = sqlx::query_as::<_, GuildRow>("SELECT * FROM guilds ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(GuildRow::into_entity).collect()
    }
}
