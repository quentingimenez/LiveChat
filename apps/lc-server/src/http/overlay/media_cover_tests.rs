//! Tests of `POST /overlay/media-board/items/{itemId}/cover`, through the full
//! router.
//!
//! The guards run without ffmpeg. Reading the frame itself is `#[ignore]`: it
//! needs `ffmpeg` on the PATH
//! (`cargo test -p lc-server media_cover_tests -- --ignored`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::{Method, StatusCode};
use lc_domain::{
    GuildRole, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind, MediaStatus,
};
use serde_json::{json, Value};
use time::Duration;

use super::guild_tests::{fixed_now, token, Harness};

/// A ready asset of the test media directory, without touching its file.
fn asset_row(harness: &Harness, name: &str, kind: MediaKind, mime: &str) -> MediaAsset {
    MediaAsset {
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
        storage_path: harness
            .state
            .config
            .media_storage_dir
            .join(name)
            .to_string_lossy()
            .into_owned(),
        size_bytes: 10,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: fixed_now(),
        expires_at: fixed_now() + Duration::days(36_500),
        created_at: fixed_now(),
    }
}

/// A ready asset whose (fake) file lives in the server's media directory.
async fn asset(harness: &Harness, name: &str, kind: MediaKind, mime: &str) -> MediaAsset {
    let asset = asset_row(harness, name, kind, mime);
    tokio::fs::write(&asset.storage_path, b"0123456789")
        .await
        .unwrap();
    harness.db.media_assets().insert(&asset).await.unwrap();
    asset
}

