//! WebSocket load harness for `lc-server` (verification pipeline, task 7).
//!
//! `xtask loadtest` boots a real `lc-server` binary against a fresh seeded
//! SQLite database, drives `--clients` overlay WebSocket clients plus a
//! parallel `/ingest` poster for `--duration-secs`, then reports the
//! HELLO→welcome latency percentiles, socket health, ingest results and —
//! on Linux — the server's file-descriptor usage, and enforces the budgets:
//!
//! - `hello_p95_ms <= --max-hello-p95-ms`,
//! - `welcomed_clients == clients` (every client got its welcome frame),
//! - `ws_errors == 0`, `disconnects == 0`,
//! - `ingest_errors == 0`, `ingest_rate_limited == 0`,
//! - `server_alive == true`,
//! - Linux only: `fd_end <= fd_start + 10`.
//!
//! fd sampling: `fd_start` is taken 5 s after the LAST welcome (falling
//! back to the last HELLO when no welcome ever arrives, and to the window
//! exit when the 5 s timer does not fit inside the window); `fd_end` is
//! taken at the window exit while the clients are still connected — they
//! linger [`CLIENT_STOP_GRACE`] past the window and only then close their
//! sockets, so both samples see the loaded server and the fd budget
//! compares two live states instead of a teardown artifact.
//!
//! Deviation from the original plan (which seeded a single guild): the server
//! enforces a per-guild cap of 16 concurrent sockets (`MAX_SOCKETS_PER_GUILD`
//! in `apps/lc-server/src/ws/hub.rs`, commit a9f214c), so the overlay clients
//! are sharded over `ceil(clients / 16)` seeded guilds in client order — the
//! first 16 clients in the first guild, the next 16 in the second, and so on
//! (block sharding, not round-robin). The ingest client and its author member
//! live in the first guild. Sharding keeps every client simultaneously
//! connected, which is what the harness must measure.

