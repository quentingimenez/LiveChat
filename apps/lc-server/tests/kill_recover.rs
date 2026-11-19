#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Kill-and-recover integration test (verification pipeline, task 8).
//!
//! Boots the real `lc-server` binary against a fresh seeded SQLite database
//! in WAL mode, confirms a stream of `POST /ingest` text jobs, kills the
//! process hard mid-stream, restarts it on the same port and the same data
//! directory, and proves that every confirmed job survived the kill:
//!
//! 1. every confirmed `kill <n>` text is present in `playback_jobs` after
//!    the restart (read through a second `lc_db::Db::open` while the
//!    restarted server is running),
//! 2. `PRAGMA integrity_check` reports `ok`,
//! 3. one more `POST /ingest` after the restart still returns 201.
//!
//! The status of every playback job after the restart is printed (not
//! asserted): no overlay ever connects in this test, so the playback
//! engine never starts and the jobs are expected to stay `PENDING` —
//! observed, not assumed.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use lc_domain::{
    Guild, GuildId, GuildMember, GuildMemberId, GuildRole, IngestClient, IngestClientId,
};
use reqwest::Client;
use serde_json::{json, Value};
use time::UtcDateTime;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

/// Bearer token of the seeded ingest client. The server stores only its
/// SHA-256 hash, computed with the production helper
/// `lc_server::media::tokens::hash_ingest_token`.
const INGEST_TOKEN: &str = "kill-ingest-token";
/// Discord id shared by the seeded author member and the ingest client: the
/// ingest client resolves its author through this id.
const DISCORD_USER_ID: &str = "42";
/// Raw id of the seeded guild (parses as a `GuildId`).
const GUILD_ID_RAW: &str = "123456789012345678";
/// Ingest posts that must be confirmed (201) before the kill.
const MIN_CONFIRMED_POSTS: usize = 30;
/// Budget for confirming `MIN_CONFIRMED_POSTS` posts.
const CONFIRM_WINDOW: Duration = Duration::from_secs(60);
/// `GET /health` polling cadence, mirroring the task-7 load harness.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// `GET /health` budget, mirroring the task-7 load harness.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);

/// Reserves an ephemeral TCP port: bind to `127.0.0.1:0`, read the assigned
/// port, then drop the listener so the server can bind it.
async fn reserve_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Seeds one guild, its ingest author member, and the ingest client.
///
/// Mirrors the load-test seeding recipe (`xtask/src/loadtest.rs::seed_database`):
/// the member carries `discord_user_id` `42`, and the ingest client is created
/// by that same Discord user, which is how `POST /ingest` resolves the author
/// (`apps/lc-server/src/http/ingest/auth.rs`).
async fn seed_database(db_path: &Path) {
    let db = lc_db::Db::open(db_path, &lc_db::DbConfig::default())
        .await
        .unwrap();
    let now = UtcDateTime::now();
    let guild = Guild {
        id: GuildId::parse(GUILD_ID_RAW).unwrap(),
        name: Some("Kill Recover Guild".to_owned()),
        busy_until: None,
        default_media_time: None,
        max_media_time: None,
        display_media_full: false,
    };
    db.guilds().upsert(&guild).await.unwrap();
    let member = GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild.id.clone(),
        display_name: "Alice".to_owned(),
        avatar: None,
        role: GuildRole::Member,
        created_at: now,
        removed_at: None,
        discord_user_id: Some(DISCORD_USER_ID.to_owned()),
        profile_custom: false,
        discord_synced_at: None,
    };
    db.guild_members().create(&member).await.unwrap();
    let ingest = IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild.id.clone(),
        label: "Kill Recover Ingest".to_owned(),
        token_hash: lc_server::media::tokens::hash_ingest_token(INGEST_TOKEN),
        default_author_name: "LiveChat Extension".to_owned(),
        default_author_image: None,
        created_by_discord_user_id: DISCORD_USER_ID.to_owned(),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    };
    db.ingest_clients().create(&ingest).await.unwrap();
    // Close the seed connection BEFORE the first spawn: between the kill and
    // the restart no process other than the server may hold the SQLite file
    // (a lingering handle in this test process would also block WAL deletion
    // during the sabotage run on Windows), and closing the pool checkpoints
    // and removes the seed WAL so the server starts with a fresh one.
    db.close().await;
}

