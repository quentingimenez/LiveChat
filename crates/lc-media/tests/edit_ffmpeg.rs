//! Edits run for real with ffmpeg on generated samples.
//!
//! Ignored by default (needs `ffmpeg` and `ffprobe` on the PATH):
//! `cargo test -p lc-media --test edit_ffmpeg -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lc_media::probe::{probe, ProbeInfo, PROBE_TIMEOUT};
use lc_media::{EditRecipe, MediaConfig, MediaKind, MediaPipeline, VideoEncoder};

fn config(directory: &Path) -> MediaConfig {
    let mut config = MediaConfig::default();
    config.storage.directory = directory.join("media");
    config.video.encoder = VideoEncoder::LibX264;
    config
}

fn ffmpeg(config: &MediaConfig, args: &[&str]) {
    let status = std::process::Command::new(&config.tools.ffmpeg)
        .args(["-y", "-hide_banner", "-v", "error"])
        .args(args)
        .status()
        .expect("ffmpeg lancé");
    assert!(status.success(), "ffmpeg a échoué : {args:?}");
}

/// 6 s, 320×240, 30 fps, with a sine track.
fn sample_video(config: &MediaConfig, directory: &Path) -> PathBuf {
    let path = directory.join("video.mp4");
    ffmpeg(
        config,
        &[
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
            path.to_str().unwrap(),
        ],
    );
    path
}

/// 6 s, 320×240, without any audio stream.
fn silent_video(config: &MediaConfig, directory: &Path) -> PathBuf {
    let path = directory.join("silent.mp4");
    ffmpeg(
        config,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=6:size=320x240:rate=30",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            path.to_str().unwrap(),
        ],
    );
    path
}

/// 6 s MP3.
fn sample_audio(config: &MediaConfig, directory: &Path) -> PathBuf {
    let path = directory.join("audio.mp3");
    ffmpeg(
        config,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=6",
            "-c:a",
            "libmp3lame",
            path.to_str().unwrap(),
        ],
    );
    path
}

async fn render(
    pipeline: &MediaPipeline,
    input: &Path,
    kind: MediaKind,
    recipe: &str,
) -> (ProbeInfo, PathBuf, lc_media::RenderedEdit) {
    let recipe: EditRecipe = serde_json::from_str(recipe).unwrap();
    let rendered = pipeline
        .render_edit(input, kind, &recipe)
        .await
        .unwrap_or_else(|error| panic!("édition refusée par ffmpeg ({recipe:?}) : {error}"));
    let probed = probe(
        &pipeline.config().tools.ffprobe,
        &rendered.path,
        PROBE_TIMEOUT,
    )
    .await
    .expect("sonde du résultat");
    assert_eq!(rendered.content_sha256.len(), 64);
    let path = rendered.path.clone();
    (probed, path, rendered)
}