use std::{
    io,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use lc_domain::{
    now_ms, Guild, GuildId, GuildMember, GuildMemberId, GuildRole, IngestClient, IngestClientId,
    OverlayClient, OverlayClientId, OverlaySessionMode,
};
use lc_protocol::{HEARTBEAT_INTERVAL_MS, PROTOCOL_VERSION};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::UtcDateTime;
use tokio::{
    net::TcpListener,
    sync::mpsc,
    time::{interval, sleep_until, Instant, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Overlay pairing tokens are `load-overlay-000`, `load-overlay-001`, …
const OVERLAY_TOKEN_PREFIX: &str = "load-overlay-";
/// Bearer token of the seeded ingest client.
const INGEST_TOKEN: &str = "load-ingest-token";
/// Overlay app version announced in HELLO and heartbeat frames.
const APP_VERSION: &str = "3.0.0";
/// Overlay platform announced in the HELLO frame.
const PLATFORM: &str = "windows";
/// First seeded guild id; shard guilds use successive numeric ids.
const BASE_GUILD_ID: u64 = 123_456_789_012_345_678;
/// Server-side cap: at most 16 concurrent sockets per guild. Keep in sync
/// with `MAX_SOCKETS_PER_GUILD` in `apps/lc-server/src/ws/hub.rs` (a drift
/// surfaces as `ws_errors` on the next load run).
const SERVER_MAX_SOCKETS_PER_GUILD: usize = 16;
/// `fd_start` is sampled this long after the last welcome (or last HELLO).
const FD_SAMPLE_DELAY: Duration = Duration::from_secs(5);
/// Clients stay connected this long past the load window: `fd_end` is
/// sampled at the window exit, and the sockets must still be open so the
/// sample sees the loaded server rather than a teardown.
const CLIENT_STOP_GRACE: Duration = Duration::from_secs(2);
/// Linux fd budget: the server may not grow by more than this many fds.
const FD_GROWTH_BUDGET: u64 = 10;
/// Health gate: one probe every 250 ms, at most 30 s.
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(30);

/// Parsed options of `xtask loadtest`.
#[derive(Debug)]
pub struct Options {
    /// Server binary to launch (default: `target/debug/lc-server`).
    pub server_bin: PathBuf,
    /// Number of overlay WebSocket clients.
    pub clients: usize,
    /// Load window in seconds, after startup.
    pub duration_secs: u64,
    /// Spread `/ingest` POSTs per second.
    pub ingest_rate: u64,
    /// Budget for the hello→welcome p95 latency, in milliseconds.
    pub max_hello_p95_ms: u64,
    /// Where to write the JSON report (always printed to stdout too).
    pub report: Option<PathBuf>,
}

/// Failures of the harness itself (mapped to exit code 1).
#[derive(Debug, Error)]
pub enum LoadtestError {
    #[error("server binary `{}` not found: build it with `cargo build -p lc-server --locked`", .0.display())]
    ServerBinaryMissing(PathBuf),
    #[error("io failure: {0}")]
    Io(#[from] io::Error),
    #[error("database seeding failed: {0}")]
    Seed(String),
    #[error("could not start the server: {0}")]
    Spawn(io::Error),
    #[error("server exited before becoming healthy (status: {0:?})")]
    EarlyExit(Option<ExitStatus>),
    #[error("server never reported a healthy database within 30 s")]
    Unhealthy,
    #[error("health probe error: {0}")]
    Health(String),
    #[error("could not serialize the report: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("could not write the report to `{}`: {}", .0.display(), .1)]
    Report(PathBuf, io::Error),
}

/// Runs the harness. Returns the process exit code: 0 when every budget
/// passes, 1 otherwise (a harness failure also maps to 1).
pub async fn execute(options: Options) -> u8 {
    match run(&options).await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("xtask loadtest: {err}");
            1
        }
    }
}

async fn run(options: &Options) -> Result<u8, LoadtestError> {
    if !options.server_bin.is_file() {
        return Err(LoadtestError::ServerBinaryMissing(
            options.server_bin.clone(),
        ));
    }

    // Fresh database in a throwaway directory: seeding happens before the
    // server process opens the same file through DATABASE_URL.
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("livechat.sqlite");
    let media_dir = tmp.path().join("media");
    seed_database(&db_path, options.clients).await?;

    let port = reserve_port().await?;
    // The config example uses forward slashes in `file:` URLs
    // (`config.rs`: "file:./data/livechat.db"): keep that form on Windows too.
    let database_url = format!(
        "file:{}/livechat.sqlite",
        tmp.path().to_string_lossy().replace('\\', "/")
    );
    let mut guard = ServerGuard {
        child: Some(
            spawn_server(&options.server_bin, port, &database_url, &media_dir)
                .map_err(LoadtestError::Spawn)?,
        ),
    };
    wait_healthy(port, &mut guard).await?;
    let server_pid = guard.pid();

    // The measured window starts now: the ingest poster stops at `end`,
    // the clients linger `CLIENT_STOP_GRACE` past it (checked_add so an
    // absurd `--duration-secs` degrades the window instead of panicking).
    let end = Instant::now()
        .checked_add(Duration::from_secs(options.duration_secs))
        .unwrap_or_else(Instant::now);
    let client_stop = end.checked_add(CLIENT_STOP_GRACE).unwrap_or(end);
    let ws_url = format!("ws://127.0.0.1:{port}/ws");
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let client_handles: Vec<_> = (0..options.clients)
        .map(|index| {
            let token = format!("{OVERLAY_TOKEN_PREFIX}{index:03}");
            let event_tx = event_tx.clone();
            tokio::spawn(run_client(
                index,
                ws_url.clone(),
                token,
                client_stop,
                event_tx,
            ))
        })
        .collect();
    drop(event_tx);
    let ingest_handle = tokio::spawn(run_ingest(
        format!("http://127.0.0.1:{port}"),
        options.ingest_rate,
        end,
    ));

    let fd_sampling = cfg!(target_os = "linux");
    let mut hello_ms: Vec<u64> = Vec::new();
    let mut ws_errors: u64 = 0;
    let mut disconnects: u64 = 0;
    let mut hello_sent: usize = 0;
    let mut welcomes: usize = 0;
    let mut fd_start: Option<u64> = None;
    let mut fd_end: Option<u64> = None;
    let mut fd_due: Option<Instant> = None;

    loop {
        if Instant::now() >= end {
            break;
        }
        // Placeholder when no fd deadline is armed: the branch guard stays
        // disabled, so `end` is never a live fd deadline.
        let fd_deadline = fd_due.unwrap_or(end);
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    None => break,
                    Some(ClientEvent::HelloSent) => {
                        hello_sent += 1;
                        if fd_sampling && fd_start.is_none() && hello_sent == options.clients && welcomes == 0 {
                            // No welcome will ever arm the timer: fall back
                            // to the last HELLO.
                            fd_due = Some(Instant::now() + FD_SAMPLE_DELAY);
                        }
                    }
                    Some(ClientEvent::Welcomed { latency_ms }) => {
                        hello_ms.push(latency_ms);
                        welcomes += 1;
                        if fd_sampling && fd_start.is_none() {
                            // Five seconds after the LAST welcome wins.
                            fd_due = Some(Instant::now() + FD_SAMPLE_DELAY);
                        }
                    }
                    Some(ClientEvent::WsError) => ws_errors += 1,
                    Some(ClientEvent::Disconnected) => disconnects += 1,
                }
            }
            _ = sleep_until(end) => break,
            _ = sleep_until(fd_deadline), if fd_sampling && fd_due.is_some() => {
                fd_start = server_pid.and_then(count_server_fds);
                fd_due = None;
            }
        }
    }

    // Tally any events that raced the end of the window.
    while let Ok(event) = event_rx.try_recv() {
        match event {
            ClientEvent::HelloSent => {}
            ClientEvent::Welcomed { latency_ms } => {
                hello_ms.push(latency_ms);
                welcomes += 1;
            }
            ClientEvent::WsError => ws_errors += 1,
            ClientEvent::Disconnected => disconnects += 1,
        }
    }

    // Sample fd_end NOW, while the clients are still connected (they
    // linger `CLIENT_STOP_GRACE` past the window): sampling after the
    // awaits would compare a torn-down server and void the fd budget.
    // When the fd_start timer never fired inside the window, sample
    // fd_start here as well — both states are the loaded server.
    if fd_sampling {
        if fd_start.is_none() {
            fd_start = server_pid.and_then(count_server_fds);
        }
        fd_end = server_pid.and_then(count_server_fds);
    }

    // Clients stop themselves at `client_stop`; abort any straggler past
    // the grace period (its socket closes on abort either way).
    for mut handle in client_handles {
        if tokio::time::timeout(CLIENT_STOP_GRACE + Duration::from_secs(5), &mut handle)
            .await
            .is_err()
        {
            handle.abort();
        }
    }
    let stats = match ingest_handle.await {
        Ok(stats) => stats,
        Err(_panic_or_abort) => IngestStats {
            sent: 0,
            created: 0,
            rate_limited: 0,
            errors: 1,
        },
    };

    let server_alive = matches!(guard.try_wait(), Ok(None));
    // Never leave the server running: kill it in every path from here on.
    guard.shutdown();

    let mut sorted = hello_ms.clone();
    sorted.sort_unstable();
    let report = Report {
        clients: options.clients,
        duration_secs: options.duration_secs,
        hello_p50_ms: percentile(&sorted, 50.0),
        hello_p95_ms: percentile(&sorted, 95.0),
        hello_max_ms: sorted.last().copied().unwrap_or(0),
        ws_errors,
        disconnects,
        ingest_sent: stats.sent,
        ingest_created: stats.created,
        ingest_rate_limited: stats.rate_limited,
        ingest_errors: stats.errors,
        server_alive,
        fd_start,
        fd_end,
    };

    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(report_path) = &options.report {
        std::fs::write(report_path, format!("{report_json}\n"))
            .map_err(|err| LoadtestError::Report(report_path.clone(), err))?;
    }
    println!("{report_json}");
    println!();

    let mut failures = 0;
    let passed = report.hello_p95_ms <= options.max_hello_p95_ms;
    budget(
        "hello_p95_ms",
        passed,
        format!(
            "{} ms <= {} ms",
            report.hello_p95_ms, options.max_hello_p95_ms
        ),
    );
    failures += u32::from(!passed);

    // Second-control hardening (R2): a server that accepts HELLOs but
    // never answers would report zero percentiles and pass; only the
    // welcomed count exposes the stall.
    let passed = welcomes == options.clients;
    budget(
        "welcomed_clients",
        passed,
        format!("{welcomes} == {}", options.clients),
    );
    failures += u32::from(!passed);

    let passed = report.ws_errors == 0;
    budget("ws_errors", passed, format!("{} == 0", report.ws_errors));
    failures += u32::from(!passed);

    let passed = report.disconnects == 0;
    budget(
        "disconnects",
        passed,
        format!("{} == 0", report.disconnects),
    );
    failures += u32::from(!passed);

    let passed = report.ingest_errors == 0;
    budget(
        "ingest_errors",
        passed,
        format!("{} == 0", report.ingest_errors),
    );
    failures += u32::from(!passed);

    let passed = report.ingest_rate_limited == 0;
    budget(
        "ingest_rate_limited",
        passed,
        format!("{} == 0", report.ingest_rate_limited),
    );
    failures += u32::from(!passed);

    let passed = report.server_alive;
    let detail = if report.server_alive {
        "still running at the end".to_owned()
    } else {
        "exited during the run".to_owned()
    };
    budget("server_alive", passed, detail);
    failures += u32::from(!passed);

    if fd_sampling {
        let (fd_passed, detail) = match (report.fd_start, report.fd_end) {
            (Some(start), Some(end_fds)) => (
                end_fds <= start + FD_GROWTH_BUDGET,
                format!("{end_fds} <= {start} + {FD_GROWTH_BUDGET}"),
            ),
            (Some(start), None) => (
                false,
                format!("fd_end unavailable (fd_start {start}); the server exited early"),
            ),
            (None, Some(end_fds)) => (
                false,
                format!("fd_start was never sampled (fd_end {end_fds})"),
            ),
            (None, None) => (false, "fd samples unavailable".to_owned()),
        };
        budget("fd_growth", fd_passed, detail);
        failures += u32::from(!fd_passed);
    }

    Ok(if failures == 0 { 0 } else { 1 })
}

