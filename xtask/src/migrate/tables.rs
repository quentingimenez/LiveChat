//! Description des 7 tables migrées : correspondance des noms legacy (Prisma,
//! camelCase) vers le schéma 3.0 (snake_case), extraction typée d'une ligne
//! legacy, extraction d'une ligne cible relue (pour la vérification) et
//! insertion.
//!
//! Chaque table sait :
//! - son nom legacy (`LEGACY_TABLE`) et cible (`TABLE`) ;
//! - ses colonnes de dates legacy (`DATE_COLUMNS_LEGACY`), utilisées par
//!   [`super::legacy`] pour construire les alias `typeof(...)` ;
//! - extraire une ligne legacy ([`from_legacy`]) et une ligne cible relue
//!   ([`from_target`]) dans la même structure — l'égalité des structures sert
//!   de vérification champ à champ ;
//! - s'insérer dans une transaction ([`insert`]).
//!
//! Les champs `Boolean` Prisma sont conservés en `INTEGER` 0/1 (contraintes
//! `CHECK` du schéma 3.0) et les champs `DateTime` en millisecondes epoch UTC.
//! Les enums legacy sont validés/normalisés contre les `CHECK` du schéma 3.0.

use sqlx::sqlite::SqliteRow;
use sqlx::Sqlite;

use super::read::{read_pk, read_rowid, ReadOut, RowCtx};
use super::Error;

/// Valeurs autorisées pour `media_assets.kind` (legacy `MediaAssetKind`).
const MEDIA_KINDS: &[&str] = &["IMAGE", "AUDIO", "VIDEO"];
/// Valeurs autorisées pour `media_assets.status` (legacy `MediaAssetStatus`).
const MEDIA_STATUSES: &[&str] = &["PROCESSING", "READY", "FAILED"];
/// Valeurs autorisées pour `playback_jobs.status` (legacy `PlaybackJobStatus`).
const JOB_STATUSES: &[&str] = &["PENDING", "PLAYING", "DONE", "FAILED"];
/// Valeurs autorisées pour les modes de session (`sessionMode`/`mode`).
const SESSION_MODES: &[&str] = &["NORMAL", "INVITE_READ_ONLY"];

/// Média ingéré (vidéo, image ou audio) et son cache de fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAsset {
    pub id: String,
    pub source_hash: String,
    pub source_url: String,
    pub kind: String,
    pub mime: String,
    pub duration_sec: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub is_vertical: i64,
    pub storage_path: String,
    pub size_bytes: i64,
    pub status: String,
    pub error: Option<String>,
    pub last_accessed_at: i64,
    pub expires_at: i64,
    pub created_at: i64,
}

