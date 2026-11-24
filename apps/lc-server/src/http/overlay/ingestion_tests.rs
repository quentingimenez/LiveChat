//! In-process tests for the asynchronous board ingestion contract and for
//! `board.changed` broadcasts.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use lc_db::{Db, DbConfig};
use lc_domain::rights::{Action, RightsRole, RightsTable, Surface};
use lc_domain::{
    FixedClock, GuildId, GuildMember, GuildMemberId, GuildRole, MediaAsset, MediaAssetId,
    MediaBoardItem, MediaBoardItemId, MediaKind, MediaStatus, OverlayClient, OverlayClientId,
    OverlaySessionMode,
};
use lc_protocol::{BoardChange, ServerMessage};
use serde_json::{json, Value};
use tempfile::TempDir;
use time::{Duration, UtcDateTime};
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::app::AppState;
use crate::config::Config;
use crate::http::admin::hub::AdminHub;
use crate::ingestions::{IngestionEntry, IngestionSource, IngestionStatus};
use crate::shutdown::Shutdown;

use super::auth;

const TOKEN: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const OTHER_TOKEN: &str = "6666666666666666666666666666666666666666666666666666666666666666";
const GUEST_TOKEN: &str = "7777777777777777777777777777777777777777777777777777777777777777";
const GUILD: &str = "123456789012345678";
const OTHER_GUILD: &str = "223456789012345678";

fn fixed_now() -> UtcDateTime {
    time::macros::utc_datetime!(2026-11-10 12:00:00)
}

fn guild() -> GuildId {
    GuildId::parse(GUILD).unwrap()
}

fn other_guild() -> GuildId {
    GuildId::parse(OTHER_GUILD).unwrap()
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
    sent: Mutex<Vec<(Uuid, ServerMessage)>>,
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

    fn client_messages(&self, client_id: &Uuid) -> Vec<ServerMessage> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target == client_id)
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

    fn send_to_client(&self, client_id: &Uuid, message: ServerMessage) -> bool {
        self.sent.lock().unwrap().push((*client_id, message));
        true
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
            device_hash: None,
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
            source_title: None,
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
            artwork_asset_id: None,
            artwork_on_overlay: false,
            display_duration_sec: None,
            display_scale_percent: None,
        };
        self.db.media_board().create(&item).await.unwrap();
        item
    }

    /// Attaches a guild member to an overlay client: its profile becomes the
    /// author of what the client triggers.
    async fn attach_member(&self, client: &OverlayClient, display_name: &str) -> GuildMember {
        let member = GuildMember {
            id: GuildMemberId::new_v4(),
            guild_id: guild(),
            display_name: display_name.to_owned(),
            avatar: Some("https://example.test/lea.png".to_owned()),
            role: GuildRole::Member,
            created_at: fixed_now(),
            removed_at: None,
            discord_user_id: None,
            profile_custom: false,
            discord_synced_at: None,
        };
        self.db.guild_members().create(&member).await.unwrap();
        self.db
            .overlay_clients()
            .set_member(client.id, member.id)
            .await
            .unwrap();
        member
    }

    /// Puts a ready image in the media cache for `url`, so an ingestion of
    /// that link reuses it without any download (an IP literal also skips the
    /// DNS lookup). Returns the asset id.
    async fn insert_cached_image(&self, url: &str) -> MediaAssetId {
        let path = self._media.path().join("cached.png");
        tokio::fs::write(&path, b"png-body").await.unwrap();
        let canonical = lc_media::source::canonicalize_source_url(url);
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: lc_media::source::source_hash(&canonical),
            source_url: url.to_owned(),
            source_title: None,
            kind: MediaKind::Image,
            mime: "image/png".to_owned(),
            duration_sec: None,
            width: Some(10),
            height: Some(10),
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: 8,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: fixed_now(),
            expires_at: fixed_now() + Duration::days(1),
            created_at: fixed_now(),
        };
        self.db.media_assets().insert(&asset).await.unwrap();
        asset.id
    }

    /// Same as [`Self::insert_cached_image`] for a video: its own length makes
    /// the difference for a play-once display time.
    async fn insert_cached_video(&self, url: &str) -> MediaAssetId {
        let path = self._media.path().join("cached.mp4");
        tokio::fs::write(&path, b"mp4-body").await.unwrap();
        let canonical = lc_media::source::canonicalize_source_url(url);
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: lc_media::source::source_hash(&canonical),
            source_url: url.to_owned(),
            source_title: None,
            kind: MediaKind::Video,
            mime: "video/mp4".to_owned(),
            duration_sec: Some(20),
            width: Some(640),
            height: Some(360),
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: 8,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: fixed_now(),
            expires_at: fixed_now() + Duration::days(1),
            created_at: fixed_now(),
        };
        self.db.media_assets().insert(&asset).await.unwrap();
        asset.id
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
        .start(
            guild(),
            client.id.as_uuid(),
            IngestionSource::link("https://example.invalid/own.mp4"),
            fixed_now(),
        )
        .unwrap();
    harness
        .state
        .ingestions
        .start(
            guild(),
            other.id.as_uuid(),
            IngestionSource::link("https://example.invalid/other.mp4"),
            fixed_now(),
        )
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
            .start(
                guild(),
                client.id.as_uuid(),
                IngestionSource::link("https://example.invalid/media.mp4"),
                fixed_now(),
            )
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
    assert_eq!(
        body,
        json!({
            "error": "too_many_ingestions",
            "message": "Trop d'ingestions simultanées. Réessayez dans un instant.",
        })
    );
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
        .start(
            guild(),
            client.id.as_uuid(),
            IngestionSource::link("https://example.invalid/media.mp4"),
            fixed_now(),
        )
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