/// Events reported by one overlay client task to the orchestrator.
enum ClientEvent {
    /// The HELLO frame was written to the socket.
    HelloSent,
    /// The first `welcome` frame arrived, `latency_ms` after the HELLO.
    Welcomed { latency_ms: u64 },
    /// Protocol/transport error: failed connect, failed send, non-JSON frame,
    /// or a socket that died before its welcome.
    WsError,
    /// A welcomed client whose socket closed or errored before the end.
    Disconnected,
}

/// Drives one overlay client: connect, HELLO, wait for `welcome`, then keep
/// reading frames while sending a heartbeat every [`HEARTBEAT_INTERVAL_MS`]
/// with the clientId/guildId of the welcome. Stops at `stop` (the window
/// end plus [`CLIENT_STOP_GRACE`]) with a clean close frame, so the fd_end
/// sample at the window exit still sees this socket open.
async fn run_client(
    index: usize,
    url: String,
    token: String,
    stop: Instant,
    events: mpsc::UnboundedSender<ClientEvent>,
) {
    let (mut sink, mut stream) = match connect_async(url).await {
        Ok((socket, _response)) => socket.split(),
        Err(_) => {
            let _ = events.send(ClientEvent::WsError);
            return;
        }
    };

    let hello = hello_envelope(index, &token);
    let sent_at = Instant::now();
    if sink
        .send(Message::Text(hello.to_string().into()))
        .await
        .is_err()
    {
        let _ = events.send(ClientEvent::WsError);
        return;
    }
    let _ = events.send(ClientEvent::HelloSent);

    let mut welcome_client_id: Option<String> = None;
    let mut welcome_guild_id: Option<String> = None;
    let mut heartbeat_seq: u32 = 0;
    let mut next_heartbeat = stop;

    loop {
        let heartbeating = welcome_client_id.is_some();
        tokio::select! {
            _ = sleep_until(stop) => {
                sink.send(Message::Close(None)).await.ok();
                return;
            }
            _ = sleep_until(next_heartbeat), if heartbeating => {
                heartbeat_seq += 1;
                let envelope = heartbeat_envelope(
                    index,
                    heartbeat_seq,
                    welcome_client_id.as_deref().unwrap_or_default(),
                    welcome_guild_id.as_deref().unwrap_or_default(),
                );
                if sink.send(Message::Text(envelope.to_string().into())).await.is_err() {
                    let _ = events.send(ClientEvent::WsError);
                    return;
                }
                next_heartbeat = Instant::now() + Duration::from_millis(HEARTBEAT_INTERVAL_MS);
            }
            frame = stream.next() => {
                let message = match frame {
                    None => {
                        report_socket_end(&events, heartbeating);
                        return;
                    }
                    Some(Err(_)) => {
                        report_socket_end(&events, heartbeating);
                        return;
                    }
                    Some(Ok(message)) => message,
                };
                match message {
                    Message::Text(text) => {
                        let frame: Value = match serde_json::from_str(text.as_str()) {
                            Ok(frame) => frame,
                            Err(_) => {
                                let _ = events.send(ClientEvent::WsError);
                                return;
                            }
                        };
                        let kind = frame.get("type").and_then(Value::as_str);
                        if kind == Some("welcome") && welcome_client_id.is_none() {
                            let client_id = frame.pointer("/payload/clientId").and_then(Value::as_str);
                            let guild_id = frame.pointer("/payload/guildId").and_then(Value::as_str);
                            match (client_id, guild_id) {
                                (Some(client_id), Some(guild_id)) => {
                                    welcome_client_id = Some(client_id.to_owned());
                                    welcome_guild_id = Some(guild_id.to_owned());
                                    let _ = events.send(ClientEvent::Welcomed {
                                        latency_ms: sent_at.elapsed().as_millis() as u64,
                                    });
                                    next_heartbeat = Instant::now()
                                        + Duration::from_millis(HEARTBEAT_INTERVAL_MS);
                                }
                                _ => {
                                    let _ = events.send(ClientEvent::WsError);
                                    return;
                                }
                            }
                        }
                    }
                    Message::Ping(_) | Message::Pong(_) => {
                        // tungstenite queues the pong reply automatically; it
                        // is flushed by this read loop.
                    }
                    Message::Close(_) => {
                        report_socket_end(&events, heartbeating);
                        return;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Reports the end of a socket: a client that never saw its welcome counts as
/// a transport/protocol error, a welcomed client as a disconnect.
fn report_socket_end(events: &mpsc::UnboundedSender<ClientEvent>, had_welcome: bool) {
    let event = if had_welcome {
        ClientEvent::Disconnected
    } else {
        ClientEvent::WsError
    };
    let _ = events.send(event);
}

/// Ingest poster: `POST /ingest` with the seeded bearer token, spread at
/// `rate_per_sec` requests per second until `end`.
async fn run_ingest(base_url: String, rate_per_sec: u64, end: Instant) -> IngestStats {
    let mut stats = IngestStats::default();
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            stats.errors += 1;
            return stats;
        }
    };
    let mut ticker = interval(Duration::from_secs_f64(1.0 / rate_per_sec as f64));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // the first tick completes immediately: skip it
    let mut counter: u64 = 0;
    loop {
        if Instant::now() >= end {
            break;
        }
        tokio::select! {
            _ = ticker.tick() => {
                counter += 1;
                stats.sent += 1;
                let request = client
                    .post(format!("{base_url}/ingest"))
                    .bearer_auth(INGEST_TOKEN)
                    .json(&json!({ "text": format!("load {counter}") }));
                match request.send().await {
                    Ok(response) => match response.status().as_u16() {
                        201 => stats.created += 1,
                        429 => stats.rate_limited += 1,
                        _ => stats.errors += 1,
                    },
                    Err(_) => stats.errors += 1,
                }
            }
            _ = sleep_until(end) => break,
        }
    }
    stats
}

/// Tallies of the ingest poster.
#[derive(Default)]
struct IngestStats {
    sent: u64,
    created: u64,
    rate_limited: u64,
    errors: u64,
}

/// The JSON report written to `--report` and printed to stdout.
#[derive(Serialize)]
struct Report {
    clients: usize,
    duration_secs: u64,
    hello_p50_ms: u64,
    hello_p95_ms: u64,
    hello_max_ms: u64,
    ws_errors: u64,
    disconnects: u64,
    ingest_sent: u64,
    ingest_created: u64,
    ingest_rate_limited: u64,
    ingest_errors: u64,
    server_alive: bool,
    fd_start: Option<u64>,
    fd_end: Option<u64>,
}

/// Prints one budget verdict line in the style of `verify`.
fn budget(name: &str, passed: bool, detail: String) {
    let label = if passed { "PASS" } else { "FAIL" };
    println!("{label} {name} ({detail})");
}

/// Nearest-rank percentile of a slice already sorted in ascending order.
///
/// Definition: `rank = ceil(p / 100 × len)` clamped to `1..=len`, and the
/// result is the value at that 1-based rank. An empty slice yields 0 so a
/// fully failed run reports zeros instead of failing to render.
fn percentile(sorted_ms: &[u64], p: f64) -> u64 {
    if sorted_ms.is_empty() {
        return 0;
    }
    let rank = ((p / 100.0) * sorted_ms.len() as f64).ceil().max(1.0) as usize;
    let rank = rank.min(sorted_ms.len());
    sorted_ms[rank - 1]
}

/// Reserves an ephemeral TCP port for the server (bind to `:0`, read the
/// assigned port, drop the listener).
async fn reserve_port() -> Result<u16, LoadtestError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

/// Seeds the load-test database: one guild per 16-client shard (the server's
/// per-guild socket cap), `clients` overlay clients with their tokens, and
/// the ingest author member plus ingest client in the first guild.
async fn seed_database(db_path: &Path, clients: usize) -> Result<(), LoadtestError> {
    let db = lc_db::Db::open(db_path, &lc_db::DbConfig::default())
        .await
        .map_err(|err| LoadtestError::Seed(err.to_string()))?;
    let now = UtcDateTime::now();
    let guild_count = clients.div_ceil(SERVER_MAX_SOCKETS_PER_GUILD).max(1);
    let mut guild_ids = Vec::with_capacity(guild_count);
    for index in 0..guild_count {
        let raw_id = format!("{}", BASE_GUILD_ID + index as u64);
        let guild_id =
            GuildId::parse(&raw_id).map_err(|err| LoadtestError::Seed(err.to_string()))?;
        let guild = Guild {
            id: guild_id.clone(),
            name: Some("Load Guild".to_owned()),
            busy_until: None,
            default_media_time: None,
            max_media_time: None,
            display_media_full: false,
        };
        db.guilds()
            .upsert(&guild)
            .await
            .map_err(|err| LoadtestError::Seed(err.to_string()))?;
        guild_ids.push(guild_id);
    }

    for index in 0..clients {
        let token = format!("{OVERLAY_TOKEN_PREFIX}{index:03}");
        let overlay = OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: guild_ids[index / SERVER_MAX_SOCKETS_PER_GUILD].clone(),
            label: "load-board".to_owned(),
            token_hash: hash_token(&token),
            session_mode: OverlaySessionMode::Normal,
            default_author_name: Some("Load".to_owned()),
            default_author_image: None,
            created_by_discord_user_id: None,
            last_seen_at: None,
            created_at: now,
            revoked_at: None,
        };
        db.overlay_clients()
            .create(&overlay)
            .await
            .map_err(|err| LoadtestError::Seed(err.to_string()))?;
    }

    let member = GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild_ids[0].clone(),
        display_name: "Alice".to_owned(),
        avatar: None,
        role: GuildRole::Member,
        created_at: now,
        removed_at: None,
        discord_user_id: Some("42".to_owned()),
        profile_custom: false,
        discord_synced_at: None,
    };
    db.guild_members()
        .create(&member)
        .await
        .map_err(|err| LoadtestError::Seed(err.to_string()))?;
    let ingest = IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild_ids[0].clone(),
        label: "Load Ingest".to_owned(),
        token_hash: hash_token(INGEST_TOKEN),
        default_author_name: "LiveChat Extension".to_owned(),
        default_author_image: None,
        created_by_discord_user_id: "42".to_owned(),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    };
    db.ingest_clients()
        .create(&ingest)
        .await
        .map_err(|err| LoadtestError::Seed(err.to_string()))?;
    Ok(())
}

/// SHA-256 hex digest of a pairing token, matching the server's
/// `ws/hub.rs::hash_token` and `media/tokens.rs::hash_ingest_token`
/// (lowercase hex; no `hex` crate in xtask).
fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// HELLO frame in the v2 wire format (`ws/tests.rs` envelope shape).
fn hello_envelope(index: usize, token: &str) -> Value {
    json!({
        "v": PROTOCOL_VERSION,
        "type": "hello",
        "id": format!("load-hello-{index}"),
        "ts": now_ms(),
        "payload": {
            "token": token,
            "appVersion": APP_VERSION,
            "platform": PLATFORM,
        }
    })
}

/// Heartbeat frame in the v2 wire format.
fn heartbeat_envelope(index: usize, seq: u32, client_id: &str, guild_id: &str) -> Value {
    json!({
        "v": PROTOCOL_VERSION,
        "type": "heartbeat",
        "id": format!("load-heartbeat-{index}-{seq}"),
        "ts": now_ms(),
        "payload": {
            "clientId": client_id,
            "guildId": guild_id,
            "appVersion": APP_VERSION,
        }
    })
}

/// Spawns the server binary with the load-test environment.
///
/// `NODE_ENV=development` is required: the loopback `/ingest` calls are
/// exempt from the rate limiter only in development mode.
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
        .spawn()
}