impl MediaAsset {
    /// Table legacy Prisma.
    pub const LEGACY_TABLE: &'static str = "MediaAsset";
    /// Table cible 3.0.
    pub const TABLE: &'static str = "media_assets";
    /// Clé primaire applicative.
    pub const PK: &'static str = "id";
    /// Colonnes `DateTime` legacy (à interroger via `typeof(...)`).
    pub const DATE_COLUMNS_LEGACY: &'static [&'static str] =
        &["lastAccessedAt", "expiresAt", "createdAt"];
    /// Requête d'insertion dans le schéma 3.0.
    pub const INSERT_SQL: &'static str = "INSERT INTO media_assets (\
        id, source_hash, source_url, kind, mime, duration_sec, width, height, \
        is_vertical, storage_path, size_bytes, status, error, last_accessed_at, \
        expires_at, created_at\
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

    /// Extrait une ligne legacy.
    pub fn from_legacy(row: &SqliteRow, out: &mut ReadOut) -> Result<Self, Error> {
        let rowid = read_rowid(row, Self::LEGACY_TABLE)?;
        let id = read_pk(row, Self::LEGACY_TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::LEGACY_TABLE, rowid, id.clone());
        Ok(Self {
            id,
            source_hash: ctx.text_req(row, "sourceHash")?,
            source_url: ctx.text_req(row, "sourceUrl")?,
            kind: ctx.enum_req(row, "kind", MEDIA_KINDS)?,
            mime: ctx.text_req(row, "mime")?,
            duration_sec: ctx.int_opt(row, "durationSec")?,
            width: ctx.int_opt(row, "width")?,
            height: ctx.int_opt(row, "height")?,
            is_vertical: ctx.bool_req(row, "isVertical")?,
            storage_path: ctx.text_req(row, "storagePath")?,
            size_bytes: ctx.int_req(row, "sizeBytes")?,
            status: ctx.enum_req(row, "status", MEDIA_STATUSES)?,
            error: ctx.text_opt(row, "error")?,
            last_accessed_at: ctx.date_req(row, "lastAccessedAt", out)?,
            expires_at: ctx.date_req(row, "expiresAt", out)?,
            created_at: ctx.date_req(row, "createdAt", out)?,
        })
    }

    /// Extrait une ligne cible relue (schéma 3.0), pour vérification.
    pub fn from_target(row: &SqliteRow) -> Result<Self, Error> {
        let id = read_pk(row, Self::TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::TABLE, 0, id.clone());
        Ok(Self {
            id,
            source_hash: ctx.text_req(row, "source_hash")?,
            source_url: ctx.text_req(row, "source_url")?,
            kind: ctx.text_req(row, "kind")?,
            mime: ctx.text_req(row, "mime")?,
            duration_sec: ctx.int_opt(row, "duration_sec")?,
            width: ctx.int_opt(row, "width")?,
            height: ctx.int_opt(row, "height")?,
            is_vertical: ctx.int_req(row, "is_vertical")?,
            storage_path: ctx.text_req(row, "storage_path")?,
            size_bytes: ctx.int_req(row, "size_bytes")?,
            status: ctx.text_req(row, "status")?,
            error: ctx.text_opt(row, "error")?,
            last_accessed_at: ctx.int_req(row, "last_accessed_at")?,
            expires_at: ctx.int_req(row, "expires_at")?,
            created_at: ctx.int_req(row, "created_at")?,
        })
    }

    /// Insère la ligne dans le fichier cible (transaction pilotée par l'appelant).
    pub async fn insert(&self, tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), Error> {
        sqlx::query(Self::INSERT_SQL)
            .bind(&self.id)
            .bind(&self.source_hash)
            .bind(&self.source_url)
            .bind(&self.kind)
            .bind(&self.mime)
            .bind(self.duration_sec)
            .bind(self.width)
            .bind(self.height)
            .bind(self.is_vertical)
            .bind(&self.storage_path)
            .bind(self.size_bytes)
            .bind(&self.status)
            .bind(self.error.as_deref())
            .bind(self.last_accessed_at)
            .bind(self.expires_at)
            .bind(self.created_at)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::TargetWrite {
                table: Self::TABLE,
                source,
            })?;
        Ok(())
    }

    /// Clé primaire, pour la vérification par échantillon.
    pub fn primary_key(&self) -> &str {
        &self.id
    }
}

/// Élément du board média d'une guilde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBoardItem {
    pub id: String,
    pub guild_id: String,
    pub media_asset_id: String,
    pub title: Option<String>,
    pub message: Option<String>,
    pub created_by_discord_user_id: Option<String>,
    pub created_by_name: Option<String>,
    pub created_at: i64,
}

