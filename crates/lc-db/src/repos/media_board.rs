//! Repository `media_board_items` (board média persistant par guilde).

use lc_domain::entities::MediaBoardItem;
use lc_domain::ids::{GuildId, GuildMemberId, MediaAssetId, MediaBoardItemId};
use sqlx::SqlitePool;

use crate::convert::{decode, decode_opt, from_ms, to_ms};
use crate::error::{conflict_on_unique, DbResult};

const TABLE: &str = "media_board_items";

/// Ligne SQLite de `media_board_items`.
#[derive(Debug, sqlx::FromRow)]
struct MediaBoardItemRow {
    id: String,
    guild_id: String,
    media_asset_id: String,
    title: Option<String>,
    message: Option<String>,
    created_by_discord_user_id: Option<String>,
    created_by_name: Option<String>,
    created_at: i64,
}

impl MediaBoardItemRow {
    fn into_entity(self) -> DbResult<MediaBoardItem> {
        Ok(MediaBoardItem {
            id: decode(TABLE, "id", &self.id, MediaBoardItemId::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            media_asset_id: decode(
                TABLE,
                "media_asset_id",
                &self.media_asset_id,
                MediaAssetId::parse,
            )?,
            title: self.title,
            message: self.message,
            created_by_discord_user_id: self.created_by_discord_user_id,
            created_by_name: self.created_by_name,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Repository du board média.
#[derive(Clone, Debug)]
pub struct MediaBoardRepository {
    pool: SqlitePool,
}

impl MediaBoardRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Ajoute un élément au board.
    ///
    /// Traduit la contrainte `UNIQUE (guild_id, media_asset_id)` en
    /// [`lc_domain::DomainError::Conflict`] : le même média ne peut être ajouté
    /// deux fois au board d'une guilde.
    pub async fn create(&self, item: &MediaBoardItem) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO media_board_items ( \
                id, guild_id, media_asset_id, title, message, \
                created_by_discord_user_id, created_by_name, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item.id.to_canonical())
        .bind(item.guild_id.as_str())
        .bind(item.media_asset_id.to_canonical())
        .bind(item.title.as_deref())
        .bind(item.message.as_deref())
        .bind(item.created_by_discord_user_id.as_deref())
        .bind(item.created_by_name.as_deref())
        .bind(to_ms(item.created_at))
        .execute(&self.pool)
        .await
        .map_err(|error| {
            conflict_on_unique(
                error,
                format!(
                    "le média {} est déjà présent sur le board de la guilde {}",
                    item.media_asset_id, item.guild_id
                ),
            )
        })?;
        Ok(())
    }

    /// Récupère un élément par identifiant.
    pub async fn get(&self, id: MediaBoardItemId) -> DbResult<Option<MediaBoardItem>> {
        let row =
            sqlx::query_as::<_, MediaBoardItemRow>("SELECT * FROM media_board_items WHERE id = ?")
                .bind(id.to_canonical())
                .fetch_optional(&self.pool)
                .await?;
        row.map(MediaBoardItemRow::into_entity).transpose()
    }

    /// Récupère l'élément d'une guilde pour un média donné.
    pub async fn get_by_guild_and_asset(
        &self,
        guild_id: &GuildId,
        media_asset_id: MediaAssetId,
    ) -> DbResult<Option<MediaBoardItem>> {
        let row = sqlx::query_as::<_, MediaBoardItemRow>(
            "SELECT * FROM media_board_items WHERE guild_id = ? AND media_asset_id = ?",
        )
        .bind(guild_id.as_str())
        .bind(media_asset_id.to_canonical())
        .fetch_optional(&self.pool)
        .await?;
        row.map(MediaBoardItemRow::into_entity).transpose()
    }

    /// Liste le board d'une guilde, du plus récent au plus ancien.
    pub async fn list_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<MediaBoardItem>> {
        let rows = sqlx::query_as::<_, MediaBoardItemRow>(
            "SELECT * FROM media_board_items WHERE guild_id = ? \
             ORDER BY created_at DESC, id ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(MediaBoardItemRow::into_entity)
            .collect()
    }

    /// Met à jour le titre et le message d'un élément.
    ///
    /// `title`/`message` à `None` **effacent** la colonne (pas de sémantique
    /// « ne pas toucher ») : l'appelant relit l'entité s'il veut conserver
    /// l'existant. Retourne `false` si l'élément n'existe pas.
    pub async fn update(
        &self,
        id: MediaBoardItemId,
        title: Option<&str>,
        message: Option<&str>,
    ) -> DbResult<bool> {
        let result =
            sqlx::query("UPDATE media_board_items SET title = ?, message = ? WHERE id = ?")
                .bind(title)
                .bind(message)
                .bind(id.to_canonical())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Enregistre le membre auteur d'un élément (droit « modifier ses propres
    /// médias »). Retourne `false` si l'élément n'existe pas.
    pub async fn set_creator_member(
        &self,
        id: MediaBoardItemId,
        member_id: GuildMemberId,
    ) -> DbResult<bool> {
        let result =
            sqlx::query("UPDATE media_board_items SET created_by_member_id = ? WHERE id = ?")
                .bind(member_id.to_canonical())
                .bind(id.to_canonical())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Membre auteur d'un élément ; `None` si l'élément est inconnu, antérieur
    /// à la migration 0003 ou ajouté depuis Discord.
    pub async fn creator_member(&self, id: MediaBoardItemId) -> DbResult<Option<GuildMemberId>> {
        let raw: Option<Option<String>> =
            sqlx::query_scalar("SELECT created_by_member_id FROM media_board_items WHERE id = ?")
                .bind(id.to_canonical())
                .fetch_optional(&self.pool)
                .await?;
        decode_opt(
            TABLE,
            "created_by_member_id",
            raw.flatten().as_deref(),
            GuildMemberId::parse,
        )
    }

    /// Supprime un élément du board.
    pub async fn delete(&self, id: MediaBoardItemId) -> DbResult<bool> {
        let result = sqlx::query("DELETE FROM media_board_items WHERE id = ?")
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
