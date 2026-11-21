//! Integration tests for the live admin stream and the hot runtime settings.
//!
//! They reuse the harness of [`super::tests`] (full application router, one
//! in-memory database and one admin session) and exercise
//! `docs/rust/admin-console-api.md` sections 7 and 8.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body, BodyDataStream};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use futures_util::StreamExt as _;
use lc_domain::{Guild, GuildId, OverlayClient, OverlayClientId, OverlaySessionMode};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use time::UtcDateTime;
use tower::ServiceExt as _;

use crate::ingestions::{IngestionEntry, IngestionStatus};

use super::tests::{Harness, TOKEN};

/// Bearer token of the overlay client inserted by the hot runtime tests.
const OVERLAY_TOKEN: &str = "7777777777777777777777777777777777777777777777777777777777777777";

/// Reads named SSE events from a response body.
struct SseReader {
    stream: BodyDataStream,
    buffer: String,
}

impl SseReader {
    fn new(stream: BodyDataStream) -> Self {
        Self {
            stream,
            buffer: String::new(),
        }
    }

    /// Returns the next named event carrying JSON data (comments/keep-alives
    /// are skipped).
    async fn next_event(&mut self) -> (String, Value) {
        loop {
            if let Some(index) = self.buffer.find("\n\n") {
                let block = self.buffer[..index].to_owned();
                self.buffer.drain(..index + 2);
                if let Some((event, data)) = parse_block(&block) {
                    return (event, data);
                }
                continue;
            }

            let chunk = tokio::time::timeout(Duration::from_secs(5), self.stream.next())
                .await
                .expect("SSE chunk timed out")
                .expect("SSE stream ended")
                .expect("SSE body chunk");
            self.buffer.push_str(&String::from_utf8_lossy(&chunk));
        }
    }

    /// Reads until an event named `name` is found (bounded by a deadline).
    async fn next_named(&mut self, name: &str) -> Value {
        for _ in 0..10 {
            let (event, data) = self.next_event().await;
            if event == name {
                return data;
            }
        }
        panic!("no `{name}` event received");
    }
}

/// Parses one SSE block into `(event, JSON data)` when it carries data.
fn parse_block(block: &str) -> Option<(String, Value)> {
    let mut event = "message".to_owned();
    let mut data = String::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event = value.trim().to_owned();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push_str(value.trim_start());
            data.push('\n');
        }
    }
    let data = data.trim();
    if data.is_empty() {
        return None;
    }
    Some((event, serde_json::from_str(data).expect("valid JSON event")))
}

/// Opens `/admin/api/events` with a session and gzip accepted.
async fn open_events(harness: &Harness) -> (StatusCode, HeaderMap, BodyDataStream) {
    let mut request = harness.request(Method::GET, "/admin/api/events");
    request
        .headers_mut()
        .insert(header::ACCEPT_ENCODING, "gzip".parse().unwrap());
    let response = harness
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("SSE response");
    let status = response.status();
    let headers = response.headers().clone();
    (status, headers, response.into_body().into_data_stream())
}

/// Inserts a guild without a `defaultMediaTime` override.
async fn insert_guild_without_override(harness: &Harness, id: &str) {
    let guild = Guild {
        id: GuildId::parse(id).expect("valid guild id"),
        name: Some(format!("Guild {id}")),
        busy_until: None,
        default_media_time: None,
        max_media_time: None,
        display_media_full: false,
    };
    harness
        .db()
        .guilds()
        .upsert(&guild)
        .await
        .expect("guild inserted");
}

/// Overview entry of one guild.
fn guild_entry<'a>(overview: &'a Value, id: &str) -> &'a Value {
    overview["guilds"]
        .as_array()
        .and_then(|guilds| guilds.iter().find(|guild| guild["id"] == id))
        .expect("guild present in the overview")
}

// ---------------------------------------------------------------------------
// GET /admin/api/events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn events_require_a_session() {
    let harness = Harness::new().await;
    let request = harness.request_without_session(Method::GET, "/admin/api/events");
    let (status, _, bytes) = harness.send(request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).expect("JSON body"),
        json!({ "error": "unauthorized" })
    );
}

