//! Tests d'intégration des primitives SQL de `lc-db` utilisées par le moteur.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use lc_domain::JobStatus;
use time::Duration as TimeDuration;

use support::{guild_id, memory_db, now, sample_job};

#[tokio::test]
async fn claim_next_respecte_l_ordre_de_soumission() {
    let db = memory_db().await;
    let guild = guild_id();
    let now = now();

    let low = sample_job(guild.clone(), now - TimeDuration::milliseconds(1));
    let high = sample_job(guild.clone(), now);

    db.playback_jobs().create(&low).await.unwrap();
    db.playback_jobs().create(&high).await.unwrap();

    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, low.id);

    let none = db.playback_jobs().claim_next(now).await.unwrap();
    assert!(none.is_none(), "un seul job PLAYING par guilde");

    db.playback_jobs()
        .finish(low.id, JobStatus::Done, now)
        .await
        .unwrap();
    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, high.id);
}

#[tokio::test]
async fn busy_until_bloque_le_claim_puis_le_libere() {
    let db = memory_db().await;
    let guild = guild_id();
    let now = now();

    let first = sample_job(guild.clone(), now);
    db.playback_jobs().create(&first).await.unwrap();
    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, first.id);

    let second = sample_job(guild.clone(), now);
    db.playback_jobs().create(&second).await.unwrap();
    assert!(
        db.playback_jobs().claim_next(now).await.unwrap().is_none(),
        "le job en cours verrouille la guilde"
    );

    db.playback_jobs()
        .finish(first.id, JobStatus::Done, now)
        .await
        .unwrap();
    db.guilds()
        .set_busy_until(&guild, Some(now + TimeDuration::seconds(5)))
        .await
        .unwrap();
    assert!(
        db.playback_jobs().claim_next(now).await.unwrap().is_none(),
        "busy_until futur bloque le claim"
    );

    db.guilds().set_busy_until(&guild, None).await.unwrap();
    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, second.id);
}

#[tokio::test]
async fn deux_jobs_meme_guilde_jouent_l_un_apres_l_autre() {
    let db = memory_db().await;
    let guild = guild_id();
    let now = now();

    let first = sample_job(guild.clone(), now - TimeDuration::milliseconds(1));
    db.playback_jobs().create(&first).await.unwrap();
    let second = sample_job(guild.clone(), now);
    db.playback_jobs().create(&second).await.unwrap();

    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, first.id);

    db.playback_jobs()
        .finish(first.id, JobStatus::Done, now)
        .await
        .unwrap();
    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, second.id);
}