fn assert_duration(probed: &ProbeInfo, expected: f64) {
    let duration = probed.duration_sec.expect("durée");
    assert!(
        (duration - expected).abs() < 0.35,
        "durée {duration} au lieu de {expected}"
    );
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn video_edits_run_with_real_ffmpeg() {
    let directory = tempfile::TempDir::new().unwrap();
    let config = config(directory.path());
    let pipeline = MediaPipeline::new(config.clone()).unwrap();
    let video = sample_video(&config, directory.path());

    // Neutral edit: copy-compatible MP4.
    let (probed, _, _kept) = render(&pipeline, &video, MediaKind::Video, "{}").await;
    assert_eq!(probed.video_codec.as_deref(), Some("h264"));
    assert_eq!(probed.audio_codec.as_deref(), Some("aac"));
    assert_eq!(probed.pixel_format.as_deref(), Some("yuv420p"));
    assert_eq!((probed.width, probed.height), (Some(320), Some(240)));
    assert_duration(&probed, 6.0);
    assert!(lc_media::transcode::can_copy_without_transcode(
        &probed,
        &config.video
    ));

    // One range.
    let (probed, _, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"segments":[[1,3.5]]}"#,
    )
    .await;
    assert_duration(&probed, 2.5);

    // Several ranges.
    let (probed, _, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"segments":[[0,1],[2,3],[4.5,6]]}"#,
    )
    .await;
    assert_duration(&probed, 3.5);
    assert!(probed.has_audio && probed.has_video);

    // Crop, then rotate: half the width, then turned on its side.
    let (probed, _, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"crop":{"x":0.25,"y":0,"width":0.5,"height":1},"rotate":90,"flipH":true,"flipV":true}"#,
    )
    .await;
    assert_eq!((probed.width, probed.height), (Some(240), Some(160)));
    let (probed, _, _kept) = render(&pipeline, &video, MediaKind::Video, r#"{"rotate":270}"#).await;
    assert_eq!((probed.width, probed.height), (Some(240), Some(320)));
    let (probed, _, _kept) = render(&pipeline, &video, MediaKind::Video, r#"{"rotate":180}"#).await;
    assert_eq!((probed.width, probed.height), (Some(320), Some(240)));

    // Odd crop sizes still give even sides.
    let (probed, _, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"crop":{"x":0.013,"y":0.017,"width":0.333,"height":0.777}}"#,
    )
    .await;
    assert_eq!(probed.width.unwrap() % 2, 0);
    assert_eq!(probed.height.unwrap() % 2, 0);

    // Speed.
    let (probed, _, _kept) = render(&pipeline, &video, MediaKind::Video, r#"{"speed":2}"#).await;
    assert_duration(&probed, 3.0);
    let (probed, _, _kept) = render(&pipeline, &video, MediaKind::Video, r#"{"speed":0.5}"#).await;
    assert_duration(&probed, 12.0);
    let (probed, _, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"speed":4,"segments":[[0,4]]}"#,
    )
    .await;
    assert_duration(&probed, 1.0);
    let (probed, _, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"speed":0.25,"segments":[[0,1]]}"#,
    )
    .await;
    assert_duration(&probed, 4.0);

    // Mute.
    let (probed, _, _kept) = render(&pipeline, &video, MediaKind::Video, r#"{"mute":true}"#).await;
    assert!(probed.has_video && !probed.has_audio);

    // Volume, fades and reverse together, on a cut.
    let (probed, _, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"segments":[[0,2],[3,5]],"volume":2.5,"fadeInSec":0.5,"fadeOutSec":1,"reverse":true,"speed":2}"#,
    )
    .await;
    assert_duration(&probed, 2.0);
    assert!(probed.has_audio);

    // Audio extraction.
    let (probed, path, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"output":"audio","segments":[[1,4]],"fadeOutSec":1}"#,
    )
    .await;
    assert_eq!(path.extension().unwrap(), "mp3");
    assert_eq!(probed.audio_codec.as_deref(), Some("mp3"));
    assert!(!probed.has_video);
    assert_duration(&probed, 3.0);

    // GIF.
    let (probed, path, _kept) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"output":"gif","segments":[[1,3]],"crop":{"x":0,"y":0,"width":0.5,"height":0.5},"fadeInSec":0.5}"#,
    )
    .await;
    assert_eq!(path.extension().unwrap(), "gif");
    assert_eq!(probed.video_codec.as_deref(), Some("gif"));
    assert_eq!((probed.width, probed.height), (Some(160), Some(120)));
    let header = std::fs::read(&path).unwrap();
    assert!(header.starts_with(b"GIF8"));
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn silent_video_and_audio_edits_run_with_real_ffmpeg() {
    let directory = tempfile::TempDir::new().unwrap();
    let config = config(directory.path());
    let pipeline = MediaPipeline::new(config.clone()).unwrap();

    // A video without an audio stream.
    let silent = silent_video(&config, directory.path());
    let (probed, _, _kept) = render(
        &pipeline,
        &silent,
        MediaKind::Video,
        r#"{"segments":[[0,1],[2,3]],"speed":2,"volume":2,"fadeInSec":0.2,"fadeOutSec":0.2,"reverse":true,"rotate":90}"#,
    )
    .await;
    assert!(probed.has_video && !probed.has_audio);
    assert_duration(&probed, 1.0);
    let recipe: EditRecipe = serde_json::from_str(r#"{"output":"audio"}"#).unwrap();
    let error = pipeline
        .render_edit(&silent, MediaKind::Video, &recipe)
        .await
        .expect_err("pas de piste audio");
    assert!(error.message().contains("no audio track"), "{error}");

    // Audio.
    let audio = sample_audio(&config, directory.path());
    let (probed, path, _kept) = render(&pipeline, &audio, MediaKind::Audio, "{}").await;
    assert_eq!(path.extension().unwrap(), "mp3");
    assert_duration(&probed, 6.0);
    let (probed, _, _kept) = render(
        &pipeline,
        &audio,
        MediaKind::Audio,
        r#"{"segments":[[0.5,2],[3,5.5]],"speed":0.5,"volume":0.4,"fadeInSec":1,"fadeOutSec":1,"reverse":true}"#,
    )
    .await;
    assert_eq!(probed.audio_codec.as_deref(), Some("mp3"));
    assert_duration(&probed, 8.0);
    let (probed, _, _kept) = render(
        &pipeline,
        &audio,
        MediaKind::Audio,
        r#"{"segments":[[2,6.4]],"speed":4}"#,
    )
    .await;
    assert_duration(&probed, 1.0);

    // The recipe is checked against the probed duration.
    let recipe: EditRecipe = serde_json::from_str(r#"{"segments":[[8,9]]}"#).unwrap();
    assert!(pipeline
        .render_edit(&audio, MediaKind::Audio, &recipe)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn edited_files_are_ingested_without_a_second_encode() {
    let directory = tempfile::TempDir::new().unwrap();
    let config = config(directory.path());
    assert!(config.video.audio_normalize, "loudnorm on by default");
    let pipeline = MediaPipeline::new(config.clone()).unwrap();

    let video = sample_video(&config, directory.path());
    let recipe: EditRecipe = serde_json::from_str(r#"{"segments":[[1,3]]}"#).unwrap();
    let rendered = pipeline
        .render_edit(&video, MediaKind::Video, &recipe)
        .await
        .unwrap();
    let ingested = pipeline
        .ingest_edited_file_with_hash(
            &rendered.path,
            "clip.mp4",
            Some(rendered.plan.output_mime()),
            &rendered.content_sha256,
        )
        .await
        .unwrap();
    assert_eq!(ingested.normalized.kind, MediaKind::Video);
    assert_eq!(
        std::fs::read(&ingested.normalized.storage_path).unwrap(),
        std::fs::read(&rendered.path).unwrap(),
        "the edited video is stored as rendered"
    );

    let audio = sample_audio(&config, directory.path());
    let recipe: EditRecipe = serde_json::from_str(r#"{"volume":0.3}"#).unwrap();
    let rendered = pipeline
        .render_edit(&audio, MediaKind::Audio, &recipe)
        .await
        .unwrap();
    let ingested = pipeline
        .ingest_edited_file_with_hash(
            &rendered.path,
            "sound.mp3",
            Some(rendered.plan.output_mime()),
            &rendered.content_sha256,
        )
        .await
        .unwrap();
    assert_eq!(ingested.normalized.kind, MediaKind::Audio);
    assert_eq!(
        std::fs::read(&ingested.normalized.storage_path).unwrap(),
        std::fs::read(&rendered.path).unwrap(),
        "the edited audio keeps its volume: no loudnorm pass"
    );

    let gif_recipe: EditRecipe =
        serde_json::from_str(r#"{"output":"gif","segments":[[0,1]]}"#).unwrap();
    let rendered = pipeline
        .render_edit(&video, MediaKind::Video, &gif_recipe)
        .await
        .unwrap();
    let ingested = pipeline
        .ingest_edited_file_with_hash(
            &rendered.path,
            "clip.gif",
            Some(rendered.plan.output_mime()),
            &rendered.content_sha256,
        )
        .await
        .unwrap();
    assert_eq!(ingested.normalized.kind, MediaKind::Image);
    assert_eq!(ingested.normalized.mime, "image/gif");
}
