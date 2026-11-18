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
