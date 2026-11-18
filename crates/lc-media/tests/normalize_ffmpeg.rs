//! Video normalization run for real with ffmpeg on generated samples.
//!
//! Ignored by default (needs `ffmpeg` and `ffprobe` on the PATH):
//! `cargo test -p lc-media --test normalize_ffmpeg -- --ignored --nocapture`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lc_media::transcode::{build_carousel_video, carousel_dimensions, normalize_local_media};
use lc_media::{MediaConfig, MediaKind, VideoEncoder};

fn config(directory: &Path) -> MediaConfig {
    let mut config = MediaConfig::default();
    config.storage.directory = directory.join("media");
    config.video.encoder = VideoEncoder::LibX264;
    config
}

fn run(binary: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new(binary)
        .args(args)
        .output()
        .expect("tool started");
    assert!(
        output.status.success(),
        "{} failed: {args:?}\n{}",
        binary.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// 4 s, 320×240, 30 fps, H.264 yuv420p; `audio` is the ffmpeg audio codec
/// (`None`: silent).
fn sample(config: &MediaConfig, directory: &Path, name: &str, audio: Option<&str>) -> PathBuf {
    let path = directory.join(name);
    let mut args = vec![
        "-y",
        "-hide_banner",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc=duration=4:size=320x240:rate=30",
    ];
    if audio.is_some() {
        args.extend(["-f", "lavfi", "-i", "sine=frequency=440:duration=4"]);
    }
    args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p"]);
    if let Some(codec) = audio {
        args.extend(["-c:a", codec, "-shortest"]);
    }
    args.push(path.to_str().unwrap());
    run(&config.tools.ffmpeg, &args);
    path
}

/// The first audio track of `path` is within 1 LU of the loudness target.
fn assert_at_target(config: &MediaConfig, path: &Path) {
    let output = std::process::Command::new(&config.tools.ffmpeg)
        .args(lc_media::transcode::loudness_args(path))
        .output()
        .unwrap();
    let measured =
        lc_media::transcode::parse_loudness(&String::from_utf8_lossy(&output.stderr)).unwrap();
    let target = config.video.loudnorm.integrated_lufs;
    assert!(
        (measured.integrated_lufs - target).abs() <= 1.0,
        "{} LUFS, target {target}",
        measured.integrated_lufs
    );
    assert!(measured.true_peak_dbtp <= config.video.loudnorm.true_peak_dbtp + 0.5);
}

/// `codec_name,…` of the first stream of a kind (`v` or `a`); empty when
/// there is none.
fn stream(config: &MediaConfig, path: &Path, kind: &str, entries: &str) -> String {
    run(
        &config.tools.ffprobe,
        &[
            "-v",
            "error",
            "-select_streams",
            &format!("{kind}:0"),
            "-show_entries",
            &format!("stream={entries}"),
            "-of",
            "csv=p=0",
            path.to_str().unwrap(),
        ],
    )
}

/// MD5 of the compressed packets of a stream: equal before and after means
/// the stream was copied, not encoded again.
fn packets_md5(config: &MediaConfig, path: &Path, map: &str) -> String {
    run(
        &config.tools.ffmpeg,
        &[
            "-v",
            "error",
            "-i",
            path.to_str().unwrap(),
            "-map",
            map,
            "-c",
            "copy",
            "-f",
            "md5",
            "-",
        ],
    )
}

/// `true` when the `moov` atom comes before the media data (faststart).
fn is_faststart(path: &Path) -> bool {
    let bytes = std::fs::read(path).unwrap();
    let find = |needle: &[u8]| bytes.windows(4).position(|window| window == needle);
    match (find(b"moov"), find(b"mdat")) {
        (Some(moov), Some(mdat)) => moov < mdat,
        _ => false,
    }
}

async fn normalize(config: &MediaConfig, directory: &Path, input: &Path) -> PathBuf {
    let normalized = normalize_local_media(input, None, &directory.join("media/out"), config)
        .await
        .expect("normalized");
    assert_eq!(normalized.kind, MediaKind::Video);
    assert_eq!(normalized.mime, "video/mp4");
    assert_eq!(normalized.width, Some(320));
    assert_eq!(normalized.height, Some(240));
    assert_eq!(normalized.duration_sec, Some(4));
    assert!(is_faststart(&normalized.storage_path));
    assert_eq!(
        stream(config, &normalized.storage_path, "v", "codec_name,pix_fmt"),
        "h264,yuv420p"
    );
    normalized.storage_path
}

/// H.264 with a non-AAC track in Matroska (an OBS recording): the picture is
/// copied untouched, the sound becomes normalized AAC at 48 kHz.
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn h264_in_matroska_keeps_its_video_stream() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let input = sample(&config, dir.path(), "clip.mkv", Some("libopus"));

    let output = normalize(&config, dir.path(), &input).await;

    assert_eq!(
        packets_md5(&config, &output, "0:V:0"),
        packets_md5(&config, &input, "0:V:0"),
        "the video stream must be copied, not encoded again"
    );
    assert_eq!(
        stream(&config, &output, "a", "codec_name,sample_rate"),
        "aac,48000"
    );
}

/// MP4 whose only problem is its MP3 track.
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn mp4_with_an_mp3_track_only_converts_the_audio() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let input = sample(&config, dir.path(), "clip.mp4", Some("libmp3lame"));

    let output = normalize(&config, dir.path(), &input).await;

    assert_eq!(
        packets_md5(&config, &output, "0:V:0"),
        packets_md5(&config, &input, "0:V:0")
    );
    // A fixed gain keeps the source rate (loudnorm used to need 48 kHz).
    assert_eq!(
        stream(&config, &output, "a", "codec_name,sample_rate"),
        "aac,44100"
    );
    assert_at_target(&config, &output);
}

