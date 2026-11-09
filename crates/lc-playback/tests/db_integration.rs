//! Tests d'intégration des primitives SQL de `lc-db` utilisées par le moteur.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use chrono::Duration as ChronoDuration;
use lc_domain::JobStatus;

use support::{guild_id, memory_db, now, sample_job};

#[tokio::test]
async fn claim_next_respecte_priorite_et_soumission() {
    let db = memory_db().await;
    let guild = guild_id();
    let now = now();

    let low = sample_job(guild.clone(), now);
    let mut high = sample_job(guild.clone(), now + ChronoDuration::milliseconds(10));
    high.priority = 5;
    high.execution_date = now;
    high.scheduled_at = now;

    db.playback_jobs().create(&low).await.unwrap();
    db.playback_jobs().create(&high).await.unwrap();

    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, high.id);

    let none = db.playback_jobs().claim_next(now).await.unwrap();
    assert!(none.is_none(), "un seul job PLAYING par guilde");

    db.playback_jobs()
        .finish(high.id, JobStatus::Done, now)
        .await
        .unwrap();
    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, low.id);
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
        .set_busy_until(&guild, Some(now + ChronoDuration::seconds(5)))
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
async fn reprise_eligible_seulement_apres_terminaison_du_parent() {
    let db = memory_db().await;
    let guild = guild_id();
    let now = now();

    let child = sample_job(guild.clone(), now);
    db.playback_jobs().create(&child).await.unwrap();

    let mut parent = sample_job(guild.clone(), now + ChronoDuration::milliseconds(1));
    parent.resumes_after_job_id = Some(child.id);
    parent.execution_date = now;
    parent.scheduled_at = now;
    db.playback_jobs().create(&parent).await.unwrap();

    // Le job de reprise reste inéligible tant que son parent est PENDING.
    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, child.id);

    assert!(
        db.playback_jobs().claim_next(now).await.unwrap().is_none(),
        "parent PENDING non terminé et un seul PLAYING"
    );

    db.playback_jobs()
        .finish(child.id, JobStatus::Done, now)
        .await
        .unwrap();
    let claimed = db.playback_jobs().claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, parent.id);
}
