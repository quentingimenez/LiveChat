//! Platform thumbnails of linked media (YouTube, TikTok, X).
//!
//! A board card shows a picture of each media. For a link, the platform
//! already publishes one: it is downloaded **after** the ingestion, never on
//! its path, and stored next to the media file as `<hash>.cover.<ext>`, like
//! the embedded cover of an audio file. Everything here is best effort: a
//! missing thumbnail only leaves the card on its fallback.

use std::path::Path;
use std::time::Duration;

use crate::config::MediaConfig;
use crate::http;
use crate::resolve::{tiktok, ResolvedMedia};
use crate::source::{MediaSource, SourceKind};
use crate::storage::{staged_path, MediaStorage};
use crate::transcode::NormalizedCover;

/// Largest thumbnail downloaded.
pub const MAX_THUMBNAIL_BYTES: u64 = 5 * 1024 * 1024;

/// Smallest picture kept: below this, it is a placeholder.
const MIN_THUMBNAIL_BYTES: usize = 1024;

/// Time allowed to download one thumbnail.
const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(15);

/// Id of a YouTube video from its canonical URL (`/watch?v=<id>`).
pub fn youtube_video_id(canonical_url: &str) -> Option<String> {
    let parsed = url::Url::parse(canonical_url).ok()?;
    parsed
        .query_pairs()
        .find(|(key, _)| key == "v")
        .map(|(_, value)| value.into_owned())
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 32
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// Pictures to try for a source, best first. YouTube's are derived from the
/// video id (no request needed); the others come from the resolution.
pub fn thumbnail_candidates(source: &MediaSource, resolved: Option<&ResolvedMedia>) -> Vec<String> {
    let mut candidates = Vec::new();
    if source.source_kind == SourceKind::YouTube {
        if let Some(id) = youtube_video_id(&source.canonical_url) {
            candidates.push(format!("https://i.ytimg.com/vi/{id}/maxresdefault.jpg"));
            candidates.push(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
        }
    }
    if let Some(url) = resolved.and_then(|resolved| resolved.thumbnail_url.as_deref()) {
        let url = url.trim();
        if url.starts_with("https://") && !candidates.iter().any(|known| known == url) {
            candidates.push(url.to_owned());
        }
    }
    candidates
}

/// [`thumbnail_candidates`], completed for a TikTok post without resolution
/// (a media reused from the cache) by the cover found on its page.
pub async fn discover_thumbnail_candidates(
    config: &MediaConfig,
    source: &MediaSource,
    resolved: Option<&ResolvedMedia>,
) -> Vec<String> {
    let mut candidates = thumbnail_candidates(source, resolved);
    if candidates.is_empty() && source.source_kind == SourceKind::TikTok {
        match tiktok::fetch_tiktok_page(config, &source.canonical_url).await {
            Ok(page) => candidates.extend(page.cover),
            Err(error) => {
                tracing::debug!(error = %error.summary(), "miniature TikTok introuvable");
            }
        }
    }
    candidates
}

/// Image format of a picture from its first bytes: `(extension, mime)`.
pub fn image_format(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("jpg", "image/jpeg"))
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(("png", "image/png"))
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else {
        None
    }
}

/// Width and height of a JPEG, PNG or WebP picture, read from its header.
pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let (extension, _) = image_format(bytes)?;
    let dimensions = match extension {
        "png" => png_dimensions(bytes),
        "webp" => webp_dimensions(bytes),
        _ => jpeg_dimensions(bytes),
    }?;
    (dimensions.0 > 0 && dimensions.1 > 0).then_some(dimensions)
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u32> {
    let pair = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_be_bytes([pair[0], pair[1]])))
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u32> {
    let pair = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_le_bytes([pair[0], pair[1]])))
}

fn le_u24(bytes: &[u8], at: usize) -> Option<u32> {
    let triple = bytes.get(at..at + 3)?;
    Some(u32::from_le_bytes([triple[0], triple[1], triple[2], 0]))
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    // Signature (8), IHDR length (4) and type (4), then width and height.
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        // Markers may be padded with extra 0xFF bytes.
        while *bytes.get(at)? == 0xFF && *bytes.get(at + 1)? == 0xFF {
            at += 1;
        }
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        // Markers without a length.
        if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            at += 2;
            continue;
        }
        let length = be_u16(bytes, at + 2)? as usize;
        // Start of frame (baseline, progressive...), not DHT/JPG/DAC.
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let height = be_u16(bytes, at + 5)?;
            let width = be_u16(bytes, at + 7)?;
            return Some((width, height));
        }
        at += 2 + length;
    }
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => {
            // Frame tag (3) and start code (3), then 14-bit sizes.
            if bytes.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            Some((le_u16(bytes, 26)? & 0x3FFF, le_u16(bytes, 28)? & 0x3FFF))
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => Some((le_u24(bytes, 24)? + 1, le_u24(bytes, 27)? + 1)),
        _ => None,
    }
}