#[tokio::test]
async fn events_start_with_an_overview_and_are_not_compressed() {
    let harness = Harness::new().await;
    insert_guild_without_override(&harness, "123456789012345678").await;

    let (status, headers, stream) = open_events(&harness).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream")),
        "SSE content type expected: {:?}",
        headers.get(header::CONTENT_TYPE)
    );
    assert!(
        headers.get(header::CONTENT_ENCODING).is_none(),
        "the event stream must not be gzip-compressed"
    );

    let mut reader = SseReader::new(stream);
    let overview = reader.next_named("overview").await;
    assert!(overview["generatedAt"].is_string());
    assert_eq!(
        guild_entry(&overview, "123456789012345678")["settings"]["defaultMediaTime"],
        5
    );
}

#[tokio::test]
async fn a_mutation_triggers_a_debounced_overview_with_the_hot_setting() {
    let harness = Harness::new().await;
    insert_guild_without_override(&harness, "123456789012345678").await;

    let (status, _, stream) = open_events(&harness).await;
    assert_eq!(status, StatusCode::OK);
    let mut reader = SseReader::new(stream);

    let first = reader.next_named("overview").await;
    assert_eq!(
        guild_entry(&first, "123456789012345678")["settings"]["defaultMediaTime"],
        5
    );

    // A mutation changes the overview and the hot default duration at once.
    // The audit event may arrive first; the reader skips it.
    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "defaultDuration": 9 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings"]["defaultDuration"]["value"], 9);

    let second = reader.next_named("overview").await;
    assert_eq!(
        guild_entry(&second, "123456789012345678")["settings"]["defaultMediaTime"],
        9
    );

    // `GET /admin/api/overview` returns the same hot value immediately.
    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        guild_entry(&body, "123456789012345678")["settings"]["defaultMediaTime"],
        9
    );
}

#[tokio::test]
async fn audit_and_ingestion_entries_are_streamed() {
    use crate::ingestions::IngestionErrorInfo;

    let harness = Harness::new().await;
    let (status, _, stream) = open_events(&harness).await;
    assert_eq!(status, StatusCode::OK);
    let mut reader = SseReader::new(stream);
    let _initial = reader.next_named("overview").await;

    // Any admin mutation appends an audit row and publishes it.
    let (status, _) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "ttsEnabled": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let audit = reader.next_named("audit").await;
    assert_eq!(audit["method"], "PATCH");
    assert_eq!(audit["action"], "settings.update");
    assert_eq!(audit["path"], "/admin/api/runtime-settings");
    assert!(audit["id"].as_i64().is_some());

    // A finished ingestion is published when it enters the admin history.
    let guild_id = GuildId::parse("123456789012345678").expect("valid guild id");
    let now = harness.state().clock.now();
    let client = uuid::Uuid::new_v4();
    let ingestion_id = harness
        .state()
        .ingestions
        .start(guild_id, client, now)
        .expect("ingestion started");
    harness.state().ingestions.complete_failure(
        &ingestion_id,
        IngestionErrorInfo {
            code: "unsupported_media".to_owned(),
            message: None,
        },
        now,
    );

    let ingestion = reader.next_named("ingestion").await;
    assert_eq!(ingestion["id"], ingestion_id);
    assert_eq!(ingestion["guildId"], "123456789012345678");
    assert_eq!(ingestion["status"], "failed");
    assert_eq!(ingestion["error"], "unsupported_media");
}

