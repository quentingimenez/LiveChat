//! Sonde `ffprobe` : extraction des caractéristiques techniques d'un média.
//!
//! Les métadonnées produites servent à décider d'un transcodage, à calculer le
//! budget de durée et à renseigner `NormalizedMedia`. La commande et le parseur
//! sont séparés pour rester testables hors ligne.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::process;

/// Délai par défaut d'une sonde ffprobe.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Caractéristiques d'un média détectées par ffprobe.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeInfo {
    /// Durée en secondes (absente pour une image).
    pub duration_sec: Option<f64>,
    /// Largeur en pixels du premier flux vidéo.
    pub width: Option<u32>,
    /// Hauteur en pixels du premier flux vidéo.
    pub height: Option<u32>,
    /// Nom du conteneur (`mov,mp4,m4a,3gp,3g2,mj2`, `mp3`, `image2`, …).
    pub format_name: Option<String>,
    /// Codec du premier flux vidéo (`h264`, `vp9`, `png`, …).
    pub video_codec: Option<String>,
    /// Codec du premier flux audio (`aac`, `mp3`, …).
    pub audio_codec: Option<String>,
    /// Format de pixels du premier flux vidéo.
    pub pixel_format: Option<String>,
    /// `true` si au moins un flux vidéo (ou image) existe.
    pub has_video: bool,
    /// `true` si au moins un flux audio existe.
    pub has_audio: bool,
}

impl ProbeInfo {
    /// `true` si la vidéo est plus haute que large.
    pub fn is_vertical(&self) -> bool {
        match (self.width, self.height) {
            (Some(width), Some(height)) => width > 0 && height > width,
            _ => false,
        }
    }

    /// Durée arrondie au supérieur en secondes.
    pub fn duration_ceil_sec(&self) -> Option<i64> {
        self.duration_sec
            .filter(|duration| duration.is_finite() && *duration > 0.0)
            .map(|duration| duration.ceil() as i64)
    }
}

#[derive(Debug, Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Debug, Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawFormat {
    duration: Option<String>,
    format_name: Option<String>,
}

/// Arguments ffprobe pour une sonde JSON.
pub fn probe_args(path: &Path) -> Vec<String> {
    vec![
        "-v".to_string(),
        "error".to_string(),
        "-show_entries".to_string(),
        "stream=codec_type,codec_name,width,height,pix_fmt,duration:format=duration,format_name"
            .to_string(),
        "-of".to_string(),
        "json".to_string(),
        path.display().to_string(),
    ]
}

/// Exécute ffprobe sur un fichier local.
pub async fn probe(ffprobe: &Path, path: &Path, timeout: Duration) -> MediaResult<ProbeInfo> {
    let output = process::run_command(
        ffprobe,
        &probe_args(path),
        timeout,
        MediaErrorCode::InvalidMedia,
    )
    .await?;
    parse_probe_json(&output.stdout)
}

/// Parse la sortie JSON de ffprobe.
pub fn parse_probe_json(json: &str) -> MediaResult<ProbeInfo> {
    let raw: RawProbe = serde_json::from_str(json).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::InvalidMedia,
            "réponse ffprobe illisible",
            error.to_string(),
        )
    })?;
    let mut info = ProbeInfo {
        format_name: raw
            .format
            .as_ref()
            .and_then(|format| format.format_name.clone()),
        ..ProbeInfo::default()
    };
    if let Some(format) = &raw.format {
        info.duration_sec = format.duration.as_deref().and_then(parse_seconds);
    }
    if let Some(stream) = raw
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
    {
        info.has_video = true;
        info.video_codec = stream.codec_name.clone().filter(|codec| codec != "none");
        info.width = stream.width.filter(|width| *width > 0);
        info.height = stream.height.filter(|height| *height > 0);
        info.pixel_format = stream.pix_fmt.clone();
        if info.duration_sec.is_none() {
            info.duration_sec = stream.duration.as_deref().and_then(parse_seconds);
        }
    }
    if let Some(stream) = raw
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
    {
        info.has_audio = true;
        info.audio_codec = stream.codec_name.clone().filter(|codec| codec != "none");
        if info.duration_sec.is_none() {
            info.duration_sec = stream.duration.as_deref().and_then(parse_seconds);
        }
    }
    Ok(info)
}

fn parse_seconds(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "N/A" {
        return None;
    }
    let parsed = if trimmed.contains(':') {
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() != 3 {
            return None;
        }
        let hours = parts[0].parse::<f64>().ok()?;
        let minutes = parts[1].parse::<f64>().ok()?;
        let seconds = parts[2].parse::<f64>().ok()?;
        hours * 3600.0 + minutes * 60.0 + seconds
    } else {
        trimmed.parse::<f64>().ok()?
    };
    (parsed.is_finite() && parsed > 0.0).then_some(parsed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const VIDEO_JSON: &str = r#"{
        "streams": [
            {"codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080,
             "pix_fmt": "yuv420p", "duration": "12.345000"},
            {"codec_type": "audio", "codec_name": "aac", "duration": "12.345000"}
        ],
        "format": {"duration": "12.345000", "format_name": "mov,mp4,m4a,3gp,3g2,mj2"}
    }"#;

    #[test]
    fn parses_video_probe() {
        let info = parse_probe_json(VIDEO_JSON).unwrap();
        assert_eq!(info.width, Some(1920));
        assert_eq!(info.height, Some(1080));
        assert_eq!(info.video_codec.as_deref(), Some("h264"));
        assert_eq!(info.audio_codec.as_deref(), Some("aac"));
        assert_eq!(info.pixel_format.as_deref(), Some("yuv420p"));
        assert!(info.has_video && info.has_audio);
        assert_eq!(info.duration_ceil_sec(), Some(13));
        assert!(!info.is_vertical());
    }

    #[test]
    fn parses_audio_only_and_image_probes() {
        let audio = parse_probe_json(
            r#"{"streams":[{"codec_type":"audio","codec_name":"mp3"}],"format":{"duration":"7.5","format_name":"mp3"}}"#,
        )
        .unwrap();
        assert!(audio.has_audio && !audio.has_video);
        assert_eq!(audio.duration_ceil_sec(), Some(8));
        let image = parse_probe_json(
            r#"{"streams":[{"codec_type":"video","codec_name":"png","width":300,"height":600}],"format":{"format_name":"png_pipe"}}"#,
        )
        .unwrap();
        assert!(image.has_video && !image.has_audio);
        assert!(image.is_vertical());
        assert_eq!(image.duration_ceil_sec(), None);
    }

    #[test]
    fn parses_colon_and_unknown_durations() {
        assert_eq!(parse_seconds("01:02:03.5"), Some(3723.5));
        assert_eq!(parse_seconds("N/A"), None);
        assert_eq!(parse_seconds(""), None);
        assert_eq!(parse_seconds("-1"), None);
        assert_eq!(parse_seconds("0"), None);
    }

    #[test]
    fn rejects_invalid_json() {
        let error = parse_probe_json("pas du json").unwrap_err();
        assert!(error.is(MediaErrorCode::InvalidMedia));
    }

    #[test]
    fn probe_args_are_stable() {
        let args = probe_args(Path::new("C:\\media\\clip.mp4"));
        assert_eq!(args[0], "-v");
        assert_eq!(args[1], "error");
        assert_eq!(args[3].split(':').count(), 2);
        assert_eq!(args.last().unwrap(), "C:\\media\\clip.mp4");
    }
}
