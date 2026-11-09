//! Repository `overlay_clients` (overlays appairés, authentifiés par hash).

use chrono::DateTime;
use lc_domain::entities::OverlayClient;
use lc_domain::enums::OverlaySessionMode;
use lc_domain::ids::{GuildId, OverlayClientId};
use sqlx::SqlitePool;

use crate::convert::{decode, from_ms, from_opt_ms, to_ms, to_opt_ms};
use crate::error::{conflict_on_unique, DbResult};

const TABLE: &str = "overlay_clients";

/// Ligne SQLite de `overlay_clients`.
#[derive(Debug, sqlx::FromRow)]
struct OverlayClientRow {
    id: String,
    guild_id: String,
    label: String,
    token_hash: String,
    session_mode: String,
    default_author_name: Option<String>,
    default_author_image: Option<String>,
    created_by_discord_user_id: Option<String>,
    last_seen_at: Option<i64>,
    created_at: i64,
    revoked_at: Option<i64>,
}

impl OverlayClientRow {
    fn into_entity(self) -> DbResult<OverlayClient> {
        Ok(OverlayClient {
            id: decode(TABLE, "id", &self.id, OverlayClientId::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            label: self.label,
            token_hash: self.token_hash,
            session_mode: decode(
                TABLE,
                "session_mode",
                &self.session_mode,
                OverlaySessionMode::from_db_str,
            )?,
            default_author_name: self.default_author_name,
            default_author_image: self.default_author_image,
            created_by_discord_user_id: self.created_by_discord_user_id,
            last_seen_at: from_opt_ms(TABLE, "last_seen_at", self.last_seen_at)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
            revoked_at: from_opt_ms(TABLE, "revoked_at", self.revoked_at)?,
        })
    }
}

/// Repository des overlays appairés.
#[derive(Clone, Debug)]
pub struct OverlayClientsRepository {
    pool: SqlitePool,
}

impl OverlayClientsRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Enregistre un overlay appairé.
    ///
    /// `token_hash` doit être l'empreinte SHA-256 du token d'appairage (jamais
    /// le token en clair). Violation `UNIQUE` → [`lc_domain::DomainError::Conflict`].
    pub async fn create(&self, client: &OverlayClient) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO overlay_clients ( \
                id, guild_id, label, token_hash, session_mode, default_author_name, \
                default_author_image, created_by_discord_user_id, last_seen_at, created_at, \
                revoked_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(client.id.to_canonical())
        .bind(client.guild_id.as_str())
        .bind(client.label.as_str())
        .bind(client.token_hash.as_str())
        .bind(client.session_mode.as_db_str())
        .bind(client.default_author_name.as_deref())
        .bind(client.default_author_image.as_deref())
        .bind(client.created_by_discord_user_id.as_deref())
        .bind(to_opt_ms(client.last_seen_at))
        .bind(to_ms(client.created_at))
        .bind(to_opt_ms(client.revoked_at))
        .execute(&self.pool)
        .await
        .map_err(|error| conflict_on_unique(error, "un overlay utilise déjà ce hash de token"))?;
        Ok(())
    }

    /// Récupère un overlay par identifiant (révoqué ou non).
    pub async fn get(&self, id: OverlayClientId) -> DbResult<Option<OverlayClient>> {
        let row =
            sqlx::query_as::<_, OverlayClientRow>("SELECT * FROM overlay_clients WHERE id = ?")
                .bind(id.to_canonical())
                .fetch_optional(&self.pool)
                .await?;
        row.map(OverlayClientRow::into_entity).transpose()
    }

    /// Authentifie un overlay par hash de token, uniquement s'il n'est pas
    /// révoqué.
    pub async fn get_by_token_hash(&self, token_hash: &str) -> DbResult<Option<OverlayClient>> {
        let row = sqlx::query_as::<_, OverlayClientRow>(
            "SELECT * FROM overlay_clients WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(OverlayClientRow::into_entity).transpose()
    }

    /// Liste les overlays d'une guilde, du plus ancien au plus récent.
    pub async fn list_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<OverlayClient>> {
        let rows = sqlx::query_as::<_, OverlayClientRow>(
            "SELECT * FROM overlay_clients WHERE guild_id = ? ORDER BY created_at ASC, id ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(OverlayClientRow::into_entity)
            .collect()
    }

    /// Met à jour le heartbeat applicatif de l'overlay.
    pub async fn touch_last_seen(
        &self,
        id: OverlayClientId,
        last_seen_at: DateTime<chrono::Utc>,
    ) -> DbResult<bool> {
        let result = sqlx::query("UPDATE overlay_clients SET last_seen_at = ? WHERE id = ?")
            .bind(to_ms(last_seen_at))
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Révoque l'overlay.
    ///
    /// Retourne `false` s'il n'existe pas ou s'il était déjà révoqué.
    pub async fn revoke(
        &self,
        id: OverlayClientId,
        revoked_at: DateTime<chrono::Utc>,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE overlay_clients SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(to_ms(revoked_at))
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Change le mode de session d'un overlay actif.
    pub async fn update_mode(
        &self,
        id: OverlayClientId,
        session_mode: OverlaySessionMode,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE overlay_clients SET session_mode = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(session_mode.as_db_str())
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
