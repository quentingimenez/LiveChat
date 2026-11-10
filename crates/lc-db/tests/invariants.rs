//! Tests de l'invariant « un seul job PLAYING par guilde » (migration 0002).
//!
//! La migration est appliquée automatiquement à l'ouverture ; ces tests
//! vérifient surtout la normalisation des doublons hérités du legacy, puis la
//! garantie apportée par l'index unique partiel.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_db::{DbError, SCHEMA_VERSION};
use lc_domain::{DomainError, JobStatus};

const MIGRATION_0002: &str = include_str!("../migrations/0002_playback_single_playing.sql");

#[tokio::test]
async fn migration_0002_normalise_les_doublons_de_playing() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");

    // Simule une base 0001 (sans l'index) contenant deux jobs PLAYING pour la
    // même guilde, comme peut en produire le legacy ou un `set_status` direct.
    sqlx::query("DROP INDEX idx_playback_jobs_single_playing")
        .execute(db.pool())
        .await
        .unwrap();
    let older = common::sample_job(&guild, now);
    let newer = common::sample_job(&guild, now);
    db.playback_jobs().create(&older).await.unwrap();
    db.playback_jobs().create(&newer).await.unwrap();
    for (job_id, started_at) in [
        (older.id, common::NOW_MS - 1_000),
        (newer.id, common::NOW_MS + 1_000),
    ] {
        sqlx::query("UPDATE playback_jobs SET status = 'PLAYING', started_at = ? WHERE id = ?")
            .bind(started_at)
            .bind(job_id.to_canonical())
            .execute(db.pool())
            .await
            .unwrap();
    }
    let playing_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM playback_jobs WHERE guild_id = ? AND status = 'PLAYING'",
    )
    .bind(guild.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(playing_before, 2, "état de départ volontairement invalide");

    // Rejoue le SQL de la migration 0002 : normalisation puis index.
    sqlx::raw_sql(MIGRATION_0002)
        .execute(db.pool())
        .await
        .unwrap();

    let playing: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM playback_jobs WHERE guild_id = ? AND status = 'PLAYING'",
    )
    .bind(guild.as_str())
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(
        playing,
        vec![newer.id.to_canonical()],
        "le plus récent est conservé"
    );

    let other = db.playback_jobs().get(older.id).await.unwrap().unwrap();
    assert_eq!(
        other.status,
        JobStatus::Pending,
        "les doublons repassent PENDING pour être rejouables"
    );
    assert_eq!(other.started_at, None);

    let index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
         AND name = 'idx_playback_jobs_single_playing'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(index_count, 1);

    // Toute nouvelle tentative de second PLAYING est refusée par le moteur.
    let error = db
        .playback_jobs()
        .set_status(older.id, JobStatus::Playing)
        .await
        .unwrap_err();
    match error {
        DbError::Domain(DomainError::InvalidState(message)) => {
            assert!(message.contains("PLAYING"), "{message}");
        }
        other => panic!("erreur inattendue : {other}"),
    }
}

#[tokio::test]
async fn schema_version_et_index_du_contrat_sont_presents() {
    let (db, _dir) = common::temp_db().await;
    assert_eq!(
        db.get_metadata("schema_version").await.unwrap().as_deref(),
        Some(SCHEMA_VERSION)
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
         AND name = 'idx_playback_jobs_single_playing'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        count, 1,
        "l'index d'invariant doit être présent après migration"
    );
}
