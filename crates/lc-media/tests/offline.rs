//! Tests hors ligne : parsing des fixtures, canonicalisation, erreurs, cache.
//!
//! Aucun réseau ni binaire externe n'est requis.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lc_media::config::MediaConfig;
use lc_media::resolve::{tiktok, twitter, youtube};
use lc_media::{MediaError, MediaErrorCode};

const TIKTOK_VIDEO: &str = include_str!("fixtures/tiktok_video.html");
const TIKTOK_PHOTO: &str = include_str!("fixtures/tiktok_photo.html");
const TIKTOK_OEMBED: &str = include_str!("fixtures/tiktok_oembed.json");
const TWITTER_SYNDICATION: &str = include_str!("fixtures/twitter_syndication.json");
const YOUTUBE_OEMBED: &str = include_str!("fixtures/youtube_oembed.json");
const YTDLP_DUMP: &str = include_str!("fixtures/yt_dlp_dump.json");

#[test]
fn parses_tiktok_video_page() {
    let page = tiktok::parse_page_html(
        TIKTOK_VIDEO,
        "https://www.tiktok.com/@livechat/video/7300000000000000001",
    );
    assert_eq!(page.item_id.as_deref(), Some("7300000000000000001"));
    assert_eq!(
        page.canonical_url.as_deref(),
        Some("https://www.tiktok.com/@livechat/video/7300000000000000001")
    );
    assert_eq!(page.title.as_deref(), Some("Chaton mignon"));
    assert_eq!(page.author.as_deref(), Some("livechat"));
    assert!(!page.auth_required);
    assert!(!page.should_build_carousel());
    assert!(page
        .video_candidates
        .iter()
        .any(|candidate| candidate.url.contains("video.mp4")));
    assert!(!page.audio_candidates.is_empty());
}

#[test]
fn parses_tiktok_photo_page() {
    let page = tiktok::parse_page_html(
        TIKTOK_PHOTO,
        "https://www.tiktok.com/@livechat/photo/7300000000000000002",
    );
    assert_eq!(page.photo_slides.len(), 3);
    assert!(page.should_build_carousel());
    assert!(!page.audio_candidates.is_empty());
    assert!(page
        .photo_slides
        .iter()
        .flatten()
        .all(|candidate| candidate.url.starts_with("http")));
}

#[test]
fn tiktok_oembed_urls_are_built() {
    assert_eq!(
        tiktok::build_embed_url("7300000000000000001"),
        "https://www.tiktok.com/embed/v2/7300000000000000001"
    );
    assert!(
        tiktok::build_oembed_url("https://www.tiktok.com/@livechat/video/1")
            .contains("url=https%3A%2F%2Fwww.tiktok.com")
    );
    assert!(TIKTOK_OEMBED.contains("Chaton mignon"));
}

#[test]
fn twitter_variants_are_collected_and_picked() {
    let value: serde_json::Value = serde_json::from_str(TWITTER_SYNDICATION).unwrap();
    let variants = twitter::collect_variants(&value);
    assert_eq!(variants.len(), 3);
    let best = twitter::pick_variant(&variants).unwrap();
    assert!(best.url.ends_with("high.mp4"));
    assert!(best.content_type.contains("mp4"));
    assert_eq!(twitter::extract_dimensions(&value), Some((1280, 720)));
    assert_eq!(
        twitter::build_syndication_url("1700000000000000001", "fr"),
        "https://cdn.syndication.twimg.com/tweet-result?id=1700000000000000001&lang=fr&token=1"
    );
    assert_eq!(
        twitter::build_fallback_url(
            "https://vxtwitter.com",
            "https://x.com/livechat/status/1700000000000000001"
        ),
        "https://vxtwitter.com/livechat/status/1700000000000000001"
    );
}

#[test]
fn youtube_oembed_and_ytdlp_dump_fixtures() {
    let oembed: youtube::YoutubeOEmbed = serde_json::from_str(YOUTUBE_OEMBED).unwrap();
    assert_eq!(oembed.title.as_deref(), Some("Vidéo de test LiveChat"));
    assert_eq!(oembed.author_name.as_deref(), Some("LiveChat"));
    let metadata = youtube::parse_ytdlp_dump(YTDLP_DUMP).unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Vidéo de test LiveChat"));
    assert_eq!(metadata.author.as_deref(), Some("LiveChat"));
    assert_eq!(metadata.width, Some(1920));
    assert_eq!(metadata.height, Some(1080));
    assert!(!metadata.is_vertical());
    let oembed_url = youtube::build_oembed_url(
        "https://www.youtube.com/oembed",
        "https://www.youtube.com/watch?v=abc",
    );
    assert!(oembed_url.contains("format=json"));
    assert!(oembed_url.contains("url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Dabc"));
}

#[test]
fn errors_map_legacy_codes_and_priorities() {
    assert_eq!(
        MediaError::from_http_status(401).code(),
        MediaErrorCode::PrivateOrAuthRequired
    );
    assert_eq!(
        MediaError::from_http_status(404).code(),
        MediaErrorCode::MediaNotFound
    );
    assert_eq!(
        MediaError::from_http_status(504).code(),
        MediaErrorCode::DownloadTimeout
    );
    let drm = MediaError::classify(
        "Widevine DRM protected stream",
        MediaErrorCode::DownloadFailed,
    );
    assert_eq!(drm.code(), MediaErrorCode::DrmProtected);
    let transcode = MediaError::from_command_failure(
        "ffmpeg",
        "",
        "Invalid data found when processing input",
        MediaErrorCode::TranscodeFailed,
    );
    assert_eq!(transcode.code(), MediaErrorCode::InvalidMedia);
    assert_eq!(drm.legacy_http_status(), 422);
    assert!(MediaErrorCode::ALL
        .windows(2)
        .all(|pair| pair[0].priority() > pair[1].priority()));
}

#[test]
fn config_overrides_storage_and_encoder() {
    let config = MediaConfig::from_pairs([
        ("MEDIA_STORAGE_DIR", "/tmp/lc-media"),
        ("MEDIA_VIDEO_ENCODER", "libx264"),
        ("MEDIA_CACHE_TTL_HOURS", "2"),
        ("MEDIA_MAX_SIZE_MB", "64"),
    ]);
    assert_eq!(
        config.storage.directory,
        std::path::PathBuf::from("/tmp/lc-media")
    );
    assert_eq!(config.video.encoder, lc_media::VideoEncoder::LibX264);
    assert_eq!(
        config.storage.cache_ttl,
        std::time::Duration::from_secs(2 * 3600)
    );
    assert_eq!(config.download.max_size_bytes, 64 * 1024 * 1024);
}
