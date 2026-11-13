//! Tests du repository `playback_jobs` : scheduling, concurrence, cycle de vie.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::Duration;
use lc_db::DbError;
use lc_domain::{DomainError, JobStatus, PlaybackJobId};

#[tokio::test]
async fn creation_get_et_listes() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("111111111111111111");
    let other_guild = common::guild("222222222222222222");

    let job = common::sample_job(&guild, now);
    repo.create(&job).await.unwrap();
    assert_eq!(repo.get(job.id).await.unwrap().unwrap(), job);
    assert!(repo.get(PlaybackJobId::new_v4()).await.unwrap().is_none());

    let other_job = common::sample_job(&other_guild, now);
    repo.create(&other_job).await.unwrap();

    let pending = repo.list_pending_by_guild(&guild).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, job.id);
    assert_eq!(repo.count_by_status(JobStatus::Pending).await.unwrap(), 2);
    assert_eq!(repo.count_by_status(JobStatus::Playing).await.unwrap(), 0);
}

#[tokio::test]
async fn liste_attente_par_guilde_ordonnee() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("333333333333333333");

    let mut medium = common::sample_job(&guild, now);
    medium.priority = 1;
    let mut high = common::sample_job(&guild, now);
    high.priority = 3;
    high.submission_date = now + Duration::seconds(1);
    let mut medium_older = common::sample_job(&guild, now);
    medium_older.priority = 1;
    medium_older.submission_date = now - Duration::seconds(1);
    repo.create(&medium).await.unwrap();
    repo.create(&high).await.unwrap();
    repo.create(&medium_older).await.unwrap();

    let listed = repo.list_pending_by_guild(&guild).await.unwrap();
    let ids: Vec<_> = listed.iter().map(|job| job.id).collect();
    assert_eq!(ids, vec![high.id, medium_older.id, medium.id]);
}

#[tokio::test]
async fn claim_respecte_scheduled_at_et_statut() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();

    let guild = common::guild("444444444444444444");
    let mut future = common::sample_job(&guild, now);
    future.scheduled_at = now + Duration::seconds(30);
    repo.create(&future).await.unwrap();

    assert!(
        repo.claim_next(now).await.unwrap().is_none(),
        "un job planifié dans le futur n'est pas éligible"
    );
    let claimed = repo
        .claim_next(now + Duration::seconds(31))
        .await
        .unwrap()
        .expect("job éligible après scheduled_at");
    assert_eq!(claimed.id, future.id);
    assert_eq!(claimed.status, JobStatus::Playing);
    assert_eq!(claimed.started_at, Some(now + Duration::seconds(31)));

    // Un job non PENDING n'est jamais réclamé.
    let guild_playing = common::guild("445555555555555555");
    let mut already_playing = common::sample_job(&guild_playing, now);
    already_playing.status = JobStatus::Playing;
    already_playing.started_at = Some(now);
    repo.create(&already_playing).await.unwrap();

    // Un job PENDING déjà terminé (`finished_at` posé) n'est pas réclamé
    // (parité `finishedAt: null` du scheduler legacy).
    let guild_finished = common::guild("446666666666666666");
    let mut finished = common::sample_job(&guild_finished, now);
    finished.finished_at = Some(now);
    repo.create(&finished).await.unwrap();

    assert!(repo.claim_next(now).await.unwrap().is_none());
}

#[tokio::test]
async fn claim_un_seul_playing_par_guilde() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("555555555555555555");

    let mut first = common::sample_job(&guild, now);
    first.priority = 10;
    let second = common::sample_job(&guild, now);
    repo.create(&first).await.unwrap();
    repo.create(&second).await.unwrap();

    let claimed = repo.claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.id, first.id, "priorité la plus haute d'abord");
    assert!(
        repo.claim_next(now).await.unwrap().is_none(),
        "une guilde ne joue qu'un job à la fois"
    );

    repo.finish(claimed.id, JobStatus::Done, now + Duration::seconds(1))
        .await
        .unwrap();
    let next = repo
        .claim_next(now + Duration::seconds(2))
        .await
        .unwrap()
        .expect("le second job devient éligible");
    assert_eq!(next.id, second.id);
}

