//! Normalisation ffmpeg : vidéo H.264/AAC MP4 faststart, audio MP3, images
//! raster copiées telles quelles, plus la construction des vidéos de carrousel
//! TikTok.
//!
//! La décision de transcoder ou de copier reprend les règles de l'ancien
//! serveur : un MP4/MOV H.264/AAC yuv420p sous la hauteur maximale est copié.
//!
//! A video whose picture already is H.264 yuv420p under the maximum height,
//! but in another container or with another audio codec (an OBS `.mkv`, a
//! `.mov` with PCM sound, an MP4 with MP3/Opus/AC-3 sound), keeps its video
//! stream untouched: only the audio is converted ([`VideoPlan::CopyVideo`]).
//! Measured on 4 cores with a 30 s 1080p clip: 11.7 s for the full transcode,
//! 0.6 s with the video stream copied (see `docs/rust/lc-media.md`).
//! Les SVG sont refusés (contenu actif, non rasterisé ici — AUD-A4-004) et le
//! type MIME d'une image est dérivé en priorité de la sonde ffprobe, pas du
//! `Content-Type` distant (AUD-A4-010).

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use lc_domain::MediaKind;

use crate::config::{LoudnormTargets, MediaConfig, VideoConfig, VideoEncoder};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::probe::{self, ProbeInfo};
use crate::process;
use crate::source::{extension_from_mime, kind_from_mime, mime_from_extension};
use crate::storage::{staged_path, MediaStorage};

/// Taille minimale d'un média normalisé valide.
pub const MIN_MEDIA_BYTES: u64 = 1024;
/// Minimum size for an authenticated local-upload image.
///
/// A valid 1×1 PNG is smaller than [`MIN_MEDIA_BYTES`]; uploaded icons are
/// legitimate, while remote sources keep the stricter anti-empty guard.
const MIN_LOCAL_IMAGE_BYTES: u64 = 8;
/// Débit audio de sortie.
pub const AUDIO_BITRATE: &str = "192k";
/// Type MIME des audio normalisés.
pub const AUDIO_MIME: &str = "audio/mpeg";
/// Type MIME des vidéos normalisées.
pub const VIDEO_MIME: &str = "video/mp4";
/// Sample rate of an audio track that went through `loudnorm`.
///
/// The filter works at 192 kHz and leaves that rate on its output: without an
/// explicit rate, the AAC encoder keeps 96 kHz, which doubles the audio
/// decoding work of every overlay for no audible gain.
pub const NORMALIZED_AUDIO_RATE: &str = "48000";

/// Résultat d'une normalisation réussie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedMedia {
    /// Type de média produit.
    pub kind: MediaKind,
    /// Type MIME du fichier produit.
    pub mime: String,
    /// Durée en secondes (absente pour une image).
    pub duration_sec: Option<i64>,
    /// Largeur en pixels.
    pub width: Option<u32>,
    /// Hauteur en pixels.
    pub height: Option<u32>,
    /// `true` si le média est vertical.
    pub is_vertical: bool,
    /// Chemin du fichier dans le stockage.
    pub storage_path: PathBuf,
    /// Taille du fichier en octets.
    pub size_bytes: u64,
    /// Embedded cover extracted from an audio file (album art), stored next
    /// to the audio file.
    pub cover: Option<NormalizedCover>,
}

/// Cover picture extracted from an audio file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedCover {
    /// MIME type of the picture (`image/jpeg`, `image/png`).
    pub mime: String,
    /// Width in pixels.
    pub width: Option<u32>,
    /// Height in pixels.
    pub height: Option<u32>,
    /// Path of the picture in the storage.
    pub storage_path: PathBuf,
    /// Size in bytes.
    pub size_bytes: u64,
}

/// Détecte le type de média à normaliser.
pub fn detect_kind(content_type: Option<&str>, probed: Option<&ProbeInfo>) -> Option<MediaKind> {
    if let Some(probed) = probed {
        if probed.has_video && probed.has_audio {
            return Some(MediaKind::Video);
        }
        if probed.has_video {
            // Une image fixe a un codec d'image et, selon FFmpeg, pas de durée
            // (ou une durée d'une image dans le démultiplexeur `image2`).
            if is_image_codec(probed.video_codec.as_deref())
                && (probed.duration_ceil_sec().is_none() || still_image_format(probed))
            {
                return Some(MediaKind::Image);
            }
            return Some(MediaKind::Video);
        }
        if probed.has_audio {
            return Some(MediaKind::Audio);
        }
    }
    content_type.and_then(kind_from_mime)
}

/// `true` si ffprobe a lu le fichier avec un démultiplexeur d'image fixe.
///
/// FFmpeg 9 annonce une durée d'une image (1/25 s) pour un JPEG ouvert par
/// `image2` : sans cette distinction, une photo était classée vidéo.
fn still_image_format(probed: &ProbeInfo) -> bool {
    probed.format_name.as_deref().is_some_and(|format| {
        format.split(',').map(str::trim).any(|name| {
            matches!(
                name,
                "image2" | "image2pipe" | "jpeg_pipe" | "png_pipe" | "webp_pipe" | "bmp_pipe"
            )
        })
    })
}

/// `true` si le codec vidéo correspond vraisemblablement à une image.
pub fn is_image_codec(codec: Option<&str>) -> bool {
    matches!(
        codec,
        Some("png" | "mjpeg" | "jpeg" | "gif" | "webp" | "bmp" | "avif" | "heic" | "tiff")
    )
}

/// Normalise un média téléchargé vers le stockage content-addressed.
///
/// `output_base` est le chemin sans extension (`<root>/<source_hash>`).
pub async fn normalize_downloaded_media(
    input: &Path,
    content_type: Option<&str>,
    output_base: &Path,
    config: &MediaConfig,
) -> MediaResult<NormalizedMedia> {
    normalize_downloaded_media_with_min_image_size(
        input,
        content_type,
        output_base,
        config,
        MIN_MEDIA_BYTES,
    )
    .await
}

/// Normalizes an authenticated local upload.
///
/// Identical to [`normalize_downloaded_media`] except that small raster images
/// are accepted: valid 1×1 PNG/JPEG/GIF icons are below the remote-source
/// anti-empty guard. Audio and video keep [`MIN_MEDIA_BYTES`].
pub async fn normalize_local_media(
    input: &Path,
    content_type: Option<&str>,
    output_base: &Path,
    config: &MediaConfig,
) -> MediaResult<NormalizedMedia> {
    normalize_downloaded_media_with_min_image_size(
        input,
        content_type,
        output_base,
        config,
        MIN_LOCAL_IMAGE_BYTES,
    )
    .await
}

async fn normalize_downloaded_media_with_min_image_size(
    input: &Path,
    content_type: Option<&str>,
    output_base: &Path,
    config: &MediaConfig,
    min_image_bytes: u64,
) -> MediaResult<NormalizedMedia> {
    let started = std::time::Instant::now();
    let probed = probe::probe(&config.tools.ffprobe, input, config.download.timeout)
        .await
        .ok();
    let probe_ms = crate::ingest::elapsed_ms(started);
    let kind = detect_kind(content_type, probed.as_ref()).ok_or_else(|| {
        MediaError::new(MediaErrorCode::InvalidMedia, "format de média non reconnu")
    })?;
    tracing::info!(?kind, probe_ms, "normalisation : média analysé");
    if let Some(parent) = output_base.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "création du stockage",
                &error,
            )
        })?;
    }
    match kind {
        MediaKind::Image => normalize_image_with_min_size(
            input,
            content_type,
            output_base,
            probed.as_ref(),
            min_image_bytes,
        ),
        MediaKind::Audio => normalize_audio(input, output_base, probed.as_ref(), config).await,
        MediaKind::Video => normalize_video(input, output_base, probed.as_ref(), config).await,
    }
}

/// Normalise une image raster en la copiant avec la bonne extension.
///
/// Les SVG sont refusés : un SVG est un contenu actif (scripts, liens) qui ne
/// doit jamais être servi tel quel depuis le stockage média (AUD-A4-004).
pub fn normalize_image(
    input: &Path,
    content_type: Option<&str>,
    output_base: &Path,
    probed: Option<&ProbeInfo>,
) -> MediaResult<NormalizedMedia> {
    normalize_image_with_min_size(input, content_type, output_base, probed, MIN_MEDIA_BYTES)
}

