//! Tests d'intégration réseau et binaires externes (`yt-dlp`, `ffmpeg`).
//!
//! Tous les tests sont `#[ignore]` : ils nécessitent un accès Internet et/ou
//! les binaires installés sur la machine. Lancer avec :
//! `cargo test -p lc-media --test network -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lc_media::{normalize_downloaded_media, MediaConfig, MediaKind, MediaPipeline};

#[tokio::test]
#[ignore = "nécessite un accès réseau + yt-dlp + ffmpeg"]
async fn ingests_youtube_source_end_to_end() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    config.download.max_size_bytes = 64 * 1024 * 1024;
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        .await
        .unwrap();
    assert_eq!(media.normalized.kind, MediaKind::Video);
    assert!(media.normalized.storage_path.exists());
    assert!(media.normalized.size_bytes > 1024);
    assert_eq!(media.source.source_kind, lc_media::SourceKind::YouTube);
}

#[tokio::test]
#[ignore = "nécessite un accès réseau + yt-dlp"]
async fn resolves_tweet_card() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://x.com/livechat/status/1700000000000000001")
        .await;
    // Le tweet de test n'existe pas forcément : on vérifie surtout que le
    // pipeline classe l'échec au lieu de paniquer.
    match media {
        Ok(media) => {
            assert!(media.resolved.is_some());
            assert!(media.normalized.storage_path.exists());
        }
        Err(error) => {
            assert!(matches!(
                error.code(),
                lc_media::MediaErrorCode::MediaNotFound
                    | lc_media::MediaErrorCode::PrivateOrAuthRequired
                    | lc_media::MediaErrorCode::DownloadFailed
                    | lc_media::MediaErrorCode::InvalidMedia
            ));
        }
    }
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn normalizes_generated_sample_with_ffmpeg() {
    let temp = tempfile::TempDir::new().unwrap();
    let config = MediaConfig::default();
    let source = temp.path().join("sample.mp4");
    let status = std::process::Command::new(&config.tools.ffmpeg)
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=10",
            "-t",
            "1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg n'est pas disponible");
    let output_base = temp.path().join("out").join("hash");
    let normalized = normalize_downloaded_media(&source, Some("video/mp4"), &output_base, &config)
        .await
        .unwrap();
    assert_eq!(normalized.kind, MediaKind::Video);
    assert!(normalized.storage_path.exists());
    assert!(normalized.duration_sec.unwrap_or_default() > 0);
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn resolves_tiktok_page_online() {
    let config = MediaConfig::default();
    let source = lc_media::source::resolve_source(
        &config,
        "https://www.tiktok.com/@tiktok/video/7300000000000000000",
    )
    .await;
    assert!(source.is_ok());
}
