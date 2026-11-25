//! Scénarios d'intégration du moteur de lecture, sur base SQLite en mémoire.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use lc_domain::{datetime_to_ms, Clock, GuildId, JobStatus};
use lc_playback::{PlaybackEndedEvent, PlaybackStateEvent, PlaybackStateOutcome};
use lc_protocol::{PlaybackEndedReason, PlaybackState, StopReason, StopRequestSource};
use time::Duration as TimeDuration;

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
async fn ordre_de_soumission() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();
    let now = h.clock.now();

    let low = sample_job(guild.clone(), now - TimeDuration::milliseconds(1));
    let mut high = sample_job(guild.clone(), now);
    high.duration_sec = 1;

    h.db.playback_jobs().create(&low).await.unwrap();
    h.db.playback_jobs().create(&high).await.unwrap();

    h.engine.tick().await.unwrap();
    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].job_id, low.id.to_canonical());

    h.engine
        .handle_playback_ended(PlaybackEndedEvent::new(
            guild.clone(),
            low.id,
            PlaybackEndedReason::Completed,
        ))
        .await
        .unwrap();

    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 2);
    assert_eq!(plays[1].job_id, high.id.to_canonical());
}

#[tokio::test]
async fn un_media_du_board_attend_la_fin_du_job_en_cours() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let parent = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&parent).await.unwrap();
    assert_eq!(h.sink.play_calls().len(), 1);

    h.advance(5_000);

    // Média du board prioritaire dans l'ancien modèle : il ne coupe plus rien,
    // il attend son tour.
    let mut item = sample_job(guild.clone(), h.clock.now());
    item.duration_sec = 2;
    h.engine.enqueue(&item).await.unwrap();

    assert!(
        h.sink.stop_calls().is_empty(),
        "un média en file ne doit pas émettre de stop"
    );
    assert_eq!(h.sink.play_calls().len(), 1, "le job en cours continue");
    assert_eq!(h.stored_job(parent.id).await.status, JobStatus::Playing);

    h.engine
        .handle_playback_ended(PlaybackEndedEvent::new(
            guild.clone(),
            parent.id,
            PlaybackEndedReason::Completed,
        ))
        .await
        .unwrap();

    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 2);
    assert_eq!(plays[1].job_id, item.id.to_canonical());
    assert!(h.sink.stop_calls().is_empty());
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
        .set_busy_until(&guild, Some(now + TimeDuration::seconds(10)))
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
        .finish(old.id, JobStatus::Done, now - TimeDuration::hours(48))
        .await
        .unwrap();

    let recent = sample_job(guild.clone(), now);
    h.db.playback_jobs().create(&recent).await.unwrap();
    h.db.playback_jobs()
        .finish(recent.id, JobStatus::Done, now - TimeDuration::hours(1))
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
    job.started_at = Some(now - TimeDuration::seconds(60));
    job.execution_date = now - TimeDuration::seconds(60);
    job.scheduled_at = now - TimeDuration::seconds(60);
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

/// An audio whose board item shows its picture on overlays carries it in the
/// `play`; unticking the box drops it.
#[tokio::test]
async fn un_audio_porte_l_image_de_son_element_du_board() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let mut audio = sample_media_asset(h.clock.now());
    audio.kind = lc_domain::MediaKind::Audio;
    audio.mime = "audio/mpeg".to_owned();
    h.db.media_assets().insert(&audio).await.unwrap();
    let mut picture = sample_media_asset(h.clock.now());
    picture.kind = lc_domain::MediaKind::Image;
    picture.mime = "image/jpeg".to_owned();
    picture.duration_sec = None;
    h.db.media_assets().insert(&picture).await.unwrap();
    let item = lc_domain::MediaBoardItem {
        id: lc_domain::MediaBoardItemId::new_v4(),
        guild_id: guild.clone(),
        media_asset_id: audio.id,
        title: None,
        message: None,
        created_by_discord_user_id: None,
        created_by_name: None,
        created_at: h.clock.now(),
        artwork_asset_id: Some(picture.id),
        artwork_on_overlay: true,
        display_duration_sec: None,
        display_scale_percent: None,
    };
    h.db.media_board().create(&item).await.unwrap();

    let first = sample_media_job(guild.clone(), h.clock.now(), audio.id, 3);
    h.engine.enqueue(&first).await.unwrap();
    let media = h.sink.play_calls()[0].media.clone().unwrap();
    assert_eq!(media.artwork_asset_id, Some(picture.id.to_canonical()));

    h.engine
        .handle_playback_ended(PlaybackEndedEvent::new(
            guild.clone(),
            first.id,
            PlaybackEndedReason::Completed,
        ))
        .await
        .unwrap();
    h.db.media_board()
        .set_artwork_on_overlay(item.id, false)
        .await
        .unwrap();
    let second = sample_media_job(guild.clone(), h.clock.now(), audio.id, 3);
    h.engine.enqueue(&second).await.unwrap();
    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 2);
    assert_eq!(plays[1].media.as_ref().unwrap().artwork_asset_id, None);
}

