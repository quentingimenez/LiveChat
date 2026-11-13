//! Scénarios d'intégration du moteur de lecture, sur base SQLite en mémoire.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use chrono::Duration as ChronoDuration;
use lc_domain::{datetime_to_ms, Clock, JobStatus};
use lc_playback::{
    PlaybackEndedEvent, PlaybackStateEvent, PlaybackStateOutcome, MEDIA_BOARD_JOB_PRIORITY,
};
use lc_protocol::{PlaybackEndedReason, PlaybackState, StopReason, StopRequestSource};

use support::{harness, sample_guild, sample_job, sample_media_asset, sample_media_job, NOW_MS};

#[tokio::test]
async fn tick_sans_job_ne_fait_rien() {
    let h = harness(1).await;
    let report = h.engine.tick().await.unwrap();
    assert_eq!(report.processed_guilds, 0);
    assert_eq!(report.active_guilds, 0);
    assert!(h.sink.calls().is_empty());
}

#[tokio::test]
async fn dispatch_texte_puis_watchdog_passe_au_suivant() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let first = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&first).await.unwrap();

    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].job_id, first.id.to_canonical());
    assert_eq!(plays[0].text.value, "média de test");
    assert!(plays[0].text.enabled);
    assert_eq!(plays[0].author.name, "Auteur");
    assert!(plays[0].author.enabled);
    assert_eq!(plays[0].duration_sec, 5);
    assert!(plays[0].media.is_none());

    let stored = h.stored_job(first.id).await;
    assert_eq!(stored.status, JobStatus::Playing);

    let busy = h.db.guilds().get(&guild).await.unwrap().unwrap();
    let busy_until = busy.busy_until.unwrap();
    assert_eq!(datetime_to_ms(busy_until), NOW_MS + 5_250);

    let second = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&second).await.unwrap();
    assert_eq!(
        h.sink.play_calls().len(),
        1,
        "un seul job PLAYING par guilde"
    );
    assert_eq!(h.stored_job(second.id).await.status, JobStatus::Pending);

    // Watchdog : fin attendue + padding (5 250 ms) + grâce de libération (10 s).
    h.advance(5_000 + 250 + 10_000);
    let report = h.engine.tick().await.unwrap();
    assert_eq!(report.processed_guilds, 1);
    assert_eq!(h.stored_job(first.id).await.status, JobStatus::Done);
    assert_eq!(h.stored_job(second.id).await.status, JobStatus::Playing);
    assert_eq!(h.sink.play_calls().len(), 2);
}

#[tokio::test]
async fn watchdog_ne_libere_pas_un_job_confirme_recemment() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let job = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&job).await.unwrap();
    assert_eq!(h.sink.play_calls().len(), 1);

    // Keep-alive de file locale 10 s après le dispatch.
    h.advance(10_000);
    let outcome = h
        .engine
        .handle_playback_state(PlaybackStateEvent::new(
            guild.clone(),
            Some(job.id),
            PlaybackState::Paused,
            Some(5_000),
        ))
        .await
        .unwrap();
    assert_eq!(outcome, PlaybackStateOutcome::Persisted);

    // 5,5 s plus tard le seuil du watchdog est dépassé, mais le dernier
    // checkpoint a moins de 15 s : le job n'est pas libéré.
    h.advance(5_500);
    let report = h.engine.tick().await.unwrap();
    assert_eq!(report.processed_guilds, 1);
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Playing);
    assert_eq!(h.sink.play_calls().len(), 1);
}

#[tokio::test]
async fn watchdog_libere_un_job_sans_keepalive_depuis_quinze_secondes() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let job = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&job).await.unwrap();

    h.advance(10_000);
    h.engine
        .handle_playback_state(PlaybackStateEvent::new(
            guild.clone(),
            Some(job.id),
            PlaybackState::Paused,
            Some(5_000),
        ))
        .await
        .unwrap();
    h.advance(5_500);
    h.engine.tick().await.unwrap();
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Playing);

    // Dernier checkpoint vieux de plus de 15 s : le watchdog reprend la main.
    h.advance(10_000);
    h.engine.tick().await.unwrap();
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Done);
}

#[tokio::test]
async fn ordre_priorite_puis_soumission() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let now = h.clock.now();

    let low = sample_job(guild.clone(), now);
    let mut high = sample_job(guild.clone(), now + ChronoDuration::milliseconds(5));
    high.priority = 5;
    high.duration_sec = 1;
    high.execution_date = now;
    high.scheduled_at = now;

    h.db.playback_jobs().create(&low).await.unwrap();
    h.db.playback_jobs().create(&high).await.unwrap();

    h.engine.tick().await.unwrap();
    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].job_id, high.id.to_canonical());

    h.engine
        .handle_playback_ended(PlaybackEndedEvent::new(
            guild.clone(),
            high.id,
            PlaybackEndedReason::Completed,
        ))
        .await
        .unwrap();

    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 2);
    assert_eq!(plays[1].job_id, low.id.to_canonical());
}