fn normalize_image_with_min_size(
    input: &Path,
    content_type: Option<&str>,
    output_base: &Path,
    probed: Option<&ProbeInfo>,
    min_size_bytes: u64,
) -> MediaResult<NormalizedMedia> {
    if is_svg_image(input, content_type) {
        return Err(MediaError::new(
            MediaErrorCode::InvalidMedia,
            "image SVG non supportée : contenu actif, rasterisation requise",
        ));
    }
    let mime = image_mime(content_type, input, probed);
    let extension = extension_from_mime(&mime).unwrap_or("png");
    let output = output_base.with_extension(extension);
    let staged = staged_path(&output);
    if let Err(error) = fs::copy(input, &staged) {
        let _removed = fs::remove_file(&staged);
        return Err(MediaError::from_io(
            MediaErrorCode::InvalidMedia,
            "copie de l'image normalisée",
            &error,
        ));
    }
    MediaStorage::commit_file(&staged, &output)?;
    finalize_with_min_size(
        MediaKind::Image,
        &mime,
        &output,
        probed,
        None,
        None,
        min_size_bytes,
    )
}

/// `true` si l'entrée est (ou prétend être) un SVG.
fn is_svg_image(input: &Path, content_type: Option<&str>) -> bool {
    if let Some(content_type) = content_type {
        let essence = content_type_essence(content_type);
        if essence == "image/svg+xml" {
            return true;
        }
    }
    if input
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        return true;
    }
    head_looks_like_svg(input)
}

/// Détection best-effort d'un SVG par signature, indépendamment du nom/MIME.
fn head_looks_like_svg(input: &Path) -> bool {
    let mut header = [0_u8; 1024];
    let read = match File::open(input) {
        Ok(mut file) => match file.read(&mut header) {
            Ok(read) => read,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    let head = String::from_utf8_lossy(&header[..read])
        .trim_start()
        .to_ascii_lowercase();
    head.starts_with("<svg")
        || head.starts_with("<!doctype svg")
        || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// Essence normalisée d'un `Content-Type`.
fn content_type_essence(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// MIME d'image sûr, sans SVG, pour l'essence fournie.
fn safe_image_mime(essence: &str) -> Option<&'static str> {
    match essence {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        "image/bmp" => Some("image/bmp"),
        "image/avif" => Some("image/avif"),
        _ => None,
    }
}

/// Détermine le type MIME d'une image téléchargée.
///
/// Priorité : codec détecté par ffprobe (fiable, non contrôlé par la source),
/// puis `Content-Type` distant restreint à une liste blanche sans SVG, puis
/// extension du fichier téléchargé. Le résutat est toujours un MIME raster.
pub fn image_mime(content_type: Option<&str>, input: &Path, probed: Option<&ProbeInfo>) -> String {
    let codec_mime = probed
        .and_then(|probed| probed.video_codec.as_deref())
        .and_then(|codec| match codec {
            "png" => Some("image/png"),
            "mjpeg" | "jpeg" => Some("image/jpeg"),
            "gif" => Some("image/gif"),
            "webp" => Some("image/webp"),
            "bmp" => Some("image/bmp"),
            "avif" => Some("image/avif"),
            _ => None,
        });
    if let Some(mime) = codec_mime {
        return mime.to_string();
    }
    if let Some(content_type) = content_type {
        if let Some(mime) = safe_image_mime(&content_type_essence(content_type)) {
            return mime.to_string();
        }
    }
    input
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(mime_from_extension)
        .and_then(|mime| safe_image_mime(mime))
        .unwrap_or("image/png")
        .to_string()
}

/// Normalise un audio en MP3 (copie si déjà MP3 sans normalisation).
pub async fn normalize_audio(
    input: &Path,
    output_base: &Path,
    probed: Option<&ProbeInfo>,
    config: &MediaConfig,
) -> MediaResult<NormalizedMedia> {
    let output = output_base.with_extension("mp3");
    let staged = staged_path(&output);
    let level = measure_audio_level(input, probed, config).await;
    let can_copy = level == AudioLevel::Keep
        && probed.is_some_and(|probed| {
            probed.audio_codec.as_deref() == Some("mp3")
                && probed
                    .format_name
                    .as_deref()
                    .is_some_and(|format| format.contains("mp3"))
        });
    if can_copy {
        if let Err(error) = fs::copy(input, &staged) {
            let _removed = fs::remove_file(&staged);
            return Err(MediaError::from_io(
                MediaErrorCode::InvalidMedia,
                "copie de l'audio normalisé",
                &error,
            ));
        }
    } else {
        let args = build_audio_args(input, &staged, &level);
        let output_command = process::run_command(
            &config.tools.ffmpeg,
            &args,
            config.download.timeout,
            MediaErrorCode::TranscodeFailed,
        )
        .await;
        if let Err(error) = output_command {
            let _removed = fs::remove_file(&staged);
            return Err(error);
        }
    }
    MediaStorage::commit_file(&staged, &output)?;
    let duration = probed.and_then(|probed| probed.duration_ceil_sec());
    let mut normalized = finalize(
        MediaKind::Audio,
        AUDIO_MIME,
        &output,
        probed,
        None,
        duration,
    )?;
    if let Some(cover) = probed.and_then(|probed| probed.cover.as_ref()) {
        normalized.cover = extract_cover(input, output_base, cover, config).await;
    }
    Ok(normalized)
}

/// Smallest cover kept: below this, the "picture" is a placeholder.
const MIN_COVER_BYTES: u64 = 256;

/// Extracts the embedded cover of an audio file next to it
/// (`<base>.cover.jpg|png`). Best effort: a failure only loses the cover.
async fn extract_cover(
    input: &Path,
    output_base: &Path,
    cover: &crate::probe::ProbeCover,
    config: &MediaConfig,
) -> Option<NormalizedCover> {
    // JPEG and PNG covers are copied as they are; other codecs become PNG.
    let (extension, mime, codec) = match cover.codec.as_str() {
        "mjpeg" | "jpeg" => ("jpg", "image/jpeg", "copy"),
        "png" => ("png", "image/png", "copy"),
        _ => ("png", "image/png", "png"),
    };
    let output = output_base.with_extension(format!("cover.{extension}"));
    let staged = staged_path(&output);
    let args = build_cover_args(input, &staged, codec);
    if let Err(error) = process::run_command(
        &config.tools.ffmpeg,
        &args,
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    {
        let _removed = fs::remove_file(&staged);
        tracing::warn!(error = %error.summary(), "extraction de la pochette impossible");
        return None;
    }
    if let Err(error) = MediaStorage::commit_file(&staged, &output) {
        let _removed = fs::remove_file(&staged);
        tracing::warn!(error = %error.summary(), "enregistrement de la pochette impossible");
        return None;
    }
    let size_bytes = MediaStorage::file_size(&output).ok()?;
    if size_bytes < MIN_COVER_BYTES {
        let _removed = fs::remove_file(&output);
        return None;
    }
    Some(NormalizedCover {
        mime: mime.to_string(),
        width: cover.width,
        height: cover.height,
        storage_path: output,
        size_bytes,
    })
}

/// ffmpeg arguments extracting the first attached picture of a file.
pub fn build_cover_args(input: &Path, output: &Path, codec: &str) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-v".to_string(),
        "error".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-an".to_string(),
        "-map".to_string(),
        "0:v:0".to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        "-c:v".to_string(),
        codec.to_string(),
        output.display().to_string(),
    ]
}

/// How a video becomes the normalized MP4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPlan {
    /// The file already is a normalized MP4: plain file copy.
    CopyFile,
    /// The video stream already is H.264 yuv420p under the maximum height:
    /// it is copied into a new faststart MP4 and only the audio is handled.
    CopyVideo {
        /// What happens to the audio track.
        audio: RemuxAudio,
    },
    /// Decode and encode everything.
    Transcode,
}

/// Audio handling of [`VideoPlan::CopyVideo`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemuxAudio {
    /// The source has no audio track.
    None,
    /// AAC track kept as it is (loudness normalization disabled).
    Copy,
    /// Track encoded to AAC, with its level change (see [`AudioLevel`]).
    Encode,
}

/// Chooses the cheapest way to produce the normalized MP4.
///
/// `keep_audio` says the audio track needs no level change (see
/// [`AudioLevel::Keep`]): only then can the file or its track be copied.
pub fn plan_video(probed: &ProbeInfo, config: &VideoConfig, keep_audio: bool) -> VideoPlan {
    let audio_untouched = keep_audio || !probed.has_audio;
    if audio_untouched && can_copy_without_transcode(probed, config) {
        return VideoPlan::CopyFile;
    }
    if !video_stream_is_normalized(probed, config) {
        return VideoPlan::Transcode;
    }
    let audio = if !probed.has_audio {
        RemuxAudio::None
    } else if probed.audio_codec.as_deref() == Some("aac") && keep_audio {
        RemuxAudio::Copy
    } else {
        RemuxAudio::Encode
    };
    VideoPlan::CopyVideo { audio }
}

