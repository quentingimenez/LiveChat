//! Résolution X/Twitter : carte oEmbed, variantes vidéo via l'API syndication,
//! repli sur le rendu vxtwitter.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use lc_domain::MediaKind;

use crate::config::MediaConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::html;
use crate::http;
use crate::source::{tweet_status_id, MediaSource, SourceKind};

use super::{encode_query_component, ResolvedMedia, TweetCard};

/// Endpoint de l'API syndication de Twitter.
pub const SYNDICATION_ENDPOINT: &str = "https://cdn.syndication.twimg.com/tweet-result";

/// Variante vidéo annoncée par Twitter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VideoVariant {
    /// URL du flux.
    pub url: String,
    /// Type MIME annoncé.
    pub content_type: String,
    /// Débit annoncé en bits/s.
    pub bitrate: Option<u64>,
}

/// Payload oEmbed d'un tweet.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TweetOEmbed {
    /// HTML de la carte.
    pub html: Option<String>,
    /// Nom d'affichage de l'auteur.
    pub author_name: Option<String>,
    /// URL du profil de l'auteur.
    pub author_url: Option<String>,
}

/// Construit l'URL oEmbed d'un tweet.
pub fn build_oembed_url(endpoint: &str, tweet_url: &str, language: &str) -> String {
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!(
        "{endpoint}{separator}url={}&omit_script=true&dnt=true&align=center&hide_thread=false&hide_media=false&lang={}",
        encode_query_component(tweet_url),
        encode_query_component(language)
    )
}

/// Construit l'URL de l'API syndication d'un tweet.
pub fn build_syndication_url(status_id: &str, language: &str) -> String {
    format!(
        "{SYNDICATION_ENDPOINT}?id={}&lang={}&token=1",
        encode_query_component(status_id),
        encode_query_component(language)
    )
}

/// Construit l'URL de repli vxtwitter à partir de l'URL du tweet.
pub fn build_fallback_url(base: &str, tweet_url: &str) -> String {
    let path = url::Url::parse(tweet_url)
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|_| tweet_url.to_string());
    format!("{}{path}", base.trim_end_matches('/'))
}

/// Nettoie le HTML de carte oEmbed (supprime scripts et iframes).
pub fn sanitize_card_html(payload_html: &str) -> String {
    html::sanitize_embed_html(payload_html).trim().to_string()
}

/// Résout un tweet : carte oEmbed + meilleure variante vidéo connue.
pub async fn resolve_tweet(
    client: &reqwest::Client,
    source: &MediaSource,
    config: &MediaConfig,
) -> MediaResult<ResolvedMedia> {
    let canonical_url = source.canonical_url.clone();
    let status_id = tweet_status_id(&canonical_url)
        .ok_or_else(|| MediaError::new(MediaErrorCode::InvalidMedia, "URL de tweet invalide"))?;
    let card = fetch_tweet_card(client, &canonical_url, config).await?;
    let (variants, dimensions) = fetch_variants(client, &status_id, &canonical_url, config).await;
    let selected = pick_variant(&variants).cloned();
    let (width, height) = match dimensions {
        Some((width, height)) => (Some(width), Some(height)),
        None => (None, None),
    };
    let mut resolved = ResolvedMedia {
        source_kind: SourceKind::Twitter,
        canonical_url,
        source_hash: source.source_hash.clone(),
        title: card.author_name.clone(),
        author: card.author_name.clone(),
        kind: Some(MediaKind::Video),
        tweet_card: Some(card),
        media_url: selected.as_ref().map(|variant| variant.url.clone()),
        mime: selected
            .as_ref()
            .map(|variant| variant.content_type.clone()),
        ..ResolvedMedia::from_source(source)
    };
    if width.is_some() || height.is_some() {
        resolved = resolved.with_dimensions(width, height);
    }
    Ok(resolved)
}

async fn fetch_tweet_card(
    client: &reqwest::Client,
    canonical_url: &str,
    config: &MediaConfig,
) -> MediaResult<TweetCard> {
    let url = build_oembed_url(
        &config.resolve.twitter_oembed_endpoint,
        canonical_url,
        &config.resolve.language,
    );
    let payload: TweetOEmbed = http::get_json(client, &url, config.resolve.oembed_timeout).await?;
    let html = payload
        .html
        .as_deref()
        .map(sanitize_card_html)
        .unwrap_or_default();
    if html.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::InvalidMedia,
            "carte oEmbed du tweet vide",
        ));
    }
    Ok(TweetCard {
        url: canonical_url.to_string(),
        html,
        author_name: payload.author_name,
        author_url: payload.author_url,
    })
}

