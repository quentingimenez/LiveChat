//! Integration tests of the operational admin routes.
//!
//! They reuse the harness of `admin::tests` and inject fake binary version
//! probes: no test ever spawns ffmpeg, yt-dlp or piper.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::http::{header, Method, StatusCode};
use lc_domain::{GuildId, JobStatus};
use serde_json::Value;
use time::{Duration, UtcDateTime};

use super::ops::{BinaryProbe, BinaryProbeResult};
use super::tests::{board_item, media_asset, playback_job, Harness};
use crate::app::router;
use crate::ingestions::IngestionSource;

/// Fake probe: returns a fixed version unless the binary is marked failed.
struct FakeProbe {
    failing: Vec<&'static str>,
}

impl FakeProbe {
    fn healthy() -> Arc<dyn BinaryProbe> {
        Arc::new(Self {
            failing: Vec::new(),
        })
    }

    fn failing(binary: &'static str) -> Arc<dyn BinaryProbe> {
        Arc::new(Self {
            failing: vec![binary],
        })
    }
}

#[async_trait::async_trait]
impl BinaryProbe for FakeProbe {
    async fn probe(&self, binary: &str, _now: time::UtcDateTime) -> BinaryProbeResult {
        if self.failing.contains(&binary) {
            BinaryProbeResult {
                ok: false,
                version: None,
                detail: format!("`{binary}` is not available"),
            }
        } else {
            BinaryProbeResult {
                ok: true,
                version: Some("1.0.0".to_owned()),
                detail: format!("{binary} version 1.0.0"),
            }
        }
    }
}

/// Rebuilds the harness router with an injected binary probe.
fn with_probe(mut harness: Harness, probe: Arc<dyn BinaryProbe>) -> Harness {
    harness.state = harness.state.clone().with_binary_probe(probe);
    harness.app = router(harness.state.clone());
    harness
}

fn check<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["checks"]
        .as_array()
        .and_then(|checks| checks.iter().find(|check| check["id"] == id))
        .unwrap_or_else(|| panic!("check `{id}` missing: {body}"))
}

fn guild_id(value: &str) -> GuildId {
    GuildId::parse(value).expect("valid guild id")
}

