//! Tests of `POST /overlay/media-board/import` (copy of board items from
//! another guild of the same device).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use axum::Router;
use lc_domain::{
    GuildId, GuildRole, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind,
    MediaStatus,
};
use serde_json::{json, Value};
use time::Duration;

use crate::http::admin::hub::AdminHub;

use super::guild_tests::{call_app, fixed_now, token, Harness};
use super::media_upload_tests::RecordingHub;

const IMPORT_URI: &str = "/overlay/media-board/import";
const MB: i64 = 1024 * 1024;

struct Setup {
    harness: Harness,
    app: Router,
    hub: Arc<RecordingHub>,
    /// Source guild (the harness guild).
    a: GuildId,
    /// Target guild.
    b: GuildId,
    a_token: String,
    b_token: String,
}

async fn setup() -> Setup {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingHub::default());
    let app = crate::app::router(
        harness
            .state
            .clone()
            .with_admin_hub(hub.clone() as Arc<dyn AdminHub>),
    );
    let a = harness.guild_id.clone();
    let b = harness.second_guild("Les Thons").await;
    let a_token = member(&harness, &a, GuildRole::Member, "Alice").await;
    let b_token = member(&harness, &b, GuildRole::Member, "Bruno").await;
    Setup {
        harness,
        app,
        hub,
        a,
        b,
        a_token,
        b_token,
    }
}

async fn member(harness: &Harness, guild_id: &GuildId, role: GuildRole, name: &str) -> String {
    token(
        &harness
            .join(&harness.invite_for(guild_id, role).await, name)
            .await,
    )
}

async fn asset(harness: &Harness, name: &str, kind: MediaKind, size_bytes: i64) -> MediaAsset {
    let path = harness.state.config.media_storage_dir.join(name);
    tokio::fs::write(&path, b"0123456789").await.unwrap();
    let asset = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: format!("hash-{name}"),
        source_url: format!("https://example.test/{name}"),
        source_title: None,
        kind,
        mime: if kind == MediaKind::Image {
            "image/png".to_owned()
        } else {
            "video/mp4".to_owned()
        },
        duration_sec: (kind != MediaKind::Image).then_some(30),
        width: Some(480),
        height: Some(360),
        is_vertical: false,
        storage_path: path.to_string_lossy().into_owned(),
        size_bytes,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: fixed_now(),
        expires_at: fixed_now() + Duration::days(36_500),
        created_at: fixed_now(),
    };
    harness.db.media_assets().insert(&asset).await.unwrap();
    asset
}

async fn item_in(harness: &Harness, guild_id: &GuildId, media: &MediaAsset) -> MediaBoardItem {
    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: media.id,
        title: Some("Clip".to_owned()),
        message: Some("aïe".to_owned()),
        created_by_discord_user_id: None,
        created_by_name: Some("Alice".to_owned()),
        created_at: fixed_now(),
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
    };
    harness.db.media_board().create(&item).await.unwrap();
    item
}

async fn import(
    setup: &Setup,
    target_token: &str,
    source_token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let headers: Vec<(&str, &str)> = source_token
        .map(|value| vec![("X-LiveChat-Source-Token", value)])
        .unwrap_or_default();
    call_app(
        &setup.app,
        Method::POST,
        IMPORT_URI,
        Some(target_token),
        &headers,
        Some(body),
    )
    .await
}

fn ids(items: &[&MediaBoardItem]) -> Value {
    json!({ "itemIds": items.iter().map(|item| item.id.to_canonical()).collect::<Vec<_>>() })
}

#[tokio::test]
async fn copies_an_item_to_the_target_board() {
    let s = setup().await;
    let media = asset(&s.harness, "clip.mp4", MediaKind::Video, 10).await;
    let source = item_in(&s.harness, &s.a, &media).await;

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), ids(&[&source])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["results"][0];
    assert_eq!(result["sourceItemId"], source.id.to_canonical());
    assert_eq!(result["status"], "added");
    let copy = s
        .harness
        .db
        .media_board()
        .get_by_guild_and_asset(&s.b, media.id)
        .await
        .unwrap()
        .expect("copy on the target board");
    assert_eq!(result["itemId"], copy.id.to_canonical());
    assert_eq!(copy.title.as_deref(), Some("Clip"));
    assert_eq!(copy.message.as_deref(), Some("aïe"));
    assert_eq!(copy.created_by_name.as_deref(), Some("Bruno"));
    assert_eq!(s.hub.added_count(&s.b), 1);
    assert_eq!(s.hub.added_count(&s.a), 0);
}

#[tokio::test]
async fn an_item_already_on_the_target_board_is_already() {
    let s = setup().await;
    let media = asset(&s.harness, "clip.mp4", MediaKind::Video, 10).await;
    let source = item_in(&s.harness, &s.a, &media).await;
    let existing = item_in(&s.harness, &s.b, &media).await;

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), ids(&[&source])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"][0]["status"], "already");
    assert_eq!(body["results"][0]["itemId"], existing.id.to_canonical());
    assert_eq!(s.hub.added_count(&s.b), 0);
}