/// Normalise une vidéo : copie si compatible, sinon transcodage H.264/AAC.
pub async fn normalize_video(
    input: &Path,
    output_base: &Path,
    probed: Option<&ProbeInfo>,
    config: &MediaConfig,
) -> MediaResult<NormalizedMedia> {
    let output = output_base.with_extension("mp4");
    let staged = staged_path(&output);
    let level = measure_audio_level(input, probed, config).await;
    let plan = probed.map_or(VideoPlan::Transcode, |probed| {
        plan_video(probed, &config.video, level == AudioLevel::Keep)
    });
    let started = std::time::Instant::now();
    match plan {
        VideoPlan::CopyFile => {
            if let Err(error) = fs::copy(input, &staged) {
                let _removed = fs::remove_file(&staged);
                return Err(MediaError::from_io(
                    MediaErrorCode::InvalidMedia,
                    "copie de la vidéo normalisée",
                    &error,
                ));
            }
        }
        VideoPlan::CopyVideo { audio } => {
            let args = build_video_remux_args(input, &staged, audio, &level);
            let remuxed = process::run_command(
                &config.tools.ffmpeg,
                &args,
                config.download.timeout,
                MediaErrorCode::TranscodeFailed,
            )
            .await;
            if let Err(error) = remuxed {
                // Some streams cannot be moved to MP4 as they are (missing
                // timestamps, exotic bitstreams): encode them instead.
                let _removed = fs::remove_file(&staged);
                tracing::warn!(
                    error = %error.summary(),
                    "copie du flux vidéo impossible, transcodage complet"
                );
                transcode_video(input, &staged, probed, &level, config).await?;
            }
        }
        VideoPlan::Transcode => transcode_video(input, &staged, probed, &level, config).await?,
    }
    MediaStorage::commit_file(&staged, &output)?;
    tracing::info!(
        ?plan,
        ?level,
        encode_ms = crate::ingest::elapsed_ms(started),
        "normalisation : vidéo produite"
    );
    // A plain copy is the probed input, byte for byte: no second probe.
    let output_probe = if plan == VideoPlan::CopyFile {
        None
    } else {
        probe::probe(&config.tools.ffprobe, &output, config.download.timeout)
            .await
            .ok()
    };
    let reference = output_probe.as_ref().or(probed);
    let durations = reference
        .and_then(|reference| reference.duration_ceil_sec())
        .or_else(|| probed.and_then(|probed| probed.duration_ceil_sec()));
    finalize(
        MediaKind::Video,
        VIDEO_MIME,
        &output,
        reference,
        Some(config.video.max_height),
        durations,
    )
}