#[tokio::test]
async fn claim_priorite_puis_ordre_de_soumission() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild_a = common::guild("556666666666666666");
    let guild_b = common::guild("557777777777777777");

    let mut low = common::sample_job(&guild_a, now);
    low.priority = 1;
    let mut high = common::sample_job(&guild_b, now);
    high.priority = 9;
    repo.create(&low).await.unwrap();
    repo.create(&high).await.unwrap();

    let first = repo.claim_next(now).await.unwrap().unwrap();
    assert_eq!(first.id, high.id, "priorité supérieure d'abord");
    let second = repo.claim_next(now).await.unwrap().unwrap();
    assert_eq!(second.id, low.id, "autre guilde ensuite");

    // À priorité égale : soumission la plus ancienne d'abord.
    let guild_c = common::guild("558888888888888888");
    let guild_d = common::guild("559999999999999999");
    let mut older = common::sample_job(&guild_c, now);
    older.submission_date = now - Duration::seconds(5);
    let newer = common::sample_job(&guild_d, now);
    repo.create(&newer).await.unwrap();
    repo.create(&older).await.unwrap();

    let third = repo.claim_next(now).await.unwrap().unwrap();
    assert_eq!(third.id, older.id, "soumission la plus ancienne d'abord");
    let fourth = repo.claim_next(now).await.unwrap().unwrap();
    assert_eq!(fourth.id, newer.id);
}

#[tokio::test]
async fn claim_respecte_busy_until() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("560000000000000000");

    repo.create(&common::sample_job(&guild, now)).await.unwrap();

    db.guilds()
        .set_busy_until(&guild, Some(now + Duration::seconds(10)))
        .await
        .unwrap();
    assert!(
        repo.claim_next(now).await.unwrap().is_none(),
        "la guilde est verrouillée par busy_until"
    );

    db.guilds().set_busy_until(&guild, None).await.unwrap();
    assert!(repo.claim_next(now).await.unwrap().is_some());
}

#[tokio::test]
async fn claim_job_de_reprise_apres_parent() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("561111111111111111");

    let parent = common::sample_job(&guild, now);
    repo.create(&parent).await.unwrap();
    let mut child = common::sample_job(&guild, now);
    child.resumes_after_job_id = Some(parent.id);
    child.priority = 100;
    repo.create(&child).await.unwrap();

    let claimed = repo.claim_next(now).await.unwrap().unwrap();
    assert_eq!(
        claimed.id, parent.id,
        "le job de reprise ne passe pas avant son parent"
    );
    assert!(repo.claim_next(now).await.unwrap().is_none());

    repo.finish(parent.id, JobStatus::Done, now + Duration::seconds(1))
        .await
        .unwrap();
    let resumed = repo
        .claim_next(now + Duration::seconds(1))
        .await
        .unwrap()
        .expect("job de reprise éligible après la fin du parent");
    assert_eq!(resumed.id, child.id);

    // Parent absent : le job de reprise est éligible.
    let guild_orphan = common::guild("562222222222222222");
    let mut orphan = common::sample_job(&guild_orphan, now);
    orphan.resumes_after_job_id = Some(PlaybackJobId::new_v4());
    repo.create(&orphan).await.unwrap();
    assert_eq!(
        repo.claim_next(now).await.unwrap().unwrap().id,
        orphan.id,
        "parent manquant = reprise éligible"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claim_concurrent_ne_prend_pas_deux_fois_un_job() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let job = common::sample_job(&common::guild("563333333333333333"), now);
    db.playback_jobs().create(&job).await.unwrap();

    let first = db.clone();
    let second = db.clone();
    let first_repo = first.playback_jobs();
    let second_repo = second.playback_jobs();
    let (first, second) = tokio::join!(first_repo.claim_next(now), second_repo.claim_next(now));
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(
        first.iter().chain(second.iter()).count(),
        1,
        "un seul claim doit réussir"
    );
    assert_eq!(
        db.playback_jobs()
            .count_by_status(JobStatus::Playing)
            .await
            .unwrap(),
        1
    );
    let stored = db.playback_jobs().get(job.id).await.unwrap().unwrap();
    assert_eq!(stored.status, JobStatus::Playing);
    assert_eq!(stored.started_at, Some(now));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claim_concurrent_deux_guildes_reussissent() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let job_a = common::sample_job(&common::guild("564444444444444444"), now);
    let job_b = common::sample_job(&common::guild("565555555555555555"), now);
    db.playback_jobs().create(&job_a).await.unwrap();
    db.playback_jobs().create(&job_b).await.unwrap();

    let first = db.clone();
    let second = db.clone();
    let first_repo = first.playback_jobs();
    let second_repo = second.playback_jobs();
    let (first, second) = tokio::join!(first_repo.claim_next(now), second_repo.claim_next(now));
    let first = first.unwrap().expect("job de la guilde A réclamé");
    let second = second.unwrap().expect("job de la guilde B réclamé");
    assert_ne!(first.id, second.id);
    assert_eq!(
        db.playback_jobs()
            .count_by_status(JobStatus::Playing)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn set_status_playing_doublon_refuse() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("567777777777777777");

    let first = common::sample_job(&guild, now);
    let second = common::sample_job(&guild, now);
    repo.create(&first).await.unwrap();
    repo.create(&second).await.unwrap();

    assert!(repo.set_status(first.id, JobStatus::Playing).await.unwrap());

    // L'index unique partiel de la migration 0002 refuse le second PLAYING.
    let error = repo
        .set_status(second.id, JobStatus::Playing)
        .await
        .unwrap_err();
    match error {
        DbError::Domain(DomainError::InvalidState(message)) => {
            assert!(message.contains("PLAYING"), "{message}");
        }
        other => panic!("erreur inattendue : {other}"),
    }
    assert_eq!(
        repo.count_by_status(JobStatus::Playing).await.unwrap(),
        1,
        "l'invariant « un seul PLAYING par guilde » doit rester respecté"
    );
    let stored = repo.get(second.id).await.unwrap().unwrap();
    assert_eq!(stored.status, JobStatus::Pending);
}

#[tokio::test]
async fn cycle_de_vie_finish_et_purge() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("566666666666666666");

    let job = common::sample_job(&guild, now);
    repo.create(&job).await.unwrap();
    let claimed = repo.claim_next(now).await.unwrap().unwrap();
    assert_eq!(claimed.status, JobStatus::Playing);

    // `finish` refuse un statut non terminal.
    let error = repo
        .finish(claimed.id, JobStatus::Playing, now)
        .await
        .unwrap_err();
    match error {
        DbError::Domain(DomainError::Validation(message)) => {
            assert!(message.contains("DONE ou FAILED"), "{message}");
        }
        other => panic!("erreur inattendue : {other}"),
    }

    assert!(repo
        .finish(claimed.id, JobStatus::Done, now + Duration::seconds(5))
        .await
        .unwrap());
    let done = repo.get(claimed.id).await.unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!(done.finished_at, Some(now + Duration::seconds(5)));

    // Purge : seul le job terminé avant le cutoff disparaît.
    let mut old_done = common::sample_job(&guild, now);
    old_done.status = JobStatus::Done;
    old_done.started_at = Some(now);
    old_done.finished_at = Some(now);
    repo.create(&old_done).await.unwrap();
    let pending = common::sample_job(&guild, now);
    repo.create(&pending).await.unwrap();

    assert_eq!(
        repo.purge_done_before(now + Duration::seconds(4))
            .await
            .unwrap(),
        1
    );
    assert!(repo.get(old_done.id).await.unwrap().is_none());
    assert!(repo.get(done.id).await.unwrap().is_some());
    assert!(repo.get(pending.id).await.unwrap().is_some());
}