async fn fetch_variants(
    client: &reqwest::Client,
    status_id: &str,
    canonical_url: &str,
    config: &MediaConfig,
) -> (Vec<VideoVariant>, Option<(u32, u32)>) {
    let syndication_url = build_syndication_url(status_id, &config.resolve.language);
    let mut variants = Vec::new();
    let mut dimensions = None;
    if let Ok(value) =
        http::get_json::<Value>(client, &syndication_url, config.resolve.oembed_timeout).await
    {
        variants = collect_variants(&value);
        dimensions = extract_dimensions(&value);
    }
    if variants.is_empty() {
        if let Ok(page) = http::get_text(
            client,
            &build_fallback_url(
                &config.resolve.twitter_video_resolver_base_url,
                canonical_url,
            ),
            config.resolve.oembed_timeout,
            &[],
        )
        .await
        {
            variants = variants_from_html(&page.body);
            if dimensions.is_none() {
                dimensions = dimensions_from_html(&page.body);
            }
        }
    }
    (variants, dimensions)
}

/// Collecte récursivement les variantes vidéo d'un payload syndication.
pub fn collect_variants(value: &Value) -> Vec<VideoVariant> {
    let mut variants = Vec::new();
    collect_variants_into(value, &mut variants, 0);
    dedupe_variants(variants)
}

fn collect_variants_into(value: &Value, variants: &mut Vec<VideoVariant>, depth: usize) {
    if depth > 6 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in ["video_info", "video"] {
                if let Some(info) = map.get(key) {
                    if let Some(array) = info.get("variants").and_then(Value::as_array) {
                        for item in array {
                            if let Some(variant) = parse_variant(item) {
                                variants.push(variant);
                            }
                        }
                    }
                }
            }
            for child in map.values() {
                collect_variants_into(child, variants, depth + 1);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_variants_into(child, variants, depth + 1);
            }
        }
        _ => {}
    }
}

