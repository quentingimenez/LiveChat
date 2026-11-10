//! Résolution YouTube : oEmbed puis, en repli, `yt-dlp --dump-json`.

use serde::Deserialize;
use serde_json::Value;

use crate::config::MediaConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::http;
use crate::process;
use crate::source::{MediaSource, SourceKind};

use super::{encode_query_component, ResolvedMedia};

/// Payload oEmbed YouTube (seuls les champs utiles sont désérialisés).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct YoutubeOEmbed {
    /// Titre de la vidéo.
    pub title: Option<String>,
    /// Nom de la chaîne.
    pub author_name: Option<String>,
    /// URL de la chaîne.
    pub author_url: Option<String>,
    /// Largeur du lecteur.
    pub width: Option<u32>,
    /// Hauteur du lecteur.
    pub height: Option<u32>,
}

/// Métadonnées extraites de `yt-dlp --dump-json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct YoutubeMetadata {
    /// Titre de la vidéo.
    pub title: Option<String>,
    /// Nom de la chaîne.
    pub author: Option<String>,
    /// Durée en secondes.
    pub duration_sec: Option<f64>,
    /// Largeur de la meilleure piste vidéo.
    pub width: Option<u32>,
    /// Hauteur de la meilleure piste vidéo.
    pub height: Option<u32>,
}

impl YoutubeMetadata {
    /// `true` si la vidéo est verticale.
    pub fn is_vertical(&self) -> bool {
        matches!((self.width, self.height), (Some(w), Some(h)) if w > 0 && h > w)
    }
}

/// Construit l'URL oEmbed YouTube d'une vidéo.
pub fn build_oembed_url(endpoint: &str, canonical_url: &str) -> String {
    format!(
        "{endpoint}{}url={}&format=json",
        separator(endpoint),
        encode_query_component(canonical_url)
    )
}

/// Arguments `yt-dlp` pour extraire les métadonnées sans télécharger.
pub fn ytdlp_dump_args(canonical_url: &str) -> Vec<String> {
    vec![
        "--dump-json".to_string(),
        "--no-playlist".to_string(),
        "--no-warnings".to_string(),
        "--skip-download".to_string(),
        canonical_url.to_string(),
    ]
}

/// Résout une vidéo YouTube.
pub async fn resolve_youtube(
    source: &MediaSource,
    config: &MediaConfig,
) -> MediaResult<ResolvedMedia> {
    let canonical_url = source.canonical_url.clone();
    let oembed_result = fetch_oembed(&canonical_url, config).await;
    if let Ok(payload) = oembed_result {
        let mut resolved = ResolvedMedia {
            source_kind: SourceKind::YouTube,
            canonical_url,
            source_hash: source.source_hash.clone(),
            title: payload.title,
            author: payload.author_name,
            kind: Some(lc_domain::MediaKind::Video),
            ..ResolvedMedia::from_source(source)
        };
        resolved = resolved.with_dimensions(payload.width, payload.height);
        return Ok(resolved);
    }
    let oembed_error = oembed_result.err();
    let dump = process::run_command(
        &config.tools.ytdlp,
        &ytdlp_dump_args(&canonical_url),
        config.download.timeout,
        MediaErrorCode::DownloadFailed,
    )
    .await;
    match dump {
        Ok(output) => {
            let metadata = parse_ytdlp_dump(&output.stdout)?;
            let mut resolved = ResolvedMedia {
                source_kind: SourceKind::YouTube,
                canonical_url,
                source_hash: source.source_hash.clone(),
                title: metadata.title,
                author: metadata.author,
                duration_sec: metadata.duration_sec,
                kind: Some(lc_domain::MediaKind::Video),
                ..ResolvedMedia::from_source(source)
            };
            resolved = resolved.with_dimensions(metadata.width, metadata.height);
            Ok(resolved)
        }
        Err(dump_error) => Err(match oembed_error {
            Some(oembed_error) => oembed_error.most_relevant(dump_error),
            None => dump_error,
        }),
    }
}