/// An image's board item carries the scale chosen for it in the `play`; a
/// media played outside the board keeps its natural size.
#[tokio::test]
async fn une_image_porte_l_echelle_choisie_sur_le_board() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let mut image = sample_media_asset(h.clock.now());
    image.kind = lc_domain::MediaKind::Image;
    image.mime = "image/png".to_owned();
    image.duration_sec = None;
    h.db.media_assets().insert(&image).await.unwrap();
    let item = lc_domain::MediaBoardItem {
        id: lc_domain::MediaBoardItemId::new_v4(),
        guild_id: guild.clone(),
        media_asset_id: image.id,
        title: None,
        message: None,
        created_by_discord_user_id: None,
        created_by_name: None,
        created_at: h.clock.now(),
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
        display_scale_percent: Some(250),
    };
    h.db.media_board().create(&item).await.unwrap();

    let first = sample_media_job(guild.clone(), h.clock.now(), image.id, 5);
    h.engine.enqueue(&first).await.unwrap();
    assert_eq!(
        h.sink.play_calls()[0]
            .media
            .as_ref()
            .unwrap()
            .display_scale_percent,
        Some(250)
    );

    h.engine
        .handle_playback_ended(PlaybackEndedEvent::new(
            guild.clone(),
            first.id,
            PlaybackEndedReason::Completed,
        ))
        .await
        .unwrap();
    h.db.media_board()
        .set_display_scale(item.id, None)
        .await
        .unwrap();
    let second = sample_media_job(guild.clone(), h.clock.now(), image.id, 5);
    h.engine.enqueue(&second).await.unwrap();
    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 2);
    assert_eq!(plays[1].media.as_ref().unwrap().display_scale_percent, None);
}

/// A video's board item carries its scale too: the overlay sizes the picture
/// of a video like a still.
#[tokio::test]
async fn une_video_porte_l_echelle_choisie_sur_le_board() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let mut video = sample_media_asset(h.clock.now());
    video.kind = lc_domain::MediaKind::Video;
    video.mime = "video/mp4".to_owned();
    video.duration_sec = Some(10);
    h.db.media_assets().insert(&video).await.unwrap();
    let item = lc_domain::MediaBoardItem {
        id: lc_domain::MediaBoardItemId::new_v4(),
        guild_id: guild.clone(),
        media_asset_id: video.id,
        title: None,
        message: None,
        created_by_discord_user_id: None,
        created_by_name: None,
        created_at: h.clock.now(),
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
        display_scale_percent: Some(300),
    };
    h.db.media_board().create(&item).await.unwrap();

    let job = sample_media_job(guild, h.clock.now(), video.id, 10);
    h.engine.enqueue(&job).await.unwrap();
    assert_eq!(
        h.sink.play_calls()[0]
            .media
            .as_ref()
            .unwrap()
            .display_scale_percent,
        Some(300)
    );
}

/// A one-second clip (an animated GIF converted to MP4) triggered from the
/// board: the job carries the clip's own duration.
async fn dispatch_video(asset_duration_sec: i64, job_duration_sec: i64) -> (u64, Option<u64>, i64) {
    let h = harness(1).await;
    let mut asset = sample_media_asset(h.clock.now());
    asset.duration_sec = Some(asset_duration_sec);
    h.db.media_assets().insert(&asset).await.unwrap();
    let job = sample_media_job(
        h.guild_id.clone(),
        h.clock.now(),
        asset.id,
        job_duration_sec,
    );
    h.engine.enqueue(&job).await.unwrap();
    let plays = h.sink.play_calls();
    assert_eq!(plays.len(), 1);
    let media_duration = plays[0].media.as_ref().and_then(|media| media.duration_sec);
    (
        plays[0].duration_sec,
        media_duration,
        h.stored_job(job.id).await.duration_sec,
    )
}

