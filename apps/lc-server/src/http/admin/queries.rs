//! Requêtes SQL transverses du panneau d'administration.
//!
//! Les repositories de `lc-db` couvrent les opérations métier ; le panneau a en
//! plus besoin d'agrégats toutes guildes confondues et de suppressions en masse
//! (purge d'une guilde, codes de pairing expirés, médias orphelins) qui n'ont
//! pas leur place dans les repositories. Ces requêtes passent par le pool
//! exposé par [`lc_db::Db`] et restent volontairement simples : pas de
//! transaction multi-tables ici (l'orchestration et les compteurs vivent dans
//! les handlers).

use lc_db::{Db, DbError};
use sqlx::FromRow;

/// Client overlay actif (non révoqué), ligne brute pour l'aperçu.
#[derive(Debug, Clone, FromRow)]
pub struct OverlayClientRow {
    pub id: String,
    pub guild_id: String,
    pub label: String,
    pub default_author_name: Option<String>,
    pub default_author_image: Option<String>,
    pub session_mode: String,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
}

/// Clients overlay actifs, triés comme le legacy (guilde puis libellé).
pub async fn overlay_clients(db: &Db) -> Result<Vec<OverlayClientRow>, DbError> {
    sqlx::query_as::<_, OverlayClientRow>(
        "SELECT id, guild_id, label, default_author_name, default_author_image, \
                session_mode, last_seen_at, created_at \
         FROM overlay_clients \
         WHERE revoked_at IS NULL \
         ORDER BY guild_id ASC, label ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(DbError::from)
}

/// Éléments du board média avec la taille de leur média : `(guild_id, size_bytes)`.
pub async fn media_board_sizes(db: &Db) -> Result<Vec<(String, i64)>, DbError> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT items.guild_id, assets.size_bytes \
         FROM media_board_items AS items \
         JOIN media_assets AS assets ON assets.id = items.media_asset_id",
    )
    .fetch_all(db.pool())
    .await
    .map_err(DbError::from)
}

/// Compteurs de playback par guilde et par statut : `(guild_id, status, count)`.
pub async fn playback_counts(db: &Db) -> Result<Vec<(String, String, i64)>, DbError> {
    sqlx::query_as::<_, (String, String, i64)>(
        "SELECT guild_id, status, COUNT(*) \
         FROM playback_jobs \
         GROUP BY guild_id, status",
    )
    .fetch_all(db.pool())
    .await
    .map_err(DbError::from)
}

/// Octets occupés par le cache : médias `READY` absents du board média.
///
/// Même définition que l'agrégat legacy (`status: READY`,
/// `mediaBoardItems: { none: {} }`).
pub async fn cache_used_bytes(db: &Db) -> Result<i64, DbError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(size_bytes), 0) \
         FROM media_assets AS assets \
         WHERE assets.status = 'READY' \
           AND NOT EXISTS ( \
               SELECT 1 FROM media_board_items AS items \
               WHERE items.media_asset_id = assets.id \
           )",
    )
    .fetch_one(db.pool())
    .await
    .map_err(DbError::from)
}

/// Médias référencés par une guilde : board média + jobs de playback.
pub async fn guild_media_asset_ids(db: &Db, guild_id: &str) -> Result<Vec<String>, DbError> {
    sqlx::query_scalar::<_, String>(
        "SELECT media_asset_id FROM media_board_items WHERE guild_id = ? \
         UNION \
         SELECT media_asset_id FROM playback_jobs \
         WHERE guild_id = ? AND media_asset_id IS NOT NULL",
    )
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(db.pool())
    .await
    .map_err(DbError::from)
}

/// Media asset ids referenced by the guild's media board only.
pub async fn guild_board_media_asset_ids(db: &Db, guild_id: &str) -> Result<Vec<String>, DbError> {
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT media_asset_id FROM media_board_items WHERE guild_id = ?1",
    )
    .bind(guild_id)
    .fetch_all(db.pool())
    .await
    .map_err(DbError::from)
}

/// Revoke the guild's active overlay clients and return their ids.
///
/// `now_ms` is an epoch-milliseconds timestamp, like the stored `revoked_at`.
pub async fn revoke_overlay_clients_for_guild(
    db: &Db,
    guild_id: &str,
    now_ms: i64,
) -> Result<Vec<String>, DbError> {
    sqlx::query_scalar::<_, String>(
        "UPDATE overlay_clients SET revoked_at = ?1 \
         WHERE guild_id = ?2 AND revoked_at IS NULL \
         RETURNING id",
    )
    .bind(now_ms)
    .bind(guild_id)
    .fetch_all(db.pool())
    .await
    .map_err(DbError::from)
}

/// Revoke the guild's active ingest clients; returns the number of rows changed.
pub async fn revoke_ingest_clients_for_guild(
    db: &Db,
    guild_id: &str,
    now_ms: i64,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE ingest_clients SET revoked_at = ?1 \
         WHERE guild_id = ?2 AND revoked_at IS NULL",
    )
    .bind(now_ms)
    .bind(guild_id)
    .execute(db.pool())
    .await?;

    Ok(result.rows_affected())
}

/// Mark the guild's active members as removed; returns the number of rows changed.
///
/// This deliberately bypasses the last-admin rule: a server admin can issue a
/// new admin code afterwards.
pub async fn remove_members_for_guild(
    db: &Db,
    guild_id: &str,
    now_ms: i64,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE guild_members SET removed_at = ?1 \
         WHERE guild_id = ?2 AND removed_at IS NULL",
    )
    .bind(now_ms)
    .bind(guild_id)
    .execute(db.pool())
    .await?;

    Ok(result.rows_affected())
}

