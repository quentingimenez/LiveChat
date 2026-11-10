-- ============================================================================
-- LiveChat 3.0 — schéma SQLite cible du serveur Rust (contrat Phase 2)
-- Statut : CONTRAT GELÉ le 2026-11-09 (Phase 0) — toute modification exige
-- l'accord du Lead + mise à jour simultanée de crates/lc-db et xtask (migrateur).
-- ============================================================================
--
-- Règles issues de l'audit dynamique (docs/rust/audit-dynamique.md §6-7) :
--   * dates : INTEGER = millisecondes Unix epoch UTC (i64). JAMAIS de
--     DEFAULT CURRENT_TIMESTAMP (Prisma/legacy écrirait du TEXT seconde).
--   * booléens : INTEGER 0/1 avec CHECK.
--   * noms : snake_case (le legacy Prisma est en camelCase ; le migrateur mappe).
--   * enums legacy stockés en TEXT uppercase (MediaAssetKind/Status,
--     PlaybackJobStatus, sessionMode/mode NORMAL|INVITE_READ_ONLY).
--   * PRAGMA par connexion : journal_mode=WAL, foreign_keys=ON,
--     busy_timeout=5000, synchronous=NORMAL.
--
-- Legacy (Prisma 5.22, server/prisma/schema.prisma) : tables MediaAsset,
-- MemeBoardItem, PlaybackJob, Guild, OverlayClient, IngestClient, PairingCode.
-- Mapping table/colonnes dans docs/rust/migration-plan.md et xtask.
-- ============================================================================

PRAGMA foreign_keys = ON;

