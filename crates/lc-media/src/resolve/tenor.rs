//! Résolution Tenor : une page `/view/<id>` est du HTML Open Graph, pas le
//! GIF. Le résolveur lit les balises `og:video` (MP4), `og:image` (GIF) et les
//! dimensions, puis `download_tenor` télécharge la vidéo et retombe sur l'image
//! quand le MP4 est absent ou inaccessible.

use url::Url;

use lc_domain::MediaKind;

use crate::config::MediaConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::html;
use crate::http;
use crate::source::{MediaSource, SourceKind};
use crate::title;

use super::ResolvedMedia;

/// Métadonnées extraites d'une page Tenor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TenorPage {
    /// Titre de la page (`og:title`), sans le suffixe « GIF - … ».
    pub title: Option<String>,
    /// URL du MP4 (`og:video`, `og:video:secure_url`, `twitter:player:stream`).
    pub video_url: Option<String>,
    /// Type MIME annoncé du MP4.
    pub video_mime: Option<String>,
    /// URL du GIF (`og:image`, `twitter:image`).
    pub image_url: Option<String>,
    /// Type MIME annoncé du GIF.
    pub image_mime: Option<String>,
    /// Largeur annoncée en pixels.
    pub width: Option<u32>,
    /// Hauteur annoncée en pixels.
    pub height: Option<u32>,
}

impl TenorPage {
    /// `true` si la page ne contient ni vidéo ni image exploitable.
    pub fn is_empty(&self) -> bool {
        self.video_url.is_none() && self.image_url.is_none()
    }

    /// URL des médias dans l'ordre de préférence (vidéo, puis image).
    pub fn media_urls(&self) -> Vec<&str> {
        let mut urls = Vec::new();
        if let Some(url) = self.video_url.as_deref() {
            urls.push(url);
        }
        if let Some(url) = self.image_url.as_deref() {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        urls
    }
}

/// Extrait les métadonnées Open Graph d'une page Tenor.
pub fn parse_page_html(page: &str) -> TenorPage {
    let video_url = first_http_url(&[
        html::meta_content(page, "og:video:secure_url"),
        html::meta_content(page, "og:video"),
        html::meta_content(page, "twitter:player:stream"),
    ]);
    let video_mime = html::meta_content(page, "og:video:type")
        .or_else(|| html::meta_content(page, "twitter:player:stream:content_type"))
        .filter(|mime| mime.to_ascii_lowercase().starts_with("video/"));
    let image_url = first_http_url(&[
        html::meta_content(page, "og:image"),
        html::meta_content(page, "twitter:image"),
    ]);
    let image_mime = html::meta_content(page, "og:image:type")
        .filter(|mime| mime.to_ascii_lowercase().starts_with("image/"))
        .or_else(|| {
            image_url
                .as_deref()
                .and_then(kind_from_image_url)
                .map(|mime| mime.to_string())
        });
    let width = dimension(page, "og:video:width").or_else(|| dimension(page, "og:image:width"));
    let height = dimension(page, "og:video:height").or_else(|| dimension(page, "og:image:height"));
    TenorPage {
        title: html::meta_content(page, "og:title")
            .or_else(|| html::meta_content(page, "twitter:title"))
            .and_then(|title| clean_title(&title)),
        video_url,
        video_mime,
        image_url,
        image_mime,
        width,
        height,
    }
}

/// Retire le suffixe de référencement Tenor (« GIF - … ») d'un titre.
///
/// `Jonathan Drouin Halifax Mooseheads GIF - Jonathan drouin … - Discover &
/// Share GIFs` devient `Jonathan Drouin Halifax Mooseheads`.
pub fn clean_title(raw: &str) -> Option<String> {
    let decoded = html::decode_html_entities(raw);
    let headline = decoded
        .split_once(" GIF - ")
        .map_or(decoded.as_str(), |(headline, _)| headline)
        .trim();
    let headline = headline.strip_suffix(" GIF").unwrap_or(headline);
    title::normalize_title(headline).or_else(|| title::normalize_title(&decoded))
}

/// Récupère et analyse la page Tenor d'une source.
pub async fn fetch_tenor_page(config: &MediaConfig, page_url: &str) -> MediaResult<TenorPage> {
    let headers = [
        ("User-Agent", config.download.user_agent.as_str()),
        ("Accept", "text/html,application/xhtml+xml"),
        ("Accept-Language", "fr-FR,fr;q=0.9,en;q=0.8"),
    ];
    let response = http::get_text(
        &config.download,
        page_url,
        config.resolve.oembed_timeout,
        &headers,
    )
    .await?;
    let page = parse_page_html(&response.body);
    if page.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::MediaNotFound,
            "la page Tenor ne contient aucun média",
        ));
    }
    Ok(page)
}