/// Full transcode to `staged`, with the libx264 retry after an NVENC failure.
/// The staged file is removed on failure.
async fn transcode_video(
    input: &Path,
    staged: &Path,
    probed: Option<&ProbeInfo>,
    level: &AudioLevel,
    config: &MediaConfig,
) -> MediaResult<()> {
    let nvenc_available = match config.video.encoder {
        VideoEncoder::LibX264 => false,
        VideoEncoder::H264Nvenc | VideoEncoder::Auto => {
            has_nvenc(&config.tools.ffmpeg, config.download.timeout).await
        }
    };
    let (encoder, preset) = resolve_video_encoder(&config.video, nvenc_available);
    let args = build_video_transcode_args(input, staged, config, probed, level, &encoder, &preset);
    let first = process::run_command(
        &config.tools.ffmpeg,
        &args,
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await;
    let Err(error) = first else {
        return Ok(());
    };
    let _removed = fs::remove_file(staged);
    if encoder != NVENC_ENCODER {
        return Err(error);
    }
    tracing::warn!(%error, "échec NVENC, nouvelle tentative en libx264");
    let (fallback_encoder, fallback_preset) = resolve_video_encoder(&config.video, false);
    let fallback_args = build_video_transcode_args(
        input,
        staged,
        config,
        probed,
        level,
        &fallback_encoder,
        &fallback_preset,
    );
    if let Err(fallback_error) = process::run_command(
        &config.tools.ffmpeg,
        &fallback_args,
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    {
        let _removed = fs::remove_file(staged);
        return Err(fallback_error);
    }
    Ok(())
}

/// Vérifie le résultat normalisé et construit [`NormalizedMedia`].
pub fn finalize(
    kind: MediaKind,
    mime: &str,
    output: &Path,
    probed: Option<&ProbeInfo>,
    max_height: Option<u32>,
    duration_sec: Option<i64>,
) -> MediaResult<NormalizedMedia> {
    finalize_with_min_size(
        kind,
        mime,
        output,
        probed,
        max_height,
        duration_sec,
        MIN_MEDIA_BYTES,
    )
}

fn finalize_with_min_size(
    kind: MediaKind,
    mime: &str,
    output: &Path,
    probed: Option<&ProbeInfo>,
    max_height: Option<u32>,
    duration_sec: Option<i64>,
    min_size_bytes: u64,
) -> MediaResult<NormalizedMedia> {
    let size_bytes = MediaStorage::file_size(output)?;
    if size_bytes < min_size_bytes {
        let _removed = fs::remove_file(output);
        return Err(MediaError::new(
            MediaErrorCode::InvalidMedia,
            "média normalisé vide ou trop petit",
        ));
    }
    let (width, height) = dimensions_for(kind, probed, max_height);
    let duration = if kind == MediaKind::Image {
        None
    } else {
        duration_sec.filter(|duration| *duration > 0).or(Some(1))
    };
    if kind == MediaKind::Video {
        let has_video = probed.map(|probed| probed.has_video).unwrap_or(true);
        if !has_video {
            return Err(MediaError::new(
                MediaErrorCode::InvalidMedia,
                "le fichier normalisé ne contient pas de flux vidéo",
            ));
        }
    }
    let is_vertical =
        matches!((width, height), (Some(width), Some(height)) if width > 0 && height > width);
    Ok(NormalizedMedia {
        kind,
        mime: mime.to_string(),
        duration_sec: duration,
        width,
        height,
        is_vertical,
        storage_path: output.to_path_buf(),
        size_bytes,
        cover: None,
    })
}

fn dimensions_for(
    kind: MediaKind,
    probed: Option<&ProbeInfo>,
    max_height: Option<u32>,
) -> (Option<u32>, Option<u32>) {
    let Some(probed) = probed else {
        return (None, None);
    };
    let (Some(width), Some(height)) = (probed.width, probed.height) else {
        return (None, None);
    };
    if kind != MediaKind::Video {
        return (Some(width), Some(height));
    }
    let max_height = max_height.filter(|max_height| *max_height >= 2);
    match max_height {
        Some(max_height) if height > max_height => {
            let scaled_width =
                ((width as f64) * (max_height as f64) / (height as f64)).round() as u32;
            (
                Some(to_even(scaled_width).max(2)),
                Some(to_even(max_height).max(2)),
            )
        }
        _ => (Some(width), Some(height)),
    }
}

/// `true` si la vidéo peut être copiée sans transcodage.
pub fn can_copy_without_transcode(probed: &ProbeInfo, config: &VideoConfig) -> bool {
    let container_ok = probed.format_name.as_deref().is_some_and(|format| {
        ["mp4", "mov", "m4a"]
            .iter()
            .any(|candidate| format.contains(candidate))
    });
    let audio_ok = matches!(probed.audio_codec.as_deref(), None | Some("aac"));
    container_ok && audio_ok && video_stream_is_normalized(probed, config)
}

/// `true` when the video stream needs no encoding: H.264, 4:2:0 8-bit, at
/// most the maximum height.
fn video_stream_is_normalized(probed: &ProbeInfo, config: &VideoConfig) -> bool {
    let video_ok = probed.has_video && probed.video_codec.as_deref() == Some("h264");
    let pixel_ok = matches!(
        probed.pixel_format.as_deref(),
        None | Some("yuv420p" | "yuvj420p")
    );
    let height_ok = probed
        .height
        .map(|height| height <= config.max_height)
        .unwrap_or(true);
    video_ok && pixel_ok && height_ok
}

/// Résout l'encodeur et le preset à utiliser.
pub fn resolve_video_encoder(config: &VideoConfig, nvenc_available: bool) -> (String, String) {
    match config.encoder {
        VideoEncoder::LibX264 => ("libx264".to_string(), config.cpu_preset.clone()),
        VideoEncoder::H264Nvenc => {
            if nvenc_available {
                ("h264_nvenc".to_string(), config.nvenc_preset.clone())
            } else {
                ("libx264".to_string(), config.cpu_preset.clone())
            }
        }
        VideoEncoder::Auto => {
            if nvenc_available {
                ("h264_nvenc".to_string(), config.nvenc_preset.clone())
            } else {
                ("libx264".to_string(), config.cpu_preset.clone())
            }
        }
    }
}

/// Longest wait for the NVENC capability test.
const NVENC_TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Answers of [`has_nvenc`], per ffmpeg binary, for the life of the process.
static NVENC_CAPABILITY: CapabilityCache = CapabilityCache::new();

/// Remembers a yes/no capability per binary.
struct CapabilityCache {
    known: Mutex<Option<HashMap<PathBuf, bool>>>,
}

impl CapabilityCache {
    const fn new() -> Self {
        Self {
            known: Mutex::new(None),
        }
    }

    fn get(&self, binary: &Path) -> Option<bool> {
        let known = self
            .known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        known.as_ref().and_then(|known| known.get(binary).copied())
    }

    fn set(&self, binary: &Path, available: bool) {
        let mut known = self
            .known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        known
            .get_or_insert_with(HashMap::new)
            .insert(binary.to_path_buf(), available);
    }

    /// Cached answer, or the result of `probe` (then remembered). Two
    /// concurrent first calls may both probe; they store the same answer.
    async fn get_or_probe<F>(&self, binary: &Path, probe: F) -> bool
    where
        F: Future<Output = bool>,
    {
        if let Some(available) = self.get(binary) {
            return available;
        }
        let available = probe.await;
        self.set(binary, available);
        available
    }
}

/// `true` when this ffmpeg can really encode with `h264_nvenc`.
///
/// Listing the encoder is not enough: distribution builds of ffmpeg list
/// `h264_nvenc` on machines without any NVIDIA GPU, where every transcode
/// then started with a failed NVENC attempt. A tiny encode is run once and
/// the answer is kept for the life of the process.
pub async fn has_nvenc(ffmpeg: &Path, timeout: Duration) -> bool {
    NVENC_CAPABILITY
        .get_or_probe(ffmpeg, probe_nvenc(ffmpeg, timeout))
        .await
}

async fn probe_nvenc(ffmpeg: &Path, timeout: Duration) -> bool {
    let timeout = timeout.min(NVENC_TEST_TIMEOUT);
    let list_args = vec!["-hide_banner".to_string(), "-encoders".to_string()];
    let listed =
        match process::run_command(ffmpeg, &list_args, timeout, MediaErrorCode::TranscodeFailed)
            .await
        {
            Ok(output) => output.combined().contains("h264_nvenc"),
            Err(_) => false,
        };
    if !listed {
        return false;
    }
    let usable = process::run_command(
        ffmpeg,
        &nvenc_test_args(),
        timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    .is_ok();
    tracing::info!(usable, "encodeur h264_nvenc testé");
    usable
}

/// ffmpeg arguments encoding a few black frames with `h264_nvenc` to nowhere.
pub fn nvenc_test_args() -> Vec<String> {
    [
        "-hide_banner",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=256x256:r=25:d=0.2",
        "-c:v",
        "h264_nvenc",
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect()
}

/// ffmpeg arguments copying the video stream into a faststart MP4.
///
/// Only the first real video stream (`V`: no attached picture) and the first
/// audio stream are kept, like the players do.
pub fn build_video_remux_args(
    input: &Path,
    output: &Path,
    audio: RemuxAudio,
    level: &AudioLevel,
) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-map".to_string(),
        "0:V:0".to_string(),
    ];
    if audio != RemuxAudio::None {
        args.push("-map".to_string());
        args.push("0:a:0".to_string());
    }
    args.push("-c:v".to_string());
    args.push("copy".to_string());
    match audio {
        RemuxAudio::None => args.push("-an".to_string()),
        RemuxAudio::Copy => {
            args.push("-c:a".to_string());
            args.push("copy".to_string());
        }
        RemuxAudio::Encode => {
            args.push("-c:a".to_string());
            args.push("aac".to_string());
            args.push("-b:a".to_string());
            args.push(AUDIO_BITRATE.to_string());
            push_audio_level(&mut args, level);
        }
    }
    args.push("-movflags".to_string());
    args.push("+faststart".to_string());
    args.push(output.display().to_string());
    args
}

/// What happens to the level of an audio track during normalization.
///
/// Every source (YouTube, TikTok, uploads...) is brought to the same
/// loudness: the track is measured first (`ebur128`, fast), then a fixed gain
/// moves it to the target and a limiter holds the peaks. It sounds like
/// single-pass `loudnorm`, which also missed the target on clips of a few
/// seconds, for a fraction of its cost.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioLevel {
    /// The track stays as it is: normalization off, no audio track, silence,
    /// or already within [`LEVEL_TOLERANCE_DB`] of the target.
    Keep,
    /// Fixed gain, followed by a limiter at the true-peak target.
    Gain {
        /// Gain applied before the limiter, dB.
        gain_db: f64,
        /// Limiter ceiling, linear (true-peak target).
        limit: f64,
    },
    /// The measurement failed: single-pass `loudnorm` filter.
    Loudnorm(String),
}

/// A track this close to the target loudness keeps its level.
pub const LEVEL_TOLERANCE_DB: f64 = 0.5;
/// At or below this integrated loudness the track is silence: no gain.
const SILENCE_LUFS: f64 = -70.0;
/// Lowest ceiling `alimiter` accepts.
const MIN_LIMIT: f64 = 0.0625;

/// Integrated loudness and true peak of a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    /// Integrated loudness, LUFS.
    pub integrated_lufs: f64,
    /// True peak, dBTP.
    pub true_peak_dbtp: f64,
}

/// ffmpeg arguments measuring the first audio track (summary on stderr).
///
/// `framelog=verbose` hides the per-frame lines at the default log level
/// while the summary stays at `info`. `framelog=quiet` would do the same but
/// does not exist before FFmpeg 6: the server's Debian FFmpeg 5.1 rejected
/// it, so every measurement failed and fell back to `loudnorm`.
pub fn loudness_args(input: &Path) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-nostats".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-map".to_string(),
        "0:a:0".to_string(),
        "-af".to_string(),
        "ebur128=peak=true:framelog=verbose".to_string(),
        "-f".to_string(),
        "null".to_string(),
        "-".to_string(),
    ]
}

/// Reads the `I:` and true `Peak:` values of the `ebur128` summary.
pub fn parse_loudness(stderr: &str) -> Option<Loudness> {
    let summary = &stderr[stderr.rfind("Summary:")?..];
    let value = |label: &str| {
        summary.lines().find_map(|line| {
            let rest = line.trim().strip_prefix(label)?;
            rest.split_whitespace().next()?.parse::<f64>().ok()
        })
    };
    Some(Loudness {
        integrated_lufs: value("I:")?,
        true_peak_dbtp: value("Peak:")?,
    })
}

/// Chooses the treatment of a measured track.
pub fn level_for(measured: Loudness, targets: &LoudnormTargets) -> AudioLevel {
    if !measured.integrated_lufs.is_finite() || measured.integrated_lufs <= SILENCE_LUFS {
        return AudioLevel::Keep;
    }
    let gain_db = targets.integrated_lufs - measured.integrated_lufs;
    let peak_ok = measured.true_peak_dbtp + gain_db.max(0.0) <= targets.true_peak_dbtp;
    if gain_db.abs() <= LEVEL_TOLERANCE_DB && peak_ok {
        return AudioLevel::Keep;
    }
    let limit = 10f64
        .powf(targets.true_peak_dbtp / 20.0)
        .clamp(MIN_LIMIT, 1.0);
    AudioLevel::Gain {
        gain_db: (gain_db * 100.0).round() / 100.0,
        limit: (limit * 1000.0).round() / 1000.0,
    }
}