#[tokio::test]
async fn an_item_of_another_guild_is_not_found() {
    let s = setup().await;
    let c = s.harness.second_guild("Les Merlus").await;
    let media = asset(&s.harness, "clip.mp4", MediaKind::Video, 10).await;
    let foreign = item_in(&s.harness, &c, &media).await;

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), ids(&[&foreign])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"][0]["status"], "not_found");
    assert!(body["results"][0].get("itemId").is_none());
    assert!(s
        .harness
        .db
        .media_board()
        .get_by_guild_and_asset(&s.b, media.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn invalid_source_token_is_refused() {
    let s = setup().await;
    let media = asset(&s.harness, "clip.mp4", MediaKind::Video, 10).await;
    let source = item_in(&s.harness, &s.a, &media).await;

    for source_token in [None, Some(""), Some("nope")] {
        let (status, body) = import(&s, &s.b_token, source_token, ids(&[&source])).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{source_token:?}: {body}");
        assert_eq!(body["error"], "invalid_source_token");
    }
}

#[tokio::test]
async fn same_guild_is_refused() {
    let s = setup().await;
    let other_b = member(&s.harness, &s.b, GuildRole::Member, "Bea").await;
    let media = asset(&s.harness, "clip.mp4", MediaKind::Video, 10).await;
    let item = item_in(&s.harness, &s.b, &media).await;

    let (status, body) = import(&s, &s.b_token, Some(&other_b), ids(&[&item])).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_payload");
}

#[tokio::test]
async fn a_guest_target_is_refused_before_the_body() {
    let s = setup().await;
    let guest_b = member(&s.harness, &s.b, GuildRole::Guest, "Gus").await;

    let (status, body) = import(&s, &guest_b, Some(&s.a_token), json!("not an object")).await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "read_only_session");
}

#[tokio::test]
async fn a_guest_source_may_be_read() {
    let s = setup().await;
    let guest_a = member(&s.harness, &s.a, GuildRole::Guest, "Gus").await;
    let media = asset(&s.harness, "clip.mp4", MediaKind::Video, 10).await;
    let source = item_in(&s.harness, &s.a, &media).await;

    let (status, body) = import(&s, &s.b_token, Some(&guest_a), ids(&[&source])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"][0]["status"], "added");
}

#[tokio::test]
async fn more_than_fifty_ids_are_refused() {
    let s = setup().await;
    let many: Vec<String> = (0..51)
        .map(|_| MediaBoardItemId::new_v4().to_canonical())
        .collect();

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), json!({ "itemIds": many })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_payload");

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), json!({ "itemIds": [] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_payload");

    let (status, body) = import(
        &s,
        &s.b_token,
        Some(&s.a_token),
        json!({ "itemIds": ["not-a-uuid"] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "invalid_payload");
}

#[tokio::test]
async fn quota_failure_does_not_stop_the_batch() {
    // The harness board limit is 64 MB.
    let s = setup().await;
    let first = asset(&s.harness, "first.mp4", MediaKind::Video, 40 * MB).await;
    let second = asset(&s.harness, "second.mp4", MediaKind::Video, 40 * MB).await;
    let third = asset(&s.harness, "third.mp4", MediaKind::Video, MB).await;
    let one = item_in(&s.harness, &s.a, &first).await;
    let two = item_in(&s.harness, &s.a, &second).await;
    let three = item_in(&s.harness, &s.a, &third).await;

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), ids(&[&one, &two, &three])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let statuses: Vec<&str> = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| result["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["added", "failed", "added"]);
    assert_eq!(body["results"][1]["error"], "media_ingestion_failed");
    assert_eq!(s.hub.added_count(&s.b), 2);
}

#[tokio::test]
async fn artwork_is_copied_with_the_item() {
    let s = setup().await;
    let song = asset(&s.harness, "song.mp3", MediaKind::Audio, 10).await;
    let cover = asset(&s.harness, "cover.png", MediaKind::Image, 10).await;
    let source = item_in(&s.harness, &s.a, &song).await;
    s.harness
        .db
        .media_board()
        .set_artwork(source.id, Some(cover.id))
        .await
        .unwrap();
    s.harness
        .db
        .media_board()
        .set_artwork_on_overlay(source.id, true)
        .await
        .unwrap();

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), ids(&[&source])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let copy = s
        .harness
        .db
        .media_board()
        .get_by_guild_and_asset(&s.b, song.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(copy.artwork_asset_id, Some(cover.id));
    assert!(copy.artwork_on_overlay);
}

#[tokio::test]
async fn import_keeps_display_duration() {
    let s = setup().await;
    let photo = asset(&s.harness, "photo.png", MediaKind::Image, 10).await;
    let source = item_in(&s.harness, &s.a, &photo).await;
    s.harness
        .db
        .media_board()
        .set_display_duration(source.id, Some(9))
        .await
        .unwrap();

    let (status, body) = import(&s, &s.b_token, Some(&s.a_token), ids(&[&source])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let copy = s
        .harness
        .db
        .media_board()
        .get_by_guild_and_asset(&s.b, photo.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(copy.display_duration_sec, Some(9));
}