impl MediaBoardItem {
    /// Table legacy Prisma.
    pub const LEGACY_TABLE: &'static str = "MemeBoardItem";
    /// Table cible 3.0, sous son nom du schéma `0001_init` que construit le
    /// migrateur ; la migration `0005_media_board` la renomme en
    /// `media_board_items` au premier démarrage du serveur.
    pub const TABLE: &'static str = "meme_board_items";
    /// Clé primaire applicative.
    pub const PK: &'static str = "id";
    /// Colonnes `DateTime` legacy.
    pub const DATE_COLUMNS_LEGACY: &'static [&'static str] = &["createdAt"];
    /// Requête d'insertion dans le schéma 3.0.
    pub const INSERT_SQL: &'static str = "INSERT INTO meme_board_items (\
        id, guild_id, media_asset_id, title, message, \
        created_by_discord_user_id, created_by_name, created_at\
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)";

    /// Extrait une ligne legacy.
    pub fn from_legacy(row: &SqliteRow, out: &mut ReadOut) -> Result<Self, Error> {
        let rowid = read_rowid(row, Self::LEGACY_TABLE)?;
        let id = read_pk(row, Self::LEGACY_TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::LEGACY_TABLE, rowid, id.clone());
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guildId")?,
            media_asset_id: ctx.text_req(row, "mediaAssetId")?,
            title: ctx.text_opt(row, "title")?,
            message: ctx.text_opt(row, "message")?,
            created_by_discord_user_id: ctx.text_opt(row, "createdByDiscordUserId")?,
            created_by_name: ctx.text_opt(row, "createdByName")?,
            created_at: ctx.date_req(row, "createdAt", out)?,
        })
    }

    /// Extrait une ligne cible relue, pour vérification.
    pub fn from_target(row: &SqliteRow) -> Result<Self, Error> {
        let id = read_pk(row, Self::TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::TABLE, 0, id.clone());
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guild_id")?,
            media_asset_id: ctx.text_req(row, "media_asset_id")?,
            title: ctx.text_opt(row, "title")?,
            message: ctx.text_opt(row, "message")?,
            created_by_discord_user_id: ctx.text_opt(row, "created_by_discord_user_id")?,
            created_by_name: ctx.text_opt(row, "created_by_name")?,
            created_at: ctx.int_req(row, "created_at")?,
        })
    }

    /// Insère la ligne dans le fichier cible.
    pub async fn insert(&self, tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), Error> {
        sqlx::query(Self::INSERT_SQL)
            .bind(&self.id)
            .bind(&self.guild_id)
            .bind(&self.media_asset_id)
            .bind(self.title.as_deref())
            .bind(self.message.as_deref())
            .bind(self.created_by_discord_user_id.as_deref())
            .bind(self.created_by_name.as_deref())
            .bind(self.created_at)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::TargetWrite {
                table: Self::TABLE,
                source,
            })?;
        Ok(())
    }

    /// Clé primaire, pour la vérification par échantillon.
    pub fn primary_key(&self) -> &str {
        &self.id
    }
}

/// Job de la file de lecture (scheduler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackJob {
    pub id: String,
    pub guild_id: String,
    pub media_asset_id: Option<String>,
    pub text: Option<String>,
    pub show_text: i64,
    pub author_name: Option<String>,
    pub author_image: Option<String>,
    pub duration_sec: i64,
    pub priority: i64,
    pub status: String,
    pub resumes_after_job_id: Option<String>,
    pub resume_offset_sec: i64,
    pub remaining_ms_snapshot: Option<i64>,
    pub last_playback_state_at: Option<i64>,
    pub submission_date: i64,
    pub execution_date: i64,
    pub scheduled_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

