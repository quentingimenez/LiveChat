//! In-process tests for the board uploads sent in parts (`/overlay/media-board/uploads`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use futures_util::stream;
use lc_db::{Db, DbConfig};
use lc_domain::rights::{Action, RightsRole, RightsTable, Surface};
use lc_domain::{
    FixedClock, GuildId, MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode,
};
use lc_protocol::{BoardChange, MediaIngestionPayload, ServerMessage};
use serde_json::{json, Value};
use tempfile::TempDir;
use time::UtcDateTime;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::app::{AppState, MAX_REQUEST_BODY_BYTES};
use crate::config::Config;
use crate::http::admin::hub::AdminHub;
use crate::ingestions::{IngestionEntry, IngestionStatus};
use crate::shutdown::Shutdown;

use super::auth;

pub(super) const TOKEN: &str = "3333333333333333333333333333333333333333333333333333333333333333";
pub(super) const READ_ONLY_TOKEN: &str =
    "4444444444444444444444444444444444444444444444444444444444444444";
const GUILD: &str = "123456789012345678";

pub(super) fn fixed_now() -> UtcDateTime {
    time::macros::utc_datetime!(2026-11-10 12:00:00)
}

pub(super) fn guild() -> GuildId {
    GuildId::parse(GUILD).unwrap()
}

fn test_config(media_dir: &Path, max_size_mb: u64) -> Config {
    Config::from_pairs(config_pairs(media_dir, max_size_mb, false)).unwrap()
}

/// `real_tools` keeps the default `ffmpeg`/`ffprobe` binaries (tests that run
/// them for real); otherwise both point at missing files.
pub(super) fn config_pairs(
    media_dir: &Path,
    max_size_mb: u64,
    real_tools: bool,
) -> Vec<(String, String)> {
    let mut pairs = vec![
        ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
        (
            "MEDIA_URL_SECRET".to_owned(),
            "secret-test-upload".to_owned(),
        ),
        (
            "MEDIA_STORAGE_DIR".to_owned(),
            media_dir.to_string_lossy().into_owned(),
        ),
        ("MEDIA_BOARD_MAX_TOTAL_MB".to_owned(), "64".to_owned()),
        ("MEDIA_MAX_SIZE_MB".to_owned(), max_size_mb.to_string()),
    ];
    if !real_tools {
        for (key, binary) in [
            ("FFPROBE_BINARY", "missing-ffprobe"),
            ("FFMPEG_BINARY", "missing-ffmpeg"),
        ] {
            pairs.push((
                key.to_owned(),
                media_dir.join(binary).to_string_lossy().into_owned(),
            ));
        }
    }
    pairs
}

/// Hub fake recording every `media.ingestion` and `board.changed` frame.
#[derive(Debug, Default)]
pub(super) struct RecordingHub {
    events: Mutex<Vec<Recorded>>,
}

#[derive(Debug, Clone)]
enum Recorded {
    Client(Uuid, ServerMessage),
    Guild(GuildId, ServerMessage),
}

impl RecordingHub {
    pub(super) fn client_messages(&self, client_id: Uuid) -> Vec<ServerMessage> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Recorded::Client(target, message) if *target == client_id => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    pub(super) fn guild_messages(&self, guild_id: &GuildId) -> Vec<ServerMessage> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Recorded::Guild(target, message) if target == guild_id => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    pub(super) fn added_count(&self, guild_id: &GuildId) -> usize {
        self.change_count(guild_id, BoardChange::Added)
    }

    pub(super) fn updated_count(&self, guild_id: &GuildId) -> usize {
        self.change_count(guild_id, BoardChange::Updated)
    }

    fn change_count(&self, guild_id: &GuildId, change: BoardChange) -> usize {
        self.guild_messages(guild_id)
            .iter()
            .filter(|message| {
                matches!(
                    message,
                    ServerMessage::BoardChanged(payload) if payload.change == change
                )
            })
            .count()
    }
}

#[async_trait::async_trait]
impl AdminHub for RecordingHub {
    fn connected_count(&self) -> usize {
        0
    }

    fn connected_for_guild(&self, _guild_id: &GuildId) -> usize {
        0
    }

    fn is_connected(&self, _client_id: &Uuid) -> bool {
        false
    }

    fn close_client(&self, _client_id: &Uuid, _code: u16) -> bool {
        false
    }

