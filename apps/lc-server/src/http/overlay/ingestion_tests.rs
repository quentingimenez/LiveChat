//! In-process tests for the asynchronous board ingestion contract and for
//! `board.changed` broadcasts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Duration, TimeZone, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{
    FixedClock, GuildId, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind,
    MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode,
};
use lc_protocol::{BoardChange, ServerMessage};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::app::AppState;
use crate::config::Config;
use crate::http::admin::hub::AdminHub;
use crate::shutdown::Shutdown;

use super::auth;

const TOKEN: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const OTHER_TOKEN: &str = "6666666666666666666666666666666666666666666666666666666666666666";
const GUEST_TOKEN: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const GUILD: &str = "123456789012345678";

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
}

fn guild() -> GuildId {
    GuildId::parse(GUILD).unwrap()
}

fn test_config(media_dir: &Path) -> Config {
    let pairs = vec![
        ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
        (
            "MEDIA_URL_SECRET".to_owned(),
            "secret-test-ingestions".to_owned(),
        ),
        (
            "MEDIA_STORAGE_DIR".to_owned(),
            media_dir.to_string_lossy().into_owned(),
        ),
        ("MEDIA_BOARD_MAX_TOTAL_MB".to_owned(), "64".to_owned()),
    ];
    Config::from_pairs(pairs).unwrap()
}

#[derive(Debug, Default)]
struct RecordingHub {
    broadcasts: Mutex<Vec<(GuildId, ServerMessage)>>,
}

impl RecordingHub {
    fn messages(&self, guild_id: &GuildId) -> Vec<ServerMessage> {
        self.broadcasts
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target == guild_id)
            .map(|(_, message)| message.clone())
            .collect()
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

    fn broadcast_to_guild(&self, guild_id: &GuildId, message: ServerMessage) -> usize {
        self.broadcasts
            .lock()
            .unwrap()
            .push((guild_id.clone(), message));
        1
    }

    async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
        0
    }

    async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
        0
    }
}

struct Harness {
    app: Router,
    state: AppState,
    db: Db,
    hub: Arc<RecordingHub>,
    clock: Arc<FixedClock>,
    _media: TempDir,
}

impl Harness {
    async fn new() -> Self {
        let media = TempDir::new().unwrap();
        let media_dir = media.path().join("media");
        std::fs::create_dir_all(&media_dir).unwrap();
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let clock = Arc::new(FixedClock::new(fixed_now()));
        let hub = Arc::new(RecordingHub::default());
        let state = AppState::new(
            Arc::new(test_config(&media_dir)),
            Some(db.clone()),
            Shutdown::detached(),
        )
        .with_clock(clock.clone())
        .with_admin_hub(hub.clone());
        let app = crate::app::router(state.clone());
        Self {
            app,
            state,
            db,
            hub,
            clock,
            _media: media,
        }
    }

    async fn insert_client(
        &self,
        mode: OverlaySessionMode,
        token: &str,
        guild_id: &GuildId,
    ) -> OverlayClient {
        let client = OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: guild_id.clone(),
            label: "ingestion-test".to_owned(),
            token_hash: auth::hash_token(token),
            session_mode: mode,
            default_author_name: Some("Alice".to_owned()),
            default_author_image: None,
            created_by_discord_user_id: None,
            last_seen_at: None,
            created_at: fixed_now(),
            revoked_at: None,
        };
        self.db.overlay_clients().create(&client).await.unwrap();
        client
    }

    async fn insert_asset(&self, media_dir: &Path) -> MediaAsset {
        let path = media_dir.join("board-item.mp4");
        tokio::fs::write(&path, b"media-body").await.unwrap();
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "hash-board-item".to_owned(),
            source_url: "https://example.test/board-item.mp4".to_owned(),
            kind: MediaKind::Video,
            mime: "video/mp4".to_owned(),
            duration_sec: Some(10),
            width: None,
            height: None,
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: 10,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: fixed_now(),
            expires_at: fixed_now() + Duration::days(1),
            created_at: fixed_now(),
        };
        self.db.media_assets().insert(&asset).await.unwrap();
        asset
    }

    async fn insert_item(&self, asset: &MediaAsset) -> MediaBoardItem {
        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: guild(),
            media_asset_id: asset.id,
            title: Some("initial".to_owned()),
            message: None,
            created_by_discord_user_id: Some("4242".to_owned()),
            created_by_name: Some("Alice".to_owned()),
            created_at: fixed_now(),
        };
        self.db.media_board().create(&item).await.unwrap();
        item
    }
}