#[tokio::test]
async fn requeue_et_set_status() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("567777777777777777");

    let job = common::sample_job(&guild, now);
    repo.create(&job).await.unwrap();
    let claimed = repo.claim_next(now).await.unwrap().unwrap();

    assert!(repo
        .set_status(claimed.id, JobStatus::Failed)
        .await
        .unwrap());
    assert_eq!(
        repo.get(claimed.id).await.unwrap().unwrap().status,
        JobStatus::Failed
    );
    assert!(!repo
        .set_status(PlaybackJobId::new_v4(), JobStatus::Done)
        .await
        .unwrap());

    let retry_at = now + Duration::seconds(60);
    assert!(repo.requeue(claimed.id, retry_at).await.unwrap());
    let requeued = repo.get(claimed.id).await.unwrap().unwrap();
    assert_eq!(requeued.status, JobStatus::Pending);
    assert_eq!(requeued.started_at, None);
    assert_eq!(requeued.finished_at, None);
    assert_eq!(requeued.scheduled_at, retry_at);
    assert!(repo
        .claim_next(now + Duration::seconds(59))
        .await
        .unwrap()
        .is_none());
    assert!(repo.claim_next(retry_at).await.unwrap().is_some());
}

#[tokio::test]
async fn etat_de_lecture_persiste() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.playback_jobs();
    let now = common::now();
    let guild = common::guild("568888888888888888");

    repo.create(&common::sample_job(&guild, now)).await.unwrap();
    let claimed = repo.claim_next(now).await.unwrap().unwrap();

    let at = now + Duration::seconds(2);
    assert!(repo
        .set_playback_state(claimed.id, 3, Some(1_500), at)
        .await
        .unwrap());
    let stored = repo.get(claimed.id).await.unwrap().unwrap();
    assert_eq!(stored.resume_offset_sec, 3);
    assert_eq!(stored.remaining_ms_snapshot, Some(1_500));
    assert_eq!(stored.last_playback_state_at, Some(at));
    assert!(!repo
        .set_playback_state(PlaybackJobId::new_v4(), 0, None, now)
        .await
        .unwrap());
}