-- Métadonnées applicatives (version de schéma, marqueur d'import legacy…).
CREATE TABLE app_metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;

-- Médias ingérés (vidéo/image/audio) + cache.
CREATE TABLE media_assets (
    id               TEXT    PRIMARY KEY,
    source_hash      TEXT    NOT NULL UNIQUE,
    source_url       TEXT    NOT NULL,
    kind             TEXT    NOT NULL CHECK (kind IN ('IMAGE', 'AUDIO', 'VIDEO')),
    mime             TEXT    NOT NULL,
    duration_sec     INTEGER,
    width            INTEGER,
    height           INTEGER,
    is_vertical      INTEGER NOT NULL DEFAULT 0 CHECK (is_vertical IN (0, 1)),
    storage_path     TEXT    NOT NULL,
    size_bytes       INTEGER NOT NULL DEFAULT 0,
    status           TEXT    NOT NULL DEFAULT 'PROCESSING'
                             CHECK (status IN ('PROCESSING', 'READY', 'FAILED')),
    error            TEXT,
    last_accessed_at INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    created_at       INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_media_assets_expires_at ON media_assets (expires_at);
CREATE INDEX idx_media_assets_status     ON media_assets (status);

-- Éléments du meme board par guilde.
CREATE TABLE meme_board_items (
    id                         TEXT    PRIMARY KEY,
    guild_id                   TEXT    NOT NULL,
    media_asset_id             TEXT    NOT NULL
                                       REFERENCES media_assets (id) ON DELETE CASCADE,
    title                      TEXT,
    message                    TEXT,
    created_by_discord_user_id TEXT,
    created_by_name            TEXT,
    created_at                 INTEGER NOT NULL,
    UNIQUE (guild_id, media_asset_id)
) STRICT;

CREATE INDEX idx_meme_board_items_guild_created ON meme_board_items (guild_id, created_at);
CREATE INDEX idx_meme_board_items_media_asset   ON meme_board_items (media_asset_id);

-- File de lecture (scheduler).
CREATE TABLE playback_jobs (
    id                     TEXT    PRIMARY KEY,
    guild_id               TEXT    NOT NULL,
    media_asset_id         TEXT    REFERENCES media_assets (id) ON DELETE SET NULL,
    text                   TEXT,
    show_text              INTEGER NOT NULL DEFAULT 0 CHECK (show_text IN (0, 1)),
    author_name            TEXT,
    author_image           TEXT,
    duration_sec           INTEGER NOT NULL,
    priority               INTEGER NOT NULL DEFAULT 0,
    status                 TEXT    NOT NULL DEFAULT 'PENDING'
                                   CHECK (status IN ('PENDING', 'PLAYING', 'DONE', 'FAILED')),
    resumes_after_job_id   TEXT,
    resume_offset_sec      INTEGER NOT NULL DEFAULT 0,
    remaining_ms_snapshot  INTEGER,
    last_playback_state_at INTEGER,
    submission_date        INTEGER NOT NULL,
    execution_date         INTEGER NOT NULL,
    scheduled_at           INTEGER NOT NULL,
    started_at             INTEGER,
    finished_at            INTEGER
) STRICT;

CREATE INDEX idx_playback_jobs_guild_execution        ON playback_jobs (guild_id, execution_date);
CREATE INDEX idx_playback_jobs_status_execution       ON playback_jobs (status, execution_date);
CREATE INDEX idx_playback_jobs_guild_media            ON playback_jobs (guild_id, media_asset_id);
CREATE INDEX idx_playback_jobs_status_finished        ON playback_jobs (status, finished_at);
CREATE INDEX idx_playback_jobs_guild_status_prio_sub  ON playback_jobs (guild_id, status, priority, submission_date);
CREATE INDEX idx_playback_jobs_guild_status_resume    ON playback_jobs (guild_id, status, resumes_after_job_id);

-- Réglages par guilde.
CREATE TABLE guilds (
    id                 TEXT    PRIMARY KEY,
    name               TEXT,
    busy_until         INTEGER,
    default_media_time INTEGER,
    max_media_time     INTEGER,
    display_media_full INTEGER NOT NULL DEFAULT 0 CHECK (display_media_full IN (0, 1))
) STRICT;

-- Overlays appairés (auth WS v2 par hash de token).
CREATE TABLE overlay_clients (
    id                        TEXT    PRIMARY KEY,
    guild_id                  TEXT    NOT NULL,
    label                     TEXT    NOT NULL,
    token_hash                TEXT    NOT NULL UNIQUE,
    session_mode              TEXT    NOT NULL DEFAULT 'NORMAL'
                                      CHECK (session_mode IN ('NORMAL', 'INVITE_READ_ONLY')),
    default_author_name       TEXT,
    default_author_image      TEXT,
    created_by_discord_user_id TEXT,
    last_seen_at              INTEGER,
    created_at                INTEGER NOT NULL,
    revoked_at                INTEGER
) STRICT;

CREATE INDEX idx_overlay_clients_guild ON overlay_clients (guild_id);

-- Clients d'ingestion (extension navigateur).
CREATE TABLE ingest_clients (
    id                         TEXT    PRIMARY KEY,
    guild_id                   TEXT    NOT NULL,
    label                      TEXT    NOT NULL,
    token_hash                 TEXT    NOT NULL UNIQUE,
    default_author_name        TEXT    NOT NULL,
    default_author_image       TEXT,
    created_by_discord_user_id TEXT    NOT NULL,
    last_seen_at               INTEGER,
    created_at                 INTEGER NOT NULL,
    revoked_at                 INTEGER
) STRICT;

CREATE INDEX idx_ingest_clients_guild ON ingest_clients (guild_id);

-- Codes d'appairage (PK = code lisible, charset sans ambiguïté).
CREATE TABLE pairing_codes (
    code                       TEXT    PRIMARY KEY,
    guild_id                   TEXT    NOT NULL,
    mode                       TEXT    NOT NULL DEFAULT 'NORMAL'
                                       CHECK (mode IN ('NORMAL', 'INVITE_READ_ONLY')),
    created_by_discord_user_id TEXT    NOT NULL,
    author_name                TEXT,
    author_image               TEXT,
    expires_at                 INTEGER,
    used_at                    INTEGER,
    created_at                 INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_pairing_codes_guild_expires ON pairing_codes (guild_id, expires_at);
CREATE INDEX idx_pairing_codes_expires       ON pairing_codes (expires_at);

-- ----------------------------------------------------------------------------
-- Données initiales
-- ----------------------------------------------------------------------------
INSERT INTO app_metadata (key, value) VALUES ('schema_version', '3.0.0');
-- Migration 0002 — invariant « un seul job PLAYING par guilde ».
--
-- AUD-A5-004 : l'unicité n'était garantie que par le SQL de claim ; l'appel
-- `set_status(id, Playing)` pouvait créer un second job PLAYING pour la même
-- guilde (double lecture). SQLite n'a pas de contrainte conditionnelle
-- déclarative, mais un index unique partiel fait respecter l'invariant par le
-- moteur, y compris pour les écritures qui ne passent pas par le claim.
--
-- Comportement après un arrêt brutal : un job resté PLAYING le reste (le
-- watchdog du moteur le libère après sa période de grâce) ; l'index empêche
-- simplement qu'un second job passe PLAYING pour la même guilde entre-temps.
-- Aucun « recovery » de masse n'est fait ici : la reprise est pilotée par le
-- moteur (watchdog + snapshots `playback.state`).
--
-- Avant de créer l'index, on normalise d'éventuels doublons hérités du legacy :
-- le job PLAYING le plus récent (started_at le plus grand, départage sur l'id)
-- est conservé ; les autres repassent PENDING pour être rejouables.

WITH ranked AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY guild_id
            ORDER BY COALESCE(started_at, -1) DESC, id DESC
        ) AS rank_in_guild
    FROM playback_jobs
    WHERE status = 'PLAYING'
)
UPDATE playback_jobs
SET status = 'PENDING',
    started_at = NULL,
    finished_at = NULL
WHERE id IN (SELECT id FROM ranked WHERE rank_in_guild > 1);

CREATE UNIQUE INDEX idx_playback_jobs_single_playing
    ON playback_jobs (guild_id)
    WHERE status = 'PLAYING';