/// Normalization off and an AAC track: nothing is encoded at all.
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn h264_aac_in_matroska_is_only_remuxed_without_loudnorm() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.video.audio_normalize = false;
    let input = sample(&config, dir.path(), "clip.mkv", Some("aac"));

    let output = normalize(&config, dir.path(), &input).await;

    for map in ["0:V:0", "0:a:0"] {
        assert_eq!(
            packets_md5(&config, &output, map),
            packets_md5(&config, &input, map),
            "{map} must be copied"
        );
    }
}

/// A silent H.264 clip outside MP4 stays silent.
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn silent_h264_in_matroska_stays_silent() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let input = sample(&config, dir.path(), "silent.mkv", None);

    let output = normalize(&config, dir.path(), &input).await;

    assert_eq!(
        packets_md5(&config, &output, "0:V:0"),
        packets_md5(&config, &input, "0:V:0")
    );
    assert_eq!(stream(&config, &output, "a", "codec_name"), "");
}

/// An already normalized MP4 keeps its video stream; its audio goes through
/// loudnorm like every other media. With normalization off it is copied byte
/// for byte.
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn normalized_mp4_keeps_its_video_and_normalizes_its_audio() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let input = sample(&config, dir.path(), "clip.mp4", Some("aac"));

    let normalized = normalize_local_media(&input, None, &dir.path().join("media/out"), &config)
        .await
        .unwrap();

    assert_ne!(
        std::fs::read(&normalized.storage_path).unwrap(),
        std::fs::read(&input).unwrap()
    );
    assert_eq!(
        packets_md5(&config, &normalized.storage_path, "0:V:0"),
        packets_md5(&config, &input, "0:V:0")
    );
    assert_eq!(
        stream(&config, &normalized.storage_path, "a", "sample_rate"),
        "44100"
    );
    assert_at_target(&config, &normalized.storage_path);
    assert_eq!(normalized.duration_sec, Some(4));
    assert_eq!(normalized.width, Some(320));
    assert_eq!(normalized.height, Some(240));

    let mut off = config.clone();
    off.video.audio_normalize = false;
    let copied = normalize_local_media(&input, None, &dir.path().join("media/copy"), &off)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(&copied.storage_path).unwrap(),
        std::fs::read(&input).unwrap()
    );
}