/// Waits (bounded) for the detached task to finish an ingestion of the client.
async fn wait_ingestion(state: &AppState, client_id: Uuid, ingestion_id: &str) -> IngestionEntry {
    let deadline = Instant::now() + StdDuration::from_secs(30);
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

/// The cached image of the play-once tests: the ingestion of that link reuses
/// it without any network access.
const PLAY_ONCE_URL: &str = "https://93.184.215.14/play-once.png";

/// The cached video of the play-once tests: a video keeps its own length.
const PLAY_ONCE_VIDEO_URL: &str = "https://93.184.215.14/play-once.mp4";

/// Un ajout au board par lien ignore le texte saisi : il n'est jamais stocké
/// comme message de l'item (seul un `playOnce` le reçoit, pour cette lecture).
#[tokio::test]
async fn link_add_ignores_the_play_text() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    harness.insert_cached_image(PLAY_ONCE_URL).await;

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({
                "url": PLAY_ONCE_URL,
                "title": "Chat qui tombe",
                "message": "coucou",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();
    let entry = wait_ingestion(&harness.state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    let item_id = entry.item_id.expect("item de board");

    let item = harness
        .db
        .media_board()
        .get(item_id.parse().unwrap())
        .await
        .unwrap()
        .expect("item persisté");
    assert_eq!(item.title.as_deref(), Some("Chat qui tombe"));
    assert_eq!(item.message, None);
    assert!(harness
        .db
        .playback_jobs()
        .list_pending_by_guild(&guild())
        .await
        .unwrap()
        .is_empty());
}

/// « Jouer une fois » par lien : la tâche détachée crée un job de lecture et
/// aucun item de board, la guilde n'apprend rien (`board.changed` absent).
#[tokio::test]
async fn play_once_link_creates_a_job_and_no_board_item() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    let member = harness.attach_member(&client, "Léa").await;
    let asset_id = harness.insert_cached_image(PLAY_ONCE_URL).await;

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({
                "url": PLAY_ONCE_URL,
                "title": "Chat qui tombe",
                "message": "coucou",
                "playOnce": true,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();

    let entry = wait_ingestion(&harness.state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    assert_eq!(entry.created, Some(false));
    assert_eq!(entry.item_id, None);
    let job_id = entry.job_id.clone().expect("jobId de l'ingestion");

    // The ready asset became a playback job for the guild, authored by the
    // overlay member like a UI trigger.
    let pending = harness
        .db
        .playback_jobs()
        .list_pending_by_guild(&guild())
        .await
        .unwrap();
    let job = pending
        .iter()
        .find(|job| job.id.to_canonical() == job_id)
        .expect("job persisté");
    assert_eq!(job.media_asset_id, Some(asset_id));
    assert_eq!(job.author_name.as_deref(), Some("Léa"));
    assert_eq!(
        job.author_image.as_deref(),
        member.avatar.as_deref(),
        "l'avatar du membre est celui du job"
    );
    assert_eq!(job.text.as_deref(), Some("coucou"));
    assert!(job.show_text);

    // No board item, no broadcast to the guild.
    assert!(harness
        .db
        .media_board()
        .list_by_guild(&guild())
        .await
        .unwrap()
        .is_empty());
    assert!(harness.hub.messages(&guild()).is_empty());

    // `media.ingestion` carries the job to the requesting client only.
    let messages = harness.hub.client_messages(&client.id.as_uuid());
    let payload = match messages.first() {
        Some(ServerMessage::MediaIngestion(payload)) => payload,
        other => panic!("message inattendu : {other:?}"),
    };
    assert!(payload.ok);
    assert_eq!(payload.ingestion_id, ingestion_id);
    assert_eq!(payload.created, Some(false));
    assert!(payload.item_id.is_none());
    assert_eq!(payload.job_id.as_deref(), Some(job_id.as_str()));
}

/// « Jouer une fois » d'une image : le temps d'affichage demandé porte la durée
/// du job au lieu de la valeur par défaut de la guilde.
#[tokio::test]
async fn play_once_link_uses_the_display_time_of_an_image() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    harness.insert_cached_image(PLAY_ONCE_URL).await;

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({
                "url": PLAY_ONCE_URL,
                "playOnce": true,
                "displayDurationSec": 12,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();
    let entry = wait_ingestion(&harness.state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    let job_id = entry.job_id.clone().expect("jobId de l'ingestion");

    let pending = harness
        .db
        .playback_jobs()
        .list_pending_by_guild(&guild())
        .await
        .unwrap();
    let job = pending
        .iter()
        .find(|job| job.id.to_canonical() == job_id)
        .expect("job persisté");
    assert_eq!(job.duration_sec, 12, "la durée demandée est celle du job");
}

/// « Jouer une fois » d'une vidéo : elle garde sa propre durée, le temps
/// d'affichage demandé est ignoré.
#[tokio::test]
async fn play_once_link_ignores_the_display_time_of_a_video() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    harness.insert_cached_video(PLAY_ONCE_VIDEO_URL).await;

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({
                "url": PLAY_ONCE_VIDEO_URL,
                "playOnce": true,
                "displayDurationSec": 12,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();
    let entry = wait_ingestion(&harness.state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    let job_id = entry.job_id.clone().expect("jobId de l'ingestion");

    let pending = harness
        .db
        .playback_jobs()
        .list_pending_by_guild(&guild())
        .await
        .unwrap();
    let job = pending
        .iter()
        .find(|job| job.id.to_canonical() == job_id)
        .expect("job persisté");
    assert_eq!(job.duration_sec, 20, "la vidéo garde sa durée");
}

/// Le temps d'affichage d'un « Jouer une fois » est borné à 1..=600 s entiers :
/// une valeur hors bornes est refusée avant toute ingestion.
#[tokio::test]
async fn play_once_link_refuses_an_out_of_bounds_display_time() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    harness.insert_cached_image(PLAY_ONCE_URL).await;

    for seconds in [json!(0), json!(601), json!(1.5), json!("12")] {
        let (status, body) = send_json(
            &harness.app,
            request(
                Method::POST,
                "/overlay/media-board/items",
                Some(TOKEN),
                Some(&json!({
                    "url": PLAY_ONCE_URL,
                    "playOnce": true,
                    "displayDurationSec": seconds,
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{seconds} : {body}");
        assert_eq!(body["error"], "invalid_payload", "{seconds} : {body}");
        assert_eq!(
            body["message"],
            "La durée d’affichage doit être un entier entre 1 et 600 secondes, ou null.",
            "{seconds} : {body}"
        );
    }
    assert!(harness
        .state
        .ingestions
        .list_for_client(client.id.as_uuid(), fixed_now())
        .is_empty());
}

/// Un temps d'affichage n'a de sens que pour un « Jouer une fois » : un ajout
/// au board le refuse.
#[tokio::test]
async fn display_time_is_refused_without_a_play_once() {
    let harness = Harness::new().await;
    harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    harness.insert_cached_image(PLAY_ONCE_URL).await;

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({
                "url": PLAY_ONCE_URL,
                "displayDurationSec": 12,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_payload");
    assert_eq!(
        body["message"],
        "La durée d’affichage n’est autorisée que pour une lecture unique."
    );
}

/// « Jouer une fois » exige le droit d'un déclenchement manuel : sans
/// `TriggerPlayback` la requête est refusée (403 `forbidden`).
#[tokio::test]
async fn play_once_link_is_refused_without_the_trigger_playback_right() {
    let harness = Harness::new().await;
    harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    let mut table = RightsTable::new();
    table.set_override(
        Surface::Overlay,
        Action::TriggerPlayback,
        RightsRole::Member,
        false,
    );
    harness.state.rights.replace(table);

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({ "url": PLAY_ONCE_URL, "playOnce": true })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "forbidden");

    // No ingestion was registered for the refused request.
    let refused_client = harness
        .db
        .overlay_clients()
        .get_by_token_hash(&auth::hash_token(TOKEN))
        .await
        .unwrap()
        .expect("client");
    assert!(harness
        .state
        .ingestions
        .list_for_client(refused_client.id.as_uuid(), fixed_now())
        .is_empty());
}

/// Un membre sans `AddMedia` peut quand même jouer une fois : le droit exigé
/// est `TriggerPlayback`, pas celui d'un ajout au board.
#[tokio::test]
async fn play_once_link_is_accepted_without_the_add_media_right() {
    let harness = Harness::new().await;
    let client = harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;
    harness.insert_cached_image(PLAY_ONCE_URL).await;
    let mut table = RightsTable::new();
    table.set_override(
        Surface::Overlay,
        Action::AddMedia,
        RightsRole::Member,
        false,
    );
    harness.state.rights.replace(table);

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(TOKEN),
            Some(&json!({ "url": PLAY_ONCE_URL, "playOnce": true })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let ingestion_id = body["ingestionId"].as_str().unwrap().to_owned();

    let entry = wait_ingestion(&harness.state, client.id.as_uuid(), &ingestion_id).await;
    assert_eq!(entry.status, IngestionStatus::Ready, "{entry:?}");
    assert!(entry.job_id.is_some());
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

#[tokio::test]
async fn cache_lookup_is_global_and_does_not_extend_the_cache() {
    let harness = Harness::new().await;
    let url = "https://example.test/global-cache.png";
    let asset_id = harness.insert_cached_image(url).await;
    let before = harness
        .db
        .media_assets()
        .get(asset_id)
        .await
        .unwrap()
        .expect("asset en cache");
    // Another guild's client proves the lookup is not scoped to the board.
    harness
        .insert_client(OverlaySessionMode::Normal, OTHER_TOKEN, &other_guild())
        .await;

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/cache",
            Some(OTHER_TOKEN),
            Some(&json!({ "url": url })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "cached": true }));

    // A lookup is a read: no lifetime extension, no metadata.
    let after = harness
        .db
        .media_assets()
        .get(asset_id)
        .await
        .unwrap()
        .expect("asset en cache");
    assert_eq!(before.expires_at, after.expires_at);
    assert_eq!(before.last_accessed_at, after.last_accessed_at);

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/cache",
            Some(OTHER_TOKEN),
            Some(&json!({ "url": "https://example.test/unknown.png" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "cached": false }));
}

#[tokio::test]
async fn cache_lookup_rejects_non_public_or_non_http_urls() {
    let harness = Harness::new().await;
    harness
        .insert_client(OverlaySessionMode::Normal, TOKEN, &guild())
        .await;

    for url in [
        "file:///etc/passwd",
        "ftp://example.test/clip",
        "http://localhost/clip",
        "http://127.0.0.1/clip",
        "https://user:pass@example.test/clip",
    ] {
        let (status, body) = send_json(
            &harness.app,
            request(
                Method::POST,
                "/overlay/media-board/cache",
                Some(TOKEN),
                Some(&json!({ "url": url })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "URL {url} : {body}");
        assert_eq!(body["error"], "invalid_payload");
    }

    let (status, _) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/cache",
            Some(TOKEN),
            Some(&json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cache_lookup_requires_a_writable_registered_session() {
    let harness = Harness::new().await;
    harness
        .insert_client(OverlaySessionMode::InviteReadOnly, GUEST_TOKEN, &guild())
        .await;

    let (status, body) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/cache",
            Some(GUEST_TOKEN),
            Some(&json!({ "url": "https://example.test/clip" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "read_only_session");

    // The session is checked before the body is parsed: a malformed body must
    // still get the read-only refusal, not a JSON error.
    let malformed = Request::builder()
        .method(Method::POST)
        .uri("/overlay/media-board/cache")
        .header(header::AUTHORIZATION, format!("Bearer {GUEST_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not-json"))
        .unwrap();
    let (status, body) = send_json(&harness.app, malformed).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "read_only_session");

    let (status, _) = send_json(
        &harness.app,
        request(
            Method::POST,
            "/overlay/media-board/cache",
            None,
            Some(&json!({ "url": "https://example.test/clip" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
