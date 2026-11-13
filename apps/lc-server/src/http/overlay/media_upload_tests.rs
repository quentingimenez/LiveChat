//! In-process tests for `POST /overlay/media-board/upload`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, TimeZone, Utc};
use futures_util::stream;
use lc_db::{Db, DbConfig};
use lc_domain::{
    FixedClock, GuildId, MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt as _;

use crate::app::{AppState, MAX_REQUEST_BODY_BYTES};
use crate::config::Config;
use crate::shutdown::Shutdown;

use super::auth;

const TOKEN: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const READ_ONLY_TOKEN: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const GUILD: &str = "123456789012345678";

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
}

fn guild() -> GuildId {
    GuildId::parse(GUILD).unwrap()
}

fn test_config(media_dir: &Path, max_size_mb: u64) -> Config {
    let pairs = vec![
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
        (
            "FFPROBE_BINARY".to_owned(),
            media_dir
                .join("missing-ffprobe")
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "FFMPEG_BINARY".to_owned(),
            media_dir
                .join("missing-ffmpeg")
                .to_string_lossy()
                .into_owned(),
        ),
    ];
    Config::from_pairs(pairs).unwrap()
}

async fn harness(media_dir: &Path, max_size_mb: u64) -> (Router, Db) {
    let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
    let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
    let state = AppState::new(
        Arc::new(test_config(media_dir, max_size_mb)),
        Some(db.clone()),
        Shutdown::detached(),
    )
    .with_clock(clock);
    (crate::app::router(state), db)
}

async fn insert_client(db: &Db, mode: OverlaySessionMode, token: &str) {
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
}

fn upload_request(uri: &str, token: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap()
}

fn items_request(token: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/overlay/media-board/items")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

async fn send_json(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
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

/// Valid 1×1 PNG (68 bytes), below the remote-source minimum size.
const TINY_PNG: &[u8] = &[
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
async fn upload_png_creates_item_and_reuses_asset() {
    let dir = TempDir::new().unwrap();
    let (app, db) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let bytes = TINY_PNG.to_vec();
    let uri = "/overlay/media-board/upload?filename=photo.png";
    let (status, body) = send_json(&app, upload_request(uri, TOKEN, bytes.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["created"], true);
    assert_eq!(body["item"]["title"], "photo");
    assert_eq!(body["item"]["media"]["kind"], "image");
    assert_eq!(body["item"]["media"]["mime"], "image/png");
    assert_eq!(body["item"]["media"]["sourceUrl"], "upload:photo.png");
    let first_item_id = body["item"]["id"].as_str().unwrap().to_owned();

    let (status, body) = send_json(&app, items_request(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["title"], "photo");

    let ready = db
        .media_assets()
        .list_by_status(MediaStatus::Ready, 0)
        .await
        .unwrap();
    assert_eq!(ready.len(), 1);
    assert!(ready[0].source_hash.starts_with("upload:sha256:"));

    let (status, body) = send_json(&app, upload_request(uri, TOKEN, bytes)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["created"], false);
    assert_eq!(body["item"]["id"], first_item_id);

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
async fn upload_over_size_limit_returns_413() {
    let dir = TempDir::new().unwrap();
    let (app, db) = harness(&dir.path().join("media"), 1).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let bytes = png_bytes(MAX_REQUEST_BODY_BYTES + 1);
    let (status, body) = send_json(
        &app,
        upload_request("/overlay/media-board/upload?filename=big.png", TOKEN, bytes),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body, json!({"error": "payload_too_large"}));

    let (_, body) = send_json(&app, items_request(TOKEN)).await;
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn upload_rejects_missing_or_unsafe_filename() {
    let dir = TempDir::new().unwrap();
    let (app, db) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;
    let bytes = png_bytes(2048);

    let (status, body) = send_json(
        &app,
        upload_request("/overlay/media-board/upload", TOKEN, bytes.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
    assert_eq!(body["message"], "filename is required");

    for uri in [
        "/overlay/media-board/upload?filename=..%2Fx.png",
        "/overlay/media-board/upload?filename=%00x.png",
        "/overlay/media-board/upload?filename=x%5Cy.png",
    ] {
        let (status, body) = send_json(&app, upload_request(uri, TOKEN, bytes.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["error"], "invalid_payload", "{uri}");
    }
}

#[tokio::test]
async fn read_only_session_is_rejected_before_body_read() {
    let dir = TempDir::new().unwrap();
    let (app, db) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::InviteReadOnly, READ_ONLY_TOKEN).await;

    let polled = Arc::new(AtomicBool::new(false));
    let polled_by_stream = Arc::clone(&polled);
    let body_stream = stream::once(async move {
        polled_by_stream.store(true, Ordering::SeqCst);
        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"not read"))
    });
    let request = Request::builder()
        .method(Method::POST)
        .uri("/overlay/media-board/upload?filename=x.png")
        .header(header::AUTHORIZATION, format!("Bearer {READ_ONLY_TOKEN}"))
        .body(Body::from_stream(body_stream))
        .unwrap();

    let (status, body) = send_json(&app, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "read_only_session");
    assert!(!polled.load(Ordering::SeqCst), "body must not be polled");
}

#[tokio::test]
async fn upload_accepts_over_one_mib_while_json_routes_keep_the_limit() {
    let dir = TempDir::new().unwrap();
    let (app, db) = harness(&dir.path().join("media"), 4).await;
    insert_client(&db, OverlaySessionMode::Normal, TOKEN).await;

    let bytes = png_bytes(MAX_REQUEST_BODY_BYTES + 2048);
    let (status, body) = send_json(
        &app,
        upload_request(
            "/overlay/media-board/upload?filename=large.png",
            TOKEN,
            bytes,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

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