/// Measures the track and chooses its treatment (see [`AudioLevel`]).
pub async fn measure_audio_level(
    input: &Path,
    probed: Option<&ProbeInfo>,
    config: &MediaConfig,
) -> AudioLevel {
    let has_audio = probed.is_none_or(|probed| probed.has_audio);
    if !config.video.audio_normalize || !has_audio {
        return AudioLevel::Keep;
    }
    let started = std::time::Instant::now();
    let measured = process::run_command(
        &config.tools.ffmpeg,
        &loudness_args(input),
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    .map(|output| parse_loudness(&output.stderr));
    let level = match measured {
        Ok(Some(measured)) => level_for(measured, &config.video.loudnorm),
        Ok(None) => {
            tracing::warn!("mesure du volume illisible, repli sur loudnorm");
            AudioLevel::Loudnorm(config.video.loudnorm.filter())
        }
        Err(error) => {
            tracing::warn!(%error, "mesure du volume impossible, repli sur loudnorm");
            AudioLevel::Loudnorm(config.video.loudnorm.filter())
        }
    };
    tracing::info!(
        ?level,
        measure_ms = crate::ingest::elapsed_ms(started),
        "normalisation : volume mesuré"
    );
    level
}

/// Audio filter of a level (and the output rate `loudnorm` needs).
fn push_audio_level(args: &mut Vec<String>, level: &AudioLevel) {
    match level {
        AudioLevel::Keep => {}
        AudioLevel::Gain { gain_db, limit } => {
            args.push("-af".to_string());
            args.push(format!(
                "volume={gain_db}dB,alimiter=limit={limit}:level=false:latency=true"
            ));
        }
        AudioLevel::Loudnorm(filter) => {
            args.push("-af".to_string());
            args.push(filter.clone());
            args.push("-ar".to_string());
            args.push(NORMALIZED_AUDIO_RATE.to_string());
        }
    }
}

/// Name of the NVIDIA H.264 encoder.
pub const NVENC_ENCODER: &str = "h264_nvenc";

/// Decoding options placed before `-i` when NVENC encodes: the GPU decodes
/// too. The CPU decode of a 1440p60 AV1 clip took 24 s on the server, the GPU
/// one 4 s. AV1 needs FFmpeg's native decoder: the default one, libdav1d,
/// ignores `-hwaccel`. Codecs the GPU cannot decode fall back to software.
pub fn nvenc_decode_args(video_codec: Option<&str>) -> Vec<String> {
    let mut args = vec!["-hwaccel".to_string(), "cuda".to_string()];
    if video_codec == Some("av1") {
        args.extend(["-c:v".to_string(), "av1".to_string()]);
    }
    args
}

/// NVENC rate control. Constant quality close to x264's default CRF 23 (the
/// NVENC default bitrate gave a blurry 2 Mbit/s 1080p60). No B-frames: with
/// them, the MP4 muxer rejected the timestamps of a 60.0002 fps clip
/// ("pts/dts pair unsupported").
pub fn nvenc_quality_args() -> Vec<String> {
    ["-rc", "vbr", "-cq", "23", "-b:v", "0", "-bf", "0"]
        .map(str::to_string)
        .to_vec()
}

/// Arguments de transcodage vidéo H.264/AAC MP4 faststart.
pub fn build_video_transcode_args(
    input: &Path,
    output: &Path,
    config: &MediaConfig,
    probed: Option<&ProbeInfo>,
    level: &AudioLevel,
    encoder: &str,
    preset: &str,
) -> Vec<String> {
    let mut args = vec!["-y".to_string(), "-hide_banner".to_string()];
    if encoder == NVENC_ENCODER {
        args.extend(nvenc_decode_args(
            probed.and_then(|probed| probed.video_codec.as_deref()),
        ));
    }
    args.extend([
        "-i".to_string(),
        input.display().to_string(),
        "-c:v".to_string(),
        encoder.to_string(),
        "-preset".to_string(),
        preset.to_string(),
    ]);
    if encoder == NVENC_ENCODER {
        args.extend(nvenc_quality_args());
    }
    args.push("-pix_fmt".to_string());
    args.push("yuv420p".to_string());
    if let Some(filter) = scale_filter(probed, config.video.max_height) {
        args.push("-vf".to_string());
        args.push(filter);
    }
    args.push("-c:a".to_string());
    args.push("aac".to_string());
    args.push("-b:a".to_string());
    args.push(AUDIO_BITRATE.to_string());
    if probed.is_some_and(|probed| probed.has_audio) {
        push_audio_level(&mut args, level);
    }
    args.push("-movflags".to_string());
    args.push("+faststart".to_string());
    args.push(output.display().to_string());
    args
}

/// Arguments de normalisation audio MP3.
pub fn build_audio_args(input: &Path, output: &Path, level: &AudioLevel) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-vn".to_string(),
        "-acodec".to_string(),
        "libmp3lame".to_string(),
        "-b:a".to_string(),
        AUDIO_BITRATE.to_string(),
    ];
    push_audio_level(&mut args, level);
    args.push(output.display().to_string());
    args
}

/// Filtre de mise à l'échelle (hauteur plafonnée à `max_height`).
pub fn scale_filter(probed: Option<&ProbeInfo>, max_height: u32) -> Option<String> {
    let even = "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string();
    let height = probed.and_then(|probed| probed.height);
    match (height, max_height) {
        (Some(height), max_height) if max_height >= 2 && height > max_height => Some(format!(
            "scale=-2:{max_height}:force_original_aspect_ratio=decrease,{even}"
        )),
        _ => Some(even),
    }
}

/// Arrondit à la baisse au pair le plus proche (minimum 2).
pub fn to_even(value: u32) -> u32 {
    let even = value - (value % 2);
    even.max(2)
}

/// Dimensions d'une vidéo de carrousel (portrait 9:16, paysage 16:9).
pub fn carousel_dimensions(max_height: u32, is_vertical: bool) -> (u32, u32) {
    let height = to_even(max_height.max(2));
    let width = to_even(((height as f64) * 9.0 / 16.0).round() as u32);
    if is_vertical {
        (width.max(2), height)
    } else {
        (height, width.max(2))
    }
}

/// Arguments ffmpeg pour assembler une vidéo de carrousel TikTok.
///
/// Une image seule est mappée explicitement (`-map 0:v:0`) : dès qu'un `-map`
/// audio est présent, ffmpeg ne produit que les flux mappés et la vidéo serait
/// perdue. Avec de la musique, l'image est bouclée sans `-t 3` et `-shortest`
/// arrête le fichier à la fin de l'audio ; sans musique, et pour un carrousel
/// de plusieurs images, chaque image dure 3 s.
pub fn build_carousel_video_args(
    images: &[PathBuf],
    audio: Option<&Path>,
    output: &Path,
    dimensions: (u32, u32),
    config: &MediaConfig,
) -> Vec<String> {
    let (width, height) = dimensions;
    let per_image = format!(
        "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30,format=yuv420p"
    );
    // Une photo unique accompagnée de musique doit durer aussi longtemps
    // qu'elle : `-loop 1` sans `-t 3` et `-shortest` coupe le fichier avec
    // l'audio.
    let picture_follows_music = images.len() == 1 && audio.is_some();
    let mut args: Vec<String> = vec!["-y".to_string(), "-hide_banner".to_string()];
    for image in images {
        args.push("-loop".to_string());
        args.push("1".to_string());
        if !picture_follows_music {
            args.push("-t".to_string());
            args.push("3".to_string());
        }
        args.push("-i".to_string());
        args.push(image.display().to_string());
    }
    if let Some(audio) = audio {
        args.push("-i".to_string());
        args.push(audio.display().to_string());
    }
    if images.len() > 1 {
        let mut filters: Vec<String> = Vec::new();
        for index in 0..images.len() {
            filters.push(format!("[{index}:v]{per_image}[v{index}]"));
        }
        let concat_inputs: String = (0..images.len())
            .map(|index| format!("[v{index}]"))
            .collect();
        filters.push(format!(
            "{concat_inputs}concat=n={}:v=1:a=0[v]",
            images.len()
        ));
        args.push("-filter_complex".to_string());
        args.push(filters.join(";"));
        args.push("-map".to_string());
        args.push("[v]".to_string());
    } else {
        args.push("-vf".to_string());
        args.push(per_image);
        // Sans ce mapping explicite, le `-map` audio plus bas ferait
        // disparaître le flux vidéo de la sortie.
        args.push("-map".to_string());
        args.push("0:v:0".to_string());
    }
    args.push("-c:v".to_string());
    args.push("libx264".to_string());
    args.push("-preset".to_string());
    args.push(config.video.cpu_preset.clone());
    args.push("-pix_fmt".to_string());
    args.push("yuv420p".to_string());
    args.push("-movflags".to_string());
    args.push("+faststart".to_string());
    if audio.is_some() {
        let audio_index = images.len();
        args.push("-map".to_string());
        args.push(format!("{audio_index}:a:0"));
        args.push("-c:a".to_string());
        args.push("aac".to_string());
        args.push("-b:a".to_string());
        args.push(AUDIO_BITRATE.to_string());
        args.push("-shortest".to_string());
    } else {
        args.push("-an".to_string());
    }
    args.push(output.display().to_string());
    args
}