fn request(method: Method, uri: &str, token: Option<&str>, body: Option<&Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            builder
                .body(Body::from(serde_json::to_vec(body).unwrap()))
                .unwrap()
        }
        None => builder.body(Body::empty()).unwrap(),
    }
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

/// Waits (bounded) for `count` guild broadcasts.
async fn wait_broadcasts(hub: &RecordingHub, count: usize) -> Vec<ServerMessage> {
    let deadline = Instant::now() + StdDuration::from_secs(5);
    loop {
        let messages = hub.messages(&guild());
        if messages.len() >= count {
            return messages;
        }
        assert!(
            Instant::now() < deadline,
            "diffusion `board.changed` non reçue dans le délai"
        );
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn get_ingestions_only_lists_the_authenticated_client_entries() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    let other = harness
        .insert_client(OverlaySessionMode::Normal, OTHER_TOKEN, &guild())
        .await;
    let guest = harness
        .insert_client(OverlaySessionMode::InviteReadOnly, GUEST_TOKEN, &guild())
        .await;

    let own_id = harness
        .state
        .ingestions
        .start(guild(), client.id.as_uuid(), fixed_now())
        .unwrap();
    harness
        .state
        .ingestions
        .start(guild(), other.id.as_uuid(), fixed_now())
        .unwrap();

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::GET,
            "/overlay/media-board/ingestions",
            Some(TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "items": [{
                "ingestionId": own_id,
                "status": "processing",
            }],
        })
    );

    // Guest sessions may read their own (empty) list.
    let (status, body) = send_json(
        &harness.app,
        request(
            Method::GET,
            "/overlay/media-board/ingestions",
            Some(GUEST_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "items": [] }));

    // Sanity: the other client's entry does exist and is not exposed to `TOKEN`.
    assert_eq!(
        harness
            .state
            .ingestions
            .list_for_client(other.id.as_uuid(), fixed_now())
            .len(),
        1
    );
    assert_ne!(client.id, guest.id);
}

#[tokio::test]
async fn processing_limit_answers_429() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    for _ in 0..crate::ingestions::MAX_PROCESSING_PER_CLIENT {
        harness
            .state
            .ingestions
            .start(guild(), client.id.as_uuid(), fixed_now())
            .unwrap();
    }

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({ "url": "https://example.test/media.mp4" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body, json!({ "error": "too_many_ingestions" }));
}

#[tokio::test]
async fn finished_ingestions_are_purged_after_one_hour() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    let ingestion_id = harness
        .state
        .ingestions
        .start(guild(), client.id.as_uuid(), fixed_now())
        .unwrap();
    harness.state.ingestions.complete_success(
        &ingestion_id,
        true,
        "item-1".to_owned(),
        fixed_now(),
    );

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::GET,
            "/overlay/media-board/ingestions",
            Some(TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["items"][0]["status"], "ready");
    assert_eq!(body["items"][0]["created"], true);
    assert_eq!(body["items"][0]["itemId"], "item-1");

    harness.clock.advance(Duration::minutes(61));
    let (status, body) = send_json(
        &harness.app,
        request(
            Method::GET,
            "/overlay/media-board/ingestions",
            Some(TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "items": [] }));
}

#[tokio::test]
async fn update_and_delete_broadcast_board_changed() {
    let harness = Harness::new().await;
    harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    let asset = harness.insert_asset(harness._media.path()).await;
    let item = harness.insert_item(&asset).await;
    let item_id = item.id.to_canonical();

    let item_uri = format!("/overlay/media-board/items/{item_id}");
    let (status, body) = send_json(
        &harness.app,
        request(
            Method::PATCH,
            &item_uri,
            Some(TOKEN),
            Some(&json!({ "title": "modifié" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let messages = wait_broadcasts(&harness.hub, 1).await;
    assert!(matches!(
        &messages[0],
        ServerMessage::BoardChanged(payload)
            if payload.change == BoardChange::Updated && payload.item_id == item_id
    ));

    let (status, body) = send_json(
        &harness.app,
        request(Method::DELETE, &item_uri, Some(TOKEN), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "deleted": true }));

    let messages = wait_broadcasts(&harness.hub, 2).await;
    assert!(matches!(
        &messages[1],
        ServerMessage::BoardChanged(payload)
            if payload.change == BoardChange::Deleted && payload.item_id == item_id
    ));
}