/// Résout une page Tenor en média téléchargeable.
pub async fn resolve_tenor(
    source: &MediaSource,
    config: &MediaConfig,
) -> MediaResult<ResolvedMedia> {
    let page = fetch_tenor_page(config, &source.canonical_url).await?;
    let (media_url, mime, kind) = if let Some(video_url) = page.video_url.clone() {
        (Some(video_url), page.video_mime.clone(), MediaKind::Video)
    } else {
        (
            page.image_url.clone(),
            page.image_mime.clone(),
            MediaKind::Image,
        )
    };
    let resolved = ResolvedMedia {
        source_kind: SourceKind::Tenor,
        canonical_url: source.canonical_url.clone(),
        source_hash: source.source_hash.clone(),
        title: page.title.clone(),
        kind: Some(kind),
        media_url,
        mime,
        ..ResolvedMedia::from_source(source)
    };
    Ok(resolved.with_dimensions(page.width, page.height))
}

fn first_http_url(candidates: &[Option<String>]) -> Option<String> {
    candidates.iter().flatten().find_map(|candidate| {
        let parsed = Url::parse(candidate.trim()).ok()?;
        (matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some())
            .then(|| candidate.trim().to_string())
    })
}

fn dimension(page: &str, key: &str) -> Option<u32> {
    html::meta_content(page, key)?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
}

fn kind_from_image_url(url: &str) -> Option<&'static str> {
    let parsed = Url::parse(url.trim()).ok()?;
    let extension = parsed
        .path_segments()?
        .next_back()?
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())?;
    match extension.as_str() {
        "gif" => Some("image/gif"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
        <html><head>
        <meta property="og:title" content="Jonathan Drouin Halifax Mooseheads GIF - Jonathan drouin Drouin Halifax mooseheads - Discover &amp; Share GIFs">
        <meta property="og:image" content="https://media1.tenor.com/m/ziAUi92ZzJ0AAAAC/jonathan-drouin-drouin.gif">
        <meta property="og:image:type" content="image/gif">
        <meta property="og:image:width" content="426">
        <meta property="og:image:height" content="210">
        <meta property="og:video" content="https://media.tenor.com/ziAUi92ZzJ0AAAPo/jonathan-drouin-drouin.mp4">
        <meta property="og:video:secure_url" content="https://media.tenor.com/ziAUi92ZzJ0AAAPo/jonathan-drouin-drouin.mp4">
        <meta property="og:video:type" content="video/mp4">
        <meta property="og:video:width" content="426">
        <meta property="og:video:height" content="210">
        </head></html>
    "#;

    #[test]
    fn parses_the_video_and_the_gif() {
        let page = parse_page_html(PAGE);
        assert_eq!(
            page.title.as_deref(),
            Some("Jonathan Drouin Halifax Mooseheads")
        );
        assert_eq!(
            page.video_url.as_deref(),
            Some("https://media.tenor.com/ziAUi92ZzJ0AAAPo/jonathan-drouin-drouin.mp4")
        );
        assert_eq!(page.video_mime.as_deref(), Some("video/mp4"));
        assert_eq!(
            page.image_url.as_deref(),
            Some("https://media1.tenor.com/m/ziAUi92ZzJ0AAAAC/jonathan-drouin-drouin.gif")
        );
        assert_eq!(page.image_mime.as_deref(), Some("image/gif"));
        assert_eq!(page.width, Some(426));
        assert_eq!(page.height, Some(210));
        assert_eq!(page.media_urls().len(), 2);
    }

    #[test]
    fn falls_back_to_the_image_without_a_video() {
        let html = PAGE
            .lines()
            .filter(|line| !line.contains("og:video"))
            .collect::<Vec<_>>()
            .join("\n");
        let page = parse_page_html(&html);
        assert!(page.video_url.is_none());
        assert_eq!(
            page.media_urls(),
            vec!["https://media1.tenor.com/m/ziAUi92ZzJ0AAAAC/jonathan-drouin-drouin.gif"]
        );
        assert_eq!(page.width, Some(426));
    }

    #[test]
    fn ignores_a_page_without_media_and_unsafe_urls() {
        assert!(parse_page_html("<html><head></head></html>").is_empty());
        let page = parse_page_html(
            r#"<meta property="og:video" content="file:///C:/clip.mp4">
               <meta property="og:image" content="javascript:alert(1)">"#,
        );
        assert!(page.is_empty());
    }
}