    fn close_client_with_reason(&self, _client_id: &Uuid, _code: u16, _reason: &str) -> bool {
        false
    }

    fn send_to_client(&self, client_id: &Uuid, message: ServerMessage) -> bool {
        self.events
            .lock()
            .unwrap()
            .push(Recorded::Client(*client_id, message));
        true
    }

    fn broadcast_to_guild(&self, guild_id: &GuildId, message: ServerMessage) -> usize {
        self.events
            .lock()
            .unwrap()
            .push(Recorded::Guild(guild_id.clone(), message));
        1
    }

    async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
        0
    }

    async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
        0
    }
}

pub(super) async fn harness(
    media_dir: &Path,
    max_size_mb: u64,
) -> (Router, Db, AppState, Arc<RecordingHub>) {
    harness_with(test_config(media_dir, max_size_mb)).await
}

/// Harness running the real `ffmpeg`/`ffprobe` found on the PATH.
pub(super) async fn harness_with_real_tools(
    media_dir: &Path,
    max_size_mb: u64,
) -> (Router, Db, AppState, Arc<RecordingHub>) {
    harness_with(Config::from_pairs(config_pairs(media_dir, max_size_mb, true)).unwrap()).await
}

pub(super) async fn harness_with(config: Config) -> (Router, Db, AppState, Arc<RecordingHub>) {
    let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
    let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
    let hub = Arc::new(RecordingHub::default());
    let state = AppState::new(Arc::new(config), Some(db.clone()), Shutdown::detached())
        .with_clock(clock)
        .with_admin_hub(hub.clone());
    (crate::app::router(state.clone()), db, state, hub)
}

pub(super) async fn insert_client(db: &Db, mode: OverlaySessionMode, token: &str) -> OverlayClient {
    let client = OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild(),
        label: "upload-test".to_owned(),
        token_hash: auth::hash_token(token),
        session_mode: mode,
        default_author_name: Some("Alice".to_owned()),
        default_author_image: None,
        created_by_discord_user_id: None,
        last_seen_at: None,
        created_at: fixed_now(),
        revoked_at: None,
    };
    db.overlay_clients().create(&client).await.unwrap();
    client
}

pub(super) fn create_request(token: &str, description: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/overlay/media-board/uploads")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(description.to_string()))
        .unwrap()
}

pub(super) fn part_request(
    token: &str,
    upload_id: &str,
    offset: usize,
    body: Body,
) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(format!(
            "/overlay/media-board/uploads/{upload_id}/parts?offset={offset}"
        ))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(body)
        .unwrap()
}

