//! Repository `media_assets` (cache média et cycle de vie du pipeline).

use chrono::{DateTime, Utc};
use lc_domain::entities::MediaAsset;
use lc_domain::enums::{MediaKind, MediaStatus};
use lc_domain::ids::MediaAssetId;
use sqlx::SqlitePool;

use crate::convert::{decode, from_bool, from_ms, to_bool, to_ms};
use crate::error::DbResult;

const TABLE: &str = "media_assets";

/// Ligne SQLite de `media_assets` (dates en ms, booléens en 0/1).
#[derive(Debug, sqlx::FromRow)]
struct MediaAssetRow {
    id: String,
    source_hash: String,
    source_url: String,
    kind: String,
    mime: String,
    duration_sec: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    is_vertical: i64,
    storage_path: String,
    size_bytes: i64,
    status: String,
    error: Option<String>,
    last_accessed_at: i64,
    expires_at: i64,
    created_at: i64,
}

impl MediaAssetRow {
    fn into_entity(self) -> DbResult<MediaAsset> {
        Ok(MediaAsset {
            id: decode(TABLE, "id", &self.id, MediaAssetId::parse)?,
            source_hash: self.source_hash,
            source_url: self.source_url,
            kind: decode(TABLE, "kind", &self.kind, MediaKind::from_db_str)?,
            mime: self.mime,
            duration_sec: self.duration_sec,
            width: self.width,
            height: self.height,
            is_vertical: from_bool(self.is_vertical),
            storage_path: self.storage_path,
            size_bytes: self.size_bytes,
            status: decode(TABLE, "status", &self.status, MediaStatus::from_db_str)?,
            error: self.error,
            last_accessed_at: from_ms(TABLE, "last_accessed_at", self.last_accessed_at)?,
            expires_at: from_ms(TABLE, "expires_at", self.expires_at)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
        })
    }
}

/// Repository des médias ingérés.
#[derive(Clone, Debug)]
pub struct MediaAssetsRepository {
    pool: SqlitePool,
}

impl MediaAssetsRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insère un média (échec `UNIQUE` si `source_hash` est déjà présent).
    pub async fn insert(&self, asset: &MediaAsset) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO media_assets ( \
                id, source_hash, source_url, kind, mime, duration_sec, width, height, \
                is_vertical, storage_path, size_bytes, status, error, last_accessed_at, \
                expires_at, created_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(asset.id.to_canonical())
        .bind(asset.source_hash.as_str())
        .bind(asset.source_url.as_str())
        .bind(asset.kind.as_db_str())
        .bind(asset.mime.as_str())
        .bind(asset.duration_sec)
        .bind(asset.width)
        .bind(asset.height)
        .bind(to_bool(asset.is_vertical))
        .bind(asset.storage_path.as_str())
        .bind(asset.size_bytes)
        .bind(asset.status.as_db_str())
        .bind(asset.error.as_deref())
        .bind(to_ms(asset.last_accessed_at))
        .bind(to_ms(asset.expires_at))
        .bind(to_ms(asset.created_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Récupère un média par identifiant.
    pub async fn get(&self, id: MediaAssetId) -> DbResult<Option<MediaAsset>> {
        let row = sqlx::query_as::<_, MediaAssetRow>("SELECT * FROM media_assets WHERE id = ?")
            .bind(id.to_canonical())
            .fetch_optional(&self.pool)
            .await?;
        row.map(MediaAssetRow::into_entity).transpose()
    }

    /// Récupère un média par empreinte de contenu (clé de dédoublonnage).
    pub async fn get_by_source_hash(&self, source_hash: &str) -> DbResult<Option<MediaAsset>> {
        let row =
            sqlx::query_as::<_, MediaAssetRow>("SELECT * FROM media_assets WHERE source_hash = ?")
                .bind(source_hash)
                .fetch_optional(&self.pool)
                .await?;
        row.map(MediaAssetRow::into_entity).transpose()
    }

    /// Met à jour l'état du pipeline et le détail d'erreur associé.
    ///
    /// Retourne `false` si aucun média ne porte cet identifiant.
    pub async fn update_status(
        &self,
        id: MediaAssetId,
        status: MediaStatus,
        error: Option<&str>,
    ) -> DbResult<bool> {
        let result = sqlx::query("UPDATE media_assets SET status = ?, error = ? WHERE id = ?")
            .bind(status.as_db_str())
            .bind(error)
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Met à jour le TTL glissant (`last_accessed_at` + `expires_at` calculé
    /// par l'appelant).
    pub async fn touch_last_accessed(
        &self,
        id: MediaAssetId,
        last_accessed_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE media_assets SET last_accessed_at = ?, expires_at = ? WHERE id = ?",
        )
        .bind(to_ms(last_accessed_at))
        .bind(to_ms(expires_at))
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Liste les médias expirés à l'instant `now`, les plus anciens d'abord.
    pub async fn list_expired(&self, now: DateTime<Utc>) -> DbResult<Vec<MediaAsset>> {
        let rows = sqlx::query_as::<_, MediaAssetRow>(
            "SELECT * FROM media_assets WHERE expires_at <= ? ORDER BY expires_at ASC, id ASC",
        )
        .bind(to_ms(now))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(MediaAssetRow::into_entity).collect()
    }

    /// Supprime les médias expirés **non référencés**.
    ///
    /// Un média expiré est conservé (et donc non retourné) s'il est encore
    /// épinglé sur le board média ou référencé par un job `PENDING`/`PLAYING` :
    /// la purge ne doit jamais casser un média visible ou une lecture en cours.
    /// Les autres sont supprimés ; les jobs terminés qui le référençaient
    /// passent `media_asset_id` à `NULL` (`ON DELETE SET NULL`).
    ///
    /// Retourne les lignes supprimées : l'appelant (worker de purge, service
    /// média) utilise `storage_path` pour supprimer les fichiers correspondants,
    /// que la couche SQL ne connaît pas.
    ///
    /// AUD-A5-007 : l'ancienne version `DELETE … WHERE expires_at <= ?` pouvait
    /// supprimer en cascade des `media_board_items` épinglés.
    pub async fn purge_expired(&self, now: DateTime<Utc>) -> DbResult<Vec<MediaAsset>> {
        let rows = sqlx::query_as::<_, MediaAssetRow>(
            "DELETE FROM media_assets \
             WHERE expires_at <= ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM media_board_items b \
                   WHERE b.media_asset_id = media_assets.id \
               ) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM playback_jobs j \
                   WHERE j.media_asset_id = media_assets.id \
                     AND j.status IN ('PENDING', 'PLAYING') \
               ) \
             RETURNING *",
        )
        .bind(to_ms(now))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(MediaAssetRow::into_entity).collect()
    }

    /// Liste les médias d'un état donné, les plus anciens d'abord.
    ///
    /// `limit <= 0` signifie « sans limite ».
    pub async fn list_by_status(
        &self,
        status: MediaStatus,
        limit: i64,
    ) -> DbResult<Vec<MediaAsset>> {
        let rows = if limit > 0 {
            sqlx::query_as::<_, MediaAssetRow>(
                "SELECT * FROM media_assets WHERE status = ? ORDER BY created_at ASC, id ASC LIMIT ?",
            )
            .bind(status.as_db_str())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MediaAssetRow>(
                "SELECT * FROM media_assets WHERE status = ? ORDER BY created_at ASC, id ASC",
            )
            .bind(status.as_db_str())
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(MediaAssetRow::into_entity).collect()
    }
}
