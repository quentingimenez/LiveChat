//! Repository `ingest_clients` (extensions d'ingestion appairées).

use chrono::DateTime;
use lc_domain::entities::IngestClient;
use lc_domain::ids::{GuildId, IngestClientId};
use sqlx::SqlitePool;

use crate::convert::{decode, from_ms, from_opt_ms, to_ms, to_opt_ms};
use crate::error::{conflict_on_unique, DbResult};

const TABLE: &str = "ingest_clients";

/// Ligne SQLite de `ingest_clients`.
#[derive(Debug, sqlx::FromRow)]
struct IngestClientRow {
    id: String,
    guild_id: String,
    label: String,
    token_hash: String,
    default_author_name: String,
    default_author_image: Option<String>,
    created_by_discord_user_id: String,
    last_seen_at: Option<i64>,
    created_at: i64,
    revoked_at: Option<i64>,
}

impl IngestClientRow {
    fn into_entity(self) -> DbResult<IngestClient> {
        Ok(IngestClient {
            id: decode(TABLE, "id", &self.id, IngestClientId::parse)?,
            guild_id: decode(TABLE, "guild_id", &self.guild_id, GuildId::parse)?,
            label: self.label,
            token_hash: self.token_hash,
            default_author_name: self.default_author_name,
            default_author_image: self.default_author_image,
            created_by_discord_user_id: self.created_by_discord_user_id,
            last_seen_at: from_opt_ms(TABLE, "last_seen_at", self.last_seen_at)?,
            created_at: from_ms(TABLE, "created_at", self.created_at)?,
            revoked_at: from_opt_ms(TABLE, "revoked_at", self.revoked_at)?,
        })
    }
}

/// Repository des clients d'ingestion.
#[derive(Clone, Debug)]
pub struct IngestClientsRepository {
    pool: SqlitePool,
}

impl IngestClientsRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Enregistre un client d'ingestion.
    ///
    /// `token_hash` est l'empreinte SHA-256 du token d'appairage. Violation
    /// `UNIQUE` → [`lc_domain::DomainError::Conflict`].
    pub async fn create(&self, client: &IngestClient) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO ingest_clients ( \
                id, guild_id, label, token_hash, default_author_name, \
                default_author_image, created_by_discord_user_id, last_seen_at, created_at, \
                revoked_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(client.id.to_canonical())
        .bind(client.guild_id.as_str())
        .bind(client.label.as_str())
        .bind(client.token_hash.as_str())
        .bind(client.default_author_name.as_str())
        .bind(client.default_author_image.as_deref())
        .bind(client.created_by_discord_user_id.as_str())
        .bind(to_opt_ms(client.last_seen_at))
        .bind(to_ms(client.created_at))
        .bind(to_opt_ms(client.revoked_at))
        .execute(&self.pool)
        .await
        .map_err(|error| {
            conflict_on_unique(error, "un client d'ingestion utilise déjà ce hash de token")
        })?;
        Ok(())
    }

    /// Récupère un client par identifiant (révoqué ou non).
    pub async fn get(&self, id: IngestClientId) -> DbResult<Option<IngestClient>> {
        let row = sqlx::query_as::<_, IngestClientRow>("SELECT * FROM ingest_clients WHERE id = ?")
            .bind(id.to_canonical())
            .fetch_optional(&self.pool)
            .await?;
        row.map(IngestClientRow::into_entity).transpose()
    }

    /// Authentifie un client par hash de token, uniquement s'il n'est pas
    /// révoqué.
    pub async fn get_by_token_hash(&self, token_hash: &str) -> DbResult<Option<IngestClient>> {
        let row = sqlx::query_as::<_, IngestClientRow>(
            "SELECT * FROM ingest_clients WHERE token_hash = ? AND revoked_at IS NULL",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(IngestClientRow::into_entity).transpose()
    }

    /// Liste tous les clients d'ingestion, du plus ancien au plus récent.
    pub async fn list(&self) -> DbResult<Vec<IngestClient>> {
        let rows = sqlx::query_as::<_, IngestClientRow>(
            "SELECT * FROM ingest_clients ORDER BY created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(IngestClientRow::into_entity).collect()
    }

    /// Liste les clients d'ingestion d'une guilde.
    pub async fn list_by_guild(&self, guild_id: &GuildId) -> DbResult<Vec<IngestClient>> {
        let rows = sqlx::query_as::<_, IngestClientRow>(
            "SELECT * FROM ingest_clients WHERE guild_id = ? ORDER BY created_at ASC, id ASC",
        )
        .bind(guild_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(IngestClientRow::into_entity).collect()
    }

    /// Met à jour le heartbeat applicatif du client.
    pub async fn touch_last_seen(
        &self,
        id: IngestClientId,
        last_seen_at: DateTime<chrono::Utc>,
    ) -> DbResult<bool> {
        let result = sqlx::query("UPDATE ingest_clients SET last_seen_at = ? WHERE id = ?")
            .bind(to_ms(last_seen_at))
            .bind(id.to_canonical())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Révoque le client.
    ///
    /// Retourne `false` s'il n'existe pas ou s'il était déjà révoqué.
    pub async fn revoke(
        &self,
        id: IngestClientId,
        revoked_at: DateTime<chrono::Utc>,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            "UPDATE ingest_clients SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(to_ms(revoked_at))
        .bind(id.to_canonical())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