/// Exécute l'assemblage d'une vidéo de carrousel.
pub async fn build_carousel_video(
    ffmpeg: &Path,
    images: &[PathBuf],
    audio: Option<&Path>,
    output: &Path,
    dimensions: (u32, u32),
    config: &MediaConfig,
) -> MediaResult<()> {
    let args = build_carousel_video_args(images, audio, output, dimensions, config);
    process::run_command(
        ffmpeg,
        &args,
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::config::LoudnormTargets;

    fn video_config() -> MediaConfig {
        MediaConfig::default()
    }

    fn probed_video() -> ProbeInfo {
        ProbeInfo {
            duration_sec: Some(12.5),
            width: Some(1920),
            height: Some(1080),
            format_name: Some("mov,mp4,m4a,3gp,3g2,mj2".to_string()),
            video_codec: Some("h264".to_string()),
            audio_codec: Some("aac".to_string()),
            pixel_format: Some("yuv420p".to_string()),
            has_video: true,
            has_audio: true,
            cover: None,
        }
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(
            detect_kind(None, Some(&probed_video())),
            Some(MediaKind::Video)
        );
        let audio = ProbeInfo {
            has_audio: true,
            audio_codec: Some("mp3".to_string()),
            ..ProbeInfo::default()
        };
        assert_eq!(detect_kind(None, Some(&audio)), Some(MediaKind::Audio));
        let image = ProbeInfo {
            has_video: true,
            video_codec: Some("png".to_string()),
            width: Some(10),
            height: Some(10),
            ..ProbeInfo::default()
        };
        assert_eq!(detect_kind(None, Some(&image)), Some(MediaKind::Image));
        // FFmpeg 9 annonce une durée d'une image pour un JPEG `image2`.
        let jpeg = ProbeInfo {
            has_video: true,
            video_codec: Some("mjpeg".to_string()),
            duration_sec: Some(0.04),
            format_name: Some("image2".to_string()),
            width: Some(1264),
            height: Some(1626),
            ..ProbeInfo::default()
        };
        assert_eq!(detect_kind(None, Some(&jpeg)), Some(MediaKind::Image));
        // Un vrai conteneur vidéo avec un codec d'image reste une vidéo.
        let slideshow = ProbeInfo {
            duration_sec: Some(3.0),
            format_name: Some("mov,mp4,m4a,3gp,3g2,mj2".to_string()),
            ..jpeg
        };
        assert_eq!(detect_kind(None, Some(&slideshow)), Some(MediaKind::Video));
        assert_eq!(
            detect_kind(Some("image/webp"), None),
            Some(MediaKind::Image)
        );
        assert_eq!(
            detect_kind(Some("audio/mpeg"), None),
            Some(MediaKind::Audio)
        );
        assert_eq!(detect_kind(None, None), None);
        assert!(is_image_codec(Some("jpeg")));
        assert!(!is_image_codec(Some("h264")));
    }

    #[test]
    fn copy_decision_matches_legacy_rules() {
        let config = video_config();
        assert!(can_copy_without_transcode(&probed_video(), &config.video));
        let mut wrong_codec = probed_video();
        wrong_codec.video_codec = Some("vp9".to_string());
        assert!(!can_copy_without_transcode(&wrong_codec, &config.video));
        let mut too_tall = probed_video();
        too_tall.height = Some(2160);
        assert!(!can_copy_without_transcode(&too_tall, &config.video));
        let mut wrong_pixel = probed_video();
        wrong_pixel.pixel_format = Some("yuv444p".to_string());
        assert!(!can_copy_without_transcode(&wrong_pixel, &config.video));
        let mut wrong_audio = probed_video();
        wrong_audio.audio_codec = Some("opus".to_string());
        assert!(!can_copy_without_transcode(&wrong_audio, &config.video));
        let mut mkv = probed_video();
        mkv.format_name = Some("matroska,webm".to_string());
        assert!(!can_copy_without_transcode(&mkv, &config.video));
    }

    #[test]
    fn plans_the_cheapest_video_path() {
        let config = video_config().video;
        assert!(config.audio_normalize);
        // Already a normalized MP4: the video is copied, the audio still goes
        // through the level change like every other media.
        assert_eq!(
            plan_video(&probed_video(), &config, false),
            VideoPlan::CopyVideo {
                audio: RemuxAudio::Encode
            }
        );
        // Plain copy when nothing has to change: the audio keeps its level,
        // or there is no audio track.
        assert_eq!(
            plan_video(&probed_video(), &config, true),
            VideoPlan::CopyFile
        );
        let mut mute_mp4 = probed_video();
        mute_mp4.has_audio = false;
        mute_mp4.audio_codec = None;
        assert_eq!(plan_video(&mute_mp4, &config, false), VideoPlan::CopyFile);

        // H.264 in another container: the video stream is copied, the audio
        // goes through the level change.
        let mut mkv = probed_video();
        mkv.format_name = Some("matroska,webm".to_string());
        assert_eq!(
            plan_video(&mkv, &config, false),
            VideoPlan::CopyVideo {
                audio: RemuxAudio::Encode
            }
        );
        // Same file with its audio already at the target: nothing is encoded.
        assert_eq!(
            plan_video(&mkv, &config, true),
            VideoPlan::CopyVideo {
                audio: RemuxAudio::Copy
            }
        );

        // MP4 with a non-AAC track: only the audio is encoded, whatever the
        // level of the track.
        let mut mp3_track = probed_video();
        mp3_track.audio_codec = Some("mp3".to_string());
        for keep in [false, true] {
            assert_eq!(
                plan_video(&mp3_track, &config, keep),
                VideoPlan::CopyVideo {
                    audio: RemuxAudio::Encode
                }
            );
        }

        // Silent H.264 outside MP4.
        let mut silent = mkv.clone();
        silent.has_audio = false;
        silent.audio_codec = None;
        assert_eq!(
            plan_video(&silent, &config, false),
            VideoPlan::CopyVideo {
                audio: RemuxAudio::None
            }
        );

        // Anything wrong with the picture itself: full transcode.
        let mut vp9 = mkv.clone();
        vp9.video_codec = Some("vp9".to_string());
        assert_eq!(plan_video(&vp9, &config, false), VideoPlan::Transcode);
        let mut too_tall = mkv.clone();
        too_tall.height = Some(1440);
        assert_eq!(plan_video(&too_tall, &config, false), VideoPlan::Transcode);
        let mut ten_bit = mkv;
        ten_bit.pixel_format = Some("yuv420p10le".to_string());
        assert_eq!(plan_video(&ten_bit, &config, false), VideoPlan::Transcode);
        let mut tall_mp4 = probed_video();
        tall_mp4.height = Some(2160);
        assert_eq!(plan_video(&tall_mp4, &config, false), VideoPlan::Transcode);
    }

    #[test]
    fn builds_remux_args() {
        let position = |args: &[String], value: &str| {
            args.iter()
                .position(|arg| arg == value)
                .unwrap_or_else(|| panic!("{value} missing in {args:?}"))
        };
        let gain = AudioLevel::Gain {
            gain_db: 4.0,
            limit: 0.841,
        };

        let args = build_video_remux_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            RemuxAudio::Encode,
            &gain,
        );
        assert_eq!(args[position(&args, "-c:v") + 1], "copy");
        assert_eq!(args[position(&args, "-c:a") + 1], "aac");
        assert_eq!(args[position(&args, "-b:a") + 1], AUDIO_BITRATE);
        assert_eq!(
            args[position(&args, "-af") + 1],
            "volume=4dB,alimiter=limit=0.841:level=false:latency=true"
        );
        assert!(!args.iter().any(|arg| arg == "-ar"));
        assert_eq!(args[position(&args, "-movflags") + 1], "+faststart");
        // The first real video stream and the first audio stream only.
        let maps: Vec<&str> = args
            .windows(2)
            .filter(|pair| pair[0] == "-map")
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(maps, ["0:V:0", "0:a:0"]);
        assert_eq!(args.last().unwrap(), "out.mp4");
        // No video encoder, no filter on the picture.
        assert!(!args.iter().any(|arg| arg == "libx264" || arg == "-vf"));

        // The loudnorm fallback brings its output rate back to 48 kHz.
        let args = build_video_remux_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            RemuxAudio::Encode,
            &AudioLevel::Loudnorm("loudnorm=I=-18:LRA=11:TP=-1.5".to_string()),
        );
        assert_eq!(
            args[position(&args, "-af") + 1],
            "loudnorm=I=-18:LRA=11:TP=-1.5"
        );
        assert_eq!(args[position(&args, "-ar") + 1], "48000");

        let args = build_video_remux_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            RemuxAudio::Encode,
            &AudioLevel::Keep,
        );
        assert!(!args.iter().any(|arg| arg == "-af" || arg == "-ar"));

        let args = build_video_remux_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            RemuxAudio::Copy,
            &AudioLevel::Keep,
        );
        assert_eq!(args[position(&args, "-c:a") + 1], "copy");
        assert!(!args.iter().any(|arg| arg == "-af" || arg == "aac"));

        let args = build_video_remux_args(
            Path::new("in.mkv"),
            Path::new("out.mp4"),
            RemuxAudio::None,
            &gain,
        );
        assert!(args.iter().any(|arg| arg == "-an"));
        assert!(!args.iter().any(|arg| arg == "0:a:0" || arg == "-c:a"));
    }

    #[test]
    fn measures_loudness_with_options_ffmpeg_5_accepts() {
        let args = loudness_args(Path::new("in.mp4"));
        let filter = &args[args.iter().position(|arg| arg == "-af").unwrap() + 1];
        assert_eq!(filter, "ebur128=peak=true:framelog=verbose");
        assert!(args.windows(2).any(|pair| pair == ["-map", "0:a:0"]));
        assert!(args.ends_with(&["-f".to_string(), "null".to_string(), "-".to_string()]));
    }

    #[test]
    fn reads_the_ebur128_summary() {
        let stderr = "[Parsed_ebur128_0 @ 0x1] t: 1 M: -20 S: -21 I: -30.0 LUFS\n\
            [Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -22.0 LUFS\n\
            Threshold: -32.4 LUFS\n\n  Loudness range:\n    LRA:         9.4 LU\n\n\
            True peak:\n    Peak:       -1.8 dBFS\n";
        assert_eq!(
            parse_loudness(stderr),
            Some(Loudness {
                integrated_lufs: -22.0,
                true_peak_dbtp: -1.8,
            })
        );
        assert_eq!(parse_loudness("no summary"), None);
        let silence = "Summary:\n    I:         -inf LUFS\n    Peak:       -inf dBFS\n";
        let parsed = parse_loudness(silence).unwrap();
        assert!(parsed.integrated_lufs.is_infinite());
    }

    #[test]
    fn chooses_the_level_of_a_track() {
        let targets = video_config().video.loudnorm;
        let measured = |integrated_lufs, true_peak_dbtp| Loudness {
            integrated_lufs,
            true_peak_dbtp,
        };
        // A quiet clip is raised to the target, peaks held by the limiter.
        assert_eq!(
            level_for(measured(-22.0, -1.8), &targets),
            AudioLevel::Gain {
                gain_db: 4.0,
                limit: 0.841,
            }
        );
        // A loud one is lowered.
        assert_eq!(
            level_for(measured(-15.0, -4.3), &targets),
            AudioLevel::Gain {
                gain_db: -3.0,
                limit: 0.841,
            }
        );
        // Already at the target with room for its peaks: kept.
        assert_eq!(level_for(measured(-18.3, -3.0), &targets), AudioLevel::Keep);
        // At the target but peaking too high: limited.
        assert!(matches!(
            level_for(measured(-18.2, -0.5), &targets),
            AudioLevel::Gain { .. }
        ));
        // Silence is never amplified.
        assert_eq!(
            level_for(measured(-75.0, -60.0), &targets),
            AudioLevel::Keep
        );
        assert_eq!(
            level_for(measured(f64::NEG_INFINITY, f64::NEG_INFINITY), &targets),
            AudioLevel::Keep
        );
    }

    #[tokio::test]
    async fn a_capability_is_probed_once_per_binary() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let cache = CapabilityCache::new();
        let probes = AtomicUsize::new(0);
        let probe = |answer: bool| {
            let probes = &probes;
            async move {
                probes.fetch_add(1, Ordering::SeqCst);
                answer
            }
        };
        let ffmpeg = Path::new("/usr/bin/ffmpeg");
        assert!(!cache.get_or_probe(ffmpeg, probe(false)).await);
        // The second answer would differ: the first one is kept.
        assert!(!cache.get_or_probe(ffmpeg, probe(true)).await);
        assert_eq!(probes.load(Ordering::SeqCst), 1);
        // Another binary has its own answer.
        assert!(
            cache
                .get_or_probe(Path::new("/opt/ffmpeg"), probe(true))
                .await
        );
        assert_eq!(probes.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_missing_ffmpeg_has_no_nvenc() {
        let missing = Path::new("definitely-missing-ffmpeg-binary");
        assert!(!has_nvenc(missing, Duration::from_secs(5)).await);
    }

    #[test]
    fn nvenc_test_encodes_to_nowhere() {
        let args = nvenc_test_args();
        assert!(args.contains(&"h264_nvenc".to_string()));
        assert_eq!(&args[args.len() - 3..], ["-f", "null", "-"]);
    }

    #[test]
    fn resolves_encoder_matrix() {
        let mut config = video_config().video;
        config.cpu_preset = "ultrafast".to_string();
        config.nvenc_preset = "p4".to_string();
        config.encoder = VideoEncoder::LibX264;
        assert_eq!(
            resolve_video_encoder(&config, true),
            ("libx264".to_string(), "ultrafast".to_string())
        );
        config.encoder = VideoEncoder::H264Nvenc;
        assert_eq!(
            resolve_video_encoder(&config, true),
            ("h264_nvenc".to_string(), "p4".to_string())
        );
        assert_eq!(
            resolve_video_encoder(&config, false),
            ("libx264".to_string(), "ultrafast".to_string())
        );
        config.encoder = VideoEncoder::Auto;
        assert_eq!(resolve_video_encoder(&config, false).0, "libx264");
    }

    #[test]
    fn nvenc_decodes_on_the_gpu_and_sets_its_quality() {
        let config = video_config();
        let mut probed = probed_video();
        probed.video_codec = Some("av1".to_string());
        let joined = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&probed),
            &AudioLevel::Keep,
            "h264_nvenc",
            "p4",
        )
        .join(" ");
        assert!(joined.starts_with("-y -hide_banner -hwaccel cuda -c:v av1 -i in.mp4 "));
        assert!(joined
            .contains("-c:v h264_nvenc -preset p4 -rc vbr -cq 23 -b:v 0 -bf 0 -pix_fmt yuv420p"));
        probed.video_codec = Some("h264".to_string());
        let joined = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&probed),
            &AudioLevel::Keep,
            "h264_nvenc",
            "p4",
        )
        .join(" ");
        assert!(joined.starts_with("-y -hide_banner -hwaccel cuda -i in.mp4 "));
        let joined = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&probed),
            &AudioLevel::Keep,
            "libx264",
            "ultrafast",
        )
        .join(" ");
        assert!(!joined.contains("-hwaccel"));
        assert!(!joined.contains("-cq"));
    }

    #[test]
    fn builds_video_args() {
        let config = video_config();
        let probed = probed_video();
        let args = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&probed),
            &AudioLevel::Keep,
            "libx264",
            "ultrafast",
        );
        assert_eq!(args[0], "-y");
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"ultrafast".to_string()));
        assert!(args.contains(&"yuv420p".to_string()));
        assert!(args.contains(&"-movflags".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
        assert!(!args.contains(&"-af".to_string()));
        let gain = AudioLevel::Gain {
            gain_db: -3.0,
            limit: 0.841,
        };
        let args = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&probed),
            &gain,
            "libx264",
            "ultrafast",
        );
        let filter = args.iter().position(|arg| arg == "-af").unwrap();
        assert_eq!(
            args[filter + 1],
            "volume=-3dB,alimiter=limit=0.841:level=false:latency=true"
        );
        // A file without audio gets no audio filter whatever the level.
        let mut no_audio = probed;
        no_audio.has_audio = false;
        let args = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&no_audio),
            &gain,
            "h264_nvenc",
            "p4",
        );
        assert!(!args.contains(&"-af".to_string()));
        assert!(!args.contains(&"-ar".to_string()));
    }

    #[test]
    fn builds_audio_args_and_loudnorm() {
        let level = AudioLevel::Gain {
            gain_db: 2.5,
            limit: 0.841,
        };
        let args = build_audio_args(Path::new("in.wav"), Path::new("out.mp3"), &level);
        assert!(args.contains(&"libmp3lame".to_string()));
        assert!(args.contains(&"192k".to_string()));
        assert!(args
            .contains(&"volume=2.5dB,alimiter=limit=0.841:level=false:latency=true".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp3");
        let args = build_audio_args(Path::new("in.wav"), Path::new("out.mp3"), &AudioLevel::Keep);
        assert!(!args.contains(&"-af".to_string()));
        let targets = LoudnormTargets::default();
        assert!(targets.filter().contains("I=-18.0") || targets.filter().contains("I=-18"));
    }

    #[test]
    fn scales_and_rounds_even_dimensions() {
        assert_eq!(to_even(1081), 1080);
        assert_eq!(to_even(1), 2);
        assert_eq!(to_even(1920), 1920);
        let probed = probed_video();
        let filter = scale_filter(Some(&probed), 720).unwrap();
        assert!(filter.contains("scale=-2:720"));
        assert!(filter.contains("trunc(iw/2)*2"));
        let filter = scale_filter(Some(&probed), 1080).unwrap();
        assert!(!filter.contains("force_original_aspect_ratio"));
        assert!(scale_filter(None, 1080).is_some());
    }

    #[test]
    fn dimensions_after_scaling() {
        let probed = probed_video();
        let (width, height) = dimensions_for(MediaKind::Video, Some(&probed), Some(1080));
        assert_eq!((width, height), (Some(1920), Some(1080)));
        let (width, height) = dimensions_for(MediaKind::Video, Some(&probed), Some(720));
        assert_eq!((width, height), (Some(1280), Some(720)));
        assert_eq!(
            dimensions_for(MediaKind::Video, None, Some(720)),
            (None, None)
        );
        let portrait = ProbeInfo {
            width: Some(1080),
            height: Some(1920),
            ..probed
        };
        let (width, height) = dimensions_for(MediaKind::Video, Some(&portrait), Some(720));
        assert_eq!((width, height), (Some(404), Some(720)));
    }

    #[test]
    fn image_mime_resolution() {
        let probed = ProbeInfo {
            video_codec: Some("png".to_string()),
            ..ProbeInfo::default()
        };
        // Le codec sonde prime sur le Content-Type distant (AUD-A4-010).
        assert_eq!(
            image_mime(
                Some("image/webp; charset=binary"),
                Path::new("x.bin"),
                Some(&probed)
            ),
            "image/png"
        );
        assert_eq!(
            image_mime(Some("image/webp"), Path::new("x.bin"), None),
            "image/webp"
        );
        assert_eq!(
            image_mime(None, Path::new("x.bin"), Some(&probed)),
            "image/png"
        );
        assert_eq!(image_mime(None, Path::new("photo.jpg"), None), "image/jpeg");
        assert_eq!(image_mime(None, Path::new("x.inconnu"), None), "image/png");
        // Un Content-Type image/svg+xml n'est jamais retenu (contenu actif).
        assert_eq!(
            image_mime(Some("image/svg+xml"), Path::new("x.bin"), None),
            "image/png"
        );
    }

    #[test]
    fn normalize_image_rejette_le_svg_par_type_mime() {
        let directory = tempfile::TempDir::new().unwrap();
        let input = directory.path().join("dessin.bin");
        std::fs::write(&input, b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").unwrap();
        let output_base = directory.path().join("sortie");
        let error = normalize_image(&input, Some("image/svg+xml"), &output_base, None).unwrap_err();
        assert!(error.is(MediaErrorCode::InvalidMedia));
        assert!(error.message().contains("SVG"));
        assert!(!output_base.with_extension("png").exists());
    }

    #[test]
    fn normalize_image_rejette_le_svg_par_signature() {
        let directory = tempfile::TempDir::new().unwrap();
        let input = directory.path().join("image.png");
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        std::fs::write(&input, svg).unwrap();
        let output_base = directory.path().join("sortie");
        let error = normalize_image(&input, Some("image/png"), &output_base, None).unwrap_err();
        assert!(error.is(MediaErrorCode::InvalidMedia));
        assert!(error.message().contains("SVG"));
        assert!(!output_base.with_extension("png").exists());
    }

    #[test]
    fn carousel_dimensions_and_args() {
        assert_eq!(carousel_dimensions(1080, true), (608, 1080));
        assert_eq!(carousel_dimensions(1080, false), (1080, 608));
        assert_eq!(carousel_dimensions(0, true), (2, 2));
        let config = video_config();
        let images = vec![PathBuf::from("a.jpg"), PathBuf::from("b.jpg")];
        let args = build_carousel_video_args(
            &images,
            Some(Path::new("music.mp3")),
            Path::new("out.mp4"),
            (608, 1080),
            &config,
        );
        assert!(args.contains(&"-filter_complex".to_string()));
        assert!(args.iter().any(|arg| arg.contains("concat=n=2:v=1:a=0[v]")));
        assert!(args.contains(&"-shortest".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
        let single = build_carousel_video_args(
            &images[..1],
            None,
            Path::new("out.mp4"),
            (1080, 608),
            &config,
        );
        assert!(single.contains(&"-vf".to_string()));
        assert!(single.contains(&"-an".to_string()));
    }

    #[test]
    fn single_photo_carousel_keeps_video_and_follows_music() {
        let config = video_config();
        let images = vec![PathBuf::from("a.jpg")];
        let with_audio = build_carousel_video_args(
            &images,
            Some(Path::new("music.mp3")),
            Path::new("out.mp4"),
            (608, 1080),
            &config,
        );
        assert!(with_audio.windows(2).any(|pair| pair == ["-map", "0:v:0"]));
        assert!(with_audio.windows(2).any(|pair| pair == ["-map", "1:a:0"]));
        let image_input = with_audio.iter().position(|arg| arg == "a.jpg").unwrap();
        assert!(!with_audio[..image_input].contains(&"-t".to_string()));

        let without_audio =
            build_carousel_video_args(&images, None, Path::new("out.mp4"), (608, 1080), &config);
        assert!(without_audio
            .windows(2)
            .any(|pair| pair == ["-map", "0:v:0"]));
        assert!(without_audio.contains(&"-an".to_string()));
        assert!(without_audio.windows(2).any(|pair| pair == ["-t", "3"]));

        let images = vec![PathBuf::from("a.jpg"), PathBuf::from("b.jpg")];
        let multi = build_carousel_video_args(
            &images,
            Some(Path::new("music.mp3")),
            Path::new("out.mp4"),
            (608, 1080),
            &config,
        );
        assert!(multi.contains(&"-filter_complex".to_string()));
        assert!(multi.windows(2).any(|pair| pair == ["-map", "[v]"]));
        assert!(multi.windows(2).any(|pair| pair == ["-map", "2:a:0"]));
    }

    #[test]
    fn finalize_rejects_tiny_files() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("tiny.bin");
        std::fs::write(&path, b"x").unwrap();
        let error = finalize(
            MediaKind::Video,
            "video/mp4",
            &path,
            Some(&probed_video()),
            Some(1080),
            Some(5),
        )
        .unwrap_err();
        assert!(error.is(MediaErrorCode::InvalidMedia));
        assert!(!path.exists());
    }

    #[test]
    fn finalize_accepts_valid_media() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("clip.mp4");
        std::fs::write(&path, vec![0_u8; 4096]).unwrap();
        let normalized = finalize(
            MediaKind::Video,
            "video/mp4",
            &path,
            Some(&probed_video()),
            Some(1080),
            Some(12),
        )
        .unwrap();
        assert_eq!(normalized.kind, MediaKind::Video);
        assert_eq!(normalized.duration_sec, Some(12));
        assert_eq!(normalized.size_bytes, 4096);
        assert_eq!(
            (normalized.width, normalized.height),
            (Some(1920), Some(1080))
        );
        assert!(!normalized.is_vertical);
        let portrait = ProbeInfo {
            width: Some(1080),
            height: Some(1920),
            ..probed_video()
        };
        let normalized = finalize(
            MediaKind::Video,
            "video/mp4",
            &path,
            Some(&portrait),
            Some(1080),
            None,
        )
        .unwrap();
        assert!(normalized.is_vertical);
        assert_eq!(normalized.duration_sec, Some(1));
        let image = finalize(
            MediaKind::Image,
            "image/png",
            &path,
            Some(&portrait),
            None,
            None,
        )
        .unwrap();
        assert_eq!(image.duration_sec, None);
    }
}