#[tokio::test]
async fn health_reports_ok_with_fake_probes() {
    let harness = with_probe(Harness::new().await, FakeProbe::healthy());

    let (status, body) = harness.call(Method::GET, "/admin/api/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["serverVersion"], env!("CARGO_PKG_VERSION"));
    assert!(body["uptimeSec"].as_i64().is_some_and(|value| value >= 0));
    assert_eq!(body["checks"].as_array().map(Vec::len), Some(6));
    assert_eq!(check(&body, "database")["status"], "ok");
    assert_eq!(check(&body, "discord")["status"], "warn");
    assert_eq!(check(&body, "ffmpeg")["status"], "ok");
    assert_eq!(check(&body, "ffmpeg")["version"], "1.0.0");
    assert_eq!(check(&body, "ytdlp")["status"], "ok");
    assert_eq!(check(&body, "piper")["status"], "ok");
    assert_eq!(check(&body, "ingestions")["status"], "ok");
}

#[tokio::test]
async fn health_degrades_when_a_probe_fails() {
    let harness = with_probe(Harness::new().await, FakeProbe::failing("yt-dlp"));

    let (status, body) = harness.call(Method::GET, "/admin/api/health").await;

    assert_eq!(status, StatusCode::OK, "a degraded health stays HTTP 200");
    assert_eq!(body["status"], "degraded");
    assert_eq!(check(&body, "ytdlp")["status"], "error");
    assert_eq!(check(&body, "ffmpeg")["status"], "ok");
}

#[tokio::test]
async fn health_warns_after_more_than_three_ingestion_failures() {
    let harness = with_probe(Harness::new().await, FakeProbe::healthy());
    let registry = Arc::clone(&harness.state.ingestions);
    let now = harness.state.clock.now();
    for _ in 0..4 {
        let id = registry
            .start(
                guild_id("42"),
                uuid::Uuid::new_v4(),
                IngestionSource::link("https://example.invalid/media.png"),
                now,
            )
            .expect("ingestion start");
        registry.complete_failure(
            &id,
            crate::ingestions::IngestionErrorInfo {
                code: "media_ingestion_failed".to_owned(),
                message: None,
            },
            now,
        );
    }

    let (_, body) = harness.call(Method::GET, "/admin/api/health").await;

    assert_eq!(check(&body, "ingestions")["status"], "warn");
    assert_eq!(body["status"], "ok", "a warn check does not degrade health");
}

#[tokio::test]
async fn ingestions_history_keeps_the_source_of_play_once_and_failed_rows() {
    let harness = Harness::new().await;
    let registry = Arc::clone(&harness.state.ingestions);
    let guild = guild_id("42");
    harness
        .db()
        .guilds()
        .upsert(&super::tests::guild("42"))
        .await
        .expect("guild");

    let base = time::macros::utc_datetime!(2026-11-10 12:00:00);
    let client = uuid::Uuid::new_v4();
    let mut ids = Vec::new();
    // A failed download, a play once (which creates no board item) and an
    // upload: none of them leaves a board item the history could join, so the
    // source has to come from the registry itself.
    let sources = [
        IngestionSource::link("https://example.invalid/media.png"),
        IngestionSource::link("https://example.invalid/clip.mp4"),
        IngestionSource::upload("clip.mp4"),
    ];
    for (index, source) in sources.into_iter().enumerate() {
        let started = base + Duration::seconds(i64::try_from(index).unwrap());
        let id = registry
            .start(guild.clone(), client, source, started)
            .expect("ingestion start");
        let finished = started + Duration::seconds(5);
        if index == 0 {
            registry.complete_failure(
                &id,
                crate::ingestions::IngestionErrorInfo {
                    code: "media_ingestion_failed".to_owned(),
                    message: Some("download failed".to_owned()),
                },
                finished,
            );
        } else {
            registry.complete_job(&id, format!("job-{index}"), finished);
        }
        ids.push(id);
    }

    let (status, body) = harness
        .call(Method::GET, "/admin/api/ingestions?limit=2")
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2, "the limit must be applied");
    assert_eq!(items[0]["id"], ids[2], "newest first");
    assert_eq!(items[1]["id"], ids[1]);
    assert_eq!(items[0]["status"], "done");
    assert_eq!(items[0]["guildId"], "42");
    assert_eq!(items[0]["guildName"], "Guilde 42");
    assert_eq!(items[0]["url"], "upload:clip.mp4");
    assert_eq!(items[0]["source"], "upload");
    assert_eq!(items[0]["durationMs"], 5000);
    assert!(items[0]["finishedAt"]
        .as_str()
        .is_some_and(|value| value.ends_with('Z')));

    let (_, body) = harness
        .call(Method::GET, "/admin/api/ingestions?limit=50")
        .await;
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 3);
    assert_eq!(items[1]["id"], ids[1]);
    assert_eq!(items[1]["status"], "done");
    assert_eq!(
        items[1]["url"], "https://example.invalid/clip.mp4",
        "a play once has no board item to resolve the source from"
    );
    assert_eq!(items[1]["source"], "link");
    assert_eq!(items[2]["id"], ids[0]);
    assert_eq!(items[2]["status"], "failed");
    assert_eq!(items[2]["url"], "https://example.invalid/media.png");
    assert_eq!(items[2]["source"], "link");
    assert_eq!(items[2]["error"], "download failed");

    let (_, body) = harness
        .call(Method::GET, "/admin/api/ingestions?limit=1")
        .await;
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], ids[2]);
}

