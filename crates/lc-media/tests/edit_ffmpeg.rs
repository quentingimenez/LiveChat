//! Edits run for real with ffmpeg on generated samples.
//!
//! Ignored by default (needs `ffmpeg` and `ffprobe` on the PATH):
//! `cargo test -p lc-media --test edit_ffmpeg -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
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
    assert!(error.message().contains("piste audio"), "{error}");

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

/// Mean luminance (0..255) of the frame at `time`.
fn brightness(config: &MediaConfig, path: &Path, time: &str) -> f64 {
    let output = std::process::Command::new(&config.tools.ffmpeg)
        .args(["-hide_banner", "-v", "error", "-ss", time, "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=32:24",
            "-pix_fmt",
            "gray",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .expect("ffmpeg lancé");
    assert!(output.status.success(), "image de {path:?} à {time}");
    assert_eq!(output.stdout.len(), 32 * 24);
    let sum: u64 = output.stdout.iter().map(|value| u64::from(*value)).sum();
    sum as f64 / output.stdout.len() as f64
}

/// Pixels of the first decoded frame at `time`, as RGB triples.
fn frame_pixels(config: &MediaConfig, path: &Path, time: &str, width: u32, height: u32) -> Vec<u8> {
    let output = std::process::Command::new(&config.tools.ffmpeg)
        .args(["-hide_banner", "-v", "error", "-ss", time, "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg lancé");
    assert!(output.status.success(), "image de {path:?} à {time}");
    assert_eq!(output.stdout.len(), (width * height * 3) as usize);
    output.stdout
}

fn pixel(frame: &[u8], width: u32, x: u32, y: u32) -> [i32; 3] {
    let start = ((y * width + x) * 3) as usize;
    [
        i32::from(frame[start]),
        i32::from(frame[start + 1]),
        i32::from(frame[start + 2]),
    ]
}

/// Variance of the luminance over a grid of samples of the frame.
fn luminance_variance(frame: &[u8], width: u32, x_range: (u32, u32), y_range: (u32, u32)) -> f64 {
    let mut values = Vec::new();
    for y in (y_range.0..y_range.1).step_by(4) {
        for x in (x_range.0..x_range.1).step_by(4) {
            let [red, green, blue] = pixel(frame, width, x, y);
            values
                .push(f64::from(red) * 0.299 + f64::from(green) * 0.587 + f64::from(blue) * 0.114);
        }
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn annotations_are_burnt_in_before_the_picture_edits() {
    let directory = tempfile::TempDir::new().unwrap();
    let config = config(directory.path());
    let pipeline = MediaPipeline::new(config.clone()).unwrap();
    let video = sample_video(&config, directory.path());

    // A transparent overlay with an opaque red square at (32, 32).
    let overlay = directory.path().join("annotations.png");
    ffmpeg(
        &config,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black@0.0:s=320x240:r=1,format=rgba",
            "-f",
            "lavfi",
            "-i",
            "color=c=red@1:s=64x64:r=1,format=rgba",
            "-filter_complex",
            "[0][1]overlay=32:32:format=auto",
            "-frames:v",
            "1",
            overlay.to_str().unwrap(),
        ],
    );
    let png = STANDARD.encode(std::fs::read(&overlay).unwrap());

    // A pixelated mask over the top-left quarter: the red and cyan bars that
    // meet around x=50 and x=60 end up in the same block, so they become one
    // colour.
    let recipe = serde_json::json!({
        "masks": [{"x": 0, "y": 0, "width": 0.25, "height": 0.25, "strength": 0.05}]
    })
    .to_string();
    let (probed, path, rendered) = render(&pipeline, &video, MediaKind::Video, &recipe).await;
    assert_eq!((probed.width, probed.height), (Some(320), Some(240)));
    assert_duration(&probed, 6.0);
    let plan = &rendered.plan;
    assert_eq!(
        plan.masks,
        vec![lc_media::edit::MaskPlan {
            x: 0,
            y: 0,
            width: 80,
            height: 60,
            mode: lc_media::edit::MaskMode::Pixel,
            strength: 16.0,
        }]
    );
    let source = frame_pixels(&config, &video, "1", 320, 240);
    let masked = frame_pixels(&config, &path, "1", 320, 240);
    let left = pixel(&masked, 320, 50, 30);
    let right = pixel(&masked, 320, 60, 30);
    for channel in 0..3 {
        assert!(
            (left[channel] - right[channel]).abs() <= 2,
            "le masque pixels devrait aplatir x=50 et x=60 : {left:?} / {right:?}"
        );
    }
    let source_left = pixel(&source, 320, 50, 30);
    assert!(
        (left[0] - source_left[0]).abs() > 60,
        "le masque pixels ne change pas x=50 : {left:?} au lieu de {source_left:?}"
    );

    // Overlay + blurred mask + brightness: the square is painted before the
    // adjustments, and the blurred half loses its detail.
    let recipe = serde_json::json!({
        "overlay": {"png": png, "width": 320, "height": 240},
        "masks": [{"x": 0.5, "y": 0.5, "width": 0.5, "height": 0.5, "mode": "blur", "strength": 0.2}],
        "adjust": {"brightness": 120}
    })
    .to_string();
    let (probed, path, rendered) = render(&pipeline, &video, MediaKind::Video, &recipe).await;
    assert!(rendered.plan.overlay && !rendered.plan.adjust.is_neutral());
    assert_eq!((probed.width, probed.height), (Some(320), Some(240)));
    assert_duration(&probed, 6.0);

    let annotated = frame_pixels(&config, &path, "2", 320, 240);
    let red = pixel(&annotated, 320, 64, 64);
    assert!(
        red[0] > 200 && red[1] < 90 && red[2] < 90,
        "le carré rouge de l'overlay manque à (64, 64) : {red:?}"
    );
    let plain = brightness(&config, &video, "2");
    let adjusted = brightness(&config, &path, "2");
    assert!(
        adjusted > plain + 5.0,
        "l'éclaircissement ne s'applique pas : {adjusted:.1} au lieu de {plain:.1}"
    );
    let source = frame_pixels(&config, &video, "2", 320, 240);
    let blurred = luminance_variance(&annotated, 320, (200, 300), (150, 230));
    let detailed = luminance_variance(&source, 320, (200, 300), (150, 230));
    assert!(
        blurred * 4.0 < detailed,
        "la zone floutée garde son détail : variance {blurred:.1} contre {detailed:.1}"
    );
}

#[tokio::test]
#[ignore = "nécessite ffmpeg/ffprobe installés"]
async fn a_long_video_is_reversed_in_pieces() {
    let directory = tempfile::TempDir::new().unwrap();
    let config = config(directory.path());
    let pipeline = MediaPipeline::new(config.clone()).unwrap();
    // 12 s whose picture gets brighter every second, with a sine track.
    let video = directory.path().join("long.mp4");
    ffmpeg(
        &config,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:r=30:d=12,format=yuv420p,geq=lum='16+T*18':cb=128:cr=128",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=12",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            video.to_str().unwrap(),
        ],
    );

    // 11 s kept over a cut: three pieces of about 3.7 s.
    let (probed, path, rendered) = render(
        &pipeline,
        &video,
        MediaKind::Video,
        r#"{"segments":[[0,5],[6,12]],"reverse":true}"#,
    )
    .await;
    assert!(rendered.plan.reverses_in_pieces());
    assert!(probed.has_video && probed.has_audio);
    assert_eq!(probed.video_codec.as_deref(), Some("h264"));
    assert_duration(&probed, 11.0);

    // Result time → source time: 0 → 11.95, 2 → 10 (first piece),
    // 5 → 7 (second piece), 7.3 → 3.7 (across the cut), 10.9 → 0.1.
    for (result, source) in [
        ("0", "11.95"),
        ("2", "10"),
        ("5", "7"),
        ("7.3", "3.7"),
        ("10.9", "0.1"),
    ] {
        let got = brightness(&config, &path, result);
        let expected = brightness(&config, &video, source);
        assert!(
            (got - expected).abs() < 4.0,
            "à {result} s : luminosité {got:.1} au lieu de {expected:.1} (source à {source} s)"
        );
    }

    // Only the result is left next to it.
    let left: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["edit.mp4".to_string()]);
}