/// Waits until `GET /health` reports `"db": "ok"`, polling every 250 ms for
/// at most 30 s. Fails early if the child process exits first.
async fn wait_healthy(port: u16, guard: &mut ServerGuard) -> Result<(), LoadtestError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|err| LoadtestError::Health(err.to_string()))?;
    let health_url = format!("http://127.0.0.1:{port}/health");
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = guard.try_wait() {
            return Err(LoadtestError::EarlyExit(Some(status)));
        }
        let probe = client.get(&health_url).send().await;
        if let Ok(response) = probe {
            if let Ok(payload) = response.json::<Value>().await {
                if payload.get("db").and_then(Value::as_str) == Some("ok") {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
    if let Ok(Some(status)) = guard.try_wait() {
        return Err(LoadtestError::EarlyExit(Some(status)));
    }
    Err(LoadtestError::Unhealthy)
}

/// Counts the open file descriptors of a process through `/proc`
/// (Linux only; `None` on any other platform or when the process is gone).
fn count_server_fds(pid: u32) -> Option<u64> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let count = entries.filter_map(|entry| entry.ok()).count() as u64;
    Some(count)
}

/// Owns the server child: `Drop` kills it so no orphan survives any early
/// return or panic, in every exit path.
struct ServerGuard {
    child: Option<Child>,
}

impl ServerGuard {
    fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self.child.as_mut() {
            Some(child) => child.try_wait(),
            None => Ok(None),
        }
    }

    fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().ok();
            child.wait().ok();
        }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn percentile_empty_slice_is_zero() {
        let empty: [u64; 0] = [];
        assert_eq!(percentile(&empty, 50.0), 0);
        assert_eq!(percentile(&empty, 95.0), 0);
    }

    #[test]
    fn percentile_single_element() {
        assert_eq!(percentile(&[7], 50.0), 7);
        assert_eq!(percentile(&[7], 95.0), 7);
        assert_eq!(percentile(&[7], 100.0), 7);
    }

    #[test]
    fn percentile_nearest_rank_p50() {
        // rank = ceil(0.50 × 4) = 2 → second value of the sorted slice.
        assert_eq!(percentile(&[10, 20, 30, 40], 50.0), 20);
    }

    #[test]
    fn percentile_nearest_rank_p95_of_twenty() {
        // Hand-computed rank: ceil(0.95 × 20) = ceil(19.0) = 19 → the 19th
        // smallest value, which is 19 (not the maximum 20).
        let values: Vec<u64> = (1..=20).collect();
        assert_eq!(percentile(&values, 95.0), 19);
    }

    #[test]
    fn percentile_boundary_ranks() {
        assert_eq!(percentile(&[10, 20, 30, 40], 100.0), 40);
        // A rank below 1 clamps to the first value.
        assert_eq!(percentile(&[10, 20, 30, 40], 0.0), 10);
    }

    #[test]
    fn hash_token_matches_known_sha256() {
        // sha256("abc") in lowercase hex: pins the exact server-side format.
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hello_envelope_matches_wire_format() {
        let envelope = hello_envelope(0, "load-overlay-000");
        assert_eq!(envelope["v"], PROTOCOL_VERSION);
        assert_eq!(envelope["type"], "hello");
        assert_eq!(envelope["id"], "load-hello-0");
        assert!(envelope["ts"].as_i64().is_some());
        assert_eq!(envelope["payload"]["token"], "load-overlay-000");
        assert_eq!(envelope["payload"]["appVersion"], "3.0.0");
        assert_eq!(envelope["payload"]["platform"], "windows");
    }

    #[test]
    fn heartbeat_envelope_matches_wire_format() {
        let envelope = heartbeat_envelope(3, 2, "client-7", "123456789012345678");
        assert_eq!(envelope["v"], PROTOCOL_VERSION);
        assert_eq!(envelope["type"], "heartbeat");
        assert_eq!(envelope["payload"]["clientId"], "client-7");
        assert_eq!(envelope["payload"]["guildId"], "123456789012345678");
        assert_eq!(envelope["payload"]["appVersion"], "3.0.0");
    }
}
