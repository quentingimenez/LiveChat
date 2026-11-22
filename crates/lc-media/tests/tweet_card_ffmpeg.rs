//! Composition réelle d'une carte tweet vidéo avec ffmpeg.
//!
//! Ignoré par défaut (nécessite `ffmpeg` sur le PATH) :
//! `cargo test -p lc-media --test tweet_card_ffmpeg -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lc_media::tweet_card::{render_tweet_card_with_slots, TweetCardContent, TweetVideo};
use lc_media::tweet_card_video::compose_tweet_card_video;
use lc_media::MediaConfig;

fn config(directory: &Path) -> MediaConfig {
    let mut config = MediaConfig::default();
    config.storage.directory = directory.join("media");
    config
}

/// Extrait la durée et les dimensions annoncées par ffmpeg.
///
/// `ffmpeg -i fichier` sans sortie termine avec un code non nul : on lit
/// seulement le compte rendu écrit sur la sortie d'erreur.
fn probe(ffmpeg: &Path, path: &Path) -> (String, String) {
    let output = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-i", path.to_str().unwrap()])
        .output()
        .expect("ffmpeg démarré");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let duration = stderr
        .lines()
        .find(|line| line.trim_start().starts_with("Duration:"))
        .map(|line| line.trim().to_string())
        .expect("durée absente");
    let stream = stderr
        .lines()
        .find(|line| line.contains("Video: h264"))
        .map(|line| line.trim().to_string())
        .expect("flux vidéo absent");
    (duration, stream)
}

fn sample_video(config: &MediaConfig, directory: &Path) -> PathBuf {
    let path = directory.join("clip.mp4");
    let status = std::process::Command::new(&config.tools.ffmpeg)
        .args([
            "-y",
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=4:size=1920x1080:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=4",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg démarré");
    assert!(
        status.success(),
        "génération de la vidéo de test impossible"
    );
    path
}

fn content(aspect: f32) -> TweetCardContent {
    TweetCardContent {
        author_name: "Elon Musk".to_string(),
        author_handle: "elonmusk".to_string(),
        text: "Entering Twitter HQ – let that sink in!".to_string(),
        created_at: Some("2022-10-26T23:11:33.000Z".to_string()),
        avatar: None,
        photos: Vec::new(),
        video: Some(TweetVideo { aspect }),
        quoted: None,
        reply_parent: None,
    }
}

#[tokio::test]
#[ignore = "nécessite ffmpeg installé"]
async fn composes_a_tweet_video_into_the_card() {
    let temp = tempfile::TempDir::new().unwrap();
    let config = config(temp.path());
    let clip = sample_video(&config, temp.path());

    // Vidéo 16:9 : la carte réserve une zone de la largeur de la carte.
    let card = render_tweet_card_with_slots(&content(16.0 / 9.0)).unwrap();
    assert_eq!(card.video_slots.len(), 1);
    let slot = card.video_slots[0];

    let output = compose_tweet_card_video(&config, temp.path(), &card, Some(&clip), None)
        .await
        .unwrap();
    assert!(output.exists(), "MP4 de carte absent");

    let (duration, stream) = probe(&config.tools.ffmpeg, &output);
    // La carte garde la durée de la vidéo du tweet.
    assert!(
        duration.contains("00:00:04"),
        "durée inattendue : {duration}"
    );
    // Le fichier garde la largeur de la carte, pas celle de la vidéo source.
    assert!(
        stream.contains("1080x"),
        "largeur de carte absente du flux : {stream}"
    );
    assert!(
        stream.contains("h264"),
        "le flux composité doit rester du H.264 : {stream}"
    );
    // La zone vidéo est bien à l'intérieur de la carte.
    assert!(slot.y > 0 && slot.height > 0);
    assert_eq!(slot.width % 2, 0);
    assert_eq!(slot.height % 2, 0);
}

#[tokio::test]
#[ignore = "nécessite ffmpeg installé"]
async fn composes_a_vertical_tweet_video_without_cropping() {
    let temp = tempfile::TempDir::new().unwrap();
    let config = config(temp.path());
    let clip = sample_video(&config, temp.path());
    let card = render_tweet_card_with_slots(&content(9.0 / 16.0)).unwrap();
    let output = compose_tweet_card_video(&config, temp.path(), &card, Some(&clip), None)
        .await
        .unwrap();
    let (_, stream) = probe(&config.tools.ffmpeg, &output);
    assert!(
        stream.contains("1080x"),
        "largeur de carte absente du flux : {stream}"
    );
}
