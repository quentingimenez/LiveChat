//! Repository `media_board_items` (board média persistant par guilde).

use lc_domain::entities::MediaBoardItem;
use lc_domain::ids::{GuildId, GuildMemberId, MediaAssetId, MediaBoardItemId};
use sqlx::SqlitePool;

use crate::convert::{decode, decode_opt, from_bool, from_ms, to_bool, to_ms};
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
    artwork_asset_id: Option<String>,
    artwork_on_overlay: i64,
    display_duration_sec: Option<i64>,
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
            artwork_asset_id: decode_opt(
                TABLE,
                "artwork_asset_id",
                self.artwork_asset_id.as_deref(),
                MediaAssetId::parse,
            )?,
            artwork_on_overlay: from_bool(self.artwork_on_overlay),
            display_duration_sec: self.display_duration_sec,
        })
    }
}

/// Edit of a board item applied by [`MediaBoardRepository::update`].
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardItemEdit<'a> {
    /// New title (`None` clears it).
    pub title: Option<&'a str>,
    /// New message (`None` clears it).
    pub message: Option<&'a str>,
    /// New picture-on-overlay flag (`None` keeps it).
    pub artwork_on_overlay: Option<bool>,
    /// New display time: `None` keeps it, `Some(None)` clears it.
    pub display_duration_sec: Option<Option<i64>>,
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
    ///
    /// An audio item without a picture takes the embedded cover of its asset
    /// (an MP3's album art), when there is one. A video's cover is its
    /// thumbnail, shown from the asset itself, never a picture of the item.
    pub async fn create(&self, item: &MediaBoardItem) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO media_board_items ( \
                id, guild_id, media_asset_id, title, message, \
                created_by_discord_user_id, created_by_name, created_at, \
                artwork_asset_id, artwork_on_overlay, display_duration_sec \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, \
                COALESCE(?, (SELECT cover_asset_id FROM media_assets WHERE id = ? AND kind = 'AUDIO')), ?, ?)",
        )
        .bind(item.id.to_canonical())
        .bind(item.guild_id.as_str())
        .bind(item.media_asset_id.to_canonical())
        .bind(item.title.as_deref())
        .bind(item.message.as_deref())
        .bind(item.created_by_discord_user_id.as_deref())
        .bind(item.created_by_name.as_deref())
        .bind(to_ms(item.created_at))
        .bind(item.artwork_asset_id.map(MediaAssetId::to_canonical))
        .bind(item.media_asset_id.to_canonical())
        .bind(to_bool(item.artwork_on_overlay))
        .bind(item.display_duration_sec)
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

    /// Applies a board item edit in one statement, so either every field is
    /// written or none is. `title` and `message` are always replaced (`None`
    /// clears them); `artwork_on_overlay` and `display_duration_sec` are kept
    /// when `None`. Returns `false` if the item does not exist.
    pub async fn update(&self, id: MediaBoardItemId, edit: &BoardItemEdit<'_>) -> DbResult<bool> {
        let (set_duration, duration) = match edit.display_duration_sec {
            Some(seconds) => (true, seconds),
            None => (false, None),
        };
        let result = sqlx::query(
            "UPDATE media_board_items SET title = ?, message = ?,              artwork_on_overlay = COALESCE(?, artwork_on_overlay),              display_duration_sec = CASE WHEN ? THEN ? ELSE display_duration_sec END              WHERE id = ?",
        )
        .bind(edit.title)
        .bind(edit.message)
        .bind(edit.artwork_on_overlay.map(to_bool))
        .bind(set_duration)
        .bind(duration)
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Sets (or clears with `None`) the display time of an item, in seconds.
    /// Returns `false` if the item does not exist.
    pub async fn set_display_duration(
        &self,
        id: MediaBoardItemId,
        seconds: Option<i64>,
    ) -> DbResult<bool> {
        let result =
            sqlx::query("UPDATE media_board_items SET display_duration_sec = ? WHERE id = ?")
                .bind(seconds)
                .bind(id.to_canonical())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Replaces the media of an item: its id, title, message, author, date,
    /// favorites and picture are kept, only `media_asset_id` changes.
    ///
    /// In the same transaction, the previous asset gets `released_expiry` when
    /// no board item shows it any more (as a media or as a picture), like a
    /// deleted item's asset. Returns the previous asset, or `None` when the
    /// guild has no such item. The `UNIQUE (guild_id, media_asset_id)`
    /// constraint becomes [`lc_domain::DomainError::Conflict`].
    pub async fn replace_media(
        &self,
        id: MediaBoardItemId,
        guild_id: &GuildId,
        media_asset_id: MediaAssetId,
        now: chrono::DateTime<chrono::Utc>,
        released_expiry: chrono::DateTime<chrono::Utc>,
    ) -> DbResult<Option<MediaAssetId>> {
        let mut transaction = self.pool.begin().await?;
        let previous_raw: Option<String> = sqlx::query_scalar(
            "SELECT media_asset_id FROM media_board_items WHERE id = ? AND guild_id = ?",
        )
        .bind(id.to_canonical())
        .bind(guild_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(previous_raw) = previous_raw else {
            return Ok(None);
        };
        let previous = decode(TABLE, "media_asset_id", &previous_raw, MediaAssetId::parse)?;
        if previous == media_asset_id {
            return Ok(Some(previous));
        }
        sqlx::query("UPDATE media_board_items SET media_asset_id = ? WHERE id = ?")
            .bind(media_asset_id.to_canonical())
            .bind(id.to_canonical())
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                conflict_on_unique(
                    error,
                    format!(
                        "le média {media_asset_id} est déjà présent sur le board de la guilde {guild_id}"
                    ),
                )
            })?;
        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM media_board_items \
              WHERE media_asset_id = ? OR artwork_asset_id = ?",
        )
        .bind(previous.to_canonical())
        .bind(previous.to_canonical())
        .fetch_one(&mut *transaction)
        .await?;
        if remaining == 0 {
            sqlx::query(
                "UPDATE media_assets SET last_accessed_at = ?, expires_at = ? WHERE id = ?",
            )
            .bind(to_ms(now))
            .bind(to_ms(released_expiry))
            .bind(previous.to_canonical())
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(Some(previous))
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

    /// Sets (or clears) the picture of an item. Returns `false` if the item
    /// does not exist.
    pub async fn set_artwork(
        &self,
        id: MediaBoardItemId,
        artwork_asset_id: Option<MediaAssetId>,
    ) -> DbResult<bool> {
        let result = sqlx::query("UPDATE media_board_items SET artwork_asset_id = ? WHERE id = ?")
            .bind(artwork_asset_id.map(MediaAssetId::to_canonical))
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Chooses whether overlays show the item's picture during playback.
    pub async fn set_artwork_on_overlay(
        &self,
        id: MediaBoardItemId,
        on_overlay: bool,
    ) -> DbResult<bool> {
        let result =
            sqlx::query("UPDATE media_board_items SET artwork_on_overlay = ? WHERE id = ?")
                .bind(to_bool(on_overlay))
                .bind(id.to_canonical())
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// `true` when the guild's board shows this asset, as an item's media, as
    /// an item's picture or as the cover of an item's media (a video's
    /// thumbnail).
    pub async fn guild_references_asset(
        &self,
        guild_id: &GuildId,
        asset_id: MediaAssetId,
    ) -> DbResult<bool> {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM media_board_items \
              WHERE guild_id = ? AND (media_asset_id = ? OR artwork_asset_id = ? \
                    OR media_asset_id IN (SELECT id FROM media_assets WHERE cover_asset_id = ?)) \
              LIMIT 1",
        )
        .bind(guild_id.as_str())
        .bind(asset_id.to_canonical())
        .bind(asset_id.to_canonical())
        .bind(asset_id.to_canonical())
        .fetch_optional(&self.pool)
        .await?;
        Ok(found.is_some())
    }

    /// Picture to show on overlays while this audio plays: the one of the
    /// guild's oldest board item holding it with « show on overlays » ticked.
    pub async fn overlay_artwork(
        &self,
        guild_id: &GuildId,
        media_asset_id: MediaAssetId,
    ) -> DbResult<Option<MediaAssetId>> {
        let raw: Option<String> = sqlx::query_scalar(
            "SELECT artwork_asset_id FROM media_board_items               WHERE guild_id = ? AND media_asset_id = ?                 AND artwork_on_overlay = 1 AND artwork_asset_id IS NOT NULL               ORDER BY created_at ASC, id ASC LIMIT 1",
        )
        .bind(guild_id.as_str())
        .bind(media_asset_id.to_canonical())
        .fetch_optional(&self.pool)
        .await?;
        raw.map(|value| decode(TABLE, "artwork_asset_id", &value, MediaAssetId::parse))
            .transpose()
    }

    /// Items of a guild that a member marked as favorite.
    pub async fn favorite_item_ids(
        &self,
        member_id: GuildMemberId,
    ) -> DbResult<Vec<MediaBoardItemId>> {
        let raw: Vec<String> = sqlx::query_scalar(
            "SELECT item_id FROM media_board_favorites WHERE member_id = ? \
              ORDER BY created_at ASC, item_id ASC",
        )
        .bind(member_id.to_canonical())
        .fetch_all(&self.pool)
        .await?;
        raw.iter()
            .map(|value| {
                decode(
                    "media_board_favorites",
                    "item_id",
                    value,
                    MediaBoardItemId::parse,
                )
            })
            .collect()
    }

    /// Adds or removes a favorite. Idempotent.
    pub async fn set_favorite(
        &self,
        member_id: GuildMemberId,
        item_id: MediaBoardItemId,
        favorite: bool,
        now: chrono::DateTime<chrono::Utc>,
    ) -> DbResult<()> {
        if favorite {
            sqlx::query(
                "INSERT INTO media_board_favorites (member_id, item_id, created_at) \
                 VALUES (?, ?, ?) ON CONFLICT (member_id, item_id) DO NOTHING",
            )
            .bind(member_id.to_canonical())
            .bind(item_id.to_canonical())
            .bind(to_ms(now))
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query("DELETE FROM media_board_favorites WHERE member_id = ? AND item_id = ?")
                .bind(member_id.to_canonical())
                .bind(item_id.to_canonical())
                .execute(&self.pool)
                .await?;
        }
        Ok(())
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