/// Another video codec is still fully transcoded, and the normalized track
/// is 48 kHz (loudnorm alone would leave 96 kHz).
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn vp9_is_transcoded_to_h264_with_48_khz_audio() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let input = dir.path().join("clip.webm");
    run(
        &config.tools.ffmpeg,
        &[
            "-y",
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=4:size=320x240:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=4",
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-c:a",
            "libopus",
            "-shortest",
            input.to_str().unwrap(),
        ],
    );

    let output = normalize(&config, dir.path(), &input).await;

    assert_eq!(
        stream(&config, &output, "a", "codec_name,sample_rate"),
        "aac,48000"
    );
}

/// A stream that cannot be copied into MP4 falls back to the full transcode
/// instead of failing the ingestion: here the probe is lied to (the "H.264"
/// file is VP9), so the copy plan is chosen and ffmpeg refuses it.
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn a_refused_stream_copy_falls_back_to_the_transcode() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let input = dir.path().join("clip.webm");
    run(
        &config.tools.ffmpeg,
        &[
            "-y",
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=4:size=320x240:rate=30",
            "-c:v",
            "libvpx",
            "-deadline",
            "realtime",
            input.to_str().unwrap(),
        ],
    );
    // VP8 cannot be stored in MP4: `-c:v copy` fails for sure.
    let mut probed = lc_media::probe::probe(
        &config.tools.ffprobe,
        &input,
        lc_media::probe::PROBE_TIMEOUT,
    )
    .await
    .unwrap();
    assert_eq!(probed.video_codec.as_deref(), Some("vp8"));
    probed.video_codec = Some("h264".to_owned());
    std::fs::create_dir_all(dir.path().join("media")).unwrap();

    let normalized = lc_media::transcode::normalize_video(
        &input,
        &dir.path().join("media/out"),
        Some(&probed),
        &config,
    )
    .await
    .expect("fallback transcode");

    assert_eq!(
        stream(&config, &normalized.storage_path, "v", "codec_name,pix_fmt"),
        "h264,yuv420p"
    );
    assert!(is_faststart(&normalized.storage_path));
}

/// Une photo unique et 5 s de musique : la vidéo de carrousel garde un flux
/// vidéo et dure aussi longtemps que l'audio (et non 3 s).
#[tokio::test]
#[ignore = "needs ffmpeg and ffprobe"]
async fn builds_single_photo_carousel_with_audio() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let image = dir.path().join("photo.png");
    run(
        &config.tools.ffmpeg,
        &[
            "-y",
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=360x640:d=1",
            "-frames:v",
            "1",
            image.to_str().unwrap(),
        ],
    );
    let audio = dir.path().join("music.m4a");
    run(
        &config.tools.ffmpeg,
        &[
            "-y",
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=5",
            "-c:a",
            "aac",
            audio.to_str().unwrap(),
        ],
    );
    let output = dir.path().join("carousel.mp4");
    let images = [image];
    build_carousel_video(
        &config.tools.ffmpeg,
        &images,
        Some(&audio),
        &output,
        carousel_dimensions(320, true),
        &config,
    )
    .await
    .expect("carrousel construit");
    let probed = lc_media::probe::probe(
        &config.tools.ffprobe,
        &output,
        lc_media::probe::PROBE_TIMEOUT,
    )
    .await
    .unwrap();
    assert!(probed.has_video, "le carrousel doit garder un flux vidéo");
    assert!(probed.has_audio, "le carrousel doit garder la musique");
    let duration = probed.duration_sec.expect("durée sondée");
    assert!(
        (4.0..=6.0).contains(&duration),
        "durée {duration}, attendu ~5 s"
    );
}