// ---------------------------------------------------------------------------
// GET / PATCH /admin/api/runtime-settings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn runtime_settings_expose_values_sources_and_read_only_settings() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);

    for key in [
        "defaultDuration",
        "mediaCacheTtlHours",
        "mediaCacheMaxTotalMb",
        "mediaBoardMaxTotalMb",
        "mediaMaxSizeMb",
        "mediaVideoMaxHeight",
        "minOverlayVersion",
        "ttsEnabled",
    ] {
        let setting = &body["settings"][key];
        assert!(
            setting.get("value").is_some() && setting.get("default").is_some(),
            "missing shape for {key}: {setting}"
        );
        assert_eq!(setting["source"], "env", "unexpected source for {key}");
    }
    assert_eq!(body["settings"]["defaultDuration"]["value"], 5);
    assert_eq!(body["settings"]["defaultDuration"]["default"], 5);
    assert_eq!(body["settings"]["minOverlayVersion"]["value"], "3.0.0");
    assert_eq!(body["settings"]["ttsEnabled"]["value"], true);

    assert_eq!(body["readOnly"]["port"], 3000);
    assert_eq!(body["readOnly"]["apiUrl"], "http://localhost:3000/");
    assert!(body["readOnly"]["mediaStorageDir"].is_string());
    assert_eq!(body["readOnly"]["databaseConfigured"], true);
    assert_eq!(body["readOnly"]["discordTokenSet"], false);
    assert_eq!(body["readOnly"]["discordClientSecretSet"], false);
    assert_eq!(body["readOnly"]["ingestApiTokenSet"], false);
    assert_eq!(body["readOnly"]["adminSecretSet"], true);
    assert_eq!(body["readOnly"]["adminDiscordUserIds"], 0);
    assert_eq!(body["readOnly"]["adminUiLocalOnly"], true);
    assert_eq!(body["readOnly"]["adminTrustForwardedFor"], false);
    // The TikTok cookie value is never returned, only its presence flag.
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], false);
}