fn upload_id_request(method: Method, token: &str, upload_id: &str, suffix: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!("/overlay/media-board/uploads/{upload_id}{suffix}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

pub(super) fn complete_request(token: &str, upload_id: &str) -> Request<Body> {
    upload_id_request(Method::POST, token, upload_id, "/complete")
}

fn cancel_request(token: &str, upload_id: &str) -> Request<Body> {
    upload_id_request(Method::DELETE, token, upload_id, "")
}

/// Opens an upload of `size` bytes and returns its id.
async fn open_upload(app: &Router, token: &str, description: Value) -> String {
    let (status, body) = send_json(app, create_request(token, &description)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["uploadId"].as_str().unwrap().to_owned()
}

/// Sends a file through the three upload steps, `part_size` bytes per part.
/// `size` defaults to the file length. Returns the first refused answer, or
/// the completion answer.
pub(super) async fn upload_parts(
    app: &Router,
    token: &str,
    mut description: Value,
    bytes: &[u8],
    part_size: usize,
) -> (StatusCode, Value) {
    if description.get("size").is_none() {
        description["size"] = json!(bytes.len());
    }
    let (status, body) = send_json(app, create_request(token, &description)).await;
    if status != StatusCode::CREATED {
        return (status, body);
    }
    let upload_id = body["uploadId"].as_str().unwrap().to_owned();
    let mut offset = 0;
    for part in bytes.chunks(part_size) {
        let request = part_request(token, &upload_id, offset, Body::from(part.to_vec()));
        let (status, body) = send_json(app, request).await;
        if status != StatusCode::OK {
            return (status, body);
        }
        offset += part.len();
        assert_eq!(body["received"], offset);
    }
    send_json(app, complete_request(token, &upload_id)).await
}

/// [`upload_parts`] with 64 KiB parts.
pub(super) async fn upload_file(
    app: &Router,
    token: &str,
    description: Value,
    bytes: &[u8],
) -> (StatusCode, Value) {
    upload_parts(app, token, description, bytes, 64 * 1024).await
}

pub(super) fn items_request(token: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/overlay/media-board/items")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

pub(super) async fn send_json(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

/// Waits (bounded) for the detached task to complete the registry entry.
pub(super) async fn wait_finished(
    state: &AppState,
    client_id: Uuid,
    ingestion_id: &str,
) -> IngestionEntry {
    let deadline = Instant::now() + StdDuration::from_secs(60);
    loop {
        if let Some(entry) = state
            .ingestions
            .get(client_id, ingestion_id, state.clock.now())
        {
            if entry.status != IngestionStatus::Processing {
                return entry;
            }
        }
        assert!(
            Instant::now() < deadline,
            "ingestion {ingestion_id} non terminée dans le délai"
        );
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
}

/// Waits (bounded) for the task to publish its WebSocket messages.
pub(super) async fn wait_client_messages(
    hub: &RecordingHub,
    client_id: Uuid,
    at_least: usize,
) -> Vec<ServerMessage> {
    let deadline = Instant::now() + StdDuration::from_secs(5);
    loop {
        let messages = hub.client_messages(client_id);
        if messages.len() >= at_least {
            return messages;
        }
        assert!(
            Instant::now() < deadline,
            "message `media.ingestion` non reçu dans le délai"
        );
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
}

/// Valid 1×1 PNG (68 bytes), below the remote-source minimum size.
pub(super) const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xB5, 0x1C, 0x0C,
    0x02, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x64, 0xF8, 0x0F, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xE3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

/// Minimal PNG-shaped payload; the image pipeline only sniffs/copies it.
fn png_bytes(min_len: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(min_len.max(bytes.len()), 0);
    bytes
}

#[tokio::test]
async fn upload_png_is_ingested_asynchronously_and_reuses_asset() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, hub) = harness(&dir.path().join("media"), 4).await;
    let client = insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    // Sent in parts of 10 bytes: the hash covers the whole file.
    let bytes = TINY_PNG.to_vec();
    let description = json!({"filename": "photo.png"});
    let (status, body) = upload_parts(&app, TOKEN, description.clone(), &bytes, 10).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();

    let entry = wait_finished(&state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    assert_eq!(entry.created, Some(true));
    let first_item_id = entry.item_id.unwrap();

    let messages = wait_client_messages(&hub, client.id.as_uuid(), 1).await;
    let payload: &MediaIngestionPayload = match &messages[0] {
        ServerMessage::MediaIngestion(payload) => payload,
        other => panic!("message inattendu : {other:?}"),
    };
    assert!(payload.ok);
    assert_eq!(payload.ingestion_id, ingestion_id);
    assert_eq!(payload.created, Some(true));
    assert_eq!(payload.item_id.as_deref(), Some(first_item_id.as_str()));
    assert_eq!(hub.added_count(&guild()), 1);

    let (status, body) = send_json(&app, items_request(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["title"], "photo");
    assert_eq!(body["items"][0]["media"]["kind"], "image");
    assert_eq!(body["items"][0]["media"]["mime"], "image/png");
    assert_eq!(body["items"][0]["media"]["sourceUrl"], "upload:photo.png");

    let ready = db
        .media_assets()
        .list_by_status(MediaStatus::Ready, 0)
        .await
        .unwrap();
    assert_eq!(ready.len(), 1);
    assert!(ready[0].source_hash.starts_with("upload:sha256:"));

    // Same bytes in one part: the cached asset is reused and the item is
    // deduplicated.
    let (status, body) = upload_file(&app, TOKEN, description, &bytes).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let duplicate_id = body["ingestionId"].as_str().unwrap().to_owned();
    let duplicate = wait_finished(&state, client.id.as_uuid(), &duplicate_id).await;
    assert_eq!(duplicate.status, IngestionStatus::Ready);
    assert_eq!(duplicate.created, Some(false));
    assert_eq!(duplicate.item_id.as_deref(), Some(first_item_id.as_str()));

    let messages = wait_client_messages(&hub, client.id.as_uuid(), 2).await;
    let duplicate_payload = match &messages[1] {
        ServerMessage::MediaIngestion(payload) => payload,
        other => panic!("message inattendu : {other:?}"),
    };
    assert!(duplicate_payload.ok);
    assert_eq!(duplicate_payload.created, Some(false));
    // No second `board.changed` for an existing item.
    assert_eq!(hub.added_count(&guild()), 1);

    let ready = db
        .media_assets()
        .list_by_status(MediaStatus::Ready, 0)
        .await
        .unwrap();
    assert_eq!(ready.len(), 1, "the cached asset must be reused");
    let (_, body) = send_json(&app, items_request(TOKEN)).await;
    assert_eq!(body["total"], 1);
}

#[tokio::test]
async fn failed_upload_reports_a_code_and_a_readable_message() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, hub) = harness(&dir.path().join("media"), 4).await;
    let client = insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let (status, body) = upload_file(
        &app,
        TOKEN,
        json!({"filename": "note.txt"}),
        b"this is not a media",
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let unsupported_id = body["ingestionId"].as_str().unwrap().to_owned();

    let entry = wait_finished(&state, client.id.as_uuid(), &unsupported_id).await;
    assert_eq!(entry.status, IngestionStatus::Failed);
    assert_eq!(entry.error.as_ref().unwrap().code, "unsupported_media");

    let messages = wait_client_messages(&hub, client.id.as_uuid(), 1).await;
    let payload = match &messages[0] {
        ServerMessage::MediaIngestion(payload) => payload,
        other => panic!("message inattendu : {other:?}"),
    };
    assert!(!payload.ok);
    assert_eq!(payload.ingestion_id, unsupported_id);
    assert_eq!(payload.error.as_ref().unwrap().code, "unsupported_media");
    assert_eq!(hub.added_count(&guild()), 0);

    // A recognisable but rejected media (SVG) keeps the REST error vocabulary
    // and carries the readable pipeline message.
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";
    let (status, body) = upload_file(&app, TOKEN, json!({"filename": "x.svg"}), svg).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let invalid_id = body["ingestionId"].as_str().unwrap().to_owned();
    let entry = wait_finished(&state, client.id.as_uuid(), &invalid_id).await;
    assert_eq!(entry.status, IngestionStatus::Failed);
    let error = entry.error.as_ref().unwrap();
    assert_eq!(error.code, "media_ingestion_failed");
    assert!(
        error
            .message
            .as_deref()
            .is_some_and(|message| !message.is_empty()),
        "message lisible attendu : {error:?}"
    );
    assert_eq!(hub.added_count(&guild()), 0);
}

#[tokio::test]
async fn declared_size_over_the_limit_is_refused_before_any_byte() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 1).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let (status, body) = send_json(
        &app,
        create_request(
            TOKEN,
            &json!({"filename": "big.png", "size": 1024 * 1024 + 1}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        body,
        json!({"error": "payload_too_large", "maxBytes": 1024 * 1024})
    );
    assert!(state.uploads.is_empty());

    for (description, message) in [
        (json!({"filename": "a.png"}), "size is required"),
        (json!({"filename": "a.png", "size": 0}), "file is empty"),
    ] {
        let (status, body) = send_json(&app, create_request(TOKEN, &description)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{description}");
        assert_eq!(body["message"], message);
    }
}

#[tokio::test]
async fn upload_rejects_missing_or_unsafe_filename() {
    let dir = TempDir::new().unwrap();
    let (app, db, _state, _hub) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let (status, body) = send_json(&app, create_request(TOKEN, &json!({"size": 10}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
    assert_eq!(body["message"], "filename is required");

    for filename in ["../x.png", "\u{0}x.png", "x\\y.png"] {
        let description = json!({"filename": filename, "size": 10});
        let (status, body) = send_json(&app, create_request(TOKEN, &description)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{filename}");
        assert_eq!(body["error"], "invalid_payload", "{filename}");
    }
}

#[tokio::test]
async fn read_only_session_is_rejected() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::InviteReadOnly, READ_ONLY_TOKEN).await;

    let description = json!({"filename": "x.png", "size": 10});
    let (status, body) = send_json(&app, create_request(READ_ONLY_TOKEN, &description)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "read_only_session");
    assert!(state.uploads.is_empty());

    // A part is refused before its body is read.
    let polled = Arc::new(AtomicBool::new(false));
    let polled_by_stream = Arc::clone(&polled);
    let body_stream = stream::once(async move {
        polled_by_stream.store(true, Ordering::SeqCst);
        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"not read"))
    });
    let request = part_request(
        READ_ONLY_TOKEN,
        &Uuid::new_v4().to_string(),
        0,
        Body::from_stream(body_stream),
    );
    let (status, body) = send_json(&app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "read_only_session");
    assert!(!polled.load(Ordering::SeqCst), "body must not be polled");
}

#[tokio::test]
async fn parts_accept_over_one_mib_while_json_routes_keep_the_limit() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    let client = insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let bytes = png_bytes(MAX_REQUEST_BODY_BYTES + 2048);
    let (status, body) = upload_parts(
        &app,
        TOKEN,
        json!({"filename": "large.png"}),
        &bytes,
        bytes.len(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();
    let entry = wait_finished(&state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");

    let json_body = vec![b' '; MAX_REQUEST_BODY_BYTES + 1];
    let request = Request::builder()
        .method(Method::POST)
        .uri("/overlay/media-board/items")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json_body))
        .unwrap();
    let (status, body) = send_json(&app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "request_error");
}

#[tokio::test]
async fn a_part_over_the_part_limit_is_refused_and_ends_the_upload() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 200).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;
    let max_part = usize::try_from(super::MAX_PART_BYTES).unwrap();
    let upload_id = open_upload(
        &app,
        TOKEN,
        json!({"filename": "big.mp4", "size": max_part + 1024 * 1024}),
    )
    .await;

    let mebibyte = axum::body::Bytes::from(vec![0_u8; 1024 * 1024]);
    let pieces = max_part / (1024 * 1024) + 1;
    let body_stream =
        stream::iter((0..pieces).map(move |_| Ok::<_, std::io::Error>(mebibyte.clone())));
    let request = part_request(TOKEN, &upload_id, 0, Body::from_stream(body_stream));
    let (status, body) = send_json(&app, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        body,
        json!({"error": "part_too_large", "maxBytes": super::MAX_PART_BYTES})
    );
    assert!(state.uploads.is_empty());
    let (status, _) = send_json(&app, complete_request(TOKEN, &upload_id)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn parts_must_follow_each_other_and_fill_the_declared_size() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;
    let description = json!({"filename": "photo.png", "size": TINY_PNG.len()});
    let upload_id = open_upload(&app, TOKEN, description.clone()).await;

    let (status, body) = send_json(
        &app,
        part_request(TOKEN, &upload_id, 0, Body::from(TINY_PNG[..20].to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["received"], 20);

    // A part sent again, or ahead of the received bytes, is refused without
    // ending the upload.
    for offset in [0, 30] {
        let (status, body) = send_json(
            &app,
            part_request(TOKEN, &upload_id, offset, Body::from(vec![1_u8; 4])),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "offset {offset}");
        assert_eq!(
            body,
            json!({"error": "upload_offset_mismatch", "received": 20})
        );
    }
    let request = Request::builder()
        .method(Method::PUT)
        .uri(format!("/overlay/media-board/uploads/{upload_id}/parts"))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from(vec![1_u8; 4]))
        .unwrap();
    let (status, body) = send_json(&app, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "offset is required");

    // Completing before the declared size was received ends the upload.
    let (status, body) = send_json(&app, complete_request(TOKEN, &upload_id)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "the file was not fully received");
    assert!(state.uploads.is_empty());

    // More bytes than declared end the upload too.
    let upload_id = open_upload(&app, TOKEN, description).await;
    let mut too_long = TINY_PNG.to_vec();
    too_long.push(0);
    let (status, body) = send_json(
        &app,
        part_request(TOKEN, &upload_id, 0, Body::from(too_long)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "more bytes than the declared size");
    assert!(state.uploads.is_empty());
}

#[tokio::test]
async fn an_upload_belongs_to_its_client_and_can_be_cancelled() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;
    let other_token = "5555555555555555555555555555555555555555555555555555555555555555";
    insert_client(&db, OverlaySessionMode::Normal, other_token).await;
    let upload_id = open_upload(
        &app,
        TOKEN,
        json!({"filename": "photo.png", "size": TINY_PNG.len()}),
    )
    .await;

    for request in [
        part_request(other_token, &upload_id, 0, Body::from(TINY_PNG.to_vec())),
        complete_request(other_token, &upload_id),
        cancel_request(other_token, &upload_id),
    ] {
        let (status, body) = send_json(&app, request).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "upload_not_found");
    }
    assert_eq!(state.uploads.len(), 1);

    let (status, _) = send_json(&app, cancel_request(TOKEN, &upload_id)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state.uploads.is_empty());
    let (status, _) = send_json(
        &app,
        part_request(TOKEN, &upload_id, 0, Body::from(TINY_PNG.to_vec())),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn open_uploads_are_limited_per_client_and_expire_when_idle() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;
    let description = json!({"filename": "photo.png", "size": 10});
    for _ in 0..super::media_upload::MAX_SESSIONS_PER_CLIENT {
        open_upload(&app, TOKEN, description.clone()).await;
    }
    let (status, body) = send_json(&app, create_request(TOKEN, &description)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "too_many_uploads");

    let idle = super::media_upload::IDLE_TIMEOUT;
    state
        .uploads
        .purge_idle(Instant::now() + idle - StdDuration::from_secs(5));
    assert_eq!(
        state.uploads.len(),
        super::media_upload::MAX_SESSIONS_PER_CLIENT
    );
    state.uploads.purge_idle(Instant::now() + idle);
    assert!(state.uploads.is_empty());
    open_upload(&app, TOKEN, description).await;
}

/// A second valid PNG: [`TINY_PNG`] followed by bytes ignored after `IEND`.
fn other_png() -> Vec<u8> {
    let mut bytes = TINY_PNG.to_vec();
    bytes.extend_from_slice(b"another picture");
    bytes
}

/// Uploads a file and waits for its ingestion; returns the finished entry.
async fn upload_and_wait(
    app: &Router,
    state: &AppState,
    client: &OverlayClient,
    description: Value,
    bytes: Vec<u8>,
) -> IngestionEntry {
    let (status, body) = upload_file(app, TOKEN, description, &bytes).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();
    wait_finished(state, client.id.as_uuid(), &ingestion_id).await
}

#[tokio::test]
async fn upload_text_is_not_stored_on_the_board() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    let client = insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let entry = upload_and_wait(
        &app,
        &state,
        &client,
        json!({"filename": "photo.png", "title": "Ma photo", "message": "coucou"}),
        TINY_PNG.to_vec(),
    )
    .await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    assert_eq!(entry.created, Some(true));

    let items = db.media_board().list_by_guild(&guild()).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title.as_deref(), Some("Ma photo"));
    assert_eq!(items[0].message, None);
    // Nothing was played: the text did not become a job either.
    assert!(db
        .playback_jobs()
        .list_pending_by_guild(&guild())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn upload_with_replace_item_id_replaces_the_media_of_the_item() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, hub) = harness(&dir.path().join("media"), 4).await;
    let client = insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let first = upload_and_wait(
        &app,
        &state,
        &client,
        json!({"filename": "photo.png", "title": "Ma photo", "message": "coucou"}),
        TINY_PNG.to_vec(),
    )
    .await;
    assert_eq!(first.created, Some(true));
    let item_id = first.item_id.unwrap();
    let (_, body) = send_json(&app, items_request(TOKEN)).await;
    let original_asset = body["items"][0]["mediaAssetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let created_at = body["items"][0]["createdAt"].clone();
    // The text sent with an uploaded media is not a board message.
    assert!(body["items"][0]["message"].is_null());

    // A message written on the board by hand is kept when the media changes.
    let stored = db
        .media_board()
        .get(item_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    db.media_board()
        .update(
            stored.id,
            &lc_db::BoardItemEdit {
                title: stored.title.as_deref(),
                message: Some("coucou"),
                artwork_on_overlay: None,
                display_duration_sec: None,
            },
        )
        .await
        .unwrap();

    // The edited picture replaces the media: same item, same texts.
    let description =
        json!({"filename": "photo-edit.png", "title": "Ignored", "replaceItemId": item_id});
    let replaced = upload_and_wait(&app, &state, &client, description.clone(), other_png()).await;
    assert_eq!(replaced.status, IngestionStatus::Ready, "{replaced:?}");
    assert_eq!(replaced.created, Some(false));
    assert_eq!(replaced.item_id.as_deref(), Some(item_id.as_str()));

    let messages = wait_client_messages(&hub, client.id.as_uuid(), 2).await;
    let payload = match &messages[1] {
        ServerMessage::MediaIngestion(payload) => payload,
        other => panic!("message inattendu : {other:?}"),
    };
    assert!(payload.ok);
    assert_eq!(payload.created, Some(false));
    assert_eq!(payload.item_id.as_deref(), Some(item_id.as_str()));
    assert_eq!(hub.added_count(&guild()), 1);
    assert_eq!(hub.updated_count(&guild()), 1);

    let (_, body) = send_json(&app, items_request(TOKEN)).await;
    assert_eq!(body["total"], 1);
    let item = &body["items"][0];
    assert_eq!(item["id"], item_id.as_str());
    assert_eq!(item["title"], "Ma photo");
    assert_eq!(item["message"], "coucou");
    assert_eq!(item["createdAt"], created_at);
    assert_ne!(item["mediaAssetId"], original_asset.as_str());
    assert_eq!(item["media"]["sourceUrl"], "upload:photo-edit.png");

    // The previous asset is no longer pinned: it expires with the cache.
    let previous = db
        .media_assets()
        .get(original_asset.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(
        previous.expires_at < fixed_now() + time::Duration::days(365),
        "previous asset released: {}",
        previous.expires_at
    );
    let current = db
        .media_assets()
        .get(item["mediaAssetId"].as_str().unwrap().parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(current.expires_at > fixed_now() + time::Duration::days(365));

    // The same bytes again: nothing to replace, no second `board.changed`.
    let same = upload_and_wait(&app, &state, &client, description, other_png()).await;
    assert_eq!(same.status, IngestionStatus::Ready);
    assert_eq!(same.created, Some(false));
    assert_eq!(same.item_id.as_deref(), Some(item_id.as_str()));
    assert_eq!(hub.updated_count(&guild()), 1);
}

#[tokio::test]
async fn replacement_upload_is_refused_for_another_kind_or_a_media_already_on_the_board() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, hub) = harness(&dir.path().join("media"), 4).await;
    let client = insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let first = upload_and_wait(
        &app,
        &state,
        &client,
        json!({"filename": "a.png"}),
        TINY_PNG.to_vec(),
    )
    .await;
    let second = upload_and_wait(
        &app,
        &state,
        &client,
        json!({"filename": "b.png"}),
        other_png(),
    )
    .await;
    let second_id = second.item_id.unwrap();

    // The first picture cannot also become the second item's media.
    let description = json!({"filename": "a.png", "replaceItemId": second_id});
    let refused = upload_and_wait(&app, &state, &client, description, TINY_PNG.to_vec()).await;
    assert_eq!(refused.status, IngestionStatus::Failed);
    let error = refused.error.unwrap();
    assert_eq!(error.code, "media_ingestion_failed");
    assert!(
        error
            .message
            .as_deref()
            .unwrap()
            .contains("déjà sur le board"),
        "{error:?}"
    );

    // A video item cannot receive a picture.
    let video_path = state.config.media_storage_dir.join("clip.mp4");
    tokio::fs::write(&video_path, b"0123456789").await.unwrap();
    let video = lc_domain::MediaAsset {
        id: lc_domain::MediaAssetId::new_v4(),
        source_hash: "hash-clip".to_owned(),
        source_url: "https://example.test/clip.mp4".to_owned(),
        source_title: None,
        kind: lc_domain::MediaKind::Video,
        mime: "video/mp4".to_owned(),
        duration_sec: Some(30),
        width: Some(480),
        height: Some(360),
        is_vertical: false,
        storage_path: video_path.to_string_lossy().into_owned(),
        size_bytes: 10,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: fixed_now(),
        expires_at: fixed_now() + time::Duration::days(36_500),
        created_at: fixed_now(),
    };
    db.media_assets().insert(&video).await.unwrap();
    let video_item = lc_domain::MediaBoardItem {
        id: lc_domain::MediaBoardItemId::new_v4(),
        guild_id: guild(),
        media_asset_id: video.id,
        title: Some("Clip".to_owned()),
        message: None,
        created_by_discord_user_id: None,
        created_by_name: Some("Alice".to_owned()),
        created_at: fixed_now(),
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
    };
    db.media_board().create(&video_item).await.unwrap();
    let description = json!({"filename": "c.png", "replaceItemId": video_item.id.to_canonical()});
    let mut third = other_png();
    third.extend_from_slice(b"third");
    let refused = upload_and_wait(&app, &state, &client, description, third).await;
    assert_eq!(refused.status, IngestionStatus::Failed);
    let error = refused.error.unwrap();
    assert_eq!(error.code, "media_ingestion_failed");
    assert!(
        error.message.as_deref().unwrap().contains("même type"),
        "{error:?}"
    );
    let unchanged = db.media_board().get(video_item.id).await.unwrap().unwrap();
    assert_eq!(unchanged.media_asset_id, video.id);

    assert_eq!(hub.updated_count(&guild()), 0);
    assert_eq!(first.created, Some(true));
}

#[tokio::test]
async fn replacement_target_is_checked_when_the_upload_opens() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    for target in [Uuid::new_v4().to_string(), "not-an-id".to_owned()] {
        let description = json!({"filename": "x.png", "size": 10, "replaceItemId": target});
        let (status, body) = send_json(&app, create_request(TOKEN, &description)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{target}");
        assert_eq!(body["error"], "item_not_found");
    }
    assert!(state.uploads.is_empty());
}

/// « Jouer une fois » par fichier : l'envoi complet crée un job de lecture et
/// aucun item de board ; la guilde n'apprend rien.
#[tokio::test]
async fn play_once_upload_creates_a_job_and_no_item() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, hub) = harness(&dir.path().join("media"), 4).await;
    let client = insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let description = json!({"filename": "photo.png", "title": "Ma photo", "message": "coucou", "playOnce": true});
    let (status, body) = upload_file(&app, TOKEN, description, TINY_PNG).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();

    let entry = wait_finished(&state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    assert_eq!(entry.created, Some(false));
    assert_eq!(entry.item_id, None);
    let job_id = entry.job_id.clone().expect("jobId de l'ingestion");

    // The ingested file became a playback job, carrying the message.
    let pending = db
        .playback_jobs()
        .list_pending_by_guild(&guild())
        .await
        .unwrap();
    let job = pending
        .iter()
        .find(|job| job.id.to_canonical() == job_id)
        .expect("job persisté");
    assert!(job.media_asset_id.is_some());
    assert_eq!(job.text.as_deref(), Some("coucou"));
    assert!(job.show_text);

    // No board item, no `board.changed`.
    assert!(db
        .media_board()
        .list_by_guild(&guild())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(hub.added_count(&guild()), 0);
    assert_eq!(hub.updated_count(&guild()), 0);

    let messages = wait_client_messages(&hub, client.id.as_uuid(), 1).await;
    let payload = match &messages[0] {
        ServerMessage::MediaIngestion(payload) => payload,
        other => panic!("message inattendu : {other:?}"),
    };
    assert!(payload.ok);
    assert_eq!(payload.ingestion_id, ingestion_id);
    assert_eq!(payload.created, Some(false));
    assert!(payload.item_id.is_none());
    assert_eq!(payload.job_id.as_deref(), Some(job_id.as_str()));
}

/// « Jouer une fois » par fichier exige `TriggerPlayback` (403 comme les
/// autres refus), et refuse un `replaceItemId` : rien ne remplacera un item
/// d'un média qui ne rejoindra pas le board.
#[tokio::test]
async fn play_once_upload_checks_the_right_and_refuses_a_replacement() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, _hub) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let mut table = RightsTable::new();
    table.set_override(
        Surface::Overlay,
        Action::TriggerPlayback,
        RightsRole::Member,
        false,
    );
    state.rights.replace(table);
    let description = json!({"filename": "photo.png", "size": 10, "playOnce": true});
    let (status, body) = send_json(&app, create_request(TOKEN, &description)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "forbidden");
    assert!(state.uploads.is_empty());

    let mut table = RightsTable::new();
    table.set_override(
        Surface::Overlay,
        Action::TriggerPlayback,
        RightsRole::Member,
        true,
    );
    state.rights.replace(table);
    let description = json!({
        "filename": "photo.png",
        "size": 10,
        "playOnce": true,
        "replaceItemId": Uuid::new_v4().to_string(),
    });
    let (status, body) = send_json(&app, create_request(TOKEN, &description)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_payload");
    assert_eq!(body["message"], "playOnce cannot replace a board item");
    assert!(state.uploads.is_empty());
}