/// Spawns the server binary. The environment mirrors the task-7 load harness
/// (`xtask/src/loadtest.rs::spawn_server`) exactly; in particular
/// `NODE_ENV=development` is required because the loopback `/ingest` calls are
/// exempt from the rate limiter only in development mode.
///
/// `kill_on_drop(true)` is a belt-and-braces addition on top of the
/// [`KillGuard`] below: a reaping or panicking test still never leaves a live
/// server process behind.
fn spawn_server(
    server_bin: &Path,
    port: u16,
    database_url: &str,
    media_dir: &Path,
) -> io::Result<Child> {
    Command::new(server_bin)
        .env("PORT", port.to_string())
        .env("API_URL", format!("http://127.0.0.1:{port}"))
        .env("DATABASE_URL", database_url)
        .env("MEDIA_STORAGE_DIR", media_dir)
        .env("NODE_ENV", "development")
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
}

/// Waits until `GET /health` reports `"db": "ok"`, polling every 250 ms for
/// at most 30 s. Panics early (with the exit status) if the child dies first.
async fn wait_healthy(port: u16, guard: &mut KillGuard) {
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let health_url = format!("http://127.0.0.1:{port}/health");
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = guard.try_wait() {
            panic!("lc-server exited before turning healthy: {status}");
        }
        if let Ok(response) = client.get(&health_url).send().await {
            if let Ok(payload) = response.json::<Value>().await {
                if payload.get("db").and_then(Value::as_str) == Some("ok") {
                    return;
                }
            }
        }
        sleep(HEALTH_POLL_INTERVAL).await;
    }
    if let Ok(Some(status)) = guard.try_wait() {
        panic!("lc-server exited before turning healthy: {status}");
    }
    panic!("lc-server did not report db ok within 30 s");
}

/// Owns the server child and the poster task. `Drop` aborts the poster and
/// kills the child, so no orphan process or runaway task survives a panic or
/// an early return; the normal flow kills and reaps the child explicitly.
struct KillGuard {
    child: Option<Child>,
    poster: Option<tokio::task::JoinHandle<()>>,
}

impl KillGuard {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self.child.as_mut() {
            Some(child) => child.try_wait(),
            None => Ok(None),
        }
    }
}

impl Drop for KillGuard {
    fn drop(&mut self) {
        if let Some(poster) = self.poster.take() {
            poster.abort();
        }
        if let Some(mut child) = self.child.take() {
            // `start_kill` is the synchronous flavor of `kill`: `Drop`
            // cannot await.
            child.start_kill().ok();
        }
    }
}

/// Posts `{"text": "kill <n>"}` ingest requests back-to-back (n = 1, 2, 3, …)
/// and sends every `n` whose response is `201 Created` through `confirmed`.
/// Stops on a transport error; the test aborts the poster after the kill
/// anyway, and the channel closing is its "recording done" signal.
async fn post_confirmed_ingests(base_url: String, confirmed: mpsc::UnboundedSender<u64>) {
    let client = match Client::builder().timeout(Duration::from_secs(5)).build() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[kill_recover] poster: cannot build an HTTP client: {error}");
            return;
        }
    };
    let mut counter: u64 = 0;
    loop {
        counter += 1;
        let response = client
            .post(format!("{base_url}/ingest"))
            .bearer_auth(INGEST_TOKEN)
            .json(&json!({ "text": format!("kill {counter}") }))
            .send()
            .await;
        match response {
            Ok(response) if response.status().as_u16() == 201 => {
                if confirmed.send(counter).is_err() {
                    return;
                }
            }
            Ok(response) => {
                eprintln!(
                    "[kill_recover] poster: POST kill {counter} returned unexpected status {}",
                    response.status()
                );
            }
            Err(error) => {
                eprintln!("[kill_recover] poster: transport error on kill {counter}: {error}");
                return;
            }
        }
    }
}