#[tokio::test]
async fn preemption_media_du_board_puis_reprise_du_parent() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let asset = sample_media_asset(h.clock.now());
    h.db.media_assets().insert(&asset).await.unwrap();

    let parent = sample_media_job(guild.clone(), h.clock.now(), asset.id, 20);
    h.engine.enqueue(&parent).await.unwrap();
    assert_eq!(h.sink.play_calls().len(), 1);
    assert_eq!(h.sink.play_calls()[0].duration_sec, 20);

    h.advance(5_000);

    let mut item = sample_job(guild.clone(), h.clock.now());
    item.priority = MEDIA_BOARD_JOB_PRIORITY;
    item.duration_sec = 2;
    h.engine.enqueue(&item).await.unwrap();

    let stops = h.sink.stop_calls();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0].reason, StopReason::Preempt);
    let parent_id = parent.id.to_canonical();
    assert_eq!(stops[0].job_id.as_deref(), Some(parent_id.as_str()));

    let suspended = h.stored_job(parent.id).await;
    assert_eq!(suspended.status, JobStatus::Pending);
    assert_eq!(suspended.resumes_after_job_id, Some(item.id));
    assert_eq!(suspended.resume_offset_sec, 5);
    assert_eq!(suspended.remaining_ms_snapshot, Some(15_000));
    assert!(suspended.started_at.is_none());

    assert_eq!(h.sink.play_calls().len(), 2);
    assert_eq!(h.sink.play_calls()[1].job_id, item.id.to_canonical());

    h.engine
        .handle_playback_ended(PlaybackEndedEvent::new(
            guild.clone(),
            item.id,
            PlaybackEndedReason::Completed,
        ))
        .await
        .unwrap();

    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 3);
    let resumed = &plays[2];
    assert_eq!(resumed.job_id, parent_id);
    assert_eq!(resumed.duration_sec, 15);
    let media = resumed.media.as_ref().unwrap();
    assert_eq!(media.start_offset_sec, Some(5));
    assert!(media.url.contains("startOffsetSec=5"));
    assert!(media.url.contains("#t=5"));
}

#[tokio::test]
async fn stop_cible_puis_global() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let first = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&first).await.unwrap();
    let second = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&second).await.unwrap();
    assert_eq!(h.sink.play_calls().len(), 1);

    let outcome = h
        .engine
        .stop(&guild, Some(first.id), StopRequestSource::User)
        .await
        .unwrap();
    assert_eq!(outcome.stopped.len(), 1);
    assert_eq!(outcome.payload.reason, StopReason::Manual);
    let first_id = first.id.to_canonical();
    assert_eq!(outcome.payload.job_id.as_deref(), Some(first_id.as_str()));
    assert_eq!(h.stored_job(first.id).await.status, JobStatus::Done);
    assert_eq!(h.sink.stop_calls().len(), 1);
    // Le job suivant démarre après la libération du verrou.
    assert_eq!(h.sink.play_calls().len(), 2);

    let outcome = h
        .engine
        .stop(&guild, None, StopRequestSource::Hotkey)
        .await
        .unwrap();
    let second_id = second.id.to_canonical();
    assert_eq!(outcome.payload.job_id.as_deref(), Some(second_id.as_str()));
    assert_eq!(h.stored_job(second.id).await.status, JobStatus::Done);
    assert_eq!(h.sink.stop_calls().len(), 2);
}

#[tokio::test]
async fn stop_avec_raison_revoked() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let job = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&job).await.unwrap();

    let outcome = h
        .engine
        .stop_with_reason(&guild, Some(job.id), StopReason::Revoked)
        .await
        .unwrap();
    assert_eq!(outcome.payload.reason, StopReason::Revoked);
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Done);
}

#[tokio::test]
async fn busy_until_futur_bloque_puis_expire() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let now = h.clock.now();
    h.db.guilds()
        .set_busy_until(&guild, Some(now + ChronoDuration::seconds(10)))
        .await
        .unwrap();

    let job = sample_job(guild.clone(), now);
    h.engine.enqueue(&job).await.unwrap();
    assert!(h.sink.play_calls().is_empty());
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Pending);

    h.advance(10_001);
    h.engine.tick().await.unwrap();
    assert_eq!(h.sink.play_calls().len(), 1);
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Playing);
}