#[tokio::test]
async fn cache_purge_keeps_board_and_queued_media() {
    let harness = Harness::new().await;
    let guild = guild_id("42");

    let board_path = harness.path("media/board.png");
    let queued_path = harness.path("media/queued.png");
    let cached_path = harness.path("media/cached.png");
    std::fs::write(&board_path, b"board").expect("board file");
    std::fs::write(&queued_path, b"queued").expect("queued file");
    std::fs::write(&cached_path, b"cached!").expect("cached file");

    let board_asset = media_asset(5, &board_path);
    let queued_asset = media_asset(6, &queued_path);
    let cached_asset = media_asset(7, &cached_path);
    for asset in [&board_asset, &queued_asset, &cached_asset] {
        harness
            .db()
            .media_assets()
            .insert(asset)
            .await
            .expect("media asset");
    }
    harness
        .db()
        .media_board()
        .create(&board_item(&guild, board_asset.id))
        .await
        .expect("board item");

    let mut job = playback_job(&guild, JobStatus::Pending);
    job.media_asset_id = Some(queued_asset.id);
    harness
        .db()
        .playback_jobs()
        .create(&job)
        .await
        .expect("queued job");

    let (status, body) = harness
        .call(Method::POST, "/admin/api/media/cache/purge")
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removedFiles"], 1);
    assert_eq!(body["freedBytes"], 7);
    assert!(board_path.exists(), "board media is pinned");
    assert!(queued_path.exists(), "queued media is active");
    assert!(!cached_path.exists(), "cached media is purged");
    assert!(harness
        .db()
        .media_assets()
        .get(cached_asset.id)
        .await
        .expect("asset lookup")
        .is_none());
    assert!(harness
        .db()
        .media_assets()
        .get(board_asset.id)
        .await
        .expect("asset lookup")
        .is_some());
    assert!(harness
        .db()
        .media_assets()
        .get(queued_asset.id)
        .await
        .expect("asset lookup")
        .is_some());
}

#[tokio::test]
async fn orphan_cleanup_removes_unreferenced_files_but_keeps_fresh_staged_files() {
    let harness = Harness::new().await;
    let referenced_path = harness.path("media/referenced.png");
    let orphan_path = harness.path("media/orphan.png");
    let staged_path = harness.path("media/staged.tmp-00000001.png");
    std::fs::write(&referenced_path, b"referenced").expect("referenced file");
    std::fs::write(&orphan_path, b"orphan").expect("orphan file");
    std::fs::write(&staged_path, b"staged").expect("staged file");
    harness
        .db()
        .media_assets()
        .insert(&media_asset(10, &referenced_path))
        .await
        .expect("asset");

    let (status, body) = harness
        .call(Method::POST, "/admin/api/media/orphans/clean")
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removedFiles"], 1);
    assert_eq!(body["freedBytes"], 6);
    assert!(referenced_path.exists(), "referenced files are kept");
    assert!(!orphan_path.exists());
    assert!(
        staged_path.exists(),
        "fresh staged files may belong to an in-flight ingestion"
    );
}

/// Opens a file the way a reader that does not share deletion does (ffmpeg,
/// most Windows programs): until it is closed, Windows refuses to delete it.
#[cfg(windows)]
fn hold_open(path: &std::path::Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_SHARE_READ: u32 = 0x1;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .expect("open without delete sharing")
}

#[cfg(windows)]
#[tokio::test]
async fn cache_purge_skips_a_file_still_open_and_purges_the_rest() {
    let harness = Harness::new().await;
    let open_path = harness.path("media/open.png");
    let free_path = harness.path("media/free.png");
    std::fs::write(&open_path, b"open").expect("open file");
    std::fs::write(&free_path, b"free!").expect("free file");
    let open_asset = media_asset(4, &open_path);
    let free_asset = media_asset(5, &free_path);
    for asset in [&open_asset, &free_asset] {
        harness
            .db()
            .media_assets()
            .insert(asset)
            .await
            .expect("media asset");
    }

    let reader = hold_open(&open_path);
    let (status, body) = harness
        .call(Method::POST, "/admin/api/media/cache/purge")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removedFiles"], 1);
    assert_eq!(body["freedBytes"], 5);
    assert!(!free_path.exists(), "the other media is still purged");
    assert!(open_path.exists(), "the open file is left in place");
    for id in [open_asset.id, free_asset.id] {
        assert!(harness
            .db()
            .media_assets()
            .get(id)
            .await
            .expect("asset lookup")
            .is_none());
    }

    // Once closed, the leftover file is an orphan: the cleanup removes it.
    drop(reader);
    let (status, body) = harness
        .call(Method::POST, "/admin/api/media/orphans/clean")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removedFiles"], 1);
    assert!(!open_path.exists());
}