/// Parse la sortie JSON de `yt-dlp --dump-json`.
pub fn parse_ytdlp_dump(json: &str) -> MediaResult<YoutubeMetadata> {
    let value: Value = serde_json::from_str(json.trim()).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::DownloadFailed,
            "métadonnées yt-dlp illisibles",
            error.to_string(),
        )
    })?;
    let mut metadata = YoutubeMetadata {
        title: string_field(&value, "title"),
        author: string_field(&value, "uploader")
            .or_else(|| string_field(&value, "channel"))
            .or_else(|| string_field(&value, "uploader_id")),
        duration_sec: number_field(&value, "duration"),
        width: None,
        height: None,
    };
    let dimensions = best_dimensions(&value);
    metadata.width = dimensions.map(|(width, _)| width);
    metadata.height = dimensions.map(|(_, height)| height);
    Ok(metadata)
}

async fn fetch_oembed(canonical_url: &str, config: &MediaConfig) -> MediaResult<YoutubeOEmbed> {
    let url = build_oembed_url(&config.resolve.youtube_oembed_endpoint, canonical_url);
    http::get_json(&config.download, &url, config.resolve.oembed_timeout).await
}

fn separator(endpoint: &str) -> &'static str {
    if endpoint.contains('?') {
        "&"
    } else {
        "?"
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn number_field(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number > 0.0)
}

/// Choisit les dimensions de la meilleure piste vidéo disponible.
///
/// Les formats de yt-dlp décrivent la piste réellement téléchargée ; à défaut
/// (dump tronqué ou format sélectionné) on retombe sur width/height globaux.
pub fn best_dimensions(value: &Value) -> Option<(u32, u32)> {
    if let Some(formats) = value.get("formats").and_then(Value::as_array) {
        let best = formats
            .iter()
            .filter_map(|format| {
                let width = format.get("width").and_then(Value::as_u64)? as u32;
                let height = format.get("height").and_then(Value::as_u64)? as u32;
                (width > 0 && height > 0).then_some((width, height))
            })
            .max_by_key(|(_, height)| *height);
        if best.is_some() {
            return best;
        }
    }
    let width = value.get("width").and_then(Value::as_u64)? as u32;
    let height = value.get("height").and_then(Value::as_u64)? as u32;
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn builds_oembed_url_with_encoding() {
        let url = build_oembed_url(
            "https://www.youtube.com/oembed",
            "https://www.youtube.com/watch?v=abc&t=5",
        );
        assert_eq!(
            url,
            "https://www.youtube.com/oembed?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Dabc%26t%3D5&format=json"
        );
        let with_query = build_oembed_url("https://example.test/oembed?x=1", "https://a.test/v");
        assert!(with_query.starts_with("https://example.test/oembed?x=1&url="));
    }

    #[test]
    fn ytdlp_dump_args_are_stable() {
        let args = ytdlp_dump_args("https://www.youtube.com/watch?v=abc");
        assert_eq!(args[0], "--dump-json");
        assert!(args.contains(&"--skip-download".to_string()));
        assert_eq!(args.last().unwrap(), "https://www.youtube.com/watch?v=abc");
    }

    #[test]
    fn parses_ytdlp_dump_metadata() {
        let json = r#"{
            "title": "Un test",
            "uploader": "Chaîne Test",
            "duration": 42.5,
            "width": 1920,
            "height": 1080,
            "formats": [
                {"width": 640, "height": 360},
                {"width": 1920, "height": 1080},
                {"width": 1080, "height": 1920}
            ]
        }"#;
        let metadata = parse_ytdlp_dump(json).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Un test"));
        assert_eq!(metadata.author.as_deref(), Some("Chaîne Test"));
        assert_eq!(metadata.duration_sec, Some(42.5));
        assert_eq!((metadata.width, metadata.height), (Some(1080), Some(1920)));
        assert!(metadata.is_vertical());
    }

    #[test]
    fn parses_ytdlp_dump_with_vertical_short() {
        let json = r#"{"title":"Short","width":1080,"height":1920}"#;
        let metadata = parse_ytdlp_dump(json).unwrap();
        assert_eq!((metadata.width, metadata.height), (Some(1080), Some(1920)));
        assert!(metadata.is_vertical());
        assert_eq!(metadata.duration_sec, None);
    }

    #[test]
    fn rejects_invalid_ytdlp_dump() {
        let error = parse_ytdlp_dump("pas du json").unwrap_err();
        assert!(error.is(MediaErrorCode::DownloadFailed));
        let empty = parse_ytdlp_dump("{}").unwrap();
        assert!(empty.title.is_none());
        assert_eq!(empty.width, None);
    }
}