/// Supprime les clients d'ingestion d'une guilde ; retourne le nombre supprimé.
pub async fn delete_ingest_clients_for_guild(db: &Db, guild_id: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM ingest_clients WHERE guild_id = ?1")
        .bind(guild_id)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Supprime les clients overlay d'une guilde ; retourne le nombre supprimé.
pub async fn delete_overlay_clients_for_guild(db: &Db, guild_id: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM overlay_clients WHERE guild_id = ?1")
        .bind(guild_id)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Supprime les codes de pairing d'une guilde ; retourne le nombre supprimé.
pub async fn delete_pairing_codes_for_guild(db: &Db, guild_id: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM pairing_codes WHERE guild_id = ?1")
        .bind(guild_id)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Supprime les jobs de playback d'une guilde ; retourne le nombre supprimé.
pub async fn delete_playback_jobs_for_guild(db: &Db, guild_id: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM playback_jobs WHERE guild_id = ?1")
        .bind(guild_id)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Supprime le board média d'une guilde ; retourne le nombre supprimé.
pub async fn delete_media_board_items_for_guild(db: &Db, guild_id: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM media_board_items WHERE guild_id = ?1")
        .bind(guild_id)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Supprime la ligne de réglages d'une guilde ; retourne 1 si elle existait.
pub async fn delete_guild_settings(db: &Db, guild_id: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM guilds WHERE id = ?1")
        .bind(guild_id)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Média orphelin candidat à la purge (chemin de stockage à effacer).
#[derive(Debug, Clone, FromRow)]
pub struct OrphanMediaRow {
    pub storage_path: String,
}

/// Retourne le média s'il n'est référencé ni par le board ni par un job.
pub async fn unreferenced_media(db: &Db, id: &str) -> Result<Option<OrphanMediaRow>, DbError> {
    sqlx::query_as::<_, OrphanMediaRow>(
        "SELECT storage_path FROM media_assets \
         WHERE id = ?1 \
           AND NOT EXISTS ( \
               SELECT 1 FROM media_board_items WHERE media_asset_id = ?1 \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 FROM playback_jobs WHERE media_asset_id = ?1 \
           )",
    )
    .bind(id)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::from)
}

/// Supprime un média par identifiant ; retourne le nombre supprimé.
pub async fn delete_media_asset(db: &Db, id: &str) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM media_assets WHERE id = ?1")
        .bind(id)
        .execute(db.pool())
        .await?;

    Ok(result.rows_affected())
}

/// Supprime les codes de pairing expirés ou utilisés.
///
/// `guild_id` restreint la suppression à une guilde (le legacy le fait avec un
/// `where` optionnel) ; `now_ms` est comparé à `expires_at`.
pub async fn delete_expired_pairing_codes(
    db: &Db,
    guild_id: Option<&str>,
    now_ms: i64,
) -> Result<u64, DbError> {
    let result = match guild_id {
        Some(guild_id) => {
            sqlx::query(
                "DELETE FROM pairing_codes \
                 WHERE guild_id = ?1 \
                   AND (used_at IS NOT NULL \
                        OR (expires_at IS NOT NULL AND expires_at <= ?2))",
            )
            .bind(guild_id)
            .bind(now_ms)
            .execute(db.pool())
            .await?
        }
        None => {
            sqlx::query(
                "DELETE FROM pairing_codes \
                 WHERE used_at IS NOT NULL \
                    OR (expires_at IS NOT NULL AND expires_at <= ?1)",
            )
            .bind(now_ms)
            .execute(db.pool())
            .await?
        }
    };

    Ok(result.rows_affected())
}

/// Révoque un code de pairing actif en le marquant utilisé et expiré.
///
/// Équivalent du `updateMany` legacy : condition `used_at IS NULL` et
/// `expires_at` nul ou futur ; le repository `PairingCodesRepository::revoke`
/// ne pose pas `expires_at`, d'où ce SQL brut. Retourne le nombre de lignes
/// touchées (0 si le code n'était plus actif).
pub async fn revoke_pairing_code(db: &Db, code: &str, now_ms: i64) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE pairing_codes \
         SET used_at = ?1, expires_at = ?1 \
         WHERE code = ?2 \
           AND used_at IS NULL \
           AND (expires_at IS NULL OR expires_at > ?1)",
    )
    .bind(now_ms)
    .bind(code)
    .execute(db.pool())
    .await?;

    Ok(result.rows_affected())
}

/// Libère les jobs `PLAYING` d'une guilde (`DONE` + `finished_at`), et retourne
/// le nombre de jobs libérés.
///
/// Le legacy filtre aussi `finishedAt: null` ; un job `PLAYING` avec une date
/// de fin renseignée reste donc intouché.
pub async fn release_playing_jobs(db: &Db, guild_id: &str, now_ms: i64) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE playback_jobs \
         SET status = 'DONE', finished_at = ?1 \
         WHERE guild_id = ?2 \
           AND status = 'PLAYING' \
           AND finished_at IS NULL",
    )
    .bind(now_ms)
    .bind(guild_id)
    .execute(db.pool())
    .await?;

    Ok(result.rows_affected())
}