#[tokio::test]
async fn runtime_settings_patch_applies_immediately_and_null_reverts() {
    let harness = Harness::new().await;
    insert_guild_without_override(&harness, "123456789012345678").await;

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({
                "defaultDuration": 9,
                "mediaCacheTtlHours": 3,
                "mediaCacheMaxTotalMb": 64,
                "mediaBoardMaxTotalMb": 128,
                "mediaMaxSizeMb": 32,
                "mediaVideoMaxHeight": 720,
                "minOverlayVersion": "3.5.0",
                "ttsEnabled": false,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updated"], true);

    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings"]["defaultDuration"]["value"], 9);
    assert_eq!(body["settings"]["defaultDuration"]["source"], "runtime");
    assert_eq!(body["settings"]["mediaCacheTtlHours"]["value"], 3);
    assert_eq!(body["settings"]["mediaCacheMaxTotalMb"]["value"], 64);
    assert_eq!(body["settings"]["mediaBoardMaxTotalMb"]["value"], 128);
    assert_eq!(body["settings"]["mediaMaxSizeMb"]["value"], 32);
    assert_eq!(body["settings"]["mediaVideoMaxHeight"]["value"], 720);
    assert_eq!(body["settings"]["minOverlayVersion"]["value"], "3.5.0");
    assert_eq!(body["settings"]["ttsEnabled"]["value"], false);

    // The overview reflects `defaultDuration` without a restart.
    let (_, overview) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(
        guild_entry(&overview, "123456789012345678")["settings"]["defaultMediaTime"],
        9
    );

    // `null` brings every environment value back.
    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({
                "defaultDuration": null,
                "mediaCacheTtlHours": null,
                "mediaCacheMaxTotalMb": null,
                "mediaBoardMaxTotalMb": null,
                "mediaMaxSizeMb": null,
                "mediaVideoMaxHeight": null,
                "minOverlayVersion": null,
                "ttsEnabled": null,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings"]["defaultDuration"]["value"], 5);
    assert_eq!(body["settings"]["defaultDuration"]["source"], "env");
    assert_eq!(body["settings"]["minOverlayVersion"]["value"], "3.0.0");
    assert_eq!(body["settings"]["ttsEnabled"]["value"], true);

    let (_, overview) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(
        guild_entry(&overview, "123456789012345678")["settings"]["defaultMediaTime"],
        5
    );

    // The file only keeps explicit overrides.
    let settings_file = harness.path(super::runtime_settings::SETTINGS_FILE_NAME);
    let contents = std::fs::read_to_string(settings_file).expect("settings file");
    assert!(!contents.contains("defaultDuration"));
}

#[tokio::test]
async fn runtime_settings_reject_invalid_payloads() {
    let harness = Harness::new().await;

    for payload in [
        json!({}),
        json!({ "defaultDuration": "neuf" }),
        json!({ "mediaVideoMaxHeight": -1 }),
        json!({ "mediaVideoMaxHeight": 4_294_967_296_u64 }),
        json!({ "minOverlayVersion": "3.0" }),
        json!({ "ttsEnabled": "oui" }),
        json!({ "unknownSetting": 1 }),
        json!({ "tiktokCookie": 42 }),
    ] {
        let (status, body) = harness
            .call_json(Method::PATCH, "/admin/api/runtime-settings", &payload)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "payload {payload}");
        assert_eq!(body["error"], "invalid_payload", "payload {payload}");
    }
}

#[tokio::test]
async fn runtime_settings_never_expose_secrets() {
    let harness = Harness::with_pairs(&[
        ("DISCORD_TOKEN", "bot-secret-value"),
        ("DISCORD_CLIENT_ID", "123456789012345678"),
        ("DISCORD_CLIENT_SECRET", "oauth-secret-value"),
        ("INGEST_API_TOKEN", "ingest-secret-value"),
        ("TIKTOK_COOKIE", "tiktok-secret-value"),
        ("ADMIN_DISCORD_USER_IDS", "1001,1002"),
    ])
    .await;

    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);
    let serialized = body.to_string();
    for secret in [
        TOKEN,
        "bot-secret-value",
        "oauth-secret-value",
        "ingest-secret-value",
        "tiktok-secret-value",
    ] {
        assert!(
            !serialized.contains(secret),
            "secret {secret} leaked in the runtime settings"
        );
    }

    assert_eq!(body["readOnly"]["discordTokenSet"], true);
    assert_eq!(body["readOnly"]["discordClientSecretSet"], true);
    assert_eq!(body["readOnly"]["ingestApiTokenSet"], true);
    assert_eq!(body["readOnly"]["adminSecretSet"], true);
    assert_eq!(body["readOnly"]["adminDiscordUserIds"], 2);
    assert_eq!(body["hasTikTokCookie"], true);
}

// ---------------------------------------------------------------------------
// Hot settings seen by the overlay routes
// ---------------------------------------------------------------------------

/// Guild used by the overlay clients of these tests.
fn overlay_guild() -> GuildId {
    GuildId::parse("123456789012345678").expect("valid guild id")
}

/// Inserts a normal overlay client authenticated by [`OVERLAY_TOKEN`].
async fn insert_overlay_client(harness: &Harness) -> OverlayClient {
    let client = OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: overlay_guild(),
        label: "hot runtime settings".to_owned(),
        token_hash: format!("{:x}", Sha256::digest(OVERLAY_TOKEN.as_bytes())),
        session_mode: OverlaySessionMode::Normal,
        default_author_name: Some("Alice".to_owned()),
        default_author_image: None,
        created_by_discord_user_id: None,
        last_seen_at: None,
        created_at: UtcDateTime::now(),
        revoked_at: None,
    };
    harness
        .db()
        .overlay_clients()
        .create(&client)
        .await
        .expect("overlay client inserted");
    client
}

/// Sends a raw request through the full router and decodes its JSON body.
async fn send_json_request(harness: &Harness, request: Request<Body>) -> (StatusCode, Value) {
    let response = harness
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("axum response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

/// Piper test double shipped with the overlay tests.
fn fake_piper() -> PathBuf {
    let name = if cfg!(windows) {
        "fake-piper.cmd"
    } else {
        "fake-piper.sh"
    };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/http/overlay/fixtures")
        .join(name)
}

/// PNG-shaped payload of at least `len` bytes (the image pipeline copies it).
fn png_bytes(len: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(len.max(bytes.len()), 0);
    bytes
}

/// Waits for the detached ingestion task to record a final status.
async fn wait_finished(
    harness: &Harness,
    client_id: uuid::Uuid,
    ingestion_id: &str,
) -> IngestionEntry {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(entry) =
            harness
                .state()
                .ingestions
                .get(client_id, ingestion_id, harness.state().clock.now())
        {
            if entry.status != IngestionStatus::Processing {
                return entry;
            }
        }
        assert!(
            Instant::now() < deadline,
            "ingestion {ingestion_id} did not finish in time"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn say_reads_the_hot_default_duration() {
    // `POST /overlay/say` only falls back to the default duration when the
    // ingested media carries no duration (an image). The fake Piper copies a
    // PNG "voice" so the job takes the current `defaultDuration` instead of
    // the audio length, which is the only observable use of the hot setting.
    let voice_dir = TempDir::new().expect("voice directory");
    let voice = voice_dir.path().join("voice.png");
    std::fs::write(&voice, png_bytes(2_048)).expect("voice sample");
    let piper = fake_piper().to_string_lossy().into_owned();
    let voice = voice.to_string_lossy().into_owned();
    let harness = Harness::with_pairs(&[
        ("PIPER_BIN", piper.as_str()),
        ("PIPER_VOICE_FR", voice.as_str()),
    ])
    .await;
    let client = insert_overlay_client(&harness).await;

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "defaultDuration": 9 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings"]["defaultDuration"]["value"], 9);

    let request = Request::builder()
        .method(Method::POST)
        .uri("/overlay/say")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {OVERLAY_TOKEN}"))
        .body(Body::from(
            serde_json::to_vec(&json!({ "text": "Bonjour tout le monde" })).expect("JSON body"),
        ))
        .expect("say request");
    let (status, body) = send_json_request(&harness, request).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["queued"], true);
    assert_eq!(body["durationSec"], 9);

    let jobs = harness
        .db()
        .playback_jobs()
        .list_pending_by_guild(&client.guild_id)
        .await
        .expect("pending jobs");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].duration_sec, 9);
}

#[tokio::test]
async fn media_board_quota_fallback_refuses_an_upload_over_the_hot_limit() {
    let harness = Harness::new().await;
    let client = insert_overlay_client(&harness).await;

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "mediaBoardMaxTotalMb": 1 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings"]["mediaBoardMaxTotalMb"]["value"], 1);

    // 2 MiB is accepted by the media pipeline but no longer fits the board.
    let bytes = png_bytes(2 * 1024 * 1024);
    let request = Request::builder()
        .method(Method::POST)
        .uri("/overlay/media-board/uploads")
        .header(header::AUTHORIZATION, format!("Bearer {OVERLAY_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "filename": "big.png", "size": bytes.len() }).to_string(),
        ))
        .expect("create upload request");
    let (status, body) = send_json_request(&harness, request).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let upload_id = body["uploadId"].as_str().expect("upload id").to_owned();
    let request = Request::builder()
        .method(Method::PUT)
        .uri(format!(
            "/overlay/media-board/uploads/{upload_id}/parts?offset=0"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {OVERLAY_TOKEN}"))
        .body(Body::from(bytes))
        .expect("part request");
    let (status, body) = send_json_request(&harness, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/overlay/media-board/uploads/{upload_id}/complete"))
        .header(header::AUTHORIZATION, format!("Bearer {OVERLAY_TOKEN}"))
        .body(Body::empty())
        .expect("complete request");
    let (status, body) = send_json_request(&harness, request).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().expect("ingestion id");

    let entry = wait_finished(&harness, client.id.as_uuid(), ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Failed, "{entry:?}");
    let error = entry.error.expect("ingestion error");
    assert_eq!(error.code, "media_ingestion_failed");
    assert!(
        error
            .message
            .as_deref()
            .is_some_and(|message| message.contains("limite de 1 Mo")),
        "unexpected message: {error:?}"
    );

    let items = harness
        .db()
        .media_board()
        .list_by_guild(&overlay_guild())
        .await
        .expect("board items");
    assert!(items.is_empty());
}
