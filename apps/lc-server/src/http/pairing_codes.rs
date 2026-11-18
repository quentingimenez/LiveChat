//! Requêtes et forme JSON partagées des codes d'appairage (`pairing_codes`).
//!
//! Le panneau admin (`/admin/api/pairing-codes`) et l'overlay
//! (`/overlay/admin/pairing-codes`) lisent, révoquent et purgent la même table :
//! la lecture triée, les deux écritures SQL et le payload commun vivent ici
//! pour que les deux surfaces restent alignées.

use lc_db::{Db, DbError};
use lc_domain::{GuildId, PairingCodeEntry};
use serde_json::{json, Value};
use time::UtcDateTime;

/// Codes du plus récent au plus ancien, bornés à une guilde si fournie.
pub(crate) async fn list(
    db: &Db,
    guild_id: Option<&GuildId>,
) -> Result<Vec<PairingCodeEntry>, DbError> {
    let mut codes = match guild_id {
        Some(guild_id) => db.pairing_codes().list_by_guild(guild_id).await?,
        None => db.pairing_codes().list().await?,
    };
    codes.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.code.as_str().cmp(right.code.as_str()))
    });
    Ok(codes)
}

/// Payload JSON commun d'un code (panneau admin et overlay).
///
/// L'overlay y ajoute `kind` et l'auteur résolu ; le panneau admin le renvoie
/// tel quel.
pub(crate) fn payload(entry: &PairingCodeEntry, now: UtcDateTime) -> Value {
    json!({
        "code": entry.code.as_str(),
        "guildId": entry.guild_id.as_str(),
        "createdByDiscordUserId": entry.created_by_discord_user_id,
        "authorName": entry.author_name,
        "expiresAt": opt_iso(entry.expires_at),
        "usedAt": opt_iso(entry.used_at),
        "createdAt": iso(entry.created_at),
        "isActive": entry.is_usable_at(now),
    })
}

/// Supprime les codes utilisés ou expirés, éventuellement d'une seule guilde.
///
/// `now_ms` est comparé à `expires_at`, comme le faisait le legacy.
pub(crate) async fn delete_expired(
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

/// Révoque un code encore actif : il est marqué utilisé et expiré.
///
/// Retourne le nombre de lignes touchées (0 si le code n'était plus actif).
pub(crate) async fn revoke(db: &Db, code: &str, now_ms: i64) -> Result<u64, DbError> {
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

/// Date ISO 8601 (millisecondes, UTC).
fn iso(date_time: UtcDateTime) -> String {
    lc_domain::time::to_rfc3339_millis(date_time)
}

/// Date ISO 8601 ou `null`.
fn opt_iso(date_time: Option<UtcDateTime>) -> Value {
    date_time.map_or(Value::Null, |date_time| json!(iso(date_time)))
}
