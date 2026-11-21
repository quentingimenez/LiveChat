//! Property-based test for the playback queue invariant: at most one
//! `PLAYING` job per guild.
//!
//! Arbitrary sequences of queue operations (`create`, `claim_next`,
//! `set_status`, `finish`) run against a fresh in-memory database, and after
//! every single operation the `playback_jobs` table is read directly to
//! check the invariant enforced by the partial unique index
//! `idx_playback_jobs_single_playing` (migration 0002).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_db::DbError;
use lc_domain::ids::PlaybackJobId;
use lc_domain::JobStatus;
use proptest::prelude::*;

/// One queue operation exercised by the property.
///
/// `pick` selects one of the jobs created so far (wrapped around the number
/// of created jobs; the whole operation is skipped when none exists yet).
#[derive(Clone, Debug)]
enum TestOp {
    /// Enqueue a new `PENDING` job for guild index `guild`.
    Create { guild: usize },
    /// Atomically claim the next eligible job.
    ClaimNext,
    /// `set_status`: `to_playing` selects `Playing`, otherwise `Pending`.
    SetStatus { pick: usize, to_playing: bool },
    /// `finish`: `to_failed` selects `Failed`, otherwise `Done`.
    Finish { pick: usize, to_failed: bool },
}

/// Uniform choice over the four operation kinds.
fn test_op_strategy() -> impl Strategy<Value = TestOp> {
    prop_oneof![
        2 => (0usize..2).prop_map(|guild| TestOp::Create { guild }),
        2 => Just(TestOp::ClaimNext),
        3 => (any::<usize>(), any::<bool>())
            .prop_map(|(pick, to_playing)| TestOp::SetStatus { pick, to_playing }),
        2 => (any::<usize>(), any::<bool>())
            .prop_map(|(pick, to_failed)| TestOp::Finish { pick, to_failed }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Sequences of 1 to 40 queue operations over two guilds keep at most
    /// one `PLAYING` job per guild after every operation. The only accepted
    /// error is the `DbError::Domain(_)` raised by `set_status(_, Playing)`
    /// when the guild already has a playing job; anything else fails the
    /// case.
    #[test]
    fn at_most_one_playing_job_per_guild(
        ops in proptest::collection::vec(test_op_strategy(), 1..=40),
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread tokio runtime");
        let now = common::now();
        let guilds = [
            common::guild("100000000000000001"),
            common::guild("100000000000000002"),
        ];
        let db = runtime.block_on(common::memory_db());
        let mut created: Vec<PlaybackJobId> = Vec::new();
        for op in ops {
            let op_label = format!("{op:?}");
            match op {
                TestOp::Create { guild } => {
                    let guild_id = &guilds[guild % guilds.len()];
                    let job = common::sample_job(guild_id, now);
                    let job_id = job.id;
                    let outcome = runtime.block_on(db.playback_jobs().create(&job));
                    prop_assert!(
                        outcome.is_ok(),
                        "unexpected create error: {:?}",
                        outcome.as_ref().err()
                    );
                    created.push(job_id);
                }
                TestOp::ClaimNext => {
                    let outcome = runtime.block_on(db.playback_jobs().claim_next(now));
                    prop_assert!(
                        outcome.is_ok(),
                        "unexpected claim_next error: {:?}",
                        outcome.as_ref().err()
                    );
                }
                TestOp::SetStatus { pick, to_playing } => {
                    if !created.is_empty() {
                        let job_id = created[pick % created.len()];
                        let status = if to_playing {
                            JobStatus::Playing
                        } else {
                            JobStatus::Pending
                        };
                        let outcome =
                            runtime.block_on(db.playback_jobs().set_status(job_id, status));
                        match outcome {
                            Ok(_) => {}
                            Err(error) => {
                                // The only accepted error: a second PLAYING
                                // job for the same guild, blocked by the
                                // partial unique index.
                                let accepted =
                                    to_playing && matches!(error, DbError::Domain(_));
                                prop_assert!(
                                    accepted,
                                    "unexpected set_status error: {error}"
                                );
                            }
                        }
                    }
                }
                TestOp::Finish { pick, to_failed } => {
                    if !created.is_empty() {
                        let job_id = created[pick % created.len()];
                        let status = if to_failed {
                            JobStatus::Failed
                        } else {
                            JobStatus::Done
                        };
                        let outcome =
                            runtime.block_on(db.playback_jobs().finish(job_id, status, now));
                        prop_assert!(
                            outcome.is_ok(),
                            "unexpected finish error: {:?}",
                            outcome.as_ref().err()
                        );
                    }
                }
            }
            // Read the table directly after every operation: no guild may
            // hold more than one PLAYING job.
            let playing_counts: Vec<(String, i64)> = runtime
                .block_on(
                    sqlx::query_as(
                        "SELECT guild_id, COUNT(*) FROM playback_jobs \
                         WHERE status = 'PLAYING' GROUP BY guild_id",
                    )
                    .fetch_all(db.pool()),
                )
                .expect("invariant query must run");
            prop_assert!(
                playing_counts.iter().all(|(_, count)| *count <= 1),
                "invariant broken after {op_label}: {playing_counts:?}"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// `claim_next` drains the queue in `submission_date ASC, id ASC` order:
    /// with strictly increasing submission dates, the k-th claim is the k-th
    /// created job. While one job is `PLAYING` the busy-guild check hides the
    /// rest of the queue (`claim_next` returns `None`), and finishing the job
    /// frees the slot for the next one.
    #[test]
    fn claim_next_returns_the_earliest_pending_job(k in 1usize..=10) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread tokio runtime");
        let now = common::now();
        let guild = common::guild("100000000000000003");
        let db = runtime.block_on(common::memory_db());

        let mut expected: Vec<PlaybackJobId> = Vec::new();
        for index in 1..=k {
            let mut job = common::sample_job(&guild, now);
            job.submission_date =
                common::date(common::NOW_MS + (index as i64 + 1) * 1000);
            let job_id = job.id;
            let outcome = runtime.block_on(db.playback_jobs().create(&job));
            prop_assert!(
                outcome.is_ok(),
                "unexpected create error: {:?}",
                outcome.as_ref().err()
            );
            expected.push(job_id);
        }

        for expected_id in expected {
            let outcome = runtime.block_on(db.playback_jobs().claim_next(now));
            prop_assert!(
                outcome.is_ok(),
                "unexpected claim_next error: {:?}",
                outcome.as_ref().err()
            );
            let claimed = outcome.unwrap();
            prop_assert!(
                claimed.is_some(),
                "claim_next returned None while {k} jobs were pending"
            );
            let job = claimed.unwrap();
            prop_assert_eq!(
                job.id,
                expected_id,
                "claim_next did not return the earliest pending job"
            );
            let busy = runtime.block_on(db.playback_jobs().claim_next(now));
            prop_assert!(
                busy.is_ok(),
                "unexpected claim_next error: {:?}",
                busy.as_ref().err()
            );
            prop_assert!(
                busy.unwrap().is_none(),
                "claim_next returned a job while the guild already had one PLAYING"
            );
            let finished =
                runtime.block_on(db.playback_jobs().finish(job.id, JobStatus::Done, now));
            prop_assert!(
                finished.is_ok(),
                "unexpected finish error: {:?}",
                finished.as_ref().err()
            );
        }

        let drained = runtime.block_on(db.playback_jobs().claim_next(now));
        prop_assert!(
            drained.is_ok(),
            "unexpected claim_next error: {:?}",
            drained.as_ref().err()
        );
        prop_assert!(
            drained.unwrap().is_none(),
            "claim_next returned a job after the queue was drained"
        );
    }
}

// ---------------------------------------------------------------------------
// Missed-mutant kills: error surfacing and no-op reporting
// ---------------------------------------------------------------------------

/// A raw backend failure must surface as `Err`: `claim_next` may not
/// swallow a broken table into `Ok(None)` (a match guard weakened to
/// `true`), and `set_status` may not translate a NON-unique failure into
/// a `Domain` error (a weakened `&&`). Dropping the table forces exactly
/// one such failure per call.
///
/// The unique-violation arm of `claim_next` itself is race-only — two
/// concurrent claims selecting the same pending job — and the in-memory
/// test backend forces a single connection, so that arm is justified as
/// unreachable rather than killed: `Ok(None)` on a race is the
/// conservative retry, and any other backend error still fails here.
#[test]
fn raw_db_errors_are_not_silenced_nor_mistranslated() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread tokio runtime");
    let now = common::now();
    let guild = common::guild("100000000000000005");
    let db = runtime.block_on(common::memory_db());
    let job = common::sample_job(&guild, now);
    runtime
        .block_on(db.playback_jobs().create(&job))
        .expect("job create");

    runtime
        .block_on(sqlx::query("DROP TABLE playback_jobs").execute(db.pool()))
        .expect("drop table");

    let claimed = runtime.block_on(db.playback_jobs().claim_next(now));
    assert!(
        claimed.is_err(),
        "claim_next on a broken backend must fail, not return Ok"
    );

    match runtime.block_on(db.playback_jobs().set_status(job.id, JobStatus::Playing)) {
        Ok(_) => panic!("set_status on a broken backend must fail"),
        Err(DbError::Domain(_)) => {
            panic!("set_status translated a non-unique backend failure into a Domain error")
        }
        Err(_) => {}
    }
}

/// `finish` and `requeue` report a missing job as `Ok(false)`; a row
/// count compared with `>= 0` instead of `> 0` would claim success on a
/// nonexistent id.
#[test]
fn finish_and_requeue_report_missing_jobs_as_no_op() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread tokio runtime");
    let now = common::now();
    let guild = common::guild("100000000000000006");
    let db = runtime.block_on(common::memory_db());
    let job = common::sample_job(&guild, now);
    runtime
        .block_on(db.playback_jobs().create(&job))
        .expect("job create");

    let missing = PlaybackJobId::new_v4();
    match runtime.block_on(db.playback_jobs().finish(missing, JobStatus::Done, now)) {
        Ok(false) => {}
        Ok(true) => panic!("finish on a missing job claimed success"),
        Err(_) => panic!("finish on a missing job must be Ok(false)"),
    }
    match runtime.block_on(db.playback_jobs().requeue(missing, now)) {
        Ok(false) => {}
        Ok(true) => panic!("requeue on a missing job claimed success"),
        Err(_) => panic!("requeue on a missing job must be Ok(false)"),
    }

    // The real job is genuinely finished and requeued, not ignored.
    match runtime.block_on(db.playback_jobs().finish(job.id, JobStatus::Done, now)) {
        Ok(true) => {}
        _ => panic!("finish on an existing job must be Ok(true)"),
    }
    match runtime.block_on(db.playback_jobs().requeue(job.id, now)) {
        Ok(true) => {}
        _ => panic!("requeue on an existing job must be Ok(true)"),
    }
}
