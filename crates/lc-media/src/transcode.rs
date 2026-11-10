//! Normalisation ffmpeg : vidéo H.264/AAC MP4 faststart, audio MP3, images
//! copiées telles quelles, plus la construction des vidéos de carrousel TikTok.
//!
//! La décision de transcoder ou de copier reprend les règles de l'ancien
//! serveur : un MP4/MOV H.264/AAC yuv420p sous la hauteur maximale est copié.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lc_domain::MediaKind;

use crate::config::{MediaConfig, VideoConfig, VideoEncoder};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::probe::{self, ProbeInfo};
use crate::process;
use crate::source::{extension_from_mime, kind_from_mime, mime_from_extension};
use crate::storage::{staged_path, MediaStorage};

/// Taille minimale d'un média normalisé valide.
pub const MIN_MEDIA_BYTES: u64 = 1024;
/// Débit audio de sortie.
pub const AUDIO_BITRATE: &str = "192k";
/// Type MIME des audio normalisés.
pub const AUDIO_MIME: &str = "audio/mpeg";
/// Type MIME des vidéos normalisées.
pub const VIDEO_MIME: &str = "video/mp4";

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
}

/// Détecte le type de média à normaliser.
pub fn detect_kind(content_type: Option<&str>, probed: Option<&ProbeInfo>) -> Option<MediaKind> {
    if let Some(probed) = probed {
        if probed.has_video && probed.has_audio {
            return Some(MediaKind::Video);
        }
        if probed.has_video {
            if probed.duration_ceil_sec().is_none() && is_image_codec(probed.video_codec.as_deref())
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
    let probed = probe::probe(&config.tools.ffprobe, input, config.download.timeout)
        .await
        .ok();
    let kind = detect_kind(content_type, probed.as_ref()).ok_or_else(|| {
        MediaError::new(MediaErrorCode::InvalidMedia, "format de média non reconnu")
    })?;
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
        MediaKind::Image => normalize_image(input, content_type, output_base, probed.as_ref()),
        MediaKind::Audio => normalize_audio(input, output_base, probed.as_ref(), config).await,
        MediaKind::Video => normalize_video(input, output_base, probed.as_ref(), config).await,
    }
}

/// Normalise une image en la copiant avec la bonne extension.
pub fn normalize_image(
    input: &Path,
    content_type: Option<&str>,
    output_base: &Path,
    probed: Option<&ProbeInfo>,
) -> MediaResult<NormalizedMedia> {
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
    finalize(MediaKind::Image, &mime, &output, probed, None, None)
}

/// Détermine le type MIME d'une image téléchargée.
pub fn image_mime(content_type: Option<&str>, input: &Path, probed: Option<&ProbeInfo>) -> String {
    if let Some(content_type) = content_type {
        let essence = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if essence.starts_with("image/") {
            return essence;
        }
    }
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
    input
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(mime_from_extension)
        .filter(|mime| mime.starts_with("image/"))
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
    let can_copy = !config.video.audio_normalize
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
        let args = build_audio_args(input, &staged, config);
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
    finalize(
        MediaKind::Audio,
        AUDIO_MIME,
        &output,
        probed,
        None,
        duration,
    )
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
    let can_copy = probed
        .as_ref()
        .is_some_and(|probed| can_copy_without_transcode(probed, &config.video));
    if can_copy {
        if let Err(error) = fs::copy(input, &staged) {
            let _removed = fs::remove_file(&staged);
            return Err(MediaError::from_io(
                MediaErrorCode::InvalidMedia,
                "copie de la vidéo normalisée",
                &error,
            ));
        }
    } else {
        let nvenc_available = match config.video.encoder {
            VideoEncoder::LibX264 => false,
            VideoEncoder::H264Nvenc | VideoEncoder::Auto => {
                has_nvenc(&config.tools.ffmpeg, config.download.timeout).await
            }
        };
        let (encoder, preset) = resolve_video_encoder(&config.video, nvenc_available);
        let args = build_video_transcode_args(input, &staged, config, probed, &encoder, &preset);
        let first = process::run_command(
            &config.tools.ffmpeg,
            &args,
            config.download.timeout,
            MediaErrorCode::TranscodeFailed,
        )
        .await;
        if let Err(error) = first {
            let _removed = fs::remove_file(&staged);
            if encoder == "h264_nvenc" {
                tracing::warn!("échec NVENC, nouvelle tentative en libx264");
                let (fallback_encoder, fallback_preset) =
                    resolve_video_encoder(&config.video, false);
                let fallback_args = build_video_transcode_args(
                    input,
                    &staged,
                    config,
                    probed,
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
                    let _removed = fs::remove_file(&staged);
                    return Err(fallback_error);
                }
            } else {
                return Err(error);
            }
        }
    }
    MediaStorage::commit_file(&staged, &output)?;
    let output_probe = probe::probe(&config.tools.ffprobe, &output, config.download.timeout)
        .await
        .ok();
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

/// Vérifie le résultat normalisé et construit [`NormalizedMedia`].
pub fn finalize(
    kind: MediaKind,
    mime: &str,
    output: &Path,
    probed: Option<&ProbeInfo>,
    max_height: Option<u32>,
    duration_sec: Option<i64>,
) -> MediaResult<NormalizedMedia> {
    let size_bytes = MediaStorage::file_size(output)?;
    if size_bytes < MIN_MEDIA_BYTES {
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
    let video_ok = probed.video_codec.as_deref() == Some("h264");
    let audio_ok = matches!(probed.audio_codec.as_deref(), None | Some("aac"));
    let pixel_ok = matches!(
        probed.pixel_format.as_deref(),
        None | Some("yuv420p" | "yuvj420p")
    );
    let height_ok = probed
        .height
        .map(|height| height <= config.max_height)
        .unwrap_or(true);
    container_ok && video_ok && audio_ok && pixel_ok && height_ok
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

/// `true` si ffmpeg expose l'encodeur `h264_nvenc`.
pub async fn has_nvenc(ffmpeg: &Path, timeout: Duration) -> bool {
    let args = vec!["-hide_banner".to_string(), "-encoders".to_string()];
    match process::run_command(ffmpeg, &args, timeout, MediaErrorCode::TranscodeFailed).await {
        Ok(output) => output.combined().contains("h264_nvenc"),
        Err(_) => false,
    }
}

/// Arguments de transcodage vidéo H.264/AAC MP4 faststart.
pub fn build_video_transcode_args(
    input: &Path,
    output: &Path,
    config: &MediaConfig,
    probed: Option<&ProbeInfo>,
    encoder: &str,
    preset: &str,
) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-c:v".to_string(),
        encoder.to_string(),
        "-preset".to_string(),
        preset.to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
    ];
    if let Some(filter) = scale_filter(probed, config.video.max_height) {
        args.push("-vf".to_string());
        args.push(filter);
    }
    args.push("-c:a".to_string());
    args.push("aac".to_string());
    args.push("-b:a".to_string());
    args.push(AUDIO_BITRATE.to_string());
    let has_audio = probed.map(|probed| probed.has_audio).unwrap_or(false);
    if config.video.audio_normalize && has_audio {
        args.push("-af".to_string());
        args.push(config.video.loudnorm.filter());
    }
    args.push("-movflags".to_string());
    args.push("+faststart".to_string());
    args.push(output.display().to_string());
    args
}

/// Arguments de normalisation audio MP3.
pub fn build_audio_args(input: &Path, output: &Path, config: &MediaConfig) -> Vec<String> {
    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-vn".to_string(),
        "-acodec".to_string(),
        "libmp3lame".to_string(),
        "-b:a".to_string(),
        AUDIO_BITRATE.to_string(),
    ];
    if config.video.audio_normalize {
        args.push("-af".to_string());
        args.push(config.video.loudnorm.filter());
    }
    args.push(output.display().to_string());
    args
}

/// Filtre de normalisation loudness legacy.
pub fn loudnorm_filter(config: &MediaConfig) -> String {
    config.video.loudnorm.filter()
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
    let mut args: Vec<String> = vec!["-y".to_string()];
    for image in images {
        args.push("-loop".to_string());
        args.push("1".to_string());
        args.push("-t".to_string());
        args.push("3".to_string());
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
    fn builds_video_args() {
        let config = video_config();
        let probed = probed_video();
        let args = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&probed),
            "libx264",
            "ultrafast",
        );
        assert_eq!(args[0], "-y");
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"ultrafast".to_string()));
        assert!(args.contains(&"yuv420p".to_string()));
        assert!(args.contains(&"-movflags".to_string()));
        assert!(args.contains(&"+faststart".to_string()));
        assert!(args.contains(&"loudnorm=I=-18:LRA=11:TP=-1.5".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp4");
        let mut no_audio = probed;
        no_audio.has_audio = false;
        let args = build_video_transcode_args(
            Path::new("in.mp4"),
            Path::new("out.mp4"),
            &config,
            Some(&no_audio),
            "h264_nvenc",
            "p4",
        );
        assert!(!args.contains(&"-af".to_string()));
    }

    #[test]
    fn builds_audio_args_and_loudnorm() {
        let config = video_config();
        let args = build_audio_args(Path::new("in.wav"), Path::new("out.mp3"), &config);
        assert!(args.contains(&"libmp3lame".to_string()));
        assert!(args.contains(&"192k".to_string()));
        assert_eq!(args.last().unwrap(), "out.mp3");
        assert_eq!(loudnorm_filter(&config), "loudnorm=I=-18:LRA=11:TP=-1.5");
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
        assert_eq!(
            image_mime(
                Some("image/webp; charset=binary"),
                Path::new("x.bin"),
                Some(&probed)
            ),
            "image/webp"
        );
        assert_eq!(
            image_mime(None, Path::new("x.bin"), Some(&probed)),
            "image/png"
        );
        assert_eq!(image_mime(None, Path::new("photo.jpg"), None), "image/jpeg");
        assert_eq!(image_mime(None, Path::new("x.inconnu"), None), "image/png");
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