impl PlaybackJob {
    /// Table legacy Prisma.
    pub const LEGACY_TABLE: &'static str = "PlaybackJob";
    /// Table cible 3.0.
    pub const TABLE: &'static str = "playback_jobs";
    /// Clé primaire applicative.
    pub const PK: &'static str = "id";
    /// Colonnes `DateTime` legacy.
    pub const DATE_COLUMNS_LEGACY: &'static [&'static str] = &[
        "lastPlaybackStateAt",
        "submissionDate",
        "executionDate",
        "scheduledAt",
        "startedAt",
        "finishedAt",
    ];
    /// Requête d'insertion dans le schéma 3.0.
    pub const INSERT_SQL: &'static str = "INSERT INTO playback_jobs (\
        id, guild_id, media_asset_id, text, show_text, author_name, author_image, \
        duration_sec, priority, status, resumes_after_job_id, resume_offset_sec, \
        remaining_ms_snapshot, last_playback_state_at, submission_date, \
        execution_date, scheduled_at, started_at, finished_at\
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

    /// Extrait une ligne legacy.
    pub fn from_legacy(row: &SqliteRow, out: &mut ReadOut) -> Result<Self, Error> {
        let rowid = read_rowid(row, Self::LEGACY_TABLE)?;
        let id = read_pk(row, Self::LEGACY_TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::LEGACY_TABLE, rowid, id.clone());
        let resume_offset_sec = ctx.int_opt(row, "resumeOffsetSec")?.unwrap_or_else(|| {
            out.warn("playback_jobs.resume_offset_sec : NULL legacy remplacé par 0");
            0
        });
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guildId")?,
            media_asset_id: ctx.text_opt(row, "mediaAssetId")?,
            text: ctx.text_opt(row, "text")?,
            show_text: ctx.bool_req(row, "showText")?,
            author_name: ctx.text_opt(row, "authorName")?,
            author_image: ctx.text_opt(row, "authorImage")?,
            duration_sec: ctx.int_req(row, "durationSec")?,
            priority: ctx.int_req(row, "priority")?,
            status: ctx.enum_req(row, "status", JOB_STATUSES)?,
            resumes_after_job_id: ctx.text_opt(row, "resumesAfterJobId")?,
            resume_offset_sec,
            remaining_ms_snapshot: ctx.int_opt(row, "remainingMsSnapshot")?,
            last_playback_state_at: ctx.date_opt(row, "lastPlaybackStateAt", out)?,
            submission_date: ctx.date_req(row, "submissionDate", out)?,
            execution_date: ctx.date_req(row, "executionDate", out)?,
            scheduled_at: ctx.date_req(row, "scheduledAt", out)?,
            started_at: ctx.date_opt(row, "startedAt", out)?,
            finished_at: ctx.date_opt(row, "finishedAt", out)?,
        })
    }

    /// Extrait une ligne cible relue, pour vérification.
    pub fn from_target(row: &SqliteRow) -> Result<Self, Error> {
        let id = read_pk(row, Self::TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::TABLE, 0, id.clone());
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guild_id")?,
            media_asset_id: ctx.text_opt(row, "media_asset_id")?,
            text: ctx.text_opt(row, "text")?,
            show_text: ctx.int_req(row, "show_text")?,
            author_name: ctx.text_opt(row, "author_name")?,
            author_image: ctx.text_opt(row, "author_image")?,
            duration_sec: ctx.int_req(row, "duration_sec")?,
            priority: ctx.int_req(row, "priority")?,
            status: ctx.text_req(row, "status")?,
            resumes_after_job_id: ctx.text_opt(row, "resumes_after_job_id")?,
            resume_offset_sec: ctx.int_req(row, "resume_offset_sec")?,
            remaining_ms_snapshot: ctx.int_opt(row, "remaining_ms_snapshot")?,
            last_playback_state_at: ctx.int_opt(row, "last_playback_state_at")?,
            submission_date: ctx.int_req(row, "submission_date")?,
            execution_date: ctx.int_req(row, "execution_date")?,
            scheduled_at: ctx.int_req(row, "scheduled_at")?,
            started_at: ctx.int_opt(row, "started_at")?,
            finished_at: ctx.int_opt(row, "finished_at")?,
        })
    }

    /// Insère la ligne dans le fichier cible.
    pub async fn insert(&self, tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), Error> {
        sqlx::query(Self::INSERT_SQL)
            .bind(&self.id)
            .bind(&self.guild_id)
            .bind(self.media_asset_id.as_deref())
            .bind(self.text.as_deref())
            .bind(self.show_text)
            .bind(self.author_name.as_deref())
            .bind(self.author_image.as_deref())
            .bind(self.duration_sec)
            .bind(self.priority)
            .bind(&self.status)
            .bind(self.resumes_after_job_id.as_deref())
            .bind(self.resume_offset_sec)
            .bind(self.remaining_ms_snapshot)
            .bind(self.last_playback_state_at)
            .bind(self.submission_date)
            .bind(self.execution_date)
            .bind(self.scheduled_at)
            .bind(self.started_at)
            .bind(self.finished_at)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::TargetWrite {
                table: Self::TABLE,
                source,
            })?;
        Ok(())
    }

    /// Clé primaire, pour la vérification par échantillon.
    pub fn primary_key(&self) -> &str {
        &self.id
    }
}

/// Réglages par guilde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guild {
    pub id: String,
    pub name: Option<String>,
    pub busy_until: Option<i64>,
    pub default_media_time: Option<i64>,
    pub max_media_time: Option<i64>,
    pub display_media_full: i64,
}

