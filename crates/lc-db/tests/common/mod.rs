//! Helpers partagés par les tests d'intégration de `lc-db`.
//!
//! Chaque test ouvre sa propre base SQLite dans un [`tempfile::TempDir`]
//! (fichier réel : on teste WAL, migrations et contraintes SQL comme en
//! production) et le dossier est supprimé à la fin du test.

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::entities::{
    Guild, IngestClient, MediaAsset, MediaBoardItem, OverlayClient, PairingCodeEntry, PlaybackJob,
};
use lc_domain::enums::{JobStatus, MediaKind, MediaStatus, OverlaySessionMode, PairingMode};
use lc_domain::ids::{
    GuildId, IngestClientId, MediaAssetId, MediaBoardItemId, OverlayClientId,
    PairingCode as PairingCodeValue, PlaybackJobId,
};
use lc_domain::ms_to_datetime;

/// Instant fixe de référence des tests (`2023-11-14T22:13:20Z`).
pub const NOW_MS: i64 = 1_700_000_000_000;

/// Convertit des millisecondes epoch en date (panique si hors plage : bug de test).
pub fn date(millis: i64) -> DateTime<Utc> {
    ms_to_datetime(millis).expect("horodatage de test valide")
}

/// Instant fixe [`NOW_MS`].
pub fn now() -> DateTime<Utc> {
    date(NOW_MS)
}

/// Construit un [`GuildId`] Discord valide.
pub fn guild(value: &str) -> GuildId {
    GuildId::parse(value).expect("guild id de test valide")
}

/// Ouvre une base SQLite sur fichier temporaire, migrations appliquées.
///
/// Le [`tempfile::TempDir`] retourné doit rester vivant pendant le test : sa
/// destruction supprime le fichier.
pub async fn temp_db() -> (Db, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("dossier temporaire");
    let path = dir.path().join("livechat.db");
    let db = Db::open(&path, &DbConfig::default())
        .await
        .expect("ouverture de la base de test");
    (db, dir)
}

/// Ouvre une base en mémoire (pool forcé à une connexion).
pub async fn memory_db() -> Db {
    Db::open_in_memory(&DbConfig::default())
        .await
        .expect("ouverture de la base mémoire de test")
}

/// Média `READY` minimal, expirant à `now`.
pub fn sample_asset(source_hash: &str, now: DateTime<Utc>) -> MediaAsset {
    MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: source_hash.to_owned(),
        source_url: format!("https://example.test/{source_hash}.mp4"),
        kind: MediaKind::Video,
        mime: "video/mp4".to_owned(),
        duration_sec: Some(12),
        width: Some(1920),
        height: Some(1080),
        is_vertical: false,
        storage_path: format!("media/{source_hash}.mp4"),
        size_bytes: 1_024,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: now,
        expires_at: now,
        created_at: now,
    }
}

/// Job de lecture `PENDING` éligible à `now`.
pub fn sample_job(guild_id: &GuildId, now: DateTime<Utc>) -> PlaybackJob {
    PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: None,
        text: Some("média de test".to_owned()),
        show_text: false,
        author_name: Some("Auteur test".to_owned()),
        author_image: None,
        duration_sec: 5,
        priority: 0,
        status: JobStatus::Pending,
        resumes_after_job_id: None,
        resume_offset_sec: 0,
        remaining_ms_snapshot: None,
        last_playback_state_at: None,
        submission_date: now,
        execution_date: now,
        scheduled_at: now,
        started_at: None,
        finished_at: None,
    }
}

/// Élément de board média minimal.
pub fn sample_board_item(
    guild_id: &GuildId,
    media_asset_id: MediaAssetId,
    now: DateTime<Utc>,
) -> MediaBoardItem {
    MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id,
        title: Some("Titre média".to_owned()),
        message: Some("Message média".to_owned()),
        created_by_discord_user_id: Some("42".to_owned()),
        created_by_name: Some("Auteur".to_owned()),
        created_at: now,
    }
}

/// Guilde avec réglages par défaut.
pub fn sample_guild(id: &GuildId) -> Guild {
    Guild {
        id: id.clone(),
        name: Some("Guilde de test".to_owned()),
        busy_until: None,
        default_media_time: None,
        max_media_time: None,
        display_media_full: false,
    }
}

/// Overlay appairé actif.
pub fn sample_overlay(guild_id: &GuildId, token_hash: &str, now: DateTime<Utc>) -> OverlayClient {
    OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: "Overlay salon".to_owned(),
        token_hash: token_hash.to_owned(),
        session_mode: OverlaySessionMode::Normal,
        default_author_name: Some("Overlay".to_owned()),
        default_author_image: None,
        created_by_discord_user_id: Some("42".to_owned()),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    }
}

/// Client d'ingestion appairé actif.
pub fn sample_ingest(guild_id: &GuildId, token_hash: &str, now: DateTime<Utc>) -> IngestClient {
    IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: "Extension navigateur".to_owned(),
        token_hash: token_hash.to_owned(),
        default_author_name: "Ingestion".to_owned(),
        default_author_image: None,
        created_by_discord_user_id: "42".to_owned(),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    }
}

/// Code d'appairage normal (expire dans 10 minutes).
pub fn sample_pairing(code: &str, guild_id: &GuildId, now: DateTime<Utc>) -> PairingCodeEntry {
    PairingCodeEntry {
        code: PairingCodeValue::parse(code).expect("code d'appairage de test valide"),
        guild_id: guild_id.clone(),
        mode: PairingMode::Normal,
        created_by_discord_user_id: "42".to_owned(),
        author_name: Some("Invité".to_owned()),
        author_image: None,
        expires_at: Some(now + Duration::minutes(10)),
        used_at: None,
        created_at: now,
    }
}