#[tokio::test]
async fn a_very_short_video_keeps_its_own_duration() {
    // The play lasts exactly the clip's duration: no minimum display time is
    // added, so the overlay stops the playback when the clip ends.
    let (play, media, stored) = dispatch_video(1, 1).await;
    assert_eq!(play, 1);
    assert_eq!(media, Some(1));
    assert_eq!(stored, 1);
    // Unset job duration: the media duration applies as-is.
    assert_eq!(dispatch_video(2, 0).await.0, 2);
}

#[tokio::test]
async fn a_video_plays_for_the_duration_it_is_given() {
    // A video plays for its own duration, without any floor.
    assert_eq!(dispatch_video(3, 3).await.0, 3);
    assert_eq!(dispatch_video(12, 12).await.0, 12);
    // An explicit duration is kept: shorter on a long video, longer on a clip.
    assert_eq!(dispatch_video(12, 2).await.0, 2);
    assert_eq!(dispatch_video(1, 10).await.0, 10);
}

#[tokio::test]
async fn the_guild_cap_still_bounds_a_video() {
    let h = harness(1).await;
    let mut guild = sample_guild(h.guild_id.clone());
    guild.max_media_time = Some(2);
    h.db.guilds().upsert(&guild).await.unwrap();
    let mut asset = sample_media_asset(h.clock.now());
    asset.duration_sec = Some(5);
    h.db.media_assets().insert(&asset).await.unwrap();
    let job = sample_media_job(h.guild_id.clone(), h.clock.now(), asset.id, 5);
    h.engine.enqueue(&job).await.unwrap();
    assert_eq!(h.sink.play_calls()[0].duration_sec, 2);
}

#[tokio::test]
async fn a_short_audio_or_image_is_not_extended() {
    for kind in [lc_domain::MediaKind::Audio, lc_domain::MediaKind::Image] {
        let h = harness(1).await;
        let mut asset = sample_media_asset(h.clock.now());
        asset.kind = kind;
        asset.duration_sec = Some(1);
        h.db.media_assets().insert(&asset).await.unwrap();
        let job = sample_media_job(h.guild_id.clone(), h.clock.now(), asset.id, 1);
        h.engine.enqueue(&job).await.unwrap();
        assert_eq!(h.sink.play_calls()[0].duration_sec, 1);
    }
}

/// Un `playback.ended` routé vers la session d'une autre guilde est refusé :
/// il ne termine pas le job et ne touche pas au verrou de la guilde qui le
/// possède.
#[tokio::test]
async fn playback_ended_from_another_guild_is_ignored() {
    let h = harness(2).await;
    let guild_a = h.guild_id.clone();
    let guild_b = GuildId::parse("223456789012345678").unwrap();
    h.db.guilds()
        .upsert(&sample_guild(guild_b.clone()))
        .await
        .unwrap();

    let first = sample_job(guild_a.clone(), h.clock.now());
    let second = sample_job(guild_a.clone(), h.clock.now());
    h.engine.enqueue(&first).await.unwrap();
    h.engine.enqueue(&second).await.unwrap();
    assert_eq!(h.stored_job(first.id).await.status, JobStatus::Playing);
    assert_eq!(h.stored_job(second.id).await.status, JobStatus::Pending);

    let other = sample_job(guild_b.clone(), h.clock.now());
    h.engine.enqueue(&other).await.unwrap();
    assert_eq!(h.stored_job(other.id).await.status, JobStatus::Playing);
    let busy_b =
        h.db.guilds()
            .get(&guild_b)
            .await
            .unwrap()
            .unwrap()
            .busy_until;

    // La fin du média de A arrive par la session de B (trame mal routée).
    let outcome = h
        .engine
        .handle_playback_ended(PlaybackEndedEvent::new(
            guild_b.clone(),
            first.id,
            PlaybackEndedReason::Completed,
        ))
        .await
        .unwrap();
    assert!(!outcome.released);

    // Le job de A reste en lecture, donc son média suivant reste en attente ;
    // le verrou de B ne bouge pas.
    assert_eq!(h.stored_job(first.id).await.status, JobStatus::Playing);
    assert_eq!(h.stored_job(second.id).await.status, JobStatus::Pending);
    assert_eq!(h.stored_job(other.id).await.status, JobStatus::Playing);
    assert_eq!(
        h.db.guilds()
            .get(&guild_b)
            .await
            .unwrap()
            .unwrap()
            .busy_until,
        busy_b
    );
}

