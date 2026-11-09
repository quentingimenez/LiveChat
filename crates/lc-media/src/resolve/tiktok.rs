//! Résolution TikTok : oEmbed puis extraction de la page HTML (carrousels
//! photo/vidéo). La logique reprend l'ancien serveur : données
//! `__UNIVERSAL_DATA_FOR_REHYDRATION__`, repli `__FRONTITY_CONNECT_STATE__`,
//! page embed, puis repérage des images `~tplv-photomode-image` dans le HTML
//! brut pour les carrousels.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use lc_domain::MediaKind;

use crate::config::{MediaConfig, TIKTOK_PAGE_USER_AGENT};
use crate::error::MediaResult;
use crate::html;
use crate::http;
use crate::source::{kind_from_url, MediaSource, SourceKind};

use super::{encode_query_component, ResolvedMedia};

/// Endpoint oEmbed TikTok.
pub const TIKTOK_OEMBED_ENDPOINT: &str = "https://www.tiktok.com/oembed";
/// Endpoint des pages embed TikTok.
pub const TIKTOK_EMBED_ENDPOINT: &str = "https://www.tiktok.com/embed/v2";
/// Nombre maximal d'images d'un carrousel legacy.
pub const MAX_CAROUSEL_IMAGES: usize = 35;
/// Nombre maximal de candidats retenus par slide.
pub const MAX_CANDIDATES_PER_SLIDE: usize = 3;
/// Nombre maximal de pistes audio candidates.
pub const MAX_AUDIO_CANDIDATES: usize = 5;
/// Marqueurs textuels d'un contenu privé/authentifié.
pub const AUTH_REQUIRED_MARKERS: &[&str] = &[
    "log in to tiktok",
    "connecte-toi à tiktok",
    "login required",
    "private video",
    "video currently unavailable",
    "this post is private",
    "this video is private",
];

static PHOTO_URL_REGEX: OnceLock<Option<Regex>> = OnceLock::new();

/// Candidat média (URL + type attendu).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaCandidate {
    /// URL directe du média.
    pub url: String,
    /// Type de média attendu.
    pub kind: MediaKind,
    /// Type MIME attendu, si connu.
    pub mime: Option<String>,
}

/// Données extraites d'une page TikTok.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TikTokPage {
    /// Identifiant de la publication, si connu.
    pub item_id: Option<String>,
    /// URL canonique de la publication, si connue.
    pub canonical_url: Option<String>,
    /// Description de la publication.
    pub title: Option<String>,
    /// Auteur (uniqueId ou pseudo).
    pub author: Option<String>,
    /// Candidats vidéo directs.
    pub video_candidates: Vec<MediaCandidate>,
    /// Slides photo : chaque slide est une liste de candidats.
    pub photo_slides: Vec<Vec<MediaCandidate>>,
    /// Candidats audio (musique de fond).
    pub audio_candidates: Vec<MediaCandidate>,
    /// La page exige une authentification ou est privée.
    pub auth_required: bool,
}

impl TikTokPage {
    /// `true` si la page ne contient aucun média exploitable.
    pub fn is_empty(&self) -> bool {
        self.video_candidates.is_empty()
            && self.photo_slides.is_empty()
            && self.audio_candidates.is_empty()
    }

