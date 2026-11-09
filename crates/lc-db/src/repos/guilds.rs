//! Repository `guilds` (réglages et verrou `busy_until`).

use chrono::DateTime;
use lc_domain::entities::Guild;
use lc_domain::ids::GuildId;
use sqlx::SqlitePool;

use crate::convert::{decode, from_bool, from_opt_ms, to_bool, to_opt_ms};
use crate::error::DbResult;

const TABLE: &str = "guilds";

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

    /// Liste toutes les guildes connues, triées par identifiant.
    pub async fn list(&self) -> DbResult<Vec<Guild>> {
        let rows = sqlx::query_as::<_, GuildRow>("SELECT * FROM guilds ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(GuildRow::into_entity).collect()
    }
}
