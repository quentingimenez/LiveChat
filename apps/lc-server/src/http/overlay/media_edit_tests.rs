//! Tests of `POST /overlay/media-board/items/{itemId}/edit`, through the full
//! router.
//!
//! The guards run without ffmpeg. The edits themselves are `#[ignore]`: they
//! need `ffmpeg` and `ffprobe` on the PATH
//! (`cargo test -p lc-server media_edit_tests -- --ignored`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration as StdDuration, Instant};

use axum::http::{Method, StatusCode};
use lc_domain::{
    GuildRole, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind, MediaStatus,
    OverlaySessionMode,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use time::Duration;

use crate::ingestions::IngestionStatus;

use super::guild_tests::{fixed_now, token, Harness};
use super::media_upload_tests as upload;

/// A ready asset whose (fake) file lives in the server's media directory.
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
        title: Some("Clip".to_owned()),
        message: Some("aïe".to_owned()),
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

fn edit_uri(item_id: &str) -> String {
    format!("/overlay/media-board/items/{item_id}/edit")
}

async fn edit(harness: &Harness, token: &str, item_id: &str, body: Value) -> (StatusCode, Value) {
    harness
        .call(Method::POST, &edit_uri(item_id), Some(token), Some(body))
        .await
}

async fn member_token(harness: &Harness, role: GuildRole, name: &str) -> String {
    token(&harness.join(&harness.server_invite(role).await, name).await)
}

/// Polls the caller's ingestion list until `ingestion_id` is finished.
async fn finished_ingestion(harness: &Harness, token: &str, ingestion_id: &str) -> Value {
    let deadline = Instant::now() + StdDuration::from_secs(30);
    loop {
        let (status, body) = harness
            .call(
                Method::GET,
                "/overlay/media-board/ingestions",
                Some(token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let entry = body["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["ingestionId"] == ingestion_id)
            .cloned();
        if let Some(entry) = entry {
            if entry["status"] != "processing" {
                return entry;
            }
        }
        assert!(Instant::now() < deadline, "édition non terminée");
        tokio::time::sleep(StdDuration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_guest_is_refused_before_the_body_is_validated() {
    let harness = Harness::new().await;
    let guest = member_token(&harness, GuildRole::Guest, "Invité").await;
    let video = asset(&harness, "a.mp4", MediaKind::Video, "video/mp4").await;
    let entry = item(&harness, &video).await;
    for body in [json!({"recipe": {}}), json!({"nonsense": true})] {
        let (status, response) = edit(&harness, &guest, &entry.id.to_canonical(), body).await;
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
        (json!({}), "La recette d’édition est requise."),
        (
            json!({"recipe": []}),
            "La recette d’édition doit être un objet.",
        ),
        (
            json!({"recipe": {}, "replace": 1}),
            "Le paramètre replace doit être un booléen.",
        ),
        (
            json!({"recipe": {}, "title": 1}),
            "Le titre doit être une chaîne de caractères.",
        ),
    ] {
        let (status, response) = edit(&harness, &member, &unknown, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["error"], "invalid_payload");
        assert_eq!(response["message"], message);
    }
    let (status, response) = edit(
        &harness,
        &member,
        &unknown,
        json!({"recipe": {"output": "webm"}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(response["message"], "La recette d’édition est invalide.");
}

#[tokio::test]
async fn an_annotated_recipe_larger_than_a_megabyte_is_read() {
    let harness = Harness::new().await;
    let member = member_token(&harness, GuildRole::Member, "Bob").await;
    let video = asset(&harness, "a.mp4", MediaKind::Video, "video/mp4").await;
    let entry = item(&harness, &video).await;
    // 1.3 MB of base64: more than the global 1 MiB body limit, which is lifted
    // for this route because the annotations of a video travel as a PNG.
    let png = "A".repeat(1_300_000);
    let (status, response) = edit(
        &harness,
        &member,
        &entry.id.to_canonical(),
        json!({"recipe": {"overlay": {"png": png, "width": 480, "height": 360}}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(response["error"], "invalid_payload");
    assert_eq!(
        response["message"],
        "La couche d’annotation n’est pas un PNG."
    );
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
        let (status, response) = edit(&harness, &member, &item_id, json!({"recipe": {}})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{item_id}");
        assert_eq!(response["error"], "item_not_found");
    }
}

#[tokio::test]
async fn replacing_needs_the_right_to_edit_the_item_and_copying_the_right_to_add() {
    let harness = Harness::new().await;
    let author = member_token(&harness, GuildRole::Member, "Alice").await;
    let other = member_token(&harness, GuildRole::Member, "Bob").await;
    let admin = member_token(&harness, GuildRole::Admin, "Chef").await;
    let video = asset(&harness, "a.mp4", MediaKind::Video, "video/mp4").await;
    let entry = item(&harness, &video).await;
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

    let (status, response) = edit(
        &harness,
        &other,
        &item_id,
        json!({"replace": true, "recipe": {"speed": 2}}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    assert_eq!(response["error"], "forbidden");

    // Anyone who may add media can save an edited copy; the author and an
    // admin can also replace the original.
    for (token, body) in [
        (&other, json!({"recipe": {"speed": 2}})),
        (&author, json!({"replace": true, "recipe": {"speed": 2}})),
        (&admin, json!({"replace": true, "recipe": {"speed": 2}})),
    ] {
        let (status, response) = edit(&harness, token, &item_id, body).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{response}");
        assert!(response["ingestionId"].as_str().is_some());
    }
}

#[tokio::test]
async fn a_recipe_that_does_not_fit_the_media_is_refused() {
    let harness = Harness::new().await;
    let member = member_token(&harness, GuildRole::Member, "Bob").await;
    let video = asset(&harness, "a.mp4", MediaKind::Video, "video/mp4").await;
    let video_item = item(&harness, &video).await.id.to_canonical();
    let audio = asset(&harness, "a.mp3", MediaKind::Audio, "audio/mpeg").await;
    let audio_item = item(&harness, &audio).await.id.to_canonical();
    let image = asset(&harness, "a.png", MediaKind::Image, "image/png").await;
    let image_item = item(&harness, &image).await.id.to_canonical();

    for (item_id, body, message) in [
        (
            &image_item,
            json!({"recipe": {}}),
            "Les images se modifient directement sur le board.",
        ),
        (
            &video_item,
            json!({"recipe": {"segments": [[10, 45]]}}),
            "Le segment 1 dépasse la durée du média (30.00 s).",
        ),
        (
            &video_item,
            json!({"recipe": {"speed": 9}}),
            "La valeur de speed doit être comprise entre 0.25 et 4.",
        ),
        (
            &audio_item,
            json!({"recipe": {"output": "gif"}}),
            "Ce format de sortie n’est disponible que pour une vidéo.",
        ),
        (
            &video_item,
            json!({"replace": true, "recipe": {"output": "gif", "segments": [[0, 5]]}}),
            "Un remplacement doit produire le même type de média que l’original.",
        ),
        (
            &video_item,
            json!({"replace": true, "recipe": {"output": "audio"}}),
            "Un remplacement doit produire le même type de média que l’original.",
        ),
    ] {
        let (status, response) = edit(&harness, &member, item_id, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["error"], "invalid_payload");
        assert_eq!(response["message"], message);
    }
}

#[tokio::test]
async fn a_media_missing_from_the_disk_is_reported_at_once() {
    let harness = Harness::new().await;
    let member = member_token(&harness, GuildRole::Member, "Bob").await;
    let video = asset(&harness, "gone.mp4", MediaKind::Video, "video/mp4").await;
    let entry = item(&harness, &video).await;
    tokio::fs::remove_file(&video.storage_path).await.unwrap();
    let (status, response) = edit(
        &harness,
        &member,
        &entry.id.to_canonical(),
        json!({"recipe": {}}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(response["error"], "media_not_found_on_disk");
}

#[tokio::test]
async fn a_failed_edit_is_reported_through_the_ingestion() {
    let harness = Harness::new().await;
    let member = member_token(&harness, GuildRole::Member, "Bob").await;
    // Ten bytes are not a video: ffprobe (or its absence) fails the edit.
    let video = asset(&harness, "fake.mp4", MediaKind::Video, "video/mp4").await;
    let entry = item(&harness, &video).await;
    let (status, response) = edit(
        &harness,
        &member,
        &entry.id.to_canonical(),
        json!({"replace": true, "recipe": {"segments": [[1, 3]]}}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{response}");
    let ingestion_id = response["ingestionId"].as_str().unwrap();

    let finished = finished_ingestion(&harness, &member, ingestion_id).await;
    assert_eq!(finished["status"], "failed", "{finished}");
    assert_eq!(finished["error"]["code"], "media_ingestion_failed");
    assert!(finished["error"]["message"].as_str().is_some());
    let untouched = harness
        .db
        .media_board()
        .get(entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.media_asset_id, video.id);
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

async fn post_edit(app: &axum::Router, item_id: &str, body: Value) -> (StatusCode, Value) {
    let request = axum::http::Request::builder()
        .method(Method::POST)
        .uri(edit_uri(item_id))
        .header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", upload::TOKEN),
        )
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    upload::send_json(app, request).await
}

fn listed<'a>(items: &'a Value, item_id: &str) -> &'a Value {
    items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == item_id)
        .expect("item listé")
}

/// Attend l'image qu'une vidéo reçoit après son ingestion.
///
/// L'image est extraite dans une tâche de fond : elle arrive après l'état
/// `Ready` et son arrivée est annoncée comme une mise à jour de l'élément, ce
/// qui rend un comptage d'événements dépendant de son délai.
async fn wait_for_cover(db: &lc_db::Db, asset_id: &str) {
    let id: MediaAssetId = asset_id.parse().unwrap();
    for _ in 0..600 {
        if db.media_assets().cover_of(id).await.unwrap().is_some() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("aucune image pour le média {asset_id}");
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn a_video_is_edited_as_a_copy_then_replaced_with_real_ffmpeg() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, hub) =
        upload::harness_with_real_tools(&dir.path().join("media"), 16).await;
    let client = upload::insert_client(&db, OverlaySessionMode::Normal, upload::TOKEN).await;
    let client_id = client.id.as_uuid();

    let sample = dir.path().join("sample.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=6:size=320x240:rate=30",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=6",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        sample.to_str().unwrap(),
    ]);
    let (status, body) = upload::upload_file(
        &app,
        upload::TOKEN,
        json!({"filename": "chat.mp4", "title": "Chat", "message": "miaou"}),
        &std::fs::read(&sample).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let uploaded =
        upload::wait_finished(&state, client_id, body["ingestionId"].as_str().unwrap()).await;
    assert_eq!(uploaded.status, IngestionStatus::Ready, "{uploaded:?}");
    let original_id = uploaded.item_id.unwrap();
    let (_, items) = upload::send_json(&app, upload::items_request(upload::TOKEN)).await;
    let original_asset = listed(&items, &original_id)["mediaAssetId"]
        .as_str()
        .unwrap()
        .to_owned();
    // `durationSec` is the ceiling of the muxed duration: the AAC padding makes
    // ffmpeg 6.1 write 6.008 s for the 6 s sample (reported as 7 s), where
    // ffmpeg 9 writes 6.000 s (reported as 6 s). Pin the second, not the build.
    let duration = listed(&items, &original_id)["media"]["durationSec"]
        .as_i64()
        .expect("durationSec");
    assert!(
        (6..=7).contains(&duration),
        "durationSec {duration}, attendu 6 ou 7 s"
    );

    // The upload text is not a board message; write one by hand, like the
    // board's inline field, so the edited copy can be checked against it.
    let stored = db
        .media_board()
        .get(original_id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    db.media_board()
        .update(
            stored.id,
            &lc_db::BoardItemEdit {
                title: stored.title.as_deref(),
                message: Some("miaou"),
                artwork_on_overlay: None,
                display_duration_sec: None,
                display_scale_percent: None,
            },
        )
        .await
        .unwrap();

    // --- an edited copy: two ranges, half the frame, turned on its side.
    let (status, body) = post_edit(
        &app,
        &original_id,
        json!({"recipe": {
            "segments": [[0, 1], [3, 4]],
            "crop": {"x": 0.5, "y": 0, "width": 0.5, "height": 1},
            "rotate": 90,
        }}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let copy =
        upload::wait_finished(&state, client_id, body["ingestionId"].as_str().unwrap()).await;
    assert_eq!(copy.status, IngestionStatus::Ready, "{copy:?}");
    assert_eq!(copy.created, Some(true));
    let copy_id = copy.item_id.unwrap();
    assert_ne!(copy_id, original_id);
    assert_eq!(hub.added_count(&upload::guild()), 2);

    let (_, items) = upload::send_json(&app, upload::items_request(upload::TOKEN)).await;
    assert_eq!(items["total"], 2);
    let copied = listed(&items, &copy_id);
    assert_eq!(copied["title"], "Chat (édité)");
    assert_eq!(copied["message"], "miaou");
    assert_eq!(copied["media"]["kind"], "video");
    assert_eq!(copied["media"]["mime"], "video/mp4");
    // The copy re-encodes the AAC track: codec padding can push the muxed
    // duration just over 2 s, so `durationSec` reports 2 s or 3 s (same ffmpeg
    // 6.1 padding as the video edit above).
    let duration = copied["media"]["durationSec"]
        .as_i64()
        .expect("durationSec");
    assert!(
        (2..=3).contains(&duration),
        "durationSec {duration}, attendu 2 ou 3 s"
    );
    assert_eq!(copied["media"]["width"], 240);
    assert_eq!(copied["media"]["height"], 160);
    assert_eq!(copied["media"]["sourceUrl"], "upload:Chat (édité).mp4");
    // The original is untouched.
    assert_eq!(
        listed(&items, &original_id)["mediaAssetId"],
        original_asset.as_str()
    );

    // --- a copy with its own title, as a sound and as a GIF.
    for (recipe, title, kind, mime) in [
        (
            json!({"output": "audio", "segments": [[1, 4]]}),
            "Le son",
            "audio",
            "audio/mpeg",
        ),
        (
            json!({"output": "gif", "segments": [[0, 2]]}),
            "Le gif",
            "image",
            "image/gif",
        ),
    ] {
        let (status, body) = post_edit(
            &app,
            &original_id,
            json!({"title": title, "recipe": recipe}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let finished =
            upload::wait_finished(&state, client_id, body["ingestionId"].as_str().unwrap()).await;
        assert_eq!(finished.status, IngestionStatus::Ready, "{finished:?}");
        assert_eq!(finished.created, Some(true));
        let (_, items) = upload::send_json(&app, upload::items_request(upload::TOKEN)).await;
        let entry = listed(&items, finished.item_id.as_deref().unwrap());
        assert_eq!(entry["title"], title);
        assert_eq!(entry["media"]["kind"], kind);
        assert_eq!(entry["media"]["mime"], mime);
    }
    assert_eq!(hub.added_count(&upload::guild()), 4);
    // Chaque vidéo reçoit une image après coup et son arrivée met l'élément à
    // jour : deux vidéos, deux mises à jour — aucune pour le son ni le GIF.
    wait_for_cover(&db, &original_asset).await;
    wait_for_cover(&db, copied["mediaAssetId"].as_str().unwrap()).await;
    assert_eq!(hub.updated_count(&upload::guild()), 2);

    // --- replacing the original: twice as fast, muted.
    let (status, body) = post_edit(
        &app,
        &original_id,
        json!({"replace": true, "title": "Ignored", "recipe": {"speed": 2, "mute": true}}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let replaced =
        upload::wait_finished(&state, client_id, body["ingestionId"].as_str().unwrap()).await;
    assert_eq!(replaced.status, IngestionStatus::Ready, "{replaced:?}");
    assert_eq!(replaced.created, Some(false));
    assert_eq!(replaced.item_id.as_deref(), Some(original_id.as_str()));
    assert_eq!(hub.added_count(&upload::guild()), 4);

    let (_, items) = upload::send_json(&app, upload::items_request(upload::TOKEN)).await;
    assert_eq!(items["total"], 4);
    let entry = listed(&items, &original_id);
    // Le remplacement met l'élément à jour, et la nouvelle vidéo annonce la
    // sienne quand son image arrive : deux événements de plus.
    wait_for_cover(&db, entry["mediaAssetId"].as_str().unwrap()).await;
    assert_eq!(hub.updated_count(&upload::guild()), 4);
    assert_eq!(entry["title"], "Chat");
    assert_eq!(entry["message"], "miaou");
    assert_ne!(entry["mediaAssetId"], original_asset.as_str());
    // 3 s and a frame, rounded up to a second.
    let duration = entry["media"]["durationSec"].as_i64().unwrap();
    assert!((3..=4).contains(&duration), "durée {duration}");
    assert_eq!(entry["media"]["kind"], "video");

    let previous = db
        .media_assets()
        .get(original_asset.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(
        previous.expires_at < upload::fixed_now() + Duration::days(365),
        "previous asset released"
    );
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn an_audio_keeps_its_edited_volume_with_real_ffmpeg() {
    let dir = TempDir::new().unwrap();
    let (app, db, state, hub) =
        upload::harness_with_real_tools(&dir.path().join("media"), 16).await;
    let client = upload::insert_client(&db, OverlaySessionMode::Normal, upload::TOKEN).await;
    let client_id = client.id.as_uuid();

    let sample = dir.path().join("sample.mp3");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=6",
        "-c:a",
        "libmp3lame",
        sample.to_str().unwrap(),
    ]);
    let (status, body) = upload::upload_file(
        &app,
        upload::TOKEN,
        json!({"filename": "bip.mp3"}),
        &std::fs::read(&sample).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let uploaded =
        upload::wait_finished(&state, client_id, body["ingestionId"].as_str().unwrap()).await;
    assert_eq!(uploaded.status, IngestionStatus::Ready, "{uploaded:?}");
    let item_id = uploaded.item_id.unwrap();

    let (status, body) = post_edit(
        &app,
        &item_id,
        json!({"replace": true, "recipe": {
            "segments": [[1, 5]], "volume": 0.1, "fadeInSec": 0.5, "fadeOutSec": 0.5, "speed": 2,
        }}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let replaced =
        upload::wait_finished(&state, client_id, body["ingestionId"].as_str().unwrap()).await;
    assert_eq!(replaced.status, IngestionStatus::Ready, "{replaced:?}");
    assert_eq!(replaced.created, Some(false));
    assert_eq!(hub.updated_count(&upload::guild()), 1);

    let (_, items) = upload::send_json(&app, upload::items_request(upload::TOKEN)).await;
    assert_eq!(items["total"], 1);
    let entry = listed(&items, &item_id);
    assert_eq!(entry["title"], "bip");
    assert_eq!(entry["media"]["kind"], "audio");
    // 4 s at speed 2 = 2 s; the AAC padding can push the muxed duration just
    // over 2 s, so `durationSec` reports 2 s or 3 s (see the video edit above).
    let duration = entry["media"]["durationSec"].as_i64().expect("durationSec");
    assert!(
        (2..=3).contains(&duration),
        "durationSec {duration}, attendu 2 ou 3 s"
    );

    // The quiet edit stayed quiet: a second loudnorm pass would have brought
    // it back to the target loudness.
    let asset = db
        .media_assets()
        .get(entry["mediaAssetId"].as_str().unwrap().parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-i", &asset.storage_path])
        .args(["-af", "volumedetect", "-f", "null", "-"])
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stderr);
    let mean = report
        .lines()
        .find_map(|line| line.split("mean_volume: ").nth(1))
        .and_then(|value| value.trim_end_matches(" dB").trim().parse::<f64>().ok())
        .unwrap_or_else(|| panic!("volumedetect illisible : {report}"));
    assert!(
        mean < -30.0,
        "volume moyen {mean} dB : édition renormalisée ?"
    );
}