#[cfg(windows)]
#[tokio::test]
async fn orphan_cleanup_skips_a_file_still_open_and_cleans_the_rest() {
    let harness = Harness::new().await;
    let open_path = harness.path("media/open-orphan.png");
    let free_path = harness.path("media/free-orphan.png");
    std::fs::write(&open_path, b"open").expect("open file");
    std::fs::write(&free_path, b"free!").expect("free file");

    let _reader = hold_open(&open_path);
    let (status, body) = harness
        .call(Method::POST, "/admin/api/media/orphans/clean")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removedFiles"], 1);
    assert_eq!(body["freedBytes"], 5);
    assert!(!free_path.exists());
    assert!(open_path.exists());
}

#[tokio::test]
#[allow(clippy::type_complexity)]
async fn audit_row_is_written_for_a_mutation_with_the_actor_but_not_for_a_get() {
    let harness = Harness::new().await;

    let (status, _) = harness
        .call(Method::POST, "/admin/api/guilds/42/stop")
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::OK);

    let rows: Vec<(
        Option<String>,
        Option<String>,
        String,
        String,
        String,
        i64,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT actor_method, actor_name, action, method, path, status, guild_id \
             FROM admin_audit ORDER BY id",
    )
    .fetch_all(harness.db().pool())
    .await
    .expect("audit rows");

    assert_eq!(rows.len(), 1, "GET requests are never audited");
    let (actor_method, actor_name, action, method, path, status, guild) = &rows[0];
    assert_eq!(actor_method.as_deref(), Some("secret"));
    assert_eq!(actor_name.as_deref(), Some("secret"));
    assert_eq!(action, "guild.stop");
    assert_eq!(method, "POST");
    assert_eq!(path, "/admin/api/guilds/42/stop");
    assert_eq!(*status, 200);
    assert_eq!(guild.as_deref(), Some("42"));
}

#[tokio::test]
async fn audit_journals_the_client_resolved_behind_two_declared_proxies() {
    let harness = Harness::with_pairs(&[("TRUSTED_PROXIES", "127.0.0.1, 10.0.0.9")]).await;
    let mut request = harness.request_from(
        Method::POST,
        "/admin/api/guilds/42/stop",
        IpAddr::V4(Ipv4Addr::LOCALHOST),
    );
    // server <- 127.0.0.1 (direct peer) <- 10.0.0.9 (outer proxy) <- client.
    request.headers_mut().insert(
        "x-forwarded-for",
        header::HeaderValue::from_static("198.51.100.7, 10.0.0.9"),
    );

    let (status, _, _) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK);

    let client_ip: Option<String> =
        sqlx::query_scalar("SELECT client_ip FROM admin_audit ORDER BY id DESC LIMIT 1")
            .fetch_one(harness.db().pool())
            .await
            .expect("audit row");

    assert_eq!(client_ip.as_deref(), Some("198.51.100.7"));
}

#[tokio::test]
async fn audit_does_not_label_a_declared_proxy_as_the_client() {
    let harness = Harness::with_pairs(&[("TRUSTED_PROXIES", "127.0.0.1")]).await;
    let request = harness.request_from(
        Method::POST,
        "/admin/api/guilds/42/stop",
        IpAddr::V4(Ipv4Addr::LOCALHOST),
    );

    let (status, _, _) = harness.send(request).await;
    assert_eq!(status, StatusCode::OK);

    let client_ip: Option<String> =
        sqlx::query_scalar("SELECT client_ip FROM admin_audit ORDER BY id DESC LIMIT 1")
            .fetch_one(harness.db().pool())
            .await
            .expect("audit row");

    // The proxy forwarded no client address: journal NULL, never the proxy.
    assert_eq!(client_ip, None);
}

