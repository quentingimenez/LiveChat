//! Favorites and pictures of board items (`board_extras`), through the full
//! router.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::{Method, StatusCode};
use lc_domain::{
    GuildRole, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind, MediaStatus,
};
use serde_json::{json, Value};
use time::Duration;

use super::guild_tests::{fixed_now, token, Harness};

/// A ready asset whose file lives in the server's media directory (files
/// outside it are never served).
async fn asset(harness: &Harness, name: &str, kind: MediaKind, mime: &str) -> MediaAsset {
    let path = harness.state.config.media_storage_dir.join(name);
    tokio::fs::write(&path, b"0123456789").await.unwrap();
    let asset = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: format!("hash-{name}"),
        source_url: format!("https://example.test/{name}"),
        source_title: None,
        kind,
        mime: mime.to_owned(),
        duration_sec: (kind != MediaKind::Image).then_some(30),
        width: Some(480),
        height: Some(360),
        is_vertical: false,
        storage_path: path.to_string_lossy().into_owned(),
        size_bytes: 10,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: fixed_now(),
        expires_at: fixed_now() + Duration::days(36_500),
        created_at: fixed_now(),
    };
    harness.db.media_assets().insert(&asset).await.unwrap();
    asset
}

async fn item(harness: &Harness, media: &MediaAsset) -> MediaBoardItem {
    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: harness.guild_id.clone(),
        media_asset_id: media.id,
        title: Some("Son".to_owned()),
        message: None,
        created_by_discord_user_id: None,
        created_by_name: Some("Alice".to_owned()),
        created_at: fixed_now(),
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
        display_scale_percent: None,
    };
    harness.db.media_board().create(&item).await.unwrap();
    item
}

async fn listed(harness: &Harness, token: &str, item_id: MediaBoardItemId) -> Value {
    let (status, body) = harness
        .call(Method::GET, "/overlay/media-board/items", Some(token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == item_id.to_canonical())
        .cloned()
        .expect("item listé")
}

#[tokio::test]
async fn favorites_are_personal_and_idempotent() {
    let harness = Harness::new().await;
    let admin = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Admin")
            .await,
    );
    let other = token(
        &harness
            .join(&harness.server_invite(GuildRole::Member).await, "Bob")
            .await,
    );
    let media = asset(&harness, "a.mp3", MediaKind::Audio, "audio/mpeg").await;
    let entry = item(&harness, &media).await;
    let uri = format!(
        "/overlay/media-board/items/{}/favorite",
        entry.id.to_canonical()
    );

    for _ in 0..2 {
        let (status, body) = harness
            .call(
                Method::PUT,
                &uri,
                Some(&admin),
                Some(json!({"favorite": true})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(listed(&harness, &admin, entry.id).await["favorite"], true);
    // Another member does not see it.
    assert_eq!(listed(&harness, &other, entry.id).await["favorite"], false);

    let (status, _) = harness
        .call(
            Method::PUT,
            &uri,
            Some(&admin),
            Some(json!({"favorite": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed(&harness, &admin, entry.id).await["favorite"], false);

    let (status, _) = harness
        .call(
            Method::PUT,
            &format!(
                "/overlay/media-board/items/{}/favorite",
                MediaBoardItemId::new_v4().to_canonical()
            ),
            Some(&admin),
            Some(json!({"favorite": true})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_embedded_cover_becomes_the_item_picture() {
    let harness = Harness::new().await;
    let admin = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Admin")
            .await,
    );
    let audio = asset(&harness, "b.mp3", MediaKind::Audio, "audio/mpeg").await;
    let cover = asset(&harness, "b.cover.jpg", MediaKind::Image, "image/jpeg").await;
    harness
        .db
        .media_assets()
        .set_cover(audio.id, Some(cover.id))
        .await
        .unwrap();

    let entry = item(&harness, &audio).await;
    let stored = harness
        .db
        .media_board()
        .get(entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.artwork_asset_id, Some(cover.id));

    let payload = listed(&harness, &admin, entry.id).await;
    let url = payload["artwork"]["url"].as_str().expect("URL de l'image");
    assert_eq!(payload["artwork"]["width"], 480);
    assert_eq!(payload["artworkOnOverlay"], false);
    // The signed picture is served to the guild.
    let path = url.split_once("8099").map(|(_, rest)| rest).unwrap();
    let (status, _) = harness.call(Method::GET, path, None, None).await;
    assert_eq!(status, StatusCode::OK, "{path}");

    let (status, body) = harness
        .call(
            Method::PATCH,
            &format!("/overlay/media-board/items/{}", entry.id.to_canonical()),
            Some(&admin),
            Some(json!({"artworkOnOverlay": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        listed(&harness, &admin, entry.id).await["artworkOnOverlay"],
        true
    );

    let (status, _) = harness
        .call(
            Method::DELETE,
            &format!(
                "/overlay/media-board/items/{}/artwork",
                entry.id.to_canonical()
            ),
            Some(&admin),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listed(&harness, &admin, entry.id).await["artwork"],
        Value::Null
    );
}

#[tokio::test]
async fn a_video_shows_its_thumbnail_without_taking_it_as_picture() {
    let harness = Harness::new().await;
    let admin = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Admin")
            .await,
    );
    let video = asset(&harness, "v.mp4", MediaKind::Video, "video/mp4").await;
    let thumbnail = asset(&harness, "v.cover.jpg", MediaKind::Image, "image/jpeg").await;
    harness
        .db
        .media_assets()
        .set_cover(video.id, Some(thumbnail.id))
        .await
        .unwrap();

    let entry = item(&harness, &video).await;
    let stored = harness
        .db
        .media_board()
        .get(entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.artwork_asset_id, None);

    let payload = listed(&harness, &admin, entry.id).await;
    assert_eq!(payload["artwork"], Value::Null);
    assert_eq!(payload["thumbnail"]["width"], 480);
    let url = payload["thumbnail"]["url"]
        .as_str()
        .expect("URL de la miniature");
    // The signed thumbnail is served to the guild.
    let path = url.split_once("8099").map(|(_, rest)| rest).unwrap();
    let (status, _) = harness.call(Method::GET, path, None, None).await;
    assert_eq!(status, StatusCode::OK, "{path}");
}

#[tokio::test]
async fn an_audio_has_no_thumbnail() {
    let harness = Harness::new().await;
    let admin = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Admin")
            .await,
    );
    let audio = asset(&harness, "c.mp3", MediaKind::Audio, "audio/mpeg").await;
    let cover = asset(&harness, "c.cover.jpg", MediaKind::Image, "image/jpeg").await;
    harness
        .db
        .media_assets()
        .set_cover(audio.id, Some(cover.id))
        .await
        .unwrap();
    let entry = item(&harness, &audio).await;
    let payload = listed(&harness, &admin, entry.id).await;
    assert_eq!(payload["thumbnail"], Value::Null);
    assert!(payload["artwork"]["url"].is_string());
}

#[tokio::test]
async fn a_guest_cannot_change_a_picture() {
    let harness = Harness::new().await;
    let guest = token(
        &harness
            .join(&harness.server_invite(GuildRole::Guest).await, "Invité")
            .await,
    );
    let audio = asset(&harness, "c.mp3", MediaKind::Audio, "audio/mpeg").await;
    let entry = item(&harness, &audio).await;
    let (status, _) = harness
        .call(
            Method::DELETE,
            &format!(
                "/overlay/media-board/items/{}/artwork",
                entry.id.to_canonical()
            ),
            Some(&guest),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