#[tokio::test]
async fn aucun_overlay_connecte_fait_echouer_le_job() {
    let h = harness(0).await;
    let guild = h.guild_id.clone();
    let job = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&job).await.unwrap();
    assert_eq!(h.sink.play_calls().len(), 1);
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Failed);

    h.sink.set_recipients(1);
    let next = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&next).await.unwrap();
    assert_eq!(h.stored_job(next.id).await.status, JobStatus::Playing);
    assert_eq!(h.sink.play_calls().len(), 2);
}

#[tokio::test]
async fn purge_respecte_la_retention() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let now = h.clock.now();

    let old = sample_job(guild.clone(), now);
    h.db.playback_jobs().create(&old).await.unwrap();
    h.db.playback_jobs()
        .finish(old.id, JobStatus::Done, now - ChronoDuration::hours(48))
        .await
        .unwrap();

    let recent = sample_job(guild.clone(), now);
    h.db.playback_jobs().create(&recent).await.unwrap();
    h.db.playback_jobs()
        .finish(recent.id, JobStatus::Done, now - ChronoDuration::hours(1))
        .await
        .unwrap();

    let report = h.engine.purge(now).await.unwrap();
    assert_eq!(report.playback_jobs, 1);
    assert!(h.db.playback_jobs().get(old.id).await.unwrap().is_none());
    assert!(h.db.playback_jobs().get(recent.id).await.unwrap().is_some());
}

#[tokio::test]
async fn current_job_reprend_la_position_et_est_throttle() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let asset = sample_media_asset(h.clock.now());
    h.db.media_assets().insert(&asset).await.unwrap();
    let job = sample_media_job(guild.clone(), h.clock.now(), asset.id, 20);
    h.engine.enqueue(&job).await.unwrap();

    h.advance(3_000);
    let outcome = h
        .engine
        .handle_playback_state(PlaybackStateEvent::new(
            guild.clone(),
            Some(job.id),
            PlaybackState::Playing,
            Some(17_000),
        ))
        .await
        .unwrap();
    assert_eq!(outcome, PlaybackStateOutcome::Persisted);

    let current = h.engine.current_job(&guild).await.unwrap().unwrap();
    assert_eq!(current.play.job_id, job.id.to_canonical());
    assert_eq!(current.started_at, NOW_MS);
    assert_eq!(current.remaining_ms, 17_000);
    assert_eq!(current.play.duration_sec, 20);

    let outcome = h
        .engine
        .handle_playback_state(PlaybackStateEvent::new(
            guild.clone(),
            Some(job.id),
            PlaybackState::Playing,
            Some(17_000),
        ))
        .await
        .unwrap();
    assert_eq!(outcome, PlaybackStateOutcome::Throttled);

    h.advance(1_000);
    let current = h.engine.current_job(&guild).await.unwrap().unwrap();
    assert_eq!(current.remaining_ms, 16_000);
}

#[tokio::test]
async fn job_texte_sans_duree_utilise_la_duree_par_defaut() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let mut job = sample_job(guild.clone(), h.clock.now());
    job.duration_sec = 0;
    h.engine.enqueue(&job).await.unwrap();

    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].duration_sec, 30);
    let busy = h.db.guilds().get(&guild).await.unwrap().unwrap();
    assert_eq!(datetime_to_ms(busy.busy_until.unwrap()), NOW_MS + 30_250);
}

#[tokio::test]
async fn watchdog_recupere_un_job_stale_apres_crash() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let now = h.clock.now();

    let mut job = sample_job(guild.clone(), now);
    job.status = JobStatus::Playing;
    job.started_at = Some(now - ChronoDuration::seconds(60));
    job.execution_date = now - ChronoDuration::seconds(60);
    job.scheduled_at = now - ChronoDuration::seconds(60);
    h.db.playback_jobs().create(&job).await.unwrap();

    h.engine.tick().await.unwrap();
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Done);
}

#[tokio::test]
async fn max_media_time_borne_la_duree_et_le_verrou() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let now = h.clock.now();

    let mut guild_row = sample_guild(guild.clone());
    guild_row.max_media_time = Some(2);
    h.db.guilds().upsert(&guild_row).await.unwrap();

    let asset = sample_media_asset(now);
    h.db.media_assets().insert(&asset).await.unwrap();
    let job = sample_media_job(guild.clone(), now, asset.id, 10);
    h.engine.enqueue(&job).await.unwrap();

    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].duration_sec, 2);
    let busy = h.db.guilds().get(&guild).await.unwrap().unwrap();
    assert_eq!(datetime_to_ms(busy.busy_until.unwrap()), NOW_MS + 2_250);
}