async fn insert_audit(
    harness: &Harness,
    at: i64,
    actor_name: &str,
    action: &str,
    method: &str,
    path: &str,
    guild: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO admin_audit \
            (at, actor_method, actor_name, action, method, path, status, guild_id, client_ip) \
         VALUES (?1, 'discord', ?2, ?3, ?4, ?5, 200, ?6, '127.0.0.1')",
    )
    .bind(at)
    .bind(actor_name)
    .bind(action)
    .bind(method)
    .bind(path)
    .bind(guild)
    .execute(harness.db().pool())
    .await
    .expect("audit insert");
}

#[tokio::test]
async fn audit_query_matches_action_and_exposes_the_id_cursor() {
    let harness = Harness::new().await;
    let now = lc_domain::datetime_to_ms(UtcDateTime::now());
    insert_audit(
        &harness,
        now - 3_000,
        "Alice",
        "overlay.revoke",
        "POST",
        "/admin/api/overlay-clients/abc/revoke",
        None,
    )
    .await;
    insert_audit(
        &harness,
        now - 2_000,
        "Alice",
        "guild.stop",
        "POST",
        "/admin/api/guilds/42/stop",
        Some("42"),
    )
    .await;
    insert_audit(
        &harness,
        now - 1_000,
        "Bob",
        "guild.purge",
        "POST",
        "/admin/api/guilds/42/purge",
        Some("42"),
    )
    .await;

    let (status, body) = harness
        .call(Method::GET, "/admin/api/audit?query=guild&limit=1")
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["action"], "guild.purge");
    assert_eq!(items[0]["actor"]["name"], "Bob");
    assert!(items[0]["at"]
        .as_str()
        .is_some_and(|value| value.ends_with('Z')));
    let cursor = body["nextBefore"]
        .as_str()
        .expect("nextBefore cursor")
        .to_owned();

    let (status, body) = harness
        .call(
            Method::GET,
            &format!("/admin/api/audit?query=guild&before={cursor}"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["action"], "guild.stop");
    assert!(body["nextBefore"].is_null(), "the cursor is exhausted");

    // The guild id and the actor are searchable too.
    let (_, body) = harness.call(Method::GET, "/admin/api/audit?query=42").await;
    assert_eq!(body["items"].as_array().map(Vec::len), Some(2));
    let (_, body) = harness
        .call(Method::GET, "/admin/api/audit?query=Bob")
        .await;
    assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn audit_csv_export_has_the_expected_headers_and_content() {
    let harness = Harness::new().await;
    insert_audit(
        &harness,
        lc_domain::datetime_to_ms(UtcDateTime::now()),
        "Alice",
        "guild.purge",
        "POST",
        "/admin/api/guilds/42/purge",
        Some("42"),
    )
    .await;
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_audit")
        .fetch_one(harness.db().pool())
        .await
        .expect("audit count");

    let (status, headers, bytes) = harness
        .send(harness.request(Method::GET, "/admin/api/audit/export"))
        .await;

    assert_eq!(status, StatusCode::OK);
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(content_type.starts_with("text/csv"));
    let disposition = headers
        .get(axum::http::header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(disposition.starts_with("attachment"));
    let csv = String::from_utf8(bytes).expect("CSV is UTF-8");
    assert!(
        csv.starts_with("id,at,actorMethod,actorName,action,method,path,status,guildId,clientIp\n")
    );
    assert!(csv.contains("guild.purge"));
    assert!(csv.contains("Alice"));

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_audit")
        .fetch_one(harness.db().pool())
        .await
        .expect("audit count");
    assert_eq!(after, before, "the CSV export is a GET and is not audited");
}

#[tokio::test]
async fn operational_routes_require_a_session() {
    let harness = Harness::new().await;

    for (method, path) in [
        (Method::GET, "/admin/api/health"),
        (Method::GET, "/admin/api/ingestions"),
        (Method::POST, "/admin/api/media/cache/purge"),
        (Method::POST, "/admin/api/media/orphans/clean"),
        (Method::GET, "/admin/api/audit"),
        (Method::GET, "/admin/api/audit/export"),
    ] {
        let (status, _, bytes) = harness
            .send(harness.request_without_session(method.clone(), path))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "route {method} {path}");
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        assert_eq!(body["error"], "unauthorized");
    }
}