impl Guild {
    /// Table legacy Prisma.
    pub const LEGACY_TABLE: &'static str = "Guild";
    /// Table cible 3.0.
    pub const TABLE: &'static str = "guilds";
    /// Clé primaire applicative.
    pub const PK: &'static str = "id";
    /// Colonnes `DateTime` legacy.
    pub const DATE_COLUMNS_LEGACY: &'static [&'static str] = &["busyUntil"];
    /// Requête d'insertion dans le schéma 3.0.
    pub const INSERT_SQL: &'static str = "INSERT INTO guilds (\
        id, name, busy_until, default_media_time, max_media_time, display_media_full\
        ) VALUES (?, ?, ?, ?, ?, ?)";

    /// Extrait une ligne legacy.
    pub fn from_legacy(row: &SqliteRow, out: &mut ReadOut) -> Result<Self, Error> {
        let rowid = read_rowid(row, Self::LEGACY_TABLE)?;
        let id = read_pk(row, Self::LEGACY_TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::LEGACY_TABLE, rowid, id.clone());
        Ok(Self {
            id,
            name: ctx.text_opt(row, "name")?,
            busy_until: ctx.date_opt(row, "busyUntil", out)?,
            default_media_time: ctx.int_opt(row, "defaultMediaTime")?,
            max_media_time: ctx.int_opt(row, "maxMediaTime")?,
            display_media_full: ctx.bool_req(row, "displayMediaFull")?,
        })
    }

    /// Extrait une ligne cible relue, pour vérification.
    pub fn from_target(row: &SqliteRow) -> Result<Self, Error> {
        let id = read_pk(row, Self::TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::TABLE, 0, id.clone());
        Ok(Self {
            id,
            name: ctx.text_opt(row, "name")?,
            busy_until: ctx.int_opt(row, "busy_until")?,
            default_media_time: ctx.int_opt(row, "default_media_time")?,
            max_media_time: ctx.int_opt(row, "max_media_time")?,
            display_media_full: ctx.int_req(row, "display_media_full")?,
        })
    }

    /// Insère la ligne dans le fichier cible.
    pub async fn insert(&self, tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), Error> {
        sqlx::query(Self::INSERT_SQL)
            .bind(&self.id)
            .bind(self.name.as_deref())
            .bind(self.busy_until)
            .bind(self.default_media_time)
            .bind(self.max_media_time)
            .bind(self.display_media_full)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::TargetWrite {
                table: Self::TABLE,
                source,
            })?;
        Ok(())
    }

    /// Clé primaire, pour la vérification par échantillon.
    pub fn primary_key(&self) -> &str {
        &self.id
    }
}

/// Overlay appairé (auth WebSocket v2 par hash de token).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayClient {
    pub id: String,
    pub guild_id: String,
    pub label: String,
    pub token_hash: String,
    pub session_mode: String,
    pub default_author_name: Option<String>,
    pub default_author_image: Option<String>,
    pub created_by_discord_user_id: Option<String>,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

impl OverlayClient {
    /// Table legacy Prisma.
    pub const LEGACY_TABLE: &'static str = "OverlayClient";
    /// Table cible 3.0.
    pub const TABLE: &'static str = "overlay_clients";
    /// Clé primaire applicative.
    pub const PK: &'static str = "id";
    /// Colonnes `DateTime` legacy.
    pub const DATE_COLUMNS_LEGACY: &'static [&'static str] =
        &["lastSeenAt", "createdAt", "revokedAt"];
    /// Requête d'insertion dans le schéma 3.0.
    pub const INSERT_SQL: &'static str = "INSERT INTO overlay_clients (\
        id, guild_id, label, token_hash, session_mode, default_author_name, \
        default_author_image, created_by_discord_user_id, last_seen_at, \
        created_at, revoked_at\
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

    /// Extrait une ligne legacy.
    pub fn from_legacy(row: &SqliteRow, out: &mut ReadOut) -> Result<Self, Error> {
        let rowid = read_rowid(row, Self::LEGACY_TABLE)?;
        let id = read_pk(row, Self::LEGACY_TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::LEGACY_TABLE, rowid, id.clone());
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guildId")?,
            label: ctx.text_req(row, "label")?,
            token_hash: ctx.text_req(row, "tokenHash")?,
            session_mode: ctx.enum_req(row, "sessionMode", SESSION_MODES)?,
            default_author_name: ctx.text_opt(row, "defaultAuthorName")?,
            default_author_image: ctx.text_opt(row, "defaultAuthorImage")?,
            created_by_discord_user_id: ctx.text_opt(row, "createdByDiscordUserId")?,
            last_seen_at: ctx.date_opt(row, "lastSeenAt", out)?,
            created_at: ctx.date_req(row, "createdAt", out)?,
            revoked_at: ctx.date_opt(row, "revokedAt", out)?,
        })
    }

    /// Extrait une ligne cible relue, pour vérification.
    pub fn from_target(row: &SqliteRow) -> Result<Self, Error> {
        let id = read_pk(row, Self::TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::TABLE, 0, id.clone());
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guild_id")?,
            label: ctx.text_req(row, "label")?,
            token_hash: ctx.text_req(row, "token_hash")?,
            session_mode: ctx.text_req(row, "session_mode")?,
            default_author_name: ctx.text_opt(row, "default_author_name")?,
            default_author_image: ctx.text_opt(row, "default_author_image")?,
            created_by_discord_user_id: ctx.text_opt(row, "created_by_discord_user_id")?,
            last_seen_at: ctx.int_opt(row, "last_seen_at")?,
            created_at: ctx.int_req(row, "created_at")?,
            revoked_at: ctx.int_opt(row, "revoked_at")?,
        })
    }

    /// Insère la ligne dans le fichier cible.
    pub async fn insert(&self, tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), Error> {
        sqlx::query(Self::INSERT_SQL)
            .bind(&self.id)
            .bind(&self.guild_id)
            .bind(&self.label)
            .bind(&self.token_hash)
            .bind(&self.session_mode)
            .bind(self.default_author_name.as_deref())
            .bind(self.default_author_image.as_deref())
            .bind(self.created_by_discord_user_id.as_deref())
            .bind(self.last_seen_at)
            .bind(self.created_at)
            .bind(self.revoked_at)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::TargetWrite {
                table: Self::TABLE,
                source,
            })?;
        Ok(())
    }

    /// Clé primaire, pour la vérification par échantillon.
    pub fn primary_key(&self) -> &str {
        &self.id
    }
}