    /// `true` si la publication doit devenir une vidéo de carrousel.
    pub fn should_build_carousel(&self) -> bool {
        self.photo_slides.len() > 1
            || (self.photo_slides.len() == 1 && !self.audio_candidates.is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct TikTokOEmbed {
    title: Option<String>,
    author_name: Option<String>,
}

/// Construit l'URL oEmbed d'une publication TikTok.
pub fn build_oembed_url(url: &str) -> String {
    format!(
        "{TIKTOK_OEMBED_ENDPOINT}?url={}",
        encode_query_component(url)
    )
}

/// Construit l'URL de la page embed d'une publication.
pub fn build_embed_url(item_id: &str) -> String {
    format!("{TIKTOK_EMBED_ENDPOINT}/{item_id}")
}

/// En-têtes HTTP utilisés pour les requêtes TikTok.
pub fn tiktok_headers(config: &MediaConfig) -> Vec<(String, String)> {
    let mut headers = vec![
        ("User-Agent".to_string(), TIKTOK_PAGE_USER_AGENT.to_string()),
        (
            "Accept-Language".to_string(),
            "fr-FR,fr;q=0.9,en;q=0.8".to_string(),
        ),
    ];
    if let Some(cookie) = &config.tools.tiktok_cookie {
        if !cookie.trim().is_empty() {
            headers.push(("Cookie".to_string(), cookie.trim().to_string()));
        }
    }
    headers
}

/// Paires d'en-têtes empruntées pour les helpers HTTP.
pub fn header_pairs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

/// Résout une publication TikTok.
pub async fn resolve_tiktok(
    client: &reqwest::Client,
    source: &MediaSource,
    config: &MediaConfig,
) -> MediaResult<ResolvedMedia> {
    let canonical_url = source.canonical_url.clone();
    let oembed = fetch_oembed(client, &canonical_url, config).await.ok();
    let page_result = fetch_tiktok_page(client, &canonical_url, config).await;
    match page_result {
        Ok(page) => {
            let canonical_url = page
                .canonical_url
                .clone()
                .unwrap_or_else(|| canonical_url.clone());
            let has_video = !page.video_candidates.is_empty();
            let kind = if has_video || page.should_build_carousel() {
                MediaKind::Video
            } else if !page.photo_slides.is_empty() {
                MediaKind::Image
            } else {
                kind_from_url(&canonical_url).unwrap_or(MediaKind::Video)
            };
            let selected = page.video_candidates.first();
            Ok(ResolvedMedia {
                source_kind: SourceKind::TikTok,
                source_hash: source.source_hash.clone(),
                title: page
                    .title
                    .or_else(|| oembed.as_ref().and_then(|oembed| oembed.title.clone())),
                author: page.author.or_else(|| {
                    oembed
                        .as_ref()
                        .and_then(|oembed| oembed.author_name.clone())
                }),
                kind: Some(kind),
                media_url: selected.map(|candidate| candidate.url.clone()),
                mime: selected.and_then(|candidate| candidate.mime.clone()),
                canonical_url,
                ..ResolvedMedia::from_source(source)
            })
        }
        Err(page_error) => match oembed {
            Some(oembed) => {
                let kind = kind_from_url(&canonical_url).unwrap_or(MediaKind::Video);
                Ok(ResolvedMedia {
                    title: oembed.title,
                    author: oembed.author_name,
                    kind: Some(kind),
                    canonical_url,
                    source_hash: source.source_hash.clone(),
                    source_kind: SourceKind::TikTok,
                    ..ResolvedMedia::from_source(source)
                })
            }
            None => Err(page_error),
        },
    }
}

async fn fetch_oembed(
    client: &reqwest::Client,
    canonical_url: &str,
    config: &MediaConfig,
) -> MediaResult<TikTokOEmbed> {
    http::get_json(
        client,
        &build_oembed_url(canonical_url),
        config.resolve.oembed_timeout,
    )
    .await
}

/// Récupère et analyse la page TikTok (avec repli page embed).
pub async fn fetch_tiktok_page(
    client: &reqwest::Client,
    canonical_url: &str,
    config: &MediaConfig,
) -> MediaResult<TikTokPage> {
    let headers = tiktok_headers(config);
    let page = http::get_text(
        client,
        canonical_url,
        config.download.timeout,
        &header_pairs(&headers),
    )
    .await?;
    let mut parsed = parse_page_html(&page.body, &page.final_url);
    let needs_embed = parsed.auth_required || parsed.is_empty();
    if needs_embed {
        if let Some(item_id) = parsed.item_id.clone() {
            if let Ok(embed) = http::get_text(
                client,
                &build_embed_url(&item_id),
                config.resolve.oembed_timeout,
                &header_pairs(&headers),
            )
            .await
            {
                let embed_page = parse_page_html(&embed.body, &embed.final_url);
                parsed.merge(embed_page);
            }
        }
    }
    Ok(parsed)
}

impl TikTokPage {
    fn merge(&mut self, other: TikTokPage) {
        if self.canonical_url.is_none() {
            self.canonical_url = other.canonical_url;
        }
        if self.item_id.is_none() {
            self.item_id = other.item_id;
        }
        if self.title.is_none() {
            self.title = other.title;
        }
        if self.author.is_none() {
            self.author = other.author;
        }
        self.auth_required = self.auth_required && other.auth_required;
        push_candidates(&mut self.video_candidates, other.video_candidates);
        push_candidates(&mut self.audio_candidates, other.audio_candidates);
        for slide in other.photo_slides {
            if self.photo_slides.len() < MAX_CAROUSEL_IMAGES && !slide.is_empty() {
                self.photo_slides.push(slide);
            }
        }
        self.photo_slides.retain(|slide| !slide.is_empty());
        self.video_candidates.truncate(8);
        self.audio_candidates.truncate(MAX_AUDIO_CANDIDATES);
    }
}

fn push_candidates(target: &mut Vec<MediaCandidate>, candidates: Vec<MediaCandidate>) {
    for candidate in candidates {
        if !target.iter().any(|known| known.url == candidate.url) {
            target.push(candidate);
        }
    }
}

/// Analyse le HTML d'une page TikTok.
pub fn parse_page_html(page_html: &str, page_url: &str) -> TikTokPage {
    let mut page = TikTokPage {
        item_id: extract_item_id(page_url),
        ..TikTokPage::default()
    };
    let mut payloads = Vec::new();
    for identifier in [
        "__UNIVERSAL_DATA_FOR_REHYDRATION__",
        "__FRONTITY_CONNECT_STATE__",
    ] {
        if let Some(value) = html::script_json(page_html, identifier) {
            payloads.push(value);
        }
    }
    for payload in &payloads {
        if page.canonical_url.is_none() {
            page.canonical_url = find_string(payload, "canonical");
        }
        if page.title.is_none() {
            page.title = find_string(payload, "desc");
        }
        if page.author.is_none() {
            page.author =
                find_string(payload, "uniqueId").or_else(|| find_string(payload, "nickname"));
        }
        if find_bool(payload, "privateItem")
            || find_bool(payload, "secret")
            || find_bool(payload, "forFriend")
        {
            page.auth_required = true;
        }
        collect_video_candidates(payload, &mut page.video_candidates);
        collect_audio_candidates(payload, &mut page.audio_candidates);
        collect_photo_slides(payload, &mut page.photo_slides, 0);
    }
    if page.canonical_url.is_none() {
        page.canonical_url = html::meta_content(page_html, "og:url");
    }
    if page.title.is_none() {
        page.title = html::meta_content(page_html, "og:title");
    }
    if page.item_id.is_none() {
        page.item_id = page
            .canonical_url
            .as_deref()
            .and_then(extract_item_id)
            .or_else(|| extract_item_id(page_url));
    }
    if page.photo_slides.is_empty() {
        page.photo_slides = photo_slides_from_raw_html(page_html);
    }
    let lowered = page_html.to_ascii_lowercase();
    if AUTH_REQUIRED_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        page.auth_required = true;
    }
    page.video_candidates.truncate(8);
    page.audio_candidates.truncate(MAX_AUDIO_CANDIDATES);
    page.photo_slides.retain(|slide| !slide.is_empty());
    page.photo_slides.truncate(MAX_CAROUSEL_IMAGES);
    page
}

/// Extrait l'identifiant d'une publication depuis une URL TikTok.
pub fn extract_item_id(raw: &str) -> Option<String> {
    let path = url::Url::parse(raw.trim())
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|_| raw.trim().to_string());
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() >= 2 {
        let marker = segments[segments.len() - 2];
        let candidate = segments[segments.len() - 1];
        if matches!(marker, "video" | "photo") && is_digits(candidate) {
            return Some(candidate.to_string());
        }
    }
    find_long_digit_run(raw)
}

fn find_long_digit_run(raw: &str) -> Option<String> {
    let mut current = String::new();
    for character in raw.chars() {
        if character.is_ascii_digit() {
            current.push(character);
        } else {
            if (15..=22).contains(&current.len()) {
                return Some(std::mem::take(&mut current));
            }
            current.clear();
        }
    }
    if (15..=22).contains(&current.len()) {
        Some(current)
    } else {
        None
    }
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn collect_video_candidates(value: &Value, out: &mut Vec<MediaCandidate>) {
    let mut urls = Vec::new();
    collect_video_urls(value, &mut urls, 0);
    for url in urls {
        if url.starts_with("http")
            && !out.iter().any(|candidate| candidate.url == url)
            && out.len() < 8
        {
            out.push(MediaCandidate {
                url,
                kind: MediaKind::Video,
                mime: Some("video/mp4".to_string()),
            });
        }
    }
}

fn collect_video_urls(value: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 10 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in [
                "downloadAddr",
                "playAddr",
                "download_addr",
                "play_addr",
                "DownloadAddrStruct",
                "playAddrStruct",
            ] {
                if let Some(found) = map.get(key) {
                    urls_from_value(found, out);
                }
            }
            for key in ["bitrateInfo", "bitrate_info"] {
                if let Some(found) = map.get(key) {
                    collect_play_addrs(found, out, 0);
                }
            }
            for child in map.values() {
                collect_video_urls(child, out, depth + 1);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_video_urls(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn collect_play_addrs(value: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 6 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in ["PlayAddr", "playAddr", "urlList", "UrlList", "url_list"] {
                if let Some(found) = map.get(key) {
                    urls_from_value(found, out);
                }
            }
            for child in map.values() {
                collect_play_addrs(child, out, depth + 1);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_play_addrs(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn collect_audio_candidates(value: &Value, out: &mut Vec<MediaCandidate>) {
    let mut urls = Vec::new();
    collect_audio_urls(value, &mut urls, 0);
    for url in urls {
        if url.starts_with("http")
            && !out.iter().any(|candidate| candidate.url == url)
            && out.len() < MAX_AUDIO_CANDIDATES
        {
            out.push(MediaCandidate {
                url,
                kind: MediaKind::Audio,
                mime: Some("audio/mpeg".to_string()),
            });
        }
    }
}

fn collect_audio_urls(value: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 10 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in ["playUrl", "play_url", "musicUrl"] {
                if let Some(found) = map.get(key) {
                    urls_from_value(found, out);
                }
            }
            for child in map.values() {
                collect_audio_urls(child, out, depth + 1);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_audio_urls(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn collect_photo_slides(value: &Value, out: &mut Vec<Vec<MediaCandidate>>, depth: usize) {
    if depth > 10 {
        return;
    }
    match value {
        Value::Object(map) => {
            for key in ["displayImages", "display_images", "images", "imageList"] {
                if let Some(Value::Array(items)) = map.get(key) {
                    // Chaque entrée de la liste est une image du carrousel :
                    // elle forme sa propre slide avec ses URL candidates.
                    for item in items {
                        let mut urls = Vec::new();
                        urls_from_value(item, &mut urls);
                        let mut slide = Vec::new();
                        for url in urls {
                            if !url.starts_with("http") {
                                continue;
                            }
                            if slide
                                .iter()
                                .any(|candidate: &MediaCandidate| candidate.url == url)
                            {
                                continue;
                            }
                            slide.push(MediaCandidate {
                                mime: Some(infer_image_mime(&url).to_string()),
                                kind: MediaKind::Image,
                                url,
                            });
                        }
                        slide.truncate(MAX_CANDIDATES_PER_SLIDE);
                        if !slide.is_empty() && out.len() < MAX_CAROUSEL_IMAGES {
                            out.push(slide);
                        }
                    }
                }
            }
            for child in map.values() {
                collect_photo_slides(child, out, depth + 1);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_photo_slides(child, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn urls_from_value(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(url) => {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
        Value::Array(array) => {
            for child in array {
                urls_from_value(child, out);
            }
        }
        Value::Object(map) => {
            for key in [
                "urlList",
                "url_list",
                "urls",
                "src",
                "url",
                "PlayAddr",
                "playAddr",
                "displayImage",
                "imageURL",
            ] {
                if let Some(child) = map.get(key) {
                    urls_from_value(child, out);
                }
            }
        }
        _ => {}
    }
}

/// Extrait les slides photo des URL brutes `~tplv-photomode-image` du HTML.
pub fn photo_slides_from_raw_html(page_html: &str) -> Vec<Vec<MediaCandidate>> {
    let Some(regex) = photo_url_regex() else {
        return Vec::new();
    };
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for matched in regex.find_iter(page_html) {
        let decoded = decode_raw_url(matched.as_str());
        let key = decorative_key(&decoded);
        match groups.iter_mut().find(|(known, _)| *known == key) {
            Some((_, urls)) => {
                if !urls.iter().any(|known| known == &decoded) {
                    urls.push(decoded);
                }
            }
            None => groups.push((key, vec![decoded])),
        }
    }
    groups
        .into_iter()
        .map(|(_, urls)| {
            urls.into_iter()
                .take(MAX_CANDIDATES_PER_SLIDE)
                .map(|url| MediaCandidate {
                    mime: Some(infer_image_mime(&url).to_string()),
                    kind: MediaKind::Image,
                    url,
                })
                .collect()
        })
        .filter(|slide: &Vec<MediaCandidate>| !slide.is_empty())
        .collect()
}

fn photo_url_regex() -> Option<&'static Regex> {
    PHOTO_URL_REGEX
        .get_or_init(|| {
            Regex::new(r#"https?:\\?/\\?/[^"'\s<>]+?~tplv-photomode-image\.(?:jpeg|jpg|png)(?:\?[^"'\s<>]*)?"#)
                .ok()
        })
        .as_ref()
}

fn decode_raw_url(raw: &str) -> String {
    let mut decoded = html::decode_json_escapes(raw);
    decoded = decoded.replace("\\/", "/");
    decoded = html::decode_html_entities(&decoded);
    decoded.replace("\\\\", "\\")
}

fn decorative_key(url: &str) -> String {
    let file_name = url
        .split('?')
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default();
    file_name.split('~').next().unwrap_or(file_name).to_string()
}

fn infer_image_mime(url: &str) -> &'static str {
    let lowered = url.to_ascii_lowercase();
    if lowered.contains(".png") || lowered.contains("_png") {
        "image/png"
    } else if lowered.contains(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    find_key_depth(value, key, 0)
}

fn find_key_depth<'a>(value: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 12 {
        return None;
    }
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            for child in map.values() {
                if let Some(found) = find_key_depth(child, key, depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(array) => {
            for child in array {
                if let Some(found) = find_key_depth(child, key, depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_string(value: &Value, key: &str) -> Option<String> {
    find_key(value, key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn find_bool(value: &Value, key: &str) -> bool {
    find_key(value, key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn universal_page(payload: &str) -> String {
        format!(
            r#"<html><head><script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{payload}</script></head><body></body></html>"#
        )
    }

    const VIDEO_PAYLOAD: &str = r#"{
        "__DEFAULT_SCOPE__": {
            "webapp.video-detail": {
                "itemInfo": {
                    "itemStruct": {
                        "id": "7311111111111111111",
                        "desc": "Un chat rigolo",
                        "createTime": "1700000000",
                        "author": {"uniqueId": "chat_officiel", "nickname": "Chat"},
                        "video": {
                            "downloadAddr": "https://v16.test/video.mp4?tag=1",
                            "playAddr": "https://v16.test/play.mp4",
                            "bitrateInfo": [
                                {"PlayAddr": {"urlList": ["https://v16.test/hd.mp4"]}}
                            ]
                        },
                        "music": {"playUrl": "https://sf.test/music.mp3"}
                    }
                }
            }
        }
    }"#;

    #[test]
    fn parses_video_page_payload() {
        let page = parse_page_html(
            &universal_page(VIDEO_PAYLOAD),
            "https://www.tiktok.com/@chat_officiel/video/7311111111111111111",
        );
        assert_eq!(page.title.as_deref(), Some("Un chat rigolo"));
        assert_eq!(page.author.as_deref(), Some("chat_officiel"));
        assert_eq!(page.item_id.as_deref(), Some("7311111111111111111"));
        assert_eq!(page.video_candidates.len(), 3);
        assert!(page
            .video_candidates
            .iter()
            .any(|candidate| candidate.url == "https://v16.test/video.mp4?tag=1"));
        assert_eq!(page.audio_candidates.len(), 1);
        assert!(!page.auth_required);
        assert!(!page.should_build_carousel());
    }

    #[test]
    fn parses_photo_slides() {
        let payload = r#"{
            "itemStruct": {
                "id": "7312222222222222222",
                "desc": "Carrousel",
                "author": {"uniqueId": "photos"},
                "imagePost": {"displayImages": [
                    {"urlList": ["https://p16.test/a~tplv-photomode-image.jpeg?x=1"]},
                    {"urlList": ["https://p16.test/b~tplv-photomode-image.jpeg"]},
                    {"urlList": ["https://p16.test/c~tplv-photomode-image.png"]}
                ]},
                "music": {"playUrl": "https://sf.test/m.mp3"}
            }
        }"#;
        let page = parse_page_html(
            &universal_page(payload),
            "https://www.tiktok.com/@photos/photo/7312222222222222222",
        );
        assert_eq!(page.photo_slides.len(), 3);
        assert_eq!(page.photo_slides[0].len(), 1);
        assert_eq!(page.photo_slides[2][0].mime.as_deref(), Some("image/png"));
        assert!(page.should_build_carousel());
    }

    #[test]
    fn falls_back_to_raw_html_photo_urls() {
        let html_page = r#"<html><body><script>
            var a = "https:\/\/p16.test\/x1~tplv-photomode-image.jpeg?a=1\u0026b=2";
            var b = "https://p16.test/x2~tplv-photomode-image.jpeg?c=3";
            var c = "https://p16.test/y1~tplv-photomode-image.jpeg";
        </script></body></html>"#;
        let slides = photo_slides_from_raw_html(html_page);
        assert_eq!(slides.len(), 3);
        let all: Vec<&str> = slides
            .iter()
            .flat_map(|slide| slide.iter().map(|candidate| candidate.url.as_str()))
            .collect();
        assert!(all
            .iter()
            .any(|url| url.contains("x1~tplv") && url.contains("&b=2")));
        assert!(all
            .iter()
            .any(|url| url == &"https://p16.test/x2~tplv-photomode-image.jpeg?c=3"));
        assert!(photo_slides_from_raw_html("<html></html>").is_empty());
    }

    #[test]
    fn detects_auth_required_pages() {
        let page = parse_page_html(
            "<html><body>Log in to TikTok to continue</body></html>",
            "https://www.tiktok.com/@x/video/1",
        );
        assert!(page.auth_required);
        let private_page = parse_page_html(
            &universal_page(r#"{"itemStruct":{"privateItem":true,"id":"1"}}"#),
            "https://www.tiktok.com/@x/video/1",
        );
        assert!(private_page.auth_required);
    }

    #[test]
    fn extracts_item_ids() {
        assert_eq!(
            extract_item_id("https://www.tiktok.com/@a/video/7311111111111111111"),
            Some("7311111111111111111".to_string())
        );
        assert_eq!(
            extract_item_id("https://www.tiktok.com/@a/photo/7311111111111111111/"),
            Some("7311111111111111111".to_string())
        );
        assert_eq!(
            extract_item_id("https://www.tiktok.com/embed/v2/7311111111111111111"),
            Some("7311111111111111111".to_string())
        );
        assert_eq!(extract_item_id("https://www.tiktok.com/@a"), None);
        assert_eq!(extract_item_id("pas une url"), None);
    }

    #[test]
    fn builds_urls_and_headers() {
        assert_eq!(
            build_embed_url("123"),
            "https://www.tiktok.com/embed/v2/123"
        );
        let oembed = build_oembed_url("https://www.tiktok.com/@a/video/1");
        assert!(oembed.starts_with("https://www.tiktok.com/oembed?url="));
        assert!(oembed.contains("%40a"));
    }

    #[test]
    fn empty_page_merges_embed_data() {
        let mut page = TikTokPage::default();
        let mut embedded = parse_page_html(
            &universal_page(VIDEO_PAYLOAD),
            "https://www.tiktok.com/embed/v2/7311111111111111111",
        );
        embedded.auth_required = false;
        page.merge(embedded);
        assert_eq!(page.video_candidates.len(), 3);
        assert_eq!(page.item_id.as_deref(), Some("7311111111111111111"));
    }
}
