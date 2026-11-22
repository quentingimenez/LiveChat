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
    future::Future,
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
    #[error("could not serialize the report: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("could not write the report to `{}`: {}", .0.display(), .1)]
    Report(PathBuf, io::Error),
}

/// Runs the harness. Returns the process exit code: 0 when every budget
/// passes, 1 otherwise (a harness failure is reported by the caller).
pub(crate) async fn run(options: &Options) -> Result<u8, LoadtestError> {
    check_server_binary(&options.server_bin)?;

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
    // Readiness gate: probe /health until the database is reported healthy,
    // the child exits, or HEALTH_TIMEOUT elapses.
    let deadline = health_deadline(Instant::now());
    let probe_client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{port}/health");
    let probe = || {
        let client = probe_client.clone();
        let url = health_url.clone();
        async move {
            // `Client::new()` cannot fail for this harness: its only error
            // mode is a TLS/rustls backend init failure, and the old builder
            // path mapped that to a `Health` error that was unreachable in
            // practice. A transport or JSON error keeps the retry loop
            // running until the deadline, as before.
            match client
                .get(&url)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(response) => match response.json::<Value>().await {
                    Ok(payload) => Ok(is_healthy_body(&payload)),
                    Err(_) => Ok(false),
                },
                Err(_) => Ok(false),
            }
        }
    };
    wait_healthy_with(
        probe,
        || guard.try_wait().ok().flatten(),
        deadline,
        HEALTH_POLL_INTERVAL,
    )
    .await?;
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
    let mut tally = Tally::default();
    let mut fd_start: Option<u64> = None;
    let mut fd_end: Option<u64> = None;
    let mut fd_due: Option<Instant> = None;

    loop {
        if window_has_ended(Instant::now(), end) {
            break;
        }
        // Placeholder when no fd deadline is armed: the branch guard stays
        // disabled, so `end` is never a live fd deadline.
        let fd_deadline = fd_due.unwrap_or(end);
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    None => break,
                    Some(event) => {
                        let was_hello_sent = matches!(&event, ClientEvent::HelloSent);
                        let was_welcomed = matches!(&event, ClientEvent::Welcomed { .. });
                        tally.record(event);
                        if fd_sampling && fd_start.is_none() {
                            if was_welcomed {
                                // Five seconds after the LAST welcome wins.
                                fd_due = Some(Instant::now() + FD_SAMPLE_DELAY);
                            } else if was_hello_sent
                                && tally.hello_sent == options.clients
                                && tally.welcomes == 0
                            {
                                // No welcome will ever arm the timer: fall
                                // back to the last HELLO.
                                fd_due = Some(Instant::now() + FD_SAMPLE_DELAY);
                            }
                        }
                    }
                }
            }
            _ = sleep_until(end) => break,
            _ = sleep_until(fd_deadline), if fd_sampling && fd_due.is_some() => {
                fd_start = server_pid.and_then(|pid| read_fd_count(format!("/proc/{pid}/fd")));
                fd_due = None;
            }
        }
    }

    // Tally any events that raced the end of the window.
    tally.drain(&mut event_rx);

    // Sample fd_end NOW, while the clients are still connected (they
    // linger `CLIENT_STOP_GRACE` past the window): sampling after the
    // awaits would compare a torn-down server and void the fd budget.
    // When the fd_start timer never fired inside the window, sample
    // fd_start here as well — both states are the loaded server.
    if fd_sampling {
        if fd_start.is_none() {
            fd_start = server_pid.and_then(|pid| read_fd_count(format!("/proc/{pid}/fd")));
        }
        fd_end = server_pid.and_then(|pid| read_fd_count(format!("/proc/{pid}/fd")));
    }

    // Clients stop themselves at `client_stop`; abort any straggler past
    // the grace period (its socket closes on abort either way).
    for mut handle in client_handles {
        if tokio::time::timeout(straggler_timeout(), &mut handle)
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

    let mut sorted = tally.hello_ms.clone();
    sorted.sort_unstable();
    let report = Report {
        clients: options.clients,
        duration_secs: options.duration_secs,
        hello_p50_ms: percentile(&sorted, 50.0),
        hello_p95_ms: percentile(&sorted, 95.0),
        hello_max_ms: sorted.last().copied().unwrap_or(0),
        ws_errors: tally.ws_errors,
        disconnects: tally.disconnects,
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

    let lines = budget_lines(&report, options, tally.welcomes, fd_sampling);
    print_budgets(&lines, &mut std::io::stdout())?;
    Ok(exit_code(count_failures(&lines)))
}

/// A single budget verdict, rendered as one `PASS`/`FAIL` line.
#[derive(Debug)]
struct BudgetLine {
    name: &'static str,
    passed: bool,
    detail: String,
}

/// Checks that the configured server binary is a file.
fn check_server_binary(path: &Path) -> Result<(), LoadtestError> {
    if !path.is_file() {
        return Err(LoadtestError::ServerBinaryMissing(path.to_path_buf()));
    }
    Ok(())
}

/// Whether the measured window closed at `now`.
fn window_has_ended(now: Instant, end: Instant) -> bool {
    now >= end
}

/// Health-gate deadline: [`HEALTH_TIMEOUT`] after `now`.
fn health_deadline(now: Instant) -> Instant {
    now + HEALTH_TIMEOUT
}

/// How long a client task may linger past the window before being aborted.
fn straggler_timeout() -> Duration {
    CLIENT_STOP_GRACE + Duration::from_secs(5)
}

/// Tick period of the ingest poster for `rate_per_sec` requests per second.
fn ingest_tick_interval(rate_per_sec: u64) -> Duration {
    Duration::from_secs_f64(1.0 / rate_per_sec as f64)
}

/// Whether a `/health` payload reports a healthy database.
fn is_healthy_body(payload: &Value) -> bool {
    payload.get("db").and_then(Value::as_str) == Some("ok")
}

/// Polls `probe` until it reports healthy, the child exits, or the deadline
/// passes. `child_exit` is consulted before every probe.
async fn wait_healthy_with<P, PF, C>(
    mut probe: P,
    mut child_exit: C,
    deadline: Instant,
    poll: Duration,
) -> Result<(), LoadtestError>
where
    P: FnMut() -> PF,
    PF: Future<Output = Result<bool, LoadtestError>>,
    C: FnMut() -> Option<ExitStatus>,
{
    while !window_has_ended(Instant::now(), deadline) {
        if let Some(status) = child_exit() {
            return Err(LoadtestError::EarlyExit(Some(status)));
        }
        if probe().await? {
            return Ok(());
        }
        tokio::time::sleep(poll).await;
    }
    if let Some(status) = child_exit() {
        return Err(LoadtestError::EarlyExit(Some(status)));
    }
    Err(LoadtestError::Unhealthy)
}

/// Evaluates the documented budgets, in report order.
fn budget_lines(
    report: &Report,
    options: &Options,
    welcomes: usize,
    fd_sampling: bool,
) -> Vec<BudgetLine> {
    let mut lines = Vec::with_capacity(if fd_sampling { 8 } else { 7 });
    lines.push(BudgetLine {
        name: "hello_p95_ms",
        passed: report.hello_p95_ms <= options.max_hello_p95_ms,
        detail: format!(
            "{} ms <= {} ms",
            report.hello_p95_ms, options.max_hello_p95_ms
        ),
    });

    // Second-control hardening (R2): a server that accepts HELLOs but
    // never answers would report zero percentiles and pass; only the
    // welcomed count exposes the stall.
    lines.push(BudgetLine {
        name: "welcomed_clients",
        passed: welcomes == options.clients,
        detail: format!("{welcomes} == {}", options.clients),
    });

    lines.push(BudgetLine {
        name: "ws_errors",
        passed: report.ws_errors == 0,
        detail: format!("{} == 0", report.ws_errors),
    });

    lines.push(BudgetLine {
        name: "disconnects",
        passed: report.disconnects == 0,
        detail: format!("{} == 0", report.disconnects),
    });

    lines.push(BudgetLine {
        name: "ingest_errors",
        passed: report.ingest_errors == 0,
        detail: format!("{} == 0", report.ingest_errors),
    });

    lines.push(BudgetLine {
        name: "ingest_rate_limited",
        passed: report.ingest_rate_limited == 0,
        detail: format!("{} == 0", report.ingest_rate_limited),
    });

    lines.push(BudgetLine {
        name: "server_alive",
        passed: report.server_alive,
        detail: if report.server_alive {
            "still running at the end".to_owned()
        } else {
            "exited during the run".to_owned()
        },
    });

    if fd_sampling {
        let (passed, detail) = match (report.fd_start, report.fd_end) {
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
        lines.push(BudgetLine {
            name: "fd_growth",
            passed,
            detail,
        });
    }

    lines
}

/// Number of failed budget lines.
fn count_failures(lines: &[BudgetLine]) -> u32 {
    lines.iter().filter(|line| !line.passed).count() as u32
}

/// Process exit code of the harness: 0 when every budget passed, else 1.
fn exit_code(failures: u32) -> u8 {
    if failures == 0 {
        0
    } else {
        1
    }
}

/// Prints one `PASS`/`FAIL` budget line per entry.
fn print_budgets(lines: &[BudgetLine], out: &mut impl io::Write) -> io::Result<()> {
    for line in lines {
        writeln!(
            out,
            "{}",
            budget_line(line.name, line.passed, line.detail.clone())
        )?;
    }
    Ok(())
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

/// Running tallies of the events reported by the client tasks.
#[derive(Default)]
struct Tally {
    hello_ms: Vec<u64>,
    hello_sent: usize,
    welcomes: usize,
    ws_errors: u64,
    disconnects: u64,
}

impl Tally {
    /// Folds one client event into the tallies.
    fn record(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::HelloSent => self.hello_sent += 1,
            ClientEvent::Welcomed { latency_ms } => {
                self.hello_ms.push(latency_ms);
                self.welcomes += 1;
            }
            ClientEvent::WsError => self.ws_errors += 1,
            ClientEvent::Disconnected => self.disconnects += 1,
        }
    }

    /// Folds every event that is already queued (the ones that raced the end
    /// of the window).
    fn drain(&mut self, events: &mut mpsc::UnboundedReceiver<ClientEvent>) {
        while let Ok(event) = events.try_recv() {
            self.record(event);
        }
    }
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

/// Outcome of one `/ingest` POST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IngestOutcome {
    /// 201: the server created the message.
    Created,
    /// 429: the rate limiter rejected the request.
    RateLimited,
    /// Any other status, or a transport failure.
    Error,
}

/// Maps an HTTP status to the tally bucket it feeds.
fn classify_ingest_status(status: u16) -> IngestOutcome {
    match status {
        201 => IngestOutcome::Created,
        429 => IngestOutcome::RateLimited,
        _ => IngestOutcome::Error,
    }
}

/// Posts one message to `/ingest` and classifies the result.
async fn post_ingest(client: &reqwest::Client, base_url: &str, counter: u64) -> IngestOutcome {
    let request = client
        .post(format!("{base_url}/ingest"))
        .timeout(Duration::from_secs(5))
        .bearer_auth(INGEST_TOKEN)
        .json(&json!({ "text": format!("load {counter}") }));
    match request.send().await {
        Ok(response) => classify_ingest_status(response.status().as_u16()),
        Err(_) => IngestOutcome::Error,
    }
}

/// Ingest poster: `POST /ingest` with the seeded bearer token, spread at
/// `rate_per_sec` requests per second until `end`.
async fn run_ingest(base_url: String, rate_per_sec: u64, end: Instant) -> IngestStats {
    let mut stats = IngestStats::default();
    // `Client::new()` cannot fail for this harness: its only error mode is a
    // TLS/rustls backend init failure, which the old builder mapped to a
    // counter that was unreachable in practice.
    let client = reqwest::Client::new();
    let mut ticker = interval(ingest_tick_interval(rate_per_sec));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // the first tick completes immediately: skip it
    let mut counter: u64 = 0;
    loop {
        if window_has_ended(Instant::now(), end) {
            break;
        }
        tokio::select! {
            _ = ticker.tick() => {
                counter += 1;
                stats.sent += 1;
                let outcome = post_ingest(&client, &base_url, counter).await;
                stats.record(outcome);
            }
            _ = sleep_until(end) => break,
        }
    }
    stats
}

/// Tallies of the ingest poster.
#[derive(Debug, Default)]
struct IngestStats {
    sent: u64,
    created: u64,
    rate_limited: u64,
    errors: u64,
}

impl IngestStats {
    /// Folds one request outcome into the tallies (`sent` is bumped by the
    /// poster at every tick, before the request).
    fn record(&mut self, outcome: IngestOutcome) {
        match outcome {
            IngestOutcome::Created => self.created += 1,
            IngestOutcome::RateLimited => self.rate_limited += 1,
            IngestOutcome::Error => self.errors += 1,
        }
    }
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

/// Renders one budget verdict line in the style of `verify`.
fn budget_line(name: &str, passed: bool, detail: String) -> String {
    let label = if passed { "PASS" } else { "FAIL" };
    format!("{label} {name} ({detail})")
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
            device_hash: None,
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

/// Counts the entries of a directory — the `/proc/<pid>/fd` directory on
/// Linux — and `None` when it cannot be read (any other platform, or a
/// process that is already gone).
fn read_fd_count(dir: impl AsRef<Path>) -> Option<u64> {
    let entries = std::fs::read_dir(dir).ok()?;
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

    #[test]
    fn report_socket_end_classifies_the_socket_end() {
        let (events, mut received) = mpsc::unbounded_channel();
        report_socket_end(&events, true);
        assert!(matches!(received.try_recv(), Ok(ClientEvent::Disconnected)));
        assert!(
            received.try_recv().is_err(),
            "exactly one event must be sent per socket end"
        );

        let (events, mut received) = mpsc::unbounded_channel();
        report_socket_end(&events, false);
        assert!(matches!(received.try_recv(), Ok(ClientEvent::WsError)));
        assert!(
            received.try_recv().is_err(),
            "exactly one event must be sent per socket end"
        );
    }

    #[test]
    fn budget_line_renders_pass_and_fail_verdicts() {
        assert_eq!(
            budget_line("hello_p95_ms", true, "42 ms <= 250".to_owned()),
            "PASS hello_p95_ms (42 ms <= 250)"
        );
        assert_eq!(
            budget_line("hello_p95_ms", false, "321 ms <= 250".to_owned()),
            "FAIL hello_p95_ms (321 ms <= 250)"
        );
    }

    #[tokio::test]
    async fn reserve_port_returns_live_ephemeral_ports() {
        let first = reserve_port().await.unwrap();
        let second = reserve_port().await.unwrap();
        for port in [first, second] {
            assert_ne!(port, 0, "an ephemeral port is never 0");
            // The OS assigns ephemeral ports from its dynamic range
            // (>= 32768 on Linux, >= 49152 on Windows): a stubbed port
            // (0 or 1) can never pass the 1024 floor.
            assert!(port > 1024, "port {port} is outside the ephemeral range");
        }
    }

    async fn overlay_client_guild(db: &lc_db::Db, token: &str) -> String {
        sqlx::query_scalar("SELECT guild_id FROM overlay_clients WHERE token_hash = ?")
            .bind(hash_token(token))
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn seed_database_shards_clients_over_guilds_and_hashes_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("livechat.sqlite");
        seed_database(&db_path, 20).await.unwrap();

        let db = lc_db::Db::open(&db_path, &lc_db::DbConfig::default())
            .await
            .unwrap();
        let guilds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guilds")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(
            guilds, 2,
            "20 clients over a 16-socket guild cap => 2 shards"
        );

        let overlay_clients: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM overlay_clients")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(overlay_clients, 20);

        let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guild_members")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(members, 1);

        let ingest_clients: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingest_clients")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(ingest_clients, 1);

        let first_guild = format!("{}", BASE_GUILD_ID);
        let second_guild = format!("{}", BASE_GUILD_ID + 1);

        // Block sharding: the first 16 clients live in the first guild,
        // the last 4 in the second one.
        let mut shards: Vec<(String, i64)> =
            sqlx::query_as("SELECT guild_id, COUNT(*) FROM overlay_clients GROUP BY guild_id")
                .fetch_all(db.pool())
                .await
                .unwrap();
        shards.sort();
        assert_eq!(
            shards,
            vec![(first_guild.clone(), 16), (second_guild.clone(), 4)]
        );

        // The per-client mapping of the sharding: tokens 000..=015 resolve
        // to the first guild, 016..=019 to the second one.
        for index in [0usize, 5, 15] {
            let token = format!("{OVERLAY_TOKEN_PREFIX}{index:03}");
            assert_eq!(
                overlay_client_guild(&db, &token).await,
                first_guild,
                "client {index} must sit in the first shard"
            );
        }
        for index in [16usize, 19] {
            let token = format!("{OVERLAY_TOKEN_PREFIX}{index:03}");
            assert_eq!(
                overlay_client_guild(&db, &token).await,
                second_guild,
                "client {index} must sit in the second shard"
            );
        }

        // The guild display name is seeded.
        let guild_name: Option<String> = sqlx::query_scalar("SELECT name FROM guilds WHERE id = ?")
            .bind(first_guild.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(guild_name.as_deref(), Some("Load Guild"));

        // Both residents of the first guild: the author member and the
        // ingest client.
        let member_guild: String = sqlx::query_scalar("SELECT guild_id FROM guild_members")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(member_guild, first_guild);
        let ingest_guild: String = sqlx::query_scalar("SELECT guild_id FROM ingest_clients")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(ingest_guild, first_guild);

        // Token hashes: client 0 and the ingest client store the SHA-256 of
        // their pairing tokens.
        let overlay_hash: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM overlay_clients WHERE token_hash = ?")
                .bind(hash_token(&format!("{OVERLAY_TOKEN_PREFIX}000")))
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            overlay_hash, 1,
            "client 0 must store the hash of load-overlay-000"
        );

        let ingest_hash: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM ingest_clients WHERE token_hash = ?")
                .bind(hash_token(INGEST_TOKEN))
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(
            ingest_hash, 1,
            "the ingest client must store the hash of its token"
        );

        // Release the SQLite file (and its WAL) before the tempdir drops.
        db.close().await;
    }

    #[tokio::test]
    async fn seed_database_with_one_client_uses_a_single_guild() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("livechat.sqlite");
        seed_database(&db_path, 1).await.unwrap();

        let db = lc_db::Db::open(&db_path, &lc_db::DbConfig::default())
            .await
            .unwrap();
        let guilds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guilds")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(guilds, 1);
        let overlay_clients: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM overlay_clients")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(overlay_clients, 1);
        db.close().await;
    }

    #[tokio::test]
    async fn seed_database_without_clients_still_seeds_one_guild() {
        // The CLI rejects `--clients 0`, but the seeding helper must keep
        // its floor of one guild: the ingest author still needs a home.
        // This is the only client count that pins the `.max(1)` of the
        // guild count (for any count >= 1, div_ceil already yields >= 1).
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("livechat.sqlite");
        seed_database(&db_path, 0).await.unwrap();

        let db = lc_db::Db::open(&db_path, &lc_db::DbConfig::default())
            .await
            .unwrap();
        let guilds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guilds")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(guilds, 1, "the guild count has a floor of 1");
        let overlay_clients: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM overlay_clients")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(overlay_clients, 0);
        db.close().await;
    }

    #[test]
    fn read_fd_count_counts_entries_and_reports_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_fd_count(dir.path()), Some(0));
        std::fs::write(dir.path().join("fd-0"), b"").unwrap();
        std::fs::write(dir.path().join("fd-1"), b"").unwrap();
        std::fs::create_dir(dir.path().join("fd-2")).unwrap();
        assert_eq!(read_fd_count(dir.path()), Some(3));
        let missing = dir.path().join("definitely-not-a-dir");
        assert_eq!(read_fd_count(missing), None);
    }

    #[cfg(windows)]
    fn spawn_exiting_child() -> Child {
        Command::new("cmd")
            .args(["/C", "exit", "0"])
            .spawn()
            .unwrap()
    }

    #[cfg(unix)]
    fn spawn_exiting_child() -> Child {
        Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap()
    }

    #[cfg(windows)]
    fn spawn_sleeping_child() -> Child {
        Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .spawn()
            .unwrap()
    }

    #[cfg(unix)]
    fn spawn_sleeping_child() -> Child {
        Command::new("sleep").args(["30"]).spawn().unwrap()
    }

    #[test]
    fn server_guard_reports_the_pid_and_reaps_a_short_lived_child() {
        let mut guard = ServerGuard {
            child: Some(spawn_exiting_child()),
        };
        let pid = guard.pid().expect("a live child has a pid");
        assert_ne!(pid, 0, "Child::id is never 0");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match guard.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(error) => panic!("try_wait failed: {error}"),
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the short-lived child was not reaped within 2 s"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn server_guard_shutdown_kills_a_living_child_once() {
        let mut guard = ServerGuard {
            child: Some(spawn_sleeping_child()),
        };
        let before = std::time::Instant::now();
        guard.shutdown();
        assert!(
            before.elapsed() < Duration::from_secs(10),
            "shutdown must not wait out the sleeping child"
        );
        assert!(
            guard.pid().is_none(),
            "shutdown takes the child out of the guard"
        );
        assert_eq!(
            guard.try_wait().unwrap(),
            None,
            "an emptied guard reports no child"
        );
        // Idempotent: a second shutdown is a no-op, not a panic.
        guard.shutdown();
        guard.shutdown();
    }

    #[test]
    fn server_guard_without_child_is_inert() {
        let mut guard = ServerGuard { child: None };
        assert!(guard.pid().is_none());
        assert_eq!(guard.try_wait().unwrap(), None);
        guard.shutdown();
        // Reaching the end of the scope drops the guard without a panic;
        // a childless guard has nothing to kill.
    }

    /// Environment switch of the guard-helper branch (see
    /// [`server_guard_drop_kills_the_child_before_it_writes_its_marker`]).
    const GUARD_HELPER_ENV: &str = "LOADTEST_GUARD_HELPER";
    /// Marker path, passed to the helper through its own variable.
    const GUARD_HELPER_MARKER_ENV: &str = "LOADTEST_GUARD_HELPER_MARKER";
    /// Full name of the ignored test that carries the helper branch.
    const GUARD_HELPER_TEST: &str = "loadtest::tests::loadtest_guard_helper";

    fn spawn_guard_helper(marker: &Path) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "--ignored", GUARD_HELPER_TEST])
            .env(GUARD_HELPER_ENV, "1")
            .env(GUARD_HELPER_MARKER_ENV, marker)
            .spawn()
            .unwrap()
    }

    #[test]
    fn server_guard_drop_kills_the_child_before_it_writes_its_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("guard-helper-ran");

        // Positive control: an unguarded helper child does write the
        // marker — otherwise a broken Drop could hide behind a broken
        // marker mechanism.
        let mut unguarded = spawn_guard_helper(&marker);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the unguarded helper never wrote its marker"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        unguarded.kill().ok();
        unguarded.wait().ok();

        // The guarded child is killed by Drop long before its 800 ms sleep
        // ends: no marker may ever appear.
        std::fs::remove_file(&marker).unwrap();
        let guard = ServerGuard {
            child: Some(spawn_guard_helper(&marker)),
        };
        drop(guard);
        std::thread::sleep(Duration::from_secs(2));
        assert!(
            !marker.exists(),
            "Drop must kill the guarded child before it writes its marker"
        );
    }

    /// Helper branch of `server_guard_drop_kills_the_child_before_it_writes_its_marker`:
    /// sleeps 800 ms, writes the marker, exits 0. Ignored by every runner
    /// (nextest, cargo test); only the spawn above ever executes it, gated
    /// by [`GUARD_HELPER_ENV`].
    #[ignore = "guard-helper branch, spawned by the Drop test only"]
    #[test]
    fn loadtest_guard_helper() {
        if std::env::var_os(GUARD_HELPER_ENV).is_none() {
            return;
        }
        let marker = std::env::var(GUARD_HELPER_MARKER_ENV).unwrap();
        std::thread::sleep(Duration::from_millis(800));
        std::fs::write(&marker, b"alive").unwrap();
        std::process::exit(0);
    }

    // ------------------------------------------------------------------
    // Seam tests: helpers extracted out of `run` so every surviving
    // mutation is killed by a deterministic local test.
    // ------------------------------------------------------------------

    use std::io::{Read, Write};
    use std::net::TcpListener as StdTcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_options(server_bin: PathBuf) -> Options {
        Options {
            server_bin,
            clients: 16,
            duration_secs: 10,
            ingest_rate: 10,
            max_hello_p95_ms: 250,
            report: None,
        }
    }

    fn green_report() -> Report {
        Report {
            clients: 16,
            duration_secs: 10,
            hello_p50_ms: 10,
            hello_p95_ms: 100,
            hello_max_ms: 120,
            ws_errors: 0,
            disconnects: 0,
            ingest_sent: 100,
            ingest_created: 100,
            ingest_rate_limited: 0,
            ingest_errors: 0,
            server_alive: true,
            fd_start: Some(10),
            fd_end: Some(20),
        }
    }

    fn line_names(lines: &[BudgetLine]) -> Vec<&'static str> {
        lines.iter().map(|line| line.name).collect()
    }

    fn read_http_request(stream: &mut impl Read) -> Vec<u8> {
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        let mut expected: Option<usize> = None;
        loop {
            if let Some(total) = expected {
                if request.len() >= total {
                    return request;
                }
            } else if let Some(position) =
                request.windows(4).position(|window| window == b"\r\n\r\n")
            {
                let header_len = position + 4;
                let headers = String::from_utf8_lossy(&request[..position]);
                let content_length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                expected = Some(header_len + content_length);
                continue;
            }
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return request,
                Ok(read) => request.extend_from_slice(&chunk[..read]),
            }
        }
    }

    /// Minimal HTTP/1.1 server for the `/ingest` seams: answers the `n`-th
    /// request with `statuses[n % statuses.len()]`, then closes the
    /// connection. Returns the base URL to point the harness at.
    fn spawn_ingest_server(statuses: Vec<u16>) -> String {
        assert!(!statuses.is_empty(), "the fake server needs a status");
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let served = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                drop(read_http_request(&mut stream));
                let index = served.fetch_add(1, Ordering::SeqCst);
                let status = statuses[index % statuses.len()];
                write!(
                    stream,
                    "HTTP/1.1 {status} Status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .ok();
                stream.flush().ok();
            }
        });
        base_url
    }

    #[tokio::test]
    async fn run_reports_a_missing_server_binary() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("definitely-not-a-server-binary");
        let options = test_options(missing.clone());
        let error = run(&options).await.expect_err("a missing binary must fail");
        match error {
            LoadtestError::ServerBinaryMissing(path) => assert_eq!(path, missing),
            other => panic!("expected ServerBinaryMissing, got {other:?}"),
        }
    }

    #[test]
    fn check_server_binary_accepts_a_file_and_rejects_a_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lc-server");
        std::fs::write(&file, b"").unwrap();
        check_server_binary(&file).expect("an existing file must pass");

        let missing = dir.path().join("missing-lc-server");
        match check_server_binary(&missing) {
            Err(LoadtestError::ServerBinaryMissing(path)) => assert_eq!(path, missing),
            other => panic!("expected ServerBinaryMissing, got {other:?}"),
        }

        // A directory is not a file either.
        match check_server_binary(dir.path()) {
            Err(LoadtestError::ServerBinaryMissing(path)) => assert_eq!(path, dir.path()),
            other => panic!("expected ServerBinaryMissing, got {other:?}"),
        }
    }

    #[test]
    fn window_has_ended_only_at_or_after_the_deadline() {
        let base = Instant::now();
        let end = base + Duration::from_millis(10);
        assert!(!window_has_ended(base, end), "strictly before the deadline");
        assert!(
            !window_has_ended(end - Duration::from_millis(1), end),
            "one millisecond before the deadline"
        );
        assert!(window_has_ended(end, end), "exactly at the deadline");
        assert!(
            window_has_ended(end + Duration::from_millis(1), end),
            "after the deadline"
        );
    }

    #[test]
    fn straggler_timeout_is_the_grace_plus_five_seconds() {
        assert_eq!(straggler_timeout(), Duration::from_secs(7));
        assert_eq!(
            straggler_timeout(),
            CLIENT_STOP_GRACE + Duration::from_secs(5)
        );
    }

    #[test]
    fn ingest_tick_interval_divides_one_second_by_the_rate() {
        assert_eq!(ingest_tick_interval(1), Duration::from_secs(1));
        assert_eq!(ingest_tick_interval(2), Duration::from_millis(500));
        assert_eq!(ingest_tick_interval(4), Duration::from_millis(250));
        assert_eq!(ingest_tick_interval(8), Duration::from_millis(125));
    }

    #[test]
    fn health_deadline_is_thirty_seconds_after_now() {
        let now = Instant::now();
        let deadline = health_deadline(now);
        assert_eq!(deadline, now + HEALTH_TIMEOUT);
        assert_eq!(deadline - now, Duration::from_secs(30));
    }

    #[test]
    fn tally_records_every_client_event() {
        let mut tally = Tally::default();
        tally.record(ClientEvent::HelloSent);
        tally.record(ClientEvent::Welcomed { latency_ms: 42 });
        tally.record(ClientEvent::Welcomed { latency_ms: 7 });
        tally.record(ClientEvent::WsError);
        tally.record(ClientEvent::Disconnected);
        assert_eq!(tally.hello_sent, 1);
        assert_eq!(
            tally.hello_ms,
            vec![42, 7],
            "latencies are appended in arrival order"
        );
        assert_eq!(tally.welcomes, 2);
        assert_eq!(tally.ws_errors, 1);
        assert_eq!(tally.disconnects, 1);
    }

    #[test]
    fn tally_drains_events_that_raced_the_window() {
        let (events, mut receiver) = mpsc::unbounded_channel();
        events.send(ClientEvent::HelloSent).unwrap();
        events
            .send(ClientEvent::Welcomed { latency_ms: 5 })
            .unwrap();
        events.send(ClientEvent::WsError).unwrap();
        drop(events);

        let mut tally = Tally::default();
        tally.drain(&mut receiver);
        assert_eq!(tally.hello_sent, 1);
        assert_eq!(tally.hello_ms, vec![5]);
        assert_eq!(tally.welcomes, 1);
        assert_eq!(tally.ws_errors, 1);
        assert_eq!(tally.disconnects, 0);
        assert!(
            receiver.try_recv().is_err(),
            "the queue is empty after a drain"
        );

        // Draining an empty queue is a no-op, not an underflow or a panic.
        tally.drain(&mut receiver);
        assert_eq!(tally.hello_sent, 1);
    }

    #[test]
    fn budget_lines_pass_an_all_green_report() {
        let report = green_report();
        let options = test_options(PathBuf::from("target/debug/lc-server"));

        let with_fd = budget_lines(&report, &options, options.clients, true);
        assert_eq!(
            line_names(&with_fd),
            vec![
                "hello_p95_ms",
                "welcomed_clients",
                "ws_errors",
                "disconnects",
                "ingest_errors",
                "ingest_rate_limited",
                "server_alive",
                "fd_growth",
            ]
        );
        assert!(with_fd.iter().all(|line| line.passed), "{with_fd:?}");
        assert_eq!(count_failures(&with_fd), 0);
        assert_eq!(exit_code(count_failures(&with_fd)), 0);

        let without_fd = budget_lines(&report, &options, options.clients, false);
        assert_eq!(without_fd.len(), 7, "no fd budget off Linux");
        assert_eq!(
            line_names(&without_fd),
            &line_names(&with_fd)[..7],
            "the fd budget is always evaluated last"
        );
        assert!(without_fd.iter().all(|line| line.passed), "{without_fd:?}");
        assert_eq!(count_failures(&without_fd), 0);
    }

    #[test]
    fn budget_lines_flag_each_violated_budget() {
        let options = test_options(PathBuf::from("target/debug/lc-server"));
        let clients = options.clients;

        let mut cases: Vec<(&'static str, Report, usize)> = Vec::new();

        let mut report = green_report();
        report.hello_p95_ms = options.max_hello_p95_ms + 1;
        cases.push(("hello_p95_ms", report, clients));

        cases.push(("welcomed_clients", green_report(), clients - 1));

        let mut report = green_report();
        report.ws_errors = 1;
        cases.push(("ws_errors", report, clients));

        let mut report = green_report();
        report.disconnects = 1;
        cases.push(("disconnects", report, clients));

        let mut report = green_report();
        report.ingest_errors = 1;
        cases.push(("ingest_errors", report, clients));

        let mut report = green_report();
        report.ingest_rate_limited = 1;
        cases.push(("ingest_rate_limited", report, clients));

        let mut report = green_report();
        report.server_alive = false;
        cases.push(("server_alive", report, clients));

        for (name, report, welcomes) in cases {
            for fd_sampling in [true, false] {
                let lines = budget_lines(&report, &options, welcomes, fd_sampling);
                let failed: Vec<&str> = lines
                    .iter()
                    .filter(|line| !line.passed)
                    .map(|line| line.name)
                    .collect();
                assert_eq!(
                    failed,
                    vec![name],
                    "budget {name} (fd_sampling {fd_sampling})"
                );
                assert_eq!(count_failures(&lines), 1);
                assert_eq!(exit_code(count_failures(&lines)), 1);
            }
        }
    }

    #[test]
    fn budget_lines_flag_every_fd_sampling_case() {
        let options = test_options(PathBuf::from("target/debug/lc-server"));
        let clients = options.clients;

        let mut report = green_report();

        // Inside the budget: 20 <= 10 + 10.
        let lines = budget_lines(&report, &options, clients, true);
        let fd = lines.last().unwrap();
        assert_eq!(fd.name, "fd_growth");
        assert!(fd.passed);
        assert_eq!(fd.detail, "20 <= 10 + 10");

        // One fd over the budget.
        report.fd_end = Some(21);
        let lines = budget_lines(&report, &options, clients, true);
        let fd = lines.last().unwrap();
        assert!(!fd.passed, "21 grows past the 10-fd budget");
        assert!(fd.detail.contains("21 <= 10 + 10"), "{}", fd.detail);
        assert_eq!(count_failures(&lines), 1);

        // fd_end missing: the server exited before the end sample.
        report.fd_start = Some(10);
        report.fd_end = None;
        let lines = budget_lines(&report, &options, clients, true);
        let fd = lines.last().unwrap();
        assert!(!fd.passed);
        assert!(
            fd.detail.contains("fd_end unavailable (fd_start 10)"),
            "{}",
            fd.detail
        );

        // fd_start never sampled.
        report.fd_start = None;
        report.fd_end = Some(7);
        let lines = budget_lines(&report, &options, clients, true);
        let fd = lines.last().unwrap();
        assert!(!fd.passed);
        assert!(
            fd.detail.contains("fd_start was never sampled (fd_end 7)"),
            "{}",
            fd.detail
        );

        // Neither sample available.
        report.fd_start = None;
        report.fd_end = None;
        let lines = budget_lines(&report, &options, clients, true);
        let fd = lines.last().unwrap();
        assert!(!fd.passed);
        assert_eq!(fd.detail, "fd samples unavailable");
        assert_eq!(count_failures(&lines), 1);
    }

    #[test]
    fn exit_code_counts_any_failure_as_one() {
        assert_eq!(exit_code(0), 0);
        for failures in [1u32, 2, 7, u32::MAX] {
            assert_eq!(exit_code(failures), 1, "{failures} failure(s)");
        }
    }

    #[test]
    fn print_budgets_renders_one_line_per_verdict() {
        let lines = vec![
            BudgetLine {
                name: "hello_p95_ms",
                passed: true,
                detail: "42 ms <= 250 ms".to_owned(),
            },
            BudgetLine {
                name: "fd_growth",
                passed: false,
                detail: "21 <= 10 + 10".to_owned(),
            },
        ];
        let mut out = Vec::new();
        print_budgets(&lines, &mut out).expect("writing to a Vec cannot fail");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "PASS hello_p95_ms (42 ms <= 250 ms)\nFAIL fd_growth (21 <= 10 + 10)\n"
        );
    }

    #[tokio::test]
    async fn run_client_reports_a_failed_connection() {
        // A port that was just released: the connect is refused immediately.
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let (events, mut receiver) = mpsc::unbounded_channel();
        let handle = tokio::spawn(run_client(
            0,
            format!("ws://127.0.0.1:{port}/ws"),
            "load-overlay-000".to_owned(),
            Instant::now() + Duration::from_secs(5),
            events,
        ));

        let event = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("the client must report its failure within 5 s")
            .expect("the client must send exactly one event");
        assert!(
            matches!(event, ClientEvent::WsError),
            "a refused connection is a ws error"
        );
        handle.await.unwrap();
    }

    #[test]
    fn classify_ingest_status_maps_the_documented_statuses() {
        assert_eq!(classify_ingest_status(201), IngestOutcome::Created);
        assert_eq!(classify_ingest_status(429), IngestOutcome::RateLimited);
        for status in [200u16, 400, 404, 500, 503] {
            assert_eq!(
                classify_ingest_status(status),
                IngestOutcome::Error,
                "status {status}"
            );
        }
    }

    #[tokio::test]
    async fn post_ingest_classifies_a_sequenced_server() {
        let base_url = spawn_ingest_server(vec![201, 429, 500]);
        let client = reqwest::Client::new();
        assert_eq!(
            post_ingest(&client, &base_url, 1).await,
            IngestOutcome::Created
        );
        assert_eq!(
            post_ingest(&client, &base_url, 2).await,
            IngestOutcome::RateLimited
        );
        assert_eq!(
            post_ingest(&client, &base_url, 3).await,
            IngestOutcome::Error
        );
    }

    #[tokio::test]
    async fn post_ingest_reports_a_transport_failure() {
        let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let client = reqwest::Client::new();
        let outcome = post_ingest(&client, &format!("http://127.0.0.1:{port}"), 1).await;
        assert_eq!(outcome, IngestOutcome::Error);
    }

    #[test]
    fn ingest_stats_record_tallies_each_outcome() {
        let mut stats = IngestStats::default();
        assert_eq!(stats.sent, 0, "the poster bumps `sent`, not the tally");
        stats.record(IngestOutcome::Created);
        stats.record(IngestOutcome::RateLimited);
        stats.record(IngestOutcome::Error);
        stats.record(IngestOutcome::Created);
        assert_eq!(stats.created, 2);
        assert_eq!(stats.rate_limited, 1);
        assert_eq!(stats.errors, 1);
        assert_eq!(stats.sent, 0);
    }

    #[tokio::test]
    async fn run_ingest_counts_created_responses() {
        let base_url = spawn_ingest_server(vec![201]);
        let stats = run_ingest(base_url, 50, Instant::now() + Duration::from_millis(300)).await;
        assert!(stats.sent >= 1, "the poster must send at least one request");
        assert_eq!(stats.created, stats.sent);
        assert_eq!(stats.rate_limited, 0);
        assert_eq!(stats.errors, 0);
    }

    #[tokio::test]
    async fn run_ingest_counts_rate_limited_responses() {
        let base_url = spawn_ingest_server(vec![429]);
        let stats = run_ingest(base_url, 50, Instant::now() + Duration::from_millis(300)).await;
        assert!(stats.sent >= 1, "the poster must send at least one request");
        assert_eq!(stats.rate_limited, stats.sent);
        assert_eq!(stats.created, 0);
        assert_eq!(stats.errors, 0);
    }

    #[tokio::test]
    async fn wait_healthy_with_returns_ok_on_the_first_healthy_probe() {
        let calls = Arc::new(AtomicUsize::new(0));
        let probe_calls = Arc::clone(&calls);
        let started = Instant::now();
        let result = wait_healthy_with(
            move || {
                probe_calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok::<bool, LoadtestError>(true))
            },
            || None,
            health_deadline(Instant::now()),
            HEALTH_POLL_INTERVAL,
        )
        .await;
        assert!(
            result.is_ok(),
            "a healthy first probe must pass: {result:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn wait_healthy_with_fails_when_the_engine_never_gets_healthy() {
        let calls = Arc::new(AtomicUsize::new(0));
        let probe_calls = Arc::clone(&calls);
        let started = Instant::now();
        // The outer timeout is a test harness bound, not part of the gate: the
        // gate itself gives up at its deadline (250 ms below). Without it, a
        // deadline check that never fires would hang the whole test run
        // instead of failing it (`window_has_ended -> false` mutant).
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            wait_healthy_with(
                move || {
                    probe_calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok::<bool, LoadtestError>(false))
                },
                || None,
                Instant::now() + Duration::from_millis(250),
                Duration::from_millis(50),
            ),
        )
        .await
        .expect("the readiness gate must give up at its deadline, not poll forever");
        match result {
            Err(LoadtestError::Unhealthy) => {}
            other => panic!("expected Unhealthy, got {other:?}"),
        }
        assert!(
            calls.load(Ordering::SeqCst) >= 2,
            "an unhealthy engine must be retried until the deadline"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn wait_healthy_with_reports_an_early_child_exit() {
        let status = spawn_exiting_child().wait().unwrap();
        let expected_code = status.code();
        let mut child_status = Some(status);
        let calls = Arc::new(AtomicUsize::new(0));
        let probe_calls = Arc::clone(&calls);
        let started = Instant::now();
        let result = wait_healthy_with(
            move || {
                probe_calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok::<bool, LoadtestError>(true))
            },
            move || child_status.take(),
            health_deadline(Instant::now()),
            HEALTH_POLL_INTERVAL,
        )
        .await;
        match result {
            Err(LoadtestError::EarlyExit(Some(reported))) => {
                assert_eq!(reported.code(), expected_code);
            }
            other => panic!("expected EarlyExit with a status, got {other:?}"),
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "the probe must not run when the child already exited"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn is_healthy_body_only_accepts_the_documented_payload() {
        assert!(is_healthy_body(&json!({ "db": "ok" })));
        assert!(is_healthy_body(&json!({ "db": "ok", "version": "3.0.0" })));
        assert!(!is_healthy_body(&json!({ "db": "down" })));
        assert!(!is_healthy_body(&json!({ "db": 1 })));
        assert!(!is_healthy_body(&json!({})));
        assert!(!is_healthy_body(&json!({ "healthy": true })));
        assert!(!is_healthy_body(&Value::Null));
    }
}