/// Client d'ingestion (extension navigateur).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestClient {
    pub id: String,
    pub guild_id: String,
    pub label: String,
    pub token_hash: String,
    pub default_author_name: String,
    pub default_author_image: Option<String>,
    pub created_by_discord_user_id: String,
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

impl IngestClient {
    /// Table legacy Prisma.
    pub const LEGACY_TABLE: &'static str = "IngestClient";
    /// Table cible 3.0.
    pub const TABLE: &'static str = "ingest_clients";
    /// Clé primaire applicative.
    pub const PK: &'static str = "id";
    /// Colonnes `DateTime` legacy.
    pub const DATE_COLUMNS_LEGACY: &'static [&'static str] =
        &["lastSeenAt", "createdAt", "revokedAt"];
    /// Requête d'insertion dans le schéma 3.0.
    pub const INSERT_SQL: &'static str = "INSERT INTO ingest_clients (\
        id, guild_id, label, token_hash, default_author_name, \
        default_author_image, created_by_discord_user_id, last_seen_at, \
        created_at, revoked_at\
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

    /// Extrait une ligne legacy.
    pub fn from_legacy(row: &SqliteRow, out: &mut ReadOut) -> Result<Self, Error> {
        let rowid = read_rowid(row, Self::LEGACY_TABLE)?;
        let id = read_pk(row, Self::LEGACY_TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::LEGACY_TABLE, rowid, id.clone());
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guildId")?,
            label: ctx.text_req(row, "label")?,
            token_hash: ctx.text_req(row, "tokenHash")?,
            default_author_name: ctx.text_req(row, "defaultAuthorName")?,
            default_author_image: ctx.text_opt(row, "defaultAuthorImage")?,
            created_by_discord_user_id: ctx.text_req(row, "createdByDiscordUserId")?,
            last_seen_at: ctx.date_opt(row, "lastSeenAt", out)?,
            created_at: ctx.date_req(row, "createdAt", out)?,
            revoked_at: ctx.date_opt(row, "revokedAt", out)?,
        })
    }

    /// Extrait une ligne cible relue, pour vérification.
    pub fn from_target(row: &SqliteRow) -> Result<Self, Error> {
        let id = read_pk(row, Self::TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::TABLE, 0, id.clone());
        Ok(Self {
            id,
            guild_id: ctx.text_req(row, "guild_id")?,
            label: ctx.text_req(row, "label")?,
            token_hash: ctx.text_req(row, "token_hash")?,
            default_author_name: ctx.text_req(row, "default_author_name")?,
            default_author_image: ctx.text_opt(row, "default_author_image")?,
            created_by_discord_user_id: ctx.text_req(row, "created_by_discord_user_id")?,
            last_seen_at: ctx.int_opt(row, "last_seen_at")?,
            created_at: ctx.int_req(row, "created_at")?,
            revoked_at: ctx.int_opt(row, "revoked_at")?,
        })
    }

    /// Insère la ligne dans le fichier cible.
    pub async fn insert(&self, tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), Error> {
        sqlx::query(Self::INSERT_SQL)
            .bind(&self.id)
            .bind(&self.guild_id)
            .bind(&self.label)
            .bind(&self.token_hash)
            .bind(&self.default_author_name)
            .bind(self.default_author_image.as_deref())
            .bind(&self.created_by_discord_user_id)
            .bind(self.last_seen_at)
            .bind(self.created_at)
            .bind(self.revoked_at)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::TargetWrite {
                table: Self::TABLE,
                source,
            })?;
        Ok(())
    }

    /// Clé primaire, pour la vérification par échantillon.
    pub fn primary_key(&self) -> &str {
        &self.id
    }
}

