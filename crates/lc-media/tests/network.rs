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
#[ignore = "nécessite un accès réseau"]
async fn ingests_photo_tweet_as_a_card_image() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://x.com/i/status/2122725457946718290")
        .await
        .unwrap();
    // Un tweet avec photo garde l'habillage façon X : la carte PNG contient
    // l'en-tête, le texte et la photo, pas seulement la photo brute.
    assert_eq!(media.normalized.kind, MediaKind::Image);
    assert_eq!(media.normalized.mime, "image/png");
    assert!(media.normalized.storage_path.exists());
    let bytes = std::fs::read(&media.normalized.storage_path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "pas une carte PNG");
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    assert!(
        height > 600,
        "la carte ne contient pas la photo du tweet : {height} px de haut"
    );
}

#[tokio::test]
#[ignore = "nécessite un accès réseau + ffmpeg"]
async fn ingests_a_video_tweet_as_a_card_video() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://x.com/elonmusk/status/1585341984679469056")
        .await
        .unwrap();
    // La vidéo du tweet est incrustée dans la carte (en-tête, texte, date) :
    // le média joué est un MP4 1080 de large et non la vidéo brute 1920×1080.
    assert_eq!(media.normalized.kind, MediaKind::Video);
    assert_eq!(media.normalized.mime, "video/mp4");
    assert_eq!(media.normalized.width, Some(1080));
    assert!(media.normalized.height.is_some_and(|height| height > 600));
    assert!(media.normalized.storage_path.exists());
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn ingests_text_tweet_as_a_card_image() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://x.com/jack/status/20")
        .await
        .unwrap();
    assert_eq!(media.normalized.kind, MediaKind::Image);
    assert!(media.normalized.storage_path.exists());
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn ingests_a_quoted_tweet_as_a_card_image() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://x.com/jspjspbref/status/2122727570772811781")
        .await
        .unwrap();
    // Le tweet parent n'a pas de média propre : la carte doit intégrer le
    // tweet cité (auteur, texte et photo), donc être plus haute qu'une carte
    // de texte seule.
    assert_eq!(media.normalized.kind, MediaKind::Image);
    assert!(media.normalized.storage_path.exists());
    assert_eq!(
        media
            .resolved
            .as_ref()
            .and_then(|resolved| resolved.title.as_deref()),
        Some("essaie de reclipser ta portière")
    );
    let bytes = std::fs::read(&media.normalized.storage_path).unwrap();
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    assert!(
        height > 600,
        "la carte ne contient pas le tweet cité : {height} px de haut"
    );
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

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn downloads_public_tiktok_video_from_cdn() {
    let temp = tempfile::TempDir::new().unwrap();
    let config = MediaConfig::default();
    let source = lc_media::source::resolve_source(
        &config,
        "https://www.tiktok.com/@tiktok/video/7106594312292453675",
    )
    .await
    .unwrap()
    .unwrap();
    let download = lc_media::download::download_tiktok(&source, &config, temp.path())
        .await
        .unwrap();
    assert_eq!(download.kind, Some(MediaKind::Video));
    assert!(download.size_bytes > 10 * 1024);
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn downloads_a_second_public_tiktok_video_from_cdn() {
    let temp = tempfile::TempDir::new().unwrap();
    let config = MediaConfig::default();
    let source = lc_media::source::resolve_source(
        &config,
        "https://www.tiktok.com/@tiktok/video/7532540099460893983",
    )
    .await
    .unwrap()
    .unwrap();
    let download = lc_media::download::download_tiktok(&source, &config, temp.path())
        .await
        .unwrap();
    assert_eq!(download.kind, Some(MediaKind::Video));
    assert!(download.size_bytes > 10 * 1024);
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn ingests_a_tenor_page_end_to_end() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url(
            "https://tenor.com/view/jonathan-drouin-drouin-halifax-mooseheads-nathan-mackinnon-natejo-gif-14852894162018749597",
        )
        .await
        .unwrap();
    assert_eq!(media.source.source_kind, lc_media::SourceKind::Tenor);
    assert_eq!(
        media.source.canonical_url,
        "https://tenor.com/view/14852894162018749597"
    );
    // Le MP4 Open Graph est préféré au GIF : le média sort en vidéo.
    assert_eq!(media.normalized.kind, MediaKind::Video);
    assert!(media.normalized.storage_path.exists());
    assert!(media.normalized.size_bytes > 10 * 1024);
    let resolved = media.resolved.as_ref().unwrap();
    assert!(
        resolved.media_url.as_deref().unwrap().ends_with(".mp4"),
        "le MP4 Open Graph doit être préféré au GIF : {:?}",
        resolved.media_url
    );
    assert_eq!(
        resolved.title.as_deref(),
        Some("Jonathan Drouin Halifax Mooseheads")
    );
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn ingests_a_giphy_page_end_to_end() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://giphy.com/gifs/will-smith-men-in-black-R7m04yMaGWVeE")
        .await
        .unwrap();
    // La page Giphy est du HTML : le média vient de son `og:video`.
    assert_eq!(media.source.source_kind, lc_media::SourceKind::DirectMedia);
    assert_eq!(media.normalized.kind, MediaKind::Video);
    assert!(media.normalized.storage_path.exists());
    assert_eq!(
        media.download.title.as_deref(),
        Some("Will Smith Mib"),
        "le titre Open Graph doit être nettoyé"
    );
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn ingests_an_imgur_page_end_to_end() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://imgur.com/gallery/V9O1C74")
        .await
        .unwrap();
    // Imgur n'expose pas de `og:title` mais un `twitter:title`, et son
    // `og:video` est un MP4 direct.
    assert_eq!(media.normalized.kind, MediaKind::Video);
    assert!(media.normalized.storage_path.exists());
    assert!(media.download.title.is_some());
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn ingests_a_direct_image_without_a_file_extension() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url(
            "https://imgs.search.brave.com/Yhv2vfS8HbFKXPdkOfZxw4jSu0MMkYGqLKymxhx3p1Q/rs:fit:860:0:0:0/g:ce/aHR0cHM6Ly9pLnBp/bmltZy5jb20vb3Jp/Z2luYWxzLzE2LzU2/LzhhLzE2NTY4YTVl/Mzg1YjYyNmMzNDg4/ZTVmNmYzNGU2NmRh/LmpwZw",
        )
        .await
        .unwrap();
    // L'URL de proxy Brave n'a pas d'extension : le média doit être reconnu
    // d'après le `Content-Type` de la réponse, pas d'après le chemin.
    println!(
        "Brave image: kind={:?}, mime={:?}, size={} octets, dimensions={}x{}",
        media.normalized.kind,
        media.download.mime,
        media.download.size_bytes,
        media.normalized.width.unwrap_or(0),
        media.normalized.height.unwrap_or(0)
    );
    assert_eq!(media.source.source_kind, lc_media::SourceKind::DirectMedia);
    assert_eq!(media.normalized.kind, MediaKind::Image);
    assert_eq!(media.download.mime.as_deref(), Some("image/jpeg"));
    assert!(media.download.size_bytes > 0);
    assert!(media.normalized.storage_path.exists());
}

#[tokio::test]
#[ignore = "nécessite un accès réseau + yt-dlp"]
async fn ingests_a_page_without_open_graph_through_the_ytdlp_fallback() {
    let temp = tempfile::TempDir::new().unwrap();
    let mut config = MediaConfig::default();
    config.storage.directory = temp.path().join("media");
    let pipeline = MediaPipeline::new(config).unwrap();
    let media = pipeline
        .ingest_url("https://www.dailymotion.com/video/xbyyybi")
        .await
        .unwrap();
    // Dailymotion ne publie aucune balise Open Graph : le média ne peut venir
    // que du repli `yt-dlp`.
    assert_eq!(media.normalized.kind, MediaKind::Video);
    assert!(media.normalized.storage_path.exists());
    assert_eq!(media.source.source_kind, lc_media::SourceKind::DirectMedia);
    assert!(media.download.title.is_some());
}

#[tokio::test]
#[ignore = "nécessite un accès réseau"]
async fn downloads_tiktok_photo_post_with_audio() {
    let temp = tempfile::TempDir::new().unwrap();
    let config = MediaConfig::default();
    let source = lc_media::source::resolve_source(
        &config,
        "https://www.tiktok.com/@nightshade.sisters/photo/7437832857160109345",
    )
    .await
    .unwrap()
    .unwrap();
    let download = lc_media::download::download_tiktok(&source, &config, temp.path())
        .await
        .unwrap();
    assert_eq!(download.kind, Some(MediaKind::Video));
    assert!(download.size_bytes > 10 * 1024);
}