/// Kill the real `lc-server` mid-stream and prove it recovers: every
/// confirmed ingest job must still be readable from the same database file
/// after a restart on the same port.
#[tokio::test(flavor = "multi_thread")]
async fn kill_and_restart_preserves_confirmed_ingest_jobs() {
    let server_bin = PathBuf::from(env!("CARGO_BIN_EXE_lc-server"));
    let data_dir = tempfile::tempdir().unwrap();
    let media_dir = tempfile::tempdir().unwrap();
    let db_path = data_dir.path().join("kill_recover.db");
    let database_url = format!("file:{}", db_path.display());
    let port = reserve_port().await;
    let base_url = format!("http://127.0.0.1:{port}");

    seed_database(&db_path).await;

    let mut guard = KillGuard {
        child: None,
        poster: None,
    };
    guard.child = Some(spawn_server(&server_bin, port, &database_url, media_dir.path()).unwrap());
    wait_healthy(port, &mut guard).await;

    let (confirmed_tx, mut confirmed_rx) = mpsc::unbounded_channel();
    guard.poster = Some(tokio::spawn(post_confirmed_ingests(
        base_url.clone(),
        confirmed_tx,
    )));

    let mut confirmed: Vec<u64> = Vec::new();
    let confirm_deadline = Instant::now() + CONFIRM_WINDOW;
    while confirmed.len() < MIN_CONFIRMED_POSTS {
        let remaining = confirm_deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "only {}/{} ingest posts confirmed within the confirm window",
            confirmed.len(),
            MIN_CONFIRMED_POSTS
        );
        match tokio::time::timeout(remaining, confirmed_rx.recv()).await {
            Ok(Some(n)) => {
                confirmed.push(n);
                eprintln!(
                    "[kill_recover] confirmed kill {n} ({} total)",
                    confirmed.len()
                );
            }
            Ok(None) => panic!(
                "poster stopped after only {}/{} confirmed posts",
                confirmed.len(),
                MIN_CONFIRMED_POSTS
            ),
            Err(_) => panic!(
                "timed out waiting for ingest confirmations: {}/{} confirmed",
                confirmed.len(),
                MIN_CONFIRMED_POSTS
            ),
        }
    }

    // Kill the server hard, mid-stream, and reap it: after `wait().await`
    // the process is truly gone.
    let mut first_child = guard.child.take().unwrap();
    first_child.kill().await.unwrap();
    let first_exit = first_child.wait().await.unwrap();
    eprintln!("[kill_recover] first server exit after kill: {first_exit}");

    // Stop the poster, then drain every confirmation that raced in before
    // the kill landed.
    if let Some(poster) = guard.poster.take() {
        poster.abort();
        // Join the aborted task so its HTTP connections are dropped before
        // the restart.
        drop(poster.await);
    }
    while let Ok(n) = confirmed_rx.try_recv() {
        confirmed.push(n);
    }
    confirmed.sort_unstable();
    confirmed.dedup();
    eprintln!(
        "[kill_recover] {} confirmed ingest posts recorded (n={}..={})",
        confirmed.len(),
        confirmed.first().copied().unwrap_or(0),
        confirmed.last().copied().unwrap_or(0)
    );

    // Restart on the same port and the same data directory.
    guard.child = Some(spawn_server(&server_bin, port, &database_url, media_dir.path()).unwrap());
    wait_healthy(port, &mut guard).await;

    // Read the database through a SECOND connection while the restarted
    // server is running.
    let db = lc_db::Db::open(&db_path, &lc_db::DbConfig::default())
        .await
        .unwrap();

    // (1) every confirmed text must be in playback_jobs.
    let texts: Vec<Option<String>> = sqlx::query_scalar("SELECT text FROM playback_jobs")
        .fetch_all(db.pool())
        .await
        .unwrap();
    let mut missing = Vec::new();
    for n in &confirmed {
        let expected = format!("kill {n}");
        if !texts
            .iter()
            .any(|text| text.as_deref() == Some(expected.as_str()))
        {
            missing.push(expected);
        }
    }
    assert!(
        missing.is_empty(),
        "confirmed ingest texts missing from playback_jobs after the restart: {missing:?}"
    );

    // (2) the database must be structurally sound.
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(integrity, "ok", "PRAGMA integrity_check must report ok");

    // Characterization (not asserted): the status of every playback job
    // after the restart. No overlay ever connects in this test, so the
    // playback engine never starts and jobs are expected to stay PENDING —
    // observed, not assumed.
    let statuses: Vec<(Option<String>, String)> =
        sqlx::query_as("SELECT text, status FROM playback_jobs")
            .fetch_all(db.pool())
            .await
            .unwrap();
    for (text, status) in &statuses {
        eprintln!("[kill_recover] playback job status={status} text={text:?}");
    }
    let tally: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM playback_jobs GROUP BY status")
            .fetch_all(db.pool())
            .await
            .unwrap();
    eprintln!("[kill_recover] status tally after restart: {tally:?}");

    // (3) one more ingest after the restart must still be accepted.
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let response = client
        .post(format!("{base_url}/ingest"))
        .bearer_auth(INGEST_TOKEN)
        .json(&json!({ "text": "kill after restart" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        201,
        "one more POST /ingest after the restart must return 201, got {}",
        response.status()
    );

    // Teardown: close the test-side connection, then kill and reap the
    // restarted server.
    db.close().await;
    let mut second_child = guard.child.take().unwrap();
    second_child.kill().await.unwrap();
    let second_exit = second_child.wait().await.unwrap();
    eprintln!("[kill_recover] second server exit after teardown kill: {second_exit}");
    eprintln!(
        "[kill_recover] OK: {} confirmed posts recovered, integrity ok, post-restart ingest accepted",
        confirmed.len()
    );
}