/// Code d'appairage (overlay ou ingestion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub guild_id: String,
    pub mode: String,
    pub created_by_discord_user_id: String,
    pub author_name: Option<String>,
    pub author_image: Option<String>,
    pub expires_at: Option<i64>,
    pub used_at: Option<i64>,
    pub created_at: i64,
}

impl PairingCode {
    /// Table legacy Prisma.
    pub const LEGACY_TABLE: &'static str = "PairingCode";
    /// Table cible 3.0.
    pub const TABLE: &'static str = "pairing_codes";
    /// Clé primaire applicative.
    pub const PK: &'static str = "code";
    /// Colonnes `DateTime` legacy.
    pub const DATE_COLUMNS_LEGACY: &'static [&'static str] = &["expiresAt", "usedAt", "createdAt"];
    /// Requête d'insertion dans le schéma 3.0.
    pub const INSERT_SQL: &'static str = "INSERT INTO pairing_codes (\
        code, guild_id, mode, created_by_discord_user_id, author_name, \
        author_image, expires_at, used_at, created_at\
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";

    /// Extrait une ligne legacy.
    pub fn from_legacy(row: &SqliteRow, out: &mut ReadOut) -> Result<Self, Error> {
        let rowid = read_rowid(row, Self::LEGACY_TABLE)?;
        let code = read_pk(row, Self::LEGACY_TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::LEGACY_TABLE, rowid, code.clone());
        Ok(Self {
            code,
            guild_id: ctx.text_req(row, "guildId")?,
            mode: ctx.enum_req(row, "mode", SESSION_MODES)?,
            created_by_discord_user_id: ctx.text_req(row, "createdByDiscordUserId")?,
            author_name: ctx.text_opt(row, "authorName")?,
            author_image: ctx.text_opt(row, "authorImage")?,
            expires_at: ctx.date_opt(row, "expiresAt", out)?,
            used_at: ctx.date_opt(row, "usedAt", out)?,
            created_at: ctx.date_req(row, "createdAt", out)?,
        })
    }

    /// Extrait une ligne cible relue, pour vérification.
    pub fn from_target(row: &SqliteRow) -> Result<Self, Error> {
        let code = read_pk(row, Self::TABLE, Self::PK)?;
        let ctx = RowCtx::new(Self::TABLE, 0, code.clone());
        Ok(Self {
            code,
            guild_id: ctx.text_req(row, "guild_id")?,
            mode: ctx.text_req(row, "mode")?,
            created_by_discord_user_id: ctx.text_req(row, "created_by_discord_user_id")?,
            author_name: ctx.text_opt(row, "author_name")?,
            author_image: ctx.text_opt(row, "author_image")?,
            expires_at: ctx.int_opt(row, "expires_at")?,
            used_at: ctx.int_opt(row, "used_at")?,
            created_at: ctx.int_req(row, "created_at")?,
        })
    }

    /// Insère la ligne dans le fichier cible.
    pub async fn insert(&self, tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), Error> {
        sqlx::query(Self::INSERT_SQL)
            .bind(&self.code)
            .bind(&self.guild_id)
            .bind(&self.mode)
            .bind(&self.created_by_discord_user_id)
            .bind(self.author_name.as_deref())
            .bind(self.author_image.as_deref())
            .bind(self.expires_at)
            .bind(self.used_at)
            .bind(self.created_at)
            .execute(&mut **tx)
            .await
            .map_err(|source| Error::TargetWrite {
                table: Self::TABLE,
                source,
            })?;
        Ok(())
    }

    /// Clé primaire, pour la vérification par échantillon.
    pub fn primary_key(&self) -> &str {
        &self.code
    }
}