/// Downloads the first candidate that is a usable picture and stores it as
/// `<hash>.cover.<ext>`. `None` when no candidate gave one.
pub async fn fetch_thumbnail(
    config: &MediaConfig,
    storage: &MediaStorage,
    source_hash: &str,
    candidates: &[String],
) -> Option<NormalizedCover> {
    for url in candidates {
        let bytes = match http::get_bytes(
            &config.download,
            url,
            THUMBNAIL_TIMEOUT,
            MAX_THUMBNAIL_BYTES,
        )
        .await
        {
            Ok(bytes) => bytes.body,
            Err(error) => {
                tracing::debug!(url, error = %error.summary(), "miniature indisponible");
                continue;
            }
        };
        if bytes.len() < MIN_THUMBNAIL_BYTES {
            continue;
        }
        let Some((extension, mime)) = image_format(&bytes) else {
            tracing::debug!(url, "miniature dans un format non pris en charge");
            continue;
        };
        let output = storage
            .output_base_path(source_hash)
            .with_extension(format!("cover.{extension}"));
        if let Err(error) = write_picture(&output, &bytes).await {
            tracing::warn!(error = %error, "enregistrement de la miniature impossible");
            return None;
        }
        let dimensions = image_dimensions(&bytes);
        return Some(NormalizedCover {
            mime: mime.to_owned(),
            width: dimensions.map(|(width, _)| width),
            height: dimensions.map(|(_, height)| height),
            storage_path: output,
            size_bytes: bytes.len() as u64,
        });
    }
    None
}

/// Writes a picture through a staged file so a reader never sees half of it.
async fn write_picture(output: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let staged = staged_path(output);
    if let Err(error) = tokio::fs::write(&staged, bytes).await {
        let _removed = tokio::fs::remove_file(&staged).await;
        return Err(error);
    }
    MediaStorage::commit_file(&staged, output).map_err(|error| {
        let _removed = std::fs::remove_file(&staged);
        std::io::Error::other(error.summary())
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn resolved_with(source: &MediaSource, thumbnail: Option<&str>) -> ResolvedMedia {
        ResolvedMedia {
            thumbnail_url: thumbnail.map(str::to_owned),
            ..ResolvedMedia::from_source(source)
        }
    }

    #[test]
    fn youtube_candidates_come_from_the_video_id() {
        let source = MediaSource::from_raw("https://youtu.be/dQw4w9WgXcQ").unwrap();
        let resolved = resolved_with(
            &source,
            Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"),
        );
        assert_eq!(
            thumbnail_candidates(&source, Some(&resolved)),
            vec![
                "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg".to_owned(),
                "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg".to_owned(),
            ]
        );
    }

    #[test]
    fn youtube_shorts_get_candidates_without_resolution() {
        let source = MediaSource::from_raw("https://www.youtube.com/shorts/abc_DEF-123").unwrap();
        let candidates = thumbnail_candidates(&source, None);
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("https://i.ytimg.com/vi/abc_DEF-123/maxresdefault.jpg")
        );
    }

    #[test]
    fn other_sources_use_the_resolved_thumbnail_over_https_only() {
        let source =
            MediaSource::from_raw("https://www.tiktok.com/@someone/video/7300000000000000000")
                .unwrap();
        let resolved = resolved_with(&source, Some("https://p16.tiktokcdn.com/thumb.image"));
        assert_eq!(
            thumbnail_candidates(&source, Some(&resolved)),
            vec!["https://p16.tiktokcdn.com/thumb.image".to_owned()]
        );
        let insecure = resolved_with(&source, Some("http://p16.tiktokcdn.com/thumb.image"));
        assert!(thumbnail_candidates(&source, Some(&insecure)).is_empty());
        assert!(thumbnail_candidates(&source, None).is_empty());
    }

    #[test]
    fn rejects_odd_youtube_ids() {
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?v=abc/../x"),
            None
        );
        assert_eq!(youtube_video_id("https://www.youtube.com/watch"), None);
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ").as_deref(),
            Some("dQw4w9WgXcQ")
        );
    }

    #[test]
    fn reads_png_dimensions() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&640_u32.to_be_bytes());
        png.extend_from_slice(&360_u32.to_be_bytes());
        assert_eq!(image_format(&png), Some(("png", "image/png")));
        assert_eq!(image_dimensions(&png), Some((640, 360)));
    }

    #[test]
    fn reads_jpeg_dimensions_after_other_segments() {
        let mut jpeg = vec![0xFF, 0xD8];
        // APP0 segment of 16 bytes (length included).
        jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        jpeg.extend_from_slice(&[0; 14]);
        // DHT segment, skipped.
        jpeg.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x04, 0, 0]);
        // Progressive SOF2: length, precision, height 720, width 1280.
        jpeg.extend_from_slice(&[0xFF, 0xC2, 0x00, 0x11, 0x08, 0x02, 0xD0, 0x05, 0x00]);
        assert_eq!(image_format(&jpeg), Some(("jpg", "image/jpeg")));
        assert_eq!(image_dimensions(&jpeg), Some((1280, 720)));
    }

    #[test]
    fn reads_webp_dimensions() {
        let mut lossy = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0".to_vec();
        lossy.extend_from_slice(&[0, 0, 0, 0x9D, 0x01, 0x2A]);
        lossy.extend_from_slice(&480_u16.to_le_bytes());
        lossy.extend_from_slice(&270_u16.to_le_bytes());
        assert_eq!(image_dimensions(&lossy), Some((480, 270)));

        let mut extended = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        extended.extend_from_slice(&[0x7F, 0x02, 0x00, 0xCF, 0x04, 0x00]);
        assert_eq!(image_dimensions(&extended), Some((640, 1232)));
    }

    #[test]
    fn unknown_bytes_are_not_pictures() {
        assert_eq!(image_format(b"<html>not an image</html>"), None);
        assert_eq!(image_dimensions(&[0xFF, 0xD8, 0xFF]), None);
    }
}