/// Une erreur de rendu signalée par une guilde ne doit pas faire échouer le
/// média d'une autre guilde, ni libérer son verrou de lecture.
#[tokio::test]
async fn erreur_d_une_autre_guilde_n_echoue_pas_le_job() {
    let h = harness(2).await;
    let guild_a = h.guild_id.clone();
    let guild_b = GuildId::parse("223456789012345678").unwrap();
    h.db.guilds()
        .upsert(&sample_guild(guild_b.clone()))
        .await
        .unwrap();

    let job = sample_job(guild_a.clone(), h.clock.now());
    h.engine.enqueue(&job).await.unwrap();
    let busy_a =
        h.db.guilds()
            .get(&guild_a)
            .await
            .unwrap()
            .unwrap()
            .busy_until;

    let outcome = h.engine.handle_error(&guild_b, job.id).await.unwrap();
    assert!(!outcome.released);
    assert_eq!(h.stored_job(job.id).await.status, JobStatus::Playing);
    assert_eq!(
        h.db.guilds()
            .get(&guild_a)
            .await
            .unwrap()
            .unwrap()
            .busy_until,
        busy_a
    );
}

/// Une pause partagée fige le job PLAYING : le watchdog ne le libère plus et
/// le job suivant reste en attente.
#[tokio::test]
async fn pause_suspend_le_watchdog_et_le_dispatch() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let first = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&first).await.unwrap();
    h.advance(2_000);

    let pause = h.engine.pause(&guild, None).await.unwrap().unwrap();
    assert_eq!(pause.job_id, Some(first.id));
    assert_eq!(pause.remaining_ms, 3_000);
    assert_eq!(h.stored_job(first.id).await.status, JobStatus::Playing);

    // Sans pause, le watchdog aurait libéré le job après 5,25 s + 10 s.
    h.advance(60_000);
    h.engine.tick().await.unwrap();
    assert_eq!(h.stored_job(first.id).await.status, JobStatus::Playing);
    assert!(h.sink.stop_calls().is_empty());

    let second = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&second).await.unwrap();
    assert_eq!(h.stored_job(second.id).await.status, JobStatus::Pending);
    assert_eq!(h.sink.play_calls().len(), 1);
}

/// La reprise exclut le temps passé en pause du temps restant : un overlay qui
/// rejoint après la reprise ne doit pas recevoir un remaining amputé.
#[tokio::test]
async fn la_reprise_exclut_le_temps_de_pause_du_temps_restant() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let job = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&job).await.unwrap();
    h.advance(2_000);
    let pause = h.engine.pause(&guild, None).await.unwrap().unwrap();
    assert_eq!(pause.remaining_ms, 3_000);

    h.advance(10_000);
    let resumed = h.engine.resume(&guild).await.unwrap().unwrap();
    assert_eq!(resumed.remaining_ms, 3_000);

    let stored = h.stored_job(job.id).await;
    assert_eq!(
        stored.started_at.map(datetime_to_ms),
        Some(NOW_MS + 10_000),
        "started_at doit être décalé du temps de pause"
    );
    let current = h.engine.current_job(&guild).await.unwrap().unwrap();
    assert_eq!(current.remaining_ms, 3_000);
}

/// Un arrêt libère le job et efface la pause : la file repart normalement.
#[tokio::test]
async fn un_arret_efface_la_pause() {
    let h = harness(1).await;
    let guild = h.guild_id.clone();

    let first = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&first).await.unwrap();
    h.advance(1_000);
    h.engine.pause(&guild, None).await.unwrap();

    h.engine
        .stop(&guild, None, StopRequestSource::User)
        .await
        .unwrap();
    assert!(h.engine.paused_state(&guild).await.is_none());

    let second = sample_job(guild.clone(), h.clock.now());
    h.engine.enqueue(&second).await.unwrap();
    assert_eq!(h.stored_job(second.id).await.status, JobStatus::Playing);
}

/// Sans job en cours, la pause et la reprise sont sans effet.
#[tokio::test]
async fn pause_et_reprise_sans_job_sont_sans_effet() {
    let h = harness(1).await;
    assert!(h.engine.pause(&h.guild_id, None).await.unwrap().is_none());
    assert!(h.engine.resume(&h.guild_id).await.unwrap().is_none());
}