async fn item(harness: &Harness, media: &MediaAsset) -> MediaBoardItem {
    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: harness.guild_id.clone(),
        media_asset_id: media.id,
        title: Some("Clip".to_owned()),
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

fn cover_uri(item_id: &str) -> String {
    format!("/overlay/media-board/items/{item_id}/cover")
}

async fn cover(harness: &Harness, token: &str, item_id: &str, body: Value) -> (StatusCode, Value) {
    harness
        .call(Method::POST, &cover_uri(item_id), Some(token), Some(body))
        .await
}

async fn member_token(harness: &Harness, role: GuildRole, name: &str) -> String {
    token(&harness.join(&harness.server_invite(role).await, name).await)
}

#[tokio::test]
async fn a_guest_is_refused_before_the_body_is_validated() {
    let harness = Harness::new().await;
    let guest = member_token(&harness, GuildRole::Guest, "Invité").await;
    let video = asset(&harness, "a.mp4", MediaKind::Video, "video/mp4").await;
    let entry = item(&harness, &video).await;
    for body in [json!({"atSec": 2}), json!({"nonsense": true}), json!([])] {
        let (status, response) = cover(&harness, &guest, &entry.id.to_canonical(), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(response["error"], "read_only_session");
    }
}

#[tokio::test]
async fn a_malformed_body_is_refused_before_the_item_is_looked_up() {
    let harness = Harness::new().await;
    let member = member_token(&harness, GuildRole::Member, "Bob").await;
    let unknown = MediaBoardItemId::new_v4().to_canonical();
    for (body, message) in [
        (
            json!({}),
            "atSec is required: the position of the player, in seconds",
        ),
        (
            json!({"atSec": "2"}),
            "atSec is required: the position of the player, in seconds",
        ),
        (
            json!({"atSec": -1}),
            "atSec must be a positive number of seconds",
        ),
        (json!([]), "a JSON object is expected"),
    ] {
        let (status, response) = cover(&harness, &member, &unknown, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["error"], "invalid_payload");
        assert_eq!(response["message"], message);
    }
}

#[tokio::test]
async fn an_item_outside_the_guild_board_is_not_found() {
    let harness = Harness::new().await;
    let member = member_token(&harness, GuildRole::Member, "Bob").await;
    let video = asset(&harness, "a.mp4", MediaKind::Video, "video/mp4").await;
    let foreign = MediaBoardItem {
        guild_id: lc_domain::GuildId::generate(),
        ..item(&harness, &video).await
    };
    let foreign = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        ..foreign
    };
    harness
        .db
        .guilds()
        .create_standalone(&foreign.guild_id, "Ailleurs", fixed_now())
        .await
        .unwrap();
    harness.db.media_board().create(&foreign).await.unwrap();

    for item_id in [
        MediaBoardItemId::new_v4().to_canonical(),
        "not-an-id".to_owned(),
        foreign.id.to_canonical(),
    ] {
        let (status, response) = cover(&harness, &member, &item_id, json!({"atSec": 2})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{item_id}");
        assert_eq!(response["error"], "item_not_found");
    }
}

#[tokio::test]
async fn only_the_items_editor_can_choose_its_picture() {
    let harness = Harness::new().await;
    let author = member_token(&harness, GuildRole::Member, "Alice").await;
    let other = member_token(&harness, GuildRole::Member, "Bob").await;
    let admin = member_token(&harness, GuildRole::Admin, "Chef").await;
    let image = asset(&harness, "a.png", MediaKind::Image, "image/png").await;
    let entry = item(&harness, &image).await;
    let (_, guild) = harness
        .call(Method::GET, "/overlay/guild", Some(&author), None)
        .await;
    let author_id = guild["member"]["id"].as_str().unwrap().parse().unwrap();
    harness
        .db
        .media_board()
        .set_creator_member(entry.id, author_id)
        .await
        .unwrap();
    let item_id = entry.id.to_canonical();

    // Another member's item is not theirs to edit.
    let (status, response) = cover(&harness, &other, &item_id, json!({"atSec": 2})).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    assert_eq!(response["error"], "forbidden");

    // The author and an admin reach the next guard, which refuses anything
    // but a video.
    for token in [&author, &admin] {
        let (status, response) = cover(&harness, token, &item_id, json!({"atSec": 2})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["error"], "invalid_payload");
        assert_eq!(
            response["message"],
            "only a video can take its picture from a frame"
        );
    }
}

#[tokio::test]
async fn a_media_missing_from_the_disk_is_reported_at_once() {
    let harness = Harness::new().await;
    let admin = member_token(&harness, GuildRole::Admin, "Chef").await;
    let video = asset(&harness, "gone.mp4", MediaKind::Video, "video/mp4").await;
    let entry = item(&harness, &video).await;
    tokio::fs::remove_file(&video.storage_path).await.unwrap();
    let (status, response) = cover(
        &harness,
        &admin,
        &entry.id.to_canonical(),
        json!({"atSec": 2}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(response["error"], "media_not_found_on_disk");
}

// ---------------------------------------------------------------- real ffmpeg

fn ffmpeg(args: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-v", "error"])
        .args(args)
        .status()
        .expect("ffmpeg lancé");
    assert!(status.success(), "ffmpeg a échoué : {args:?}");
}

/// A video dark on its first two seconds, then its test picture: the frame the
/// editor seeks to decides how bright the cover comes out.
fn dark_then_light_video(harness: &Harness) -> String {
    let path = harness.state.config.media_storage_dir.join("edit.mp4");
    let path_text = path.to_string_lossy().into_owned();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=2:size=320x240:rate=30",
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=2:size=320x240:rate=30",
        "-filter_complex",
        "[0:v]eq=brightness=-0.4[dark];[dark][1:v]concat=n=2:v=1:a=0[v]",
        "-map",
        "[v]",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        &path_text,
    ]);
    path_text
}

/// Mean luma of the picture a cover holds, through `ffmpeg`.
fn mean_luma(path: &str) -> f64 {
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "info", "-i", path])
        .args(["-vf", "format=gray,scale=96:-2", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg lancé");
    let raster = output.stdout;
    assert!(!raster.is_empty(), "image vide : {path}");
    let total: f64 = raster.iter().map(|value| f64::from(*value)).sum();
    total / raster.len() as f64
}

#[tokio::test]
#[ignore = "nécessite ffmpeg installé"]
async fn the_frame_the_editor_shows_becomes_the_picture() {
    let harness = Harness::new().await;
    let admin = member_token(&harness, GuildRole::Admin, "Chef").await;
    dark_then_light_video(&harness);
    let mut video = asset_row(&harness, "edit.mp4", MediaKind::Video, "video/mp4");
    // The size an ingest measured on the file itself.
    video.width = Some(320);
    video.height = Some(240);
    harness.db.media_assets().insert(&video).await.unwrap();
    let entry = item(&harness, &video).await;
    let item_id = entry.id.to_canonical();

    let (status, response) = cover(&harness, &admin, &item_id, json!({"atSec": 3.0})).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["cover"]["mime"], "image/jpeg");
    assert_eq!(response["cover"]["width"], 320);
    assert_eq!(response["cover"]["height"], 240);

    let cover_id = harness
        .db
        .media_assets()
        .cover_of(video.id)
        .await
        .unwrap()
        .expect("image enregistrée");
    let picture = harness
        .db
        .media_assets()
        .get(cover_id)
        .await
        .unwrap()
        .expect("ligne de l'image");
    assert_eq!(picture.mime, "image/jpeg");
    assert!(std::path::Path::new(&picture.storage_path).exists());
    // The player stood at 3 s: the bright half, not the dark opening.
    assert!(
        mean_luma(&picture.storage_path) > 60.0,
        "image noire : {}",
        picture.storage_path
    );

    let (_, items) = harness
        .call(
            Method::GET,
            "/overlay/media-board/items",
            Some(&admin),
            None,
        )
        .await;
    let listed = items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == item_id.as_str())
        .expect("élément listé");
    assert!(listed["thumbnail"]["url"].as_str().is_some());
    // The picture keeps its URL when it is replaced: the board tells the
    // clients its size, which changes with the frame.
    assert!(listed["thumbnail"]["version"].as_u64().unwrap() > 0);
}
