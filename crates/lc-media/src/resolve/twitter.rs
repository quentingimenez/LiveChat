//! Résolution X/Twitter : carte oEmbed, variantes vidéo et photos via l'API
//! syndication, repli sur le rendu vxtwitter.

use serde::Deserialize;
use serde_json::Value;

use lc_domain::MediaKind;

use crate::config::MediaConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::html;
use crate::http;
use crate::source::{tweet_status_id, MediaSource, SourceKind};
use crate::tweet_card::{QuoteImageFocus, QuotedTweetContent, QuotedTweetImage, TweetCardContent};

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

/// Résout un tweet : carte oEmbed + média connu (vidéo, photo ou carte).
pub async fn resolve_tweet(
    source: &MediaSource,
    config: &MediaConfig,
) -> MediaResult<ResolvedMedia> {
    let canonical_url = source.canonical_url.clone();
    let status_id = tweet_status_id(&canonical_url)
        .ok_or_else(|| MediaError::new(MediaErrorCode::InvalidMedia, "URL de tweet invalide"))?;
    let card = fetch_tweet_card(&canonical_url, config).await?;
    let media = fetch_tweet_media(&status_id, &canonical_url, config).await;
    let selected = pick_variant(&media.variants).cloned();
    let (width, height) = match media.dimensions {
        Some((width, height)) => (Some(width), Some(height)),
        None => (None, None),
    };
    let has_video = selected.is_some();
    let kind = tweet_media_kind(has_video, media.photos.len());
    // Une photo seule se télécharge en taille originale ; 2 à 4 photos
    // deviennent un carrousel vidéo assemblé au téléchargement (media_url
    // reste vide), et une carte est rendue quand il n'y a aucun média.
    let media_url = selected
        .as_ref()
        .map(|variant| variant.url.clone())
        .or_else(|| {
            (kind == MediaKind::Image && media.photos.len() == 1)
                .then(|| photo_original_url(&media.photos[0]))
        });
    let mime = selected
        .as_ref()
        .map(|variant| variant.content_type.clone())
        .or_else(|| (kind == MediaKind::Image).then(|| "image/jpeg".to_string()))
        .or_else(|| (kind == MediaKind::Video).then(|| "video/mp4".to_string()));
    let thumbnail_url = media
        .poster
        .clone()
        .or_else(|| media.photos.first().cloned());
    let mut resolved = ResolvedMedia {
        source_kind: SourceKind::Twitter,
        canonical_url,
        source_hash: source.source_hash.clone(),
        // Un tweet n'a pas de titre : le texte de la publication (connu quand
        // l'API de syndication a répondu) en tient lieu ; l'auteur reste
        // disponible pour le repli « Post de <auteur> ».
        title: media.text,
        author: card.author_name.clone(),
        kind: Some(kind),
        thumbnail_url,
        tweet_card: Some(card),
        media_url,
        mime,
        ..ResolvedMedia::from_source(source)
    };
    if width.is_some() || height.is_some() {
        resolved = resolved.with_dimensions(width, height);
    }
    Ok(resolved)
}

/// Nombre maximal de photos conservées pour un carrousel de tweet.
pub const MAX_TWEET_PHOTOS: usize = 4;

/// Nombre minimal de photos pour assembler un carrousel vidéo.
pub const CAROUSEL_MIN_PHOTOS: usize = 2;

/// Type de média d'un tweet selon ce que l'API de syndication a annoncé.
///
/// Une vidéo reste une vidéo ; une photo seule est une image ; 2 à 4 photos
/// deviennent un carrousel vidéo ; sans aucun média le tweet devient une carte
/// rendue en image.
pub fn tweet_media_kind(has_video: bool, photo_count: usize) -> MediaKind {
    if has_video || photo_count >= CAROUSEL_MIN_PHOTOS {
        MediaKind::Video
    } else {
        MediaKind::Image
    }
}