fn parse_variant(item: &Value) -> Option<VideoVariant> {
    let url = item
        .get("url")
        .or_else(|| item.get("src"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if url.is_empty() {
        return None;
    }
    let content_type = item
        .get("content_type")
        .or_else(|| item.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| infer_content_type(&url))
        .to_string();
    let bitrate = item.get("bitrate").and_then(Value::as_u64);
    Some(VideoVariant {
        url,
        content_type,
        bitrate,
    })
}

/// Déduplique les variantes en gardant le meilleur débit par URL.
pub fn dedupe_variants(variants: Vec<VideoVariant>) -> Vec<VideoVariant> {
    let mut deduped: Vec<VideoVariant> = Vec::new();
    for variant in variants {
        match deduped.iter_mut().find(|known| known.url == variant.url) {
            Some(known) => {
                if variant.bitrate.unwrap_or_default() > known.bitrate.unwrap_or_default() {
                    *known = variant;
                }
            }
            None => deduped.push(variant),
        }
    }
    deduped
}

/// Choisit la meilleure variante : MP4 au débit maximal, sinon HLS, sinon la première.
pub fn pick_variant(variants: &[VideoVariant]) -> Option<&VideoVariant> {
    let mut mp4: Vec<&VideoVariant> = variants
        .iter()
        .filter(|variant| variant.content_type.contains("mp4"))
        .collect();
    mp4.sort_by_key(|variant| std::cmp::Reverse(variant.bitrate.unwrap_or_default()));
    if let Some(best) = mp4.first() {
        return Some(*best);
    }
    if let Some(hls) = variants
        .iter()
        .find(|variant| variant.content_type.contains("mpegurl") || variant.url.contains(".m3u8"))
    {
        return Some(hls);
    }
    variants.first()
}

/// Extrait les dimensions annoncées (`original_info` ou `aspect_ratio`).
pub fn extract_dimensions(value: &Value) -> Option<(u32, u32)> {
    let mut dimensions = None;
    find_dimensions(value, &mut dimensions, 0);
    dimensions
}

fn find_dimensions(value: &Value, out: &mut Option<(u32, u32)>, depth: usize) {
    if depth > 6 || out.is_some() {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(info) = map.get("original_info") {
                let width = info
                    .get("width")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as u32;
                let height = info
                    .get("height")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as u32;
                if width > 0 && height > 0 {
                    *out = Some((width, height));
                    return;
                }
            }
            if let Some(aspect) = map.get("aspect_ratio").and_then(Value::as_array) {
                if aspect.len() == 2 {
                    let width = aspect[0].as_f64().unwrap_or_default();
                    let height = aspect[1].as_f64().unwrap_or_default();
                    if width > 0.0 && height > 0.0 {
                        *out = Some((width.round() as u32, height.round() as u32));
                        return;
                    }
                }
            }
            for child in map.values() {
                find_dimensions(child, out, depth + 1);
            }
        }
        Value::Array(array) => {
            for child in array {
                find_dimensions(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn variants_from_html(page: &str) -> Vec<VideoVariant> {
    let mut variants = Vec::new();
    for key in [
        "og:video:secure_url",
        "og:video:url",
        "og:video",
        "twitter:player:stream",
    ] {
        if let Some(content) = html::meta_content(page, key) {
            let url = content.trim().to_string();
            if !url.is_empty() {
                variants.push(VideoVariant {
                    content_type: infer_content_type(&url).to_string(),
                    url,
                    bitrate: None,
                });
            }
        }
    }
    if variants.is_empty() {
        for url in html::urls_containing(page, "video.twimg.com") {
            if url.contains(".mp4") || url.contains(".m3u8") {
                variants.push(VideoVariant {
                    content_type: infer_content_type(&url).to_string(),
                    url,
                    bitrate: None,
                });
            }
        }
    }
    dedupe_variants(variants)
}

fn dimensions_from_html(page: &str) -> Option<(u32, u32)> {
    let width = html::meta_content(page, "og:video:width")
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0);
    let height = html::meta_content(page, "og:video:height")
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0);
    match (width, height) {
        (Some(width), Some(height)) => Some((width, height)),
        _ => None,
    }
}

fn infer_content_type(url: &str) -> &'static str {
    if url.contains(".m3u8") {
        "application/x-mpegURL"
    } else if url.contains(".m4a") {
        "audio/mp4"
    } else {
        "video/mp4"
    }
}

/// Récupère le HTML d'une carte de tweet (utilitaire public pour les tests).
pub async fn fetch_card_html(
    client: &reqwest::Client,
    tweet_url: &str,
    timeout: Duration,
) -> MediaResult<String> {
    let page = http::get_text(client, tweet_url, timeout, &[]).await?;
    Ok(page.body)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn builds_oembed_and_syndication_urls() {
        let oembed = build_oembed_url(
            "https://publish.twitter.com/oembed",
            "https://x.com/user/status/123",
            "fr",
        );
        assert!(oembed.starts_with("https://publish.twitter.com/oembed?url="));
        assert!(oembed.contains("omit_script=true"));
        assert!(oembed.contains("lang=fr"));
        let syndication = build_syndication_url("123", "fr");
        assert_eq!(
            syndication,
            "https://cdn.syndication.twimg.com/tweet-result?id=123&lang=fr&token=1"
        );
        assert_eq!(
            build_fallback_url("https://vxtwitter.com/", "https://x.com/user/status/123"),
            "https://vxtwitter.com/user/status/123"
        );
        assert_eq!(
            build_fallback_url("https://vxtwitter.com", "https://x.com/a/status/1?s=20"),
            "https://vxtwitter.com/a/status/1"
        );
    }

    #[test]
    fn collects_and_dedupes_variants() {
        let payload: Value = serde_json::from_str(
            r#"{
                "id_str": "123",
                "video": {"variants": [
                    {"src": "https://video.twimg.com/a.mp4", "type": "video/mp4", "bitrate": 100},
                    {"url": "https://video.twimg.com/a.mp4", "content_type": "video/mp4", "bitrate": 900},
                    {"url": "https://video.twimg.com/b.m3u8", "content_type": "application/x-mpegURL"}
                ]},
                "quoted_tweet": {"mediaDetails": [
                    {"video_info": {"variants": [{"url": "https://video.twimg.com/c.mp4", "content_type": "video/mp4", "bitrate": 500}]},
                     "original_info": {"width": 1080, "height": 1920}}
                ]}
            }"#,
        )
        .unwrap();
        let variants = collect_variants(&payload);
        assert_eq!(variants.len(), 3);
        let best = pick_variant(&variants).unwrap();
        assert_eq!(best.url, "https://video.twimg.com/a.mp4");
        assert_eq!(best.bitrate, Some(900));
        assert_eq!(extract_dimensions(&payload), Some((1080, 1920)));
    }

    #[test]
    fn picks_hls_when_no_mp4_and_first_otherwise() {
        let hls = vec![VideoVariant {
            url: "https://video.twimg.com/x.m3u8".to_string(),
            content_type: "application/x-mpegURL".to_string(),
            bitrate: None,
        }];
        assert_eq!(
            pick_variant(&hls).unwrap().url,
            "https://video.twimg.com/x.m3u8"
        );
        let none = vec![VideoVariant {
            url: "https://video.twimg.com/y.webm".to_string(),
            content_type: "video/webm".to_string(),
            bitrate: None,
        }];
        assert_eq!(
            pick_variant(&none).unwrap().url,
            "https://video.twimg.com/y.webm"
        );
        assert!(pick_variant(&[]).is_none());
    }

    #[test]
    fn extracts_variants_from_fallback_html() {
        let page = r#"<html><head>
            <meta property="og:video:secure_url" content="https://video.twimg.com/ext/a.mp4?tag=1">
            <meta property="og:video:width" content="1280">
            <meta property="og:video:height" content="720">
        </head></html>"#;
        let variants = variants_from_html(page);
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].content_type, "video/mp4");
        assert_eq!(dimensions_from_html(page), Some((1280, 720)));
    }

    #[test]
    fn sanitizes_card_html() {
        let cleaned = sanitize_card_html(
            r#"<blockquote>Bonjour<script>alert(1)</script><iframe src="x"></iframe></blockquote>"#,
        );
        assert!(cleaned.contains("Bonjour"));
        assert!(!cleaned.contains("script"));
        assert!(!cleaned.contains("iframe"));
    }
}