/// URL d'une photo de tweet en taille originale (`?format=<ext>&name=orig`).
///
/// Les URL `pbs.twimg.com` servent une miniature par défaut : la forme
/// `name=orig` demande l'original. Toute autre URL est renvoyée telle quelle.
pub fn photo_original_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url.trim()) else {
        return url.to_string();
    };
    if parsed.host_str() != Some("pbs.twimg.com") {
        return url.to_string();
    }
    let extension = parsed
        .path()
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .filter(|extension| matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif"))
        .map(|extension| {
            if extension == "jpeg" {
                "jpg".to_string()
            } else {
                extension
            }
        })
        .unwrap_or_else(|| "jpg".to_string());
    let query = format!("format={extension}&name=orig");
    parsed.set_query(Some(&query));
    parsed.to_string()
}

async fn fetch_tweet_card(canonical_url: &str, config: &MediaConfig) -> MediaResult<TweetCard> {
    let url = build_oembed_url(
        &config.resolve.twitter_oembed_endpoint,
        canonical_url,
        &config.resolve.language,
    );
    let payload: TweetOEmbed =
        http::get_json(&config.download, &url, config.resolve.oembed_timeout).await?;
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

/// Ce que la résolution d'un tweet retire du réseau (API syndication puis
/// repli vxtwitter).
#[derive(Clone, Debug, Default)]
struct TweetMedia {
    /// Variantes vidéo annoncées.
    variants: Vec<VideoVariant>,
    /// Dimensions annoncées.
    dimensions: Option<(u32, u32)>,
    /// Poster de la vidéo.
    poster: Option<String>,
    /// Texte du tweet, si l'API de syndication l'a donné.
    text: Option<String>,
    /// Photos du tweet (une seule ou carrousel).
    photos: Vec<String>,
}

/// Récupère le payload de l'API syndication d'un tweet (aucune erreur levée).
pub(crate) async fn fetch_syndication_payload(
    config: &MediaConfig,
    status_id: &str,
) -> Option<Value> {
    let url = build_syndication_url(status_id, &config.resolve.language);
    http::get_json::<Value>(&config.download, &url, config.resolve.oembed_timeout)
        .await
        .ok()
}

async fn fetch_tweet_media(
    status_id: &str,
    canonical_url: &str,
    config: &MediaConfig,
) -> TweetMedia {
    let mut media = TweetMedia::default();
    if let Some(value) = fetch_syndication_payload(config, status_id).await {
        media.variants = collect_variants(&value);
        media.dimensions = extract_dimensions(&value);
        media.poster = extract_poster(&value);
        media.text = extract_tweet_text(&value);
        media.photos = collect_photos(&value);
    }
    if media.variants.is_empty() {
        if let Ok(page) = http::get_text(
            &config.download,
            &build_fallback_url(
                &config.resolve.twitter_video_resolver_base_url,
                canonical_url,
            ),
            config.resolve.oembed_timeout,
            &[],
        )
        .await
        {
            media.variants = variants_from_html(&page.body);
            if media.dimensions.is_none() {
                media.dimensions = dimensions_from_html(&page.body);
            }
        }
    }
    media
}

/// Texte d'un tweet dans un payload de syndication (servira de titre).
pub fn extract_tweet_text(value: &Value) -> Option<String> {
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Texte complet d'un tweet : `note_tweet.text` s'il est présent (tweets
/// longs), sinon `text`.
pub fn extract_full_tweet_text(value: &Value) -> Option<String> {
    let note = value
        .get("note_tweet")
        .and_then(|note| note.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    if note.is_some() {
        return note;
    }
    extract_tweet_text(value).map(|text| trim_trailing_media_link(value, text))
}

/// Coupe le texte au `display_text_range` du payload.
///
/// Twitter laisse dans `text` le lien `t.co` qui pointe vers le média joint ;
/// l'interface ne l'affiche pas, et une carte ne doit pas montrer cette URL.
fn trim_trailing_media_link(value: &Value, text: String) -> String {
    let Some(end) = value
        .get("display_text_range")
        .and_then(Value::as_array)
        .and_then(|range| range.get(1))
        .and_then(Value::as_u64)
    else {
        return text;
    };
    let end = usize::try_from(end).unwrap_or(usize::MAX);
    if end == 0 || text.chars().count() <= end {
        return text;
    }
    text.chars()
        .take(end)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Nom d'affichage de l'auteur d'un tweet.
pub fn extract_author_name(value: &Value) -> Option<String> {
    non_empty_str(value.get("user").and_then(|user| user.get("name")))
}

/// Identifiant (`@handle`) de l'auteur d'un tweet.
pub fn extract_author_handle(value: &Value) -> Option<String> {
    non_empty_str(value.get("user").and_then(|user| user.get("screen_name")))
}

/// Date de publication ISO 8601 annoncée par l'API de syndication.
pub fn extract_created_at(value: &Value) -> Option<String> {
    non_empty_str(value.get("created_at"))
}

/// URL de l'avatar de l'auteur, en 400×400 quand Twitter utilise `_normal.`.
pub fn extract_avatar_url(value: &Value) -> Option<String> {
    let url = value
        .get("user")
        .and_then(|user| user.get("profile_image_url_https"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| url.starts_with("https://"))?;
    Some(url.replace("_normal.", "_400x400."))
}

/// Collecte les photos d'un tweet : `mediaDetails[]` de type `photo`
/// (`media_url_https`), sinon `photos[].url`. Dédupliquées, 4 au maximum.
pub fn collect_photos(value: &Value) -> Vec<String> {
    let mut photos: Vec<String> = Vec::new();
    if let Some(details) = value.get("mediaDetails").and_then(Value::as_array) {
        for media in details {
            if media.get("type").and_then(Value::as_str) == Some("photo") {
                push_photo(&mut photos, media.get("media_url_https"));
            }
        }
    }
    if photos.is_empty() {
        if let Some(list) = value.get("photos").and_then(Value::as_array) {
            for photo in list {
                push_photo(&mut photos, photo.get("url"));
            }
        }
    }
    photos
}

/// Ajoute une photo HTTPS dédupliquée, sans dépasser [`MAX_TWEET_PHOTOS`].
fn push_photo(photos: &mut Vec<String>, raw: Option<&Value>) {
    let Some(url) = https_url(raw) else {
        return;
    };
    if photos.len() < MAX_TWEET_PHOTOS && !photos.contains(&url) {
        photos.push(url);
    }
}

/// Contenu textuel d'une carte tweet construit depuis un payload syndication.
pub fn tweet_card_content(value: &Value) -> TweetCardContent {
    TweetCardContent {
        author_name: extract_author_name(value).unwrap_or_default(),
        author_handle: extract_author_handle(value).unwrap_or_default(),
        text: extract_full_tweet_text(value).unwrap_or_default(),
        created_at: extract_created_at(value),
        avatar: None,
        quoted: quoted_tweet_content(value),
    }
}

/// Contenu du tweet cité (`quoted_tweet`) d'un payload syndication.
///
/// `None` quand le payload ne cite aucun tweet, ou quand la citation ne porte
/// ni auteur, ni texte, ni image.
pub fn quoted_tweet_content(value: &Value) -> Option<QuotedTweetContent> {
    let quoted = value.get("quoted_tweet")?;
    let author_name = extract_author_name(quoted).unwrap_or_default();
    let author_handle = extract_author_handle(quoted).unwrap_or_default();
    let text = extract_full_tweet_text(quoted).unwrap_or_default();
    let images = quoted_tweet_images(value);
    if author_name.is_empty() && author_handle.is_empty() && text.is_empty() && images.is_empty() {
        return None;
    }
    Some(QuotedTweetContent {
        author_name,
        author_handle,
        text,
        images,
    })
}

/// Photos du tweet cité, dans l'ordre du payload, avec leurs recadrages.
///
/// Retourne au plus [`MAX_TWEET_PHOTOS`] photos ; une vidéo citée fournit son
/// poster en repli pour que le bloc de citation garde une vignette. Les octets
/// PNG sont récupérés plus tard, au téléchargement du média.
pub fn quoted_tweet_images(value: &Value) -> Vec<QuotedTweetImage> {
    let Some(quoted) = value.get("quoted_tweet") else {
        return Vec::new();
    };
    let mut images: Vec<QuotedTweetImage> = quoted
        .get("mediaDetails")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("photo"))
        .filter_map(|item| {
            Some(QuotedTweetImage {
                source_url: https_url(item.get("media_url_https"))?,
                data: None,
                focus: parse_focus_rects(item.get("original_info")),
            })
        })
        .take(MAX_TWEET_PHOTOS)
        .collect();
    if images.is_empty() {
        images = quoted
            .get("photos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|photo| {
                Some(QuotedTweetImage {
                    source_url: https_url(photo.get("url"))?,
                    data: None,
                    focus: Vec::new(),
                })
            })
            .take(MAX_TWEET_PHOTOS)
            .collect();
    }
    if images.is_empty() {
        if let Some(poster) = extract_poster(quoted) {
            images.push(QuotedTweetImage {
                source_url: poster,
                data: None,
                focus: Vec::new(),
            });
        }
    }
    images
}

/// Lit `original_info.focus_rects` en rectangles relatifs (0.0 à 1.0).
fn parse_focus_rects(original_info: Option<&Value>) -> Vec<QuoteImageFocus> {
    let Some(info) = original_info else {
        return Vec::new();
    };
    let Some(rects) = info.get("focus_rects").and_then(Value::as_array) else {
        return Vec::new();
    };
    let field = |rect: &Value, name: &str| -> Option<u32> {
        let value = rect.get(name).and_then(Value::as_u64)?;
        u32::try_from(value).ok()
    };
    let parsed: Vec<(u32, u32, u32, u32)> = rects
        .iter()
        .filter_map(|rect| {
            Some((
                field(rect, "x")?,
                field(rect, "y")?,
                field(rect, "w")?,
                field(rect, "h")?,
            ))
        })
        .filter(|(_, _, width, height)| *width > 0 && *height > 0)
        .collect();
    if parsed.is_empty() {
        return Vec::new();
    }
    // Les dimensions déclarées priment ; à défaut, le plus grand rectangle
    // couvre l'image entière.
    let declared = |name: &str| {
        info.get(name)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    let width = declared("width").unwrap_or_else(|| {
        parsed
            .iter()
            .map(|(x, _, rect_width, _)| x + rect_width)
            .max()
            .unwrap_or(0)
    });
    let height = declared("height").unwrap_or_else(|| {
        parsed
            .iter()
            .map(|(_, y, _, rect_height)| y + rect_height)
            .max()
            .unwrap_or(0)
    });
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let (width, height) = (width as f32, height as f32);
    parsed
        .iter()
        .map(|(x, y, rect_width, rect_height)| QuoteImageFocus {
            x: *x as f32 / width,
            y: *y as f32 / height,
            width: *rect_width as f32 / width,
            height: *rect_height as f32 / height,
        })
        .collect()
}

/// Chaîne non vide d'une valeur JSON optionnelle.
fn non_empty_str(raw: Option<&Value>) -> Option<String> {
    raw.and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// URL HTTPS non vide d'une valeur JSON optionnelle.
fn https_url(raw: Option<&Value>) -> Option<String> {
    raw.and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| url.starts_with("https://"))
        .map(str::to_owned)
}

/// Poster of the tweet's video in a syndication payload: `video.poster`, else
/// the picture of the first media.
pub fn extract_poster(value: &Value) -> Option<String> {
    let https = |raw: Option<&Value>| {
        raw.and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| url.starts_with("https://"))
            .map(str::to_owned)
    };
    https(value.get("video").and_then(|video| video.get("poster"))).or_else(|| {
        https(
            value
                .get("mediaDetails")
                .and_then(Value::as_array)
                .and_then(|details| details.first())
                .and_then(|media| media.get("media_url_https")),
        )
    })
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

    #[test]
    fn finds_the_video_poster() {
        let payload = serde_json::json!({
            "video": {"poster": "https://pbs.twimg.com/poster.jpg"},
            "mediaDetails": [{"media_url_https": "https://pbs.twimg.com/media.jpg"}]
        });
        assert_eq!(
            extract_poster(&payload).as_deref(),
            Some("https://pbs.twimg.com/poster.jpg")
        );
        let details_only = serde_json::json!({
            "mediaDetails": [{"media_url_https": "https://pbs.twimg.com/media.jpg"}]
        });
        assert_eq!(
            extract_poster(&details_only).as_deref(),
            Some("https://pbs.twimg.com/media.jpg")
        );
        let insecure = serde_json::json!({"video": {"poster": "http://pbs.twimg.com/p.jpg"}});
        assert_eq!(extract_poster(&insecure), None);
    }

    #[test]
    fn extracts_the_tweet_text_ignoring_blank_values() {
        let payload = serde_json::json!({
            "id_str": "123",
            "text": "Regardez ce chat trop mignon !\nSuite du tweet…",
            "user": {"name": "Jeanne"}
        });
        assert_eq!(
            extract_tweet_text(&payload).as_deref(),
            Some("Regardez ce chat trop mignon !\nSuite du tweet…")
        );
        assert_eq!(
            extract_tweet_text(&serde_json::json!({"text": "   "})),
            None
        );
        assert_eq!(extract_tweet_text(&serde_json::json!({})), None);
        assert_eq!(extract_tweet_text(&serde_json::json!({"text": 42})), None);
    }

    const PHOTO_FIXTURE: &str = include_str!("../../tests/fixtures/twitter_syndication_photo.json");
    const TEXT_FIXTURE: &str = include_str!("../../tests/fixtures/twitter_syndication_text.json");
    const VIDEO_FIXTURE: &str = include_str!("../../tests/fixtures/twitter_syndication_video.json");
    const QUOTE_FIXTURE: &str = include_str!("../../tests/fixtures/twitter_syndication_quote.json");

    fn fixture(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn collects_photos_from_the_syndication_payload() {
        let photo = fixture(PHOTO_FIXTURE);
        let photos = collect_photos(&photo);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0], "https://pbs.twimg.com/media/HXVvi--WQAALdVX.jpg");

        assert!(collect_photos(&fixture(TEXT_FIXTURE)).is_empty());
        // La fixture vidéo n'a qu'une entrée `mediaDetails` de type vidéo et
        // un tableau `photos` vide.
        assert!(collect_photos(&fixture(VIDEO_FIXTURE)).is_empty());

        // Deux photos construites depuis la fixture photo, doublon compris.
        let mut two = photo;
        let first = two["mediaDetails"][0].clone();
        let mut second = first.clone();
        second["media_url_https"] = Value::String("https://pbs.twimg.com/media/second.jpg".into());
        let details = two["mediaDetails"].as_array_mut().unwrap();
        details.push(second);
        details.push(first);
        let photos = collect_photos(&two);
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[1], "https://pbs.twimg.com/media/second.jpg");
    }

    #[test]
    fn caps_photo_collection_at_four() {
        let mut payload = serde_json::json!({"mediaDetails": []});
        let details = payload["mediaDetails"].as_array_mut().unwrap();
        for index in 0..6 {
            details.push(serde_json::json!({
                "type": "photo",
                "media_url_https": format!("https://pbs.twimg.com/media/{index}.jpg")
            }));
        }
        assert_eq!(collect_photos(&payload).len(), MAX_TWEET_PHOTOS);
    }

    #[test]
    fn falls_back_to_the_photos_array() {
        let payload = serde_json::json!({
            "mediaDetails": [{"type": "video"}],
            "photos": [
                {"url": "https://pbs.twimg.com/media/a.jpg"},
                {"url": "http://pbs.twimg.com/media/b.jpg"}
            ]
        });
        assert_eq!(
            collect_photos(&payload),
            vec!["https://pbs.twimg.com/media/a.jpg".to_string()]
        );
    }

    #[test]
    fn decides_the_media_kind_from_the_payload() {
        assert_eq!(tweet_media_kind(true, 0), MediaKind::Video);
        assert_eq!(tweet_media_kind(true, 1), MediaKind::Video);
        assert_eq!(tweet_media_kind(false, 0), MediaKind::Image);
        assert_eq!(tweet_media_kind(false, 1), MediaKind::Image);
        assert_eq!(tweet_media_kind(false, 2), MediaKind::Video);
        assert_eq!(tweet_media_kind(false, 4), MediaKind::Video);
    }

    #[test]
    fn builds_original_photo_urls() {
        assert_eq!(
            photo_original_url("https://pbs.twimg.com/media/HXVvi--WQAALdVX.jpg"),
            "https://pbs.twimg.com/media/HXVvi--WQAALdVX.jpg?format=jpg&name=orig"
        );
        assert_eq!(
            photo_original_url("https://pbs.twimg.com/media/x.png?tag=1"),
            "https://pbs.twimg.com/media/x.png?format=png&name=orig"
        );
        // Une URL déjà paramétrée est réécrite proprement.
        assert_eq!(
            photo_original_url("https://pbs.twimg.com/media/x.jpeg?name=small"),
            "https://pbs.twimg.com/media/x.jpeg?format=jpg&name=orig"
        );
        // Hors pbs.twimg.com : inchangée.
        assert_eq!(
            photo_original_url("https://example.com/photo.jpg"),
            "https://example.com/photo.jpg"
        );
        assert_eq!(photo_original_url("pas une url"), "pas une url");
    }

    #[test]
    fn extracts_the_card_author_and_date() {
        let text = fixture(TEXT_FIXTURE);
        assert_eq!(extract_author_name(&text).as_deref(), Some("jack"));
        assert_eq!(extract_author_handle(&text).as_deref(), Some("jack"));
        assert_eq!(
            extract_created_at(&text).as_deref(),
            Some("2006-03-21T20:50:14.000Z")
        );
        assert_eq!(
            extract_avatar_url(&text).as_deref(),
            Some("https://pbs.twimg.com/profile_images/1661201415899951105/azNjKOSH_400x400.jpg")
        );
        let content = tweet_card_content(&text);
        assert_eq!(content.author_name, "jack");
        assert_eq!(content.author_handle, "jack");
        assert_eq!(content.text, "just setting up my twttr");
        assert_eq!(
            content.created_at.as_deref(),
            Some("2006-03-21T20:50:14.000Z")
        );
        assert_eq!(content.avatar, None);
    }

    #[test]
    fn prefers_the_note_tweet_text() {
        let payload = serde_json::json!({
            "text": "Début…",
            "note_tweet": {"text": "Texte long complet"}
        });
        assert_eq!(
            extract_full_tweet_text(&payload).as_deref(),
            Some("Texte long complet")
        );
        assert_eq!(
            extract_full_tweet_text(&serde_json::json!({"text": "court"})).as_deref(),
            Some("court")
        );
        assert_eq!(extract_full_tweet_text(&serde_json::json!({})), None);
    }

    #[test]
    fn strips_the_media_link_from_the_tweet_text() {
        let quote = fixture(QUOTE_FIXTURE);
        let quoted = quote.get("quoted_tweet").unwrap();
        assert_eq!(
            extract_full_tweet_text(quoted).as_deref(),
            Some("Allez wAllah")
        );
        // Sans `display_text_range`, le texte reste intact.
        assert_eq!(
            extract_full_tweet_text(&serde_json::json!({"text": "bonjour tout le monde"}))
                .as_deref(),
            Some("bonjour tout le monde")
        );
        // Un lien externe (hors média) n'est pas retiré.
        let payload = serde_json::json!({
            "text": "regarde ça https://example.com/article",
            "display_text_range": [0, 38]
        });
        assert_eq!(
            extract_full_tweet_text(&payload).as_deref(),
            Some("regarde ça https://example.com/article")
        );
    }

    #[test]
    fn extracts_the_quoted_tweet() {
        let quote = fixture(QUOTE_FIXTURE);
        let content = tweet_card_content(&quote);
        let quoted = content.quoted.as_ref().unwrap();
        assert_eq!(quoted.author_name, "berkani");
        assert_eq!(quoted.author_handle, "inconnu95267");
        assert_eq!(quoted.text, "Allez wAllah");
        // Le tweet parent n'a aucun média : sa carte doit citer les photos du
        // tweet cité, les deux.
        assert!(collect_photos(&quote).is_empty());
        assert_eq!(
            quoted
                .images
                .iter()
                .map(|image| image.source_url.as_str())
                .collect::<Vec<_>>(),
            vec![
                "https://pbs.twimg.com/media/HXUsq5hWEAA20_W.jpg",
                "https://pbs.twimg.com/media/HXUsq5iWMAAasFj.jpg",
            ]
        );
        // Les octets sont téléchargés plus tard, au téléchargement du média.
        assert!(quoted.images.iter().all(|image| image.data.is_none()));
        // X suggère plusieurs recadrages par photo : la carte en choisit un
        // pour ne pas couper le sujet. Les coordonnées sont relatives à
        // l'image source.
        let (width, height) = (1152.0_f32, 2048.0_f32);
        assert_eq!(
            quoted.images[0].focus,
            vec![
                QuoteImageFocus {
                    x: 0.0,
                    y: 958.0 / height,
                    width: 1.0,
                    height: 645.0 / height,
                },
                QuoteImageFocus {
                    x: 0.0,
                    y: 704.0 / height,
                    width: 1.0,
                    height: 1152.0 / height,
                },
                QuoteImageFocus {
                    x: 0.0,
                    y: 624.0 / height,
                    width: 1.0,
                    height: 1313.0 / height,
                },
                QuoteImageFocus {
                    x: 51.0 / width,
                    y: 0.0,
                    width: 1024.0 / width,
                    height: 1.0,
                },
                QuoteImageFocus {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
            ]
        );
        assert_eq!(quoted.images[1].focus.len(), 5);
    }

    #[test]
    fn ignores_media_outside_the_quoted_tweet() {
        let payload = serde_json::json!({
            "mediaDetails": [{
                "type": "photo",
                "media_url_https": "https://pbs.twimg.com/media/parent.jpg"
            }],
            "quoted_tweet": {
                "text": "Sans média",
                "user": {"name": "berkani", "screen_name": "inconnu95267"}
            }
        });
        let quoted = quoted_tweet_content(&payload).unwrap();
        assert!(quoted.images.is_empty());
    }

    #[test]
    fn skips_the_quoted_video_when_photos_follow() {
        // Une vidéo citée n'est pas une photo : seules les photos du payload
        // alimentent le bloc de citation.
        let payload = serde_json::json!({
            "quoted_tweet": {
                "text": "Deux médias",
                "user": {"name": "berkani", "screen_name": "inconnu95267"},
                "mediaDetails": [
                    {
                        "type": "video",
                        "media_url_https": "https://pbs.twimg.com/amplify_video_thumb/1/img/x.jpg"
                    },
                    {
                        "type": "photo",
                        "media_url_https": "https://pbs.twimg.com/media/photo.jpg",
                        "original_info": {
                            "width": 200,
                            "height": 100,
                            "focus_rects": [{"x": 50, "y": 0, "w": 100, "h": 100}]
                        }
                    }
                ]
            }
        });
        let images = quoted_tweet_images(&payload);
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].source_url,
            "https://pbs.twimg.com/media/photo.jpg"
        );
        assert_eq!(images[0].focus.len(), 1);
        assert_eq!(images[0].focus[0].x, 0.25);
    }

    #[test]
    fn caps_quoted_photos_at_four() {
        let media: Vec<serde_json::Value> = (0..6)
            .map(|index| {
                serde_json::json!({
                    "type": "photo",
                    "media_url_https": format!("https://pbs.twimg.com/media/photo{index}.jpg")
                })
            })
            .collect();
        let payload = serde_json::json!({
            "quoted_tweet": {
                "text": "Beaucoup de photos",
                "user": {"name": "berkani", "screen_name": "inconnu95267"},
                "mediaDetails": media
            }
        });
        assert_eq!(quoted_tweet_images(&payload).len(), MAX_TWEET_PHOTOS);
    }

    #[test]
    fn ignores_a_payload_without_quoted_tweet() {
        let text = fixture(TEXT_FIXTURE);
        assert!(quoted_tweet_content(&text).is_none());
        assert!(quoted_tweet_images(&text).is_empty());
        assert!(tweet_card_content(&text).quoted.is_none());
    }

    #[test]
    fn falls_back_to_the_quoted_video_poster() {
        let payload = serde_json::json!({
            "quoted_tweet": {
                "text": "Regarde ça",
                "user": {"name": "berkani", "screen_name": "inconnu95267"},
                "mediaDetails": [{
                    "type": "video",
                    "media_url_https": "https://pbs.twimg.com/amplify_video_thumb/1/img/x.jpg"
                }]
            }
        });
        let quoted = quoted_tweet_content(&payload).unwrap();
        assert_eq!(quoted.text, "Regarde ça");
        assert_eq!(quoted.images.len(), 1);
        assert_eq!(
            quoted.images[0].source_url,
            "https://pbs.twimg.com/amplify_video_thumb/1/img/x.jpg"
        );
        assert!(quoted.images[0].data.is_none());
    }

    #[test]
    fn ignores_an_empty_quoted_tweet() {
        let payload = serde_json::json!({"quoted_tweet": {"id_str": "42"}});
        assert!(quoted_tweet_content(&payload).is_none());
    }
}
