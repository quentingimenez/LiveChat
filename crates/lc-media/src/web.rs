//! Pages web génériques : médias annoncés en Open Graph et repli `yt-dlp`.
//!
//! Une URL [`SourceKind::DirectMedia`](crate::SourceKind::DirectMedia) peut
//! répondre du HTML : la page n'est alors pas le média, mais elle l'annonce
//! souvent dans ses balises `og:video` / `og:image`. [`parse_page`] extrait ces
//! candidats, dans l'ordre où ils doivent être essayés ; [`allows_ytdlp_fallback`]
//! décide ensuite si un hôte de médias connu peut être confié à `yt-dlp` quand
//! aucune balise ne donne de fichier téléchargeable.
//!
//! Le choix des images est volontairement prudent : sur une page vidéo,
//! `og:image` est une vignette (Vimeo, Imgur), pas le média. Une image n'est
//! retenue que si la page se déclare elle-même comme image (`og:type` `image.*`)
//! ou si elle annonce un GIF animé — le format que les plateformes de GIF
//! publient en repli de leur MP4.

use url::Url;

use lc_domain::MediaKind;

use crate::html;
use crate::title;

/// Un média annoncé par une page web.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebMedia {
    /// URL du fichier annoncé.
    pub url: String,
    /// Type MIME annoncé ou déduit de l'extension.
    pub mime: Option<String>,
    /// Nature du média.
    pub kind: MediaKind,
}

/// Page web analysée : titre et médias téléchargeables par ordre de préférence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebPage {
    /// Titre Open Graph (`og:title`, sinon `twitter:title`), nettoyé.
    pub title: Option<String>,
    /// Largeur annoncée en pixels.
    pub width: Option<u32>,
    /// Hauteur annoncée en pixels.
    pub height: Option<u32>,
    /// Vidéos puis images retenues, sans doublon.
    pub media: Vec<WebMedia>,
}

impl WebPage {
    /// `true` si la page n'annonce aucun média téléchargeable.
    pub fn is_empty(&self) -> bool {
        self.media.is_empty()
    }

    /// URL des médias dans l'ordre de préférence.
    pub fn media_urls(&self) -> Vec<&str> {
        self.media.iter().map(|media| media.url.as_str()).collect()
    }

    /// Premier média de la nature demandée.
    pub fn first_of_kind(&self, kind: MediaKind) -> Option<&WebMedia> {
        self.media.iter().find(|media| media.kind == kind)
    }
}

/// Extrait les médias Open Graph d'une page HTML.
///
/// Les vidéos passent avant les images : le MP4 d'une page Giphy ou Tenor est
/// meilleur que son GIF, et la vignette d'une page vidéo n'est jamais retenue.
pub fn parse_page(page: &str) -> WebPage {
    let title = html::meta_content(page, "og:title")
        .or_else(|| html::meta_content(page, "twitter:title"))
        .and_then(|title| clean_title(&title));
    let og_type = html::meta_content(page, "og:type")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let width = dimension(page, "og:video:width").or_else(|| dimension(page, "og:image:width"));
    let height = dimension(page, "og:video:height").or_else(|| dimension(page, "og:image:height"));

    let mut media = Vec::new();
    let mut seen = Vec::new();

    // Une page Vimeo annonce son lecteur dans `og:video` avec le type
    // `text/html` : ce n'est pas un fichier, c'est un signal pour `yt-dlp`.
    let declared_video_mime = html::meta_content(page, "og:video:type")
        .or_else(|| html::meta_content(page, "twitter:player:stream:content_type"));
    let embedded_player = declared_video_mime
        .as_deref()
        .is_some_and(|mime| mime.to_ascii_lowercase().starts_with("text/html"));
    let video_mime_hint = declared_video_mime
        .clone()
        .filter(|mime| is_video_mime(mime));
    for url in http_urls(
        page,
        &[
            "og:video:secure_url",
            "og:video",
            "og:video:url",
            "twitter:player:stream",
        ],
    ) {
        let inferred = mime_from_url(&url).filter(|mime| is_video_mime(mime));
        if embedded_player && inferred.is_none() {
            continue;
        }
        // Un manifeste HLS/DASH n'est pas un fichier : `download_http` ne sait
        // pas assembler ses segments. La page est laissée à `yt-dlp`.
        if is_streaming_manifest(&url, declared_video_mime.as_deref()) {
            continue;
        }
        push_media(
            &mut media,
            &mut seen,
            url,
            inferred.or_else(|| video_mime_hint.clone()),
            MediaKind::Video,
        );
    }

    let image_types = html::meta_contents(page, "og:image:type");
    let mut images = Vec::new();
    for (index, value) in html::meta_contents(page, "og:image")
        .into_iter()
        .enumerate()
    {
        let Some(url) = http_url(&value) else {
            continue;
        };
        let mime = image_types
            .get(index)
            .filter(|mime| is_image_mime(mime))
            .cloned()
            .or_else(|| mime_from_url(&url));
        push_image(&mut images, url, mime);
    }
    for value in html::meta_contents(page, "twitter:image") {
        let Some(url) = http_url(&value) else {
            continue;
        };
        let mime = mime_from_url(&url);
        push_image(&mut images, url, mime);
    }

    // `og:type` vaut `image.other` pour une image seule, `video.other` pour une
    // vidéo dont l'image est une vignette. Un GIF est toujours un média animé.
    let has_gif = images.iter().any(|(_, mime)| {
        mime.as_deref()
            .is_some_and(|mime| mime.eq_ignore_ascii_case("image/gif"))
    });
    if has_gif {
        // Dès qu'un GIF existe, il est le repli animé : un WebP ou un JPEG de
        // la même page n'est qu'une vignette statique.
        images.retain(|(_, mime)| {
            mime.as_deref()
                .is_some_and(|mime| mime.eq_ignore_ascii_case("image/gif"))
        });
    }
    if og_type.starts_with("image.") || has_gif {
        // GIF d'abord (animé), puis WebP, PNG et JPEG : les captures statiques
        // d'une page GIF ne doivent pas passer avant le GIF lui-même.
        images.sort_by_key(|(url, mime)| image_rank(mime.as_deref(), url));
        for (url, mime) in images {
            push_media(&mut media, &mut seen, url, mime, MediaKind::Image);
        }
    }

    WebPage {
        title,
        width,
        height,
        media,
    }
}

/// Retire le suffixe de référencement d'un titre de page de GIF.
///
/// `Will Smith Mib GIF - Find &amp; Share on GIPHY` devient `Will Smith Mib` ;
/// un titre sans ce motif est seulement normalisé.
pub fn clean_title(raw: &str) -> Option<String> {
    let decoded = html::decode_html_entities(raw);
    let headline = decoded
        .split_once(" GIF - ")
        .map_or(decoded.as_str(), |(headline, _)| headline)
        .trim();
    let headline = headline.strip_suffix(" GIF").unwrap_or(headline);
    title::normalize_title(headline).or_else(|| title::normalize_title(&decoded))
}

/// Hôtes de médias dont les pages peuvent être confiées à `yt-dlp` quand les
/// balises Open Graph ne donnent aucun fichier téléchargeable.
///
/// La liste est explicite : `yt-dlp` est un extracteur générique, pas un
/// navigateur, et une page quelconque (article, documentation) ne doit pas
/// lancer un processus pour échouer ensuite. Les sous-domaines sont acceptés
/// (`player.vimeo.com`, `www.reddit.com`, …).
pub const YTDLP_FALLBACK_HOSTS: &[&str] = &[
    "bilibili.com",
    "dailymotion.com",
    "facebook.com",
    "fb.watch",
    "giphy.com",
    "imgur.com",
    "instagram.com",
    "kick.com",
    "medal.tv",
    "reddit.com",
    "redd.it",
    "streamable.com",
    "tumblr.com",
    "twitch.tv",
    "vimeo.com",
];

/// `true` si l'URL appartient à un hôte autorisé au repli `yt-dlp`.
pub fn allows_ytdlp_fallback(raw_url: &str) -> bool {
    let Ok(parsed) = Url::parse(raw_url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    YTDLP_FALLBACK_HOSTS
        .iter()
        .any(|allowed| host == *allowed || is_subdomain_of(host, allowed))
}

fn is_subdomain_of(host: &str, domain: &str) -> bool {
    host.strip_suffix(domain)
        .is_some_and(|prefix| prefix.ends_with('.'))
}

fn push_image(images: &mut Vec<(String, Option<String>)>, url: String, mime: Option<String>) {
    if images
        .iter()
        .any(|(existing, _)| existing.eq_ignore_ascii_case(&url))
    {
        return;
    }
    images.push((url, mime));
}

fn push_media(
    media: &mut Vec<WebMedia>,
    seen: &mut Vec<String>,
    url: String,
    mime: Option<String>,
    kind: MediaKind,
) {
    if seen.iter().any(|existing| existing == &url) {
        return;
    }
    if mime
        .as_deref()
        .is_some_and(|mime| mime.to_ascii_lowercase().starts_with("text/"))
    {
        return;
    }
    seen.push(url.clone());
    media.push(WebMedia { url, mime, kind });
}

fn http_urls(page: &str, keys: &[&str]) -> Vec<String> {
    let mut urls = Vec::new();
    for key in keys {
        for value in html::meta_contents(page, key) {
            let Some(url) = http_url(&value) else {
                continue;
            };
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

fn http_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let parsed = Url::parse(trimmed).ok()?;
    (matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some())
        .then(|| trimmed.to_owned())
}

fn dimension(page: &str, key: &str) -> Option<u32> {
    html::meta_content(page, key)?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
}

fn is_video_mime(mime: &str) -> bool {
    mime.to_ascii_lowercase().starts_with("video/")
}

fn is_image_mime(mime: &str) -> bool {
    mime.to_ascii_lowercase().starts_with("image/")
}

fn is_streaming_manifest(url: &str, declared_mime: Option<&str>) -> bool {
    let manifest_mime = declared_mime.is_some_and(|mime| {
        let essence = mime
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        matches!(
            essence.as_str(),
            "application/vnd.apple.mpegurl" | "application/x-mpegurl" | "application/dash+xml"
        )
    });
    if manifest_mime {
        return true;
    }
    url_extension(url).is_some_and(|extension| extension == "m3u8" || extension == "mpd")
}

fn url_extension(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let name = parsed.path_segments()?.next_back()?;
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
}

/// MIME déduit de l'extension du chemin, quand la page ne l'annonce pas.
fn mime_from_url(url: &str) -> Option<String> {
    let mime = match url_extension(url)?.as_str() {
        "gif" => "image/gif",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        _ => return None,
    };
    Some(mime.to_owned())
}

fn image_rank(mime: Option<&str>, url: &str) -> u8 {
    let mime = mime
        .map(str::to_ascii_lowercase)
        .or_else(|| mime_from_url(url));
    match mime.as_deref() {
        Some("image/gif") => 0,
        Some("image/webp") => 1,
        Some("image/png") => 2,
        Some("image/jpeg") => 3,
        _ => 4,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const GIPHY: &str = r#"
        <html><head>
        <meta property="og:title" content="Will Smith Mib GIF - Find &amp; Share on GIPHY">
        <meta property="og:type" content="video.other">
        <meta property="og:image" content="https://media0.giphy.com/media/abc/giphy.webp">
        <meta property="og:image:type" content="image/webp">
        <meta property="og:image" content="https://media0.giphy.com/media/abc/giphy.gif">
        <meta property="og:image:type" content="image/gif">
        <meta property="og:video" content="https://media0.giphy.com/media/abc/giphy.mp4">
        <meta property="og:video:secure_url" content="https://media0.giphy.com/media/abc/giphy.mp4">
        <meta property="og:video:type" content="video/mp4">
        <meta property="og:video:width" content="312">
        <meta property="og:video:height" content="234">
        </head></html>
    "#;

    #[test]
    fn parses_the_video_before_the_animated_image() {
        let page = parse_page(GIPHY);
        assert_eq!(page.title.as_deref(), Some("Will Smith Mib"));
        assert_eq!(
            page.media_urls(),
            vec![
                "https://media0.giphy.com/media/abc/giphy.mp4",
                "https://media0.giphy.com/media/abc/giphy.gif",
            ],
            "le MP4 passe avant le GIF, et le WebP statique est écarté"
        );
        assert_eq!(page.media[0].kind, MediaKind::Video);
        assert_eq!(page.media[0].mime.as_deref(), Some("video/mp4"));
        assert_eq!(page.width, Some(312));
        assert_eq!(page.height, Some(234));
    }

    #[test]
    fn ignores_a_poster_image_on_a_video_page() {
        // Extrait de la page Vimeo : `og:video` est le lecteur HTML, `og:image`
        // n'est que la vignette du lecteur.
        let page = parse_page(
            r#"<meta property="og:title" content="The New Vimeo Player">
               <meta property="og:type" content="video.other">
               <meta property="og:image" content="https://i.vimeocdn.com/video/abc-d?f=webp">
               <meta property="og:image:type" content="image/webp">
               <meta property="og:video" content="https://player.vimeo.com/video/76979871">
               <meta property="og:video:type" content="text/html">"#,
        );
        assert_eq!(page.title.as_deref(), Some("The New Vimeo Player"));
        assert!(page.is_empty(), "la vignette ne doit pas devenir le média");
        assert!(allows_ytdlp_fallback("https://vimeo.com/76979871"));
    }

    #[test]
    fn ignores_streaming_manifests() {
        // `download_http` ne sait pas assembler les segments d'un HLS/DASH :
        // le candidat ne doit pas être retenu comme un fichier.
        let page = parse_page(
            r#"<meta property="og:type" content="video.other">
               <meta property="og:video" content="https://cdn.example/live/master.m3u8">
               <meta property="og:video:type" content="application/vnd.apple.mpegurl">"#,
        );
        assert!(page.is_empty());
        assert!(!page.media_urls().iter().any(|url| url.ends_with(".m3u8")));
    }

    #[test]
    fn accepts_an_image_page_and_rejects_unsafe_or_html_media() {
        let page = parse_page(
            r#"<meta property="og:title" content="Un chat">
               <meta property="og:type" content="image.other">
               <meta property="og:image" content="https://cdn.example/chat.png">
               <meta property="og:image" content="file:///C:/secret.png">
               <meta property="og:video" content="javascript:alert(1)">
               <meta property="og:video:type" content="text/html">"#,
        );
        assert_eq!(page.media_urls(), vec!["https://cdn.example/chat.png"]);
        assert_eq!(page.media[0].kind, MediaKind::Image);
    }

    #[test]
    fn missing_mime_is_inferred_from_the_extension() {
        let page = parse_page(
            r#"<meta property="og:type" content="image.other">
               <meta property="og:image" content="https://cdn.example/chat.jpg?v=2">"#,
        );
        assert_eq!(page.media[0].mime.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn keeps_the_secure_video_url_first() {
        let page = parse_page(
            r#"<meta property="og:type" content="video.other">
               <meta property="og:video" content="http://cdn.example/clip.mp4">
               <meta property="og:video:secure_url" content="https://cdn.example/clip.mp4">
               <meta property="og:video:type" content="video/mp4">"#,
        );
        assert_eq!(
            page.media_urls().first().copied(),
            Some("https://cdn.example/clip.mp4")
        );
        assert_eq!(page.media[0].mime.as_deref(), Some("video/mp4"));
    }

    #[test]
    fn orders_webp_before_jpeg_on_an_image_page() {
        let page = parse_page(
            r#"<meta property="og:type" content="image.other">
               <meta property="og:image" content="https://cdn.example/photo.jpg">
               <meta property="og:image" content="https://cdn.example/photo.webp">
               <meta property="og:image:type" content="image/jpeg">
               <meta property="og:image:type" content="image/webp">"#,
        );
        assert_eq!(
            page.media_urls(),
            vec![
                "https://cdn.example/photo.webp",
                "https://cdn.example/photo.jpg",
            ],
            "le WebP est préféré au JPEG, quel que soit l'ordre du document"
        );
    }

    #[test]
    fn accepts_a_gif_page_without_og_type() {
        let page = parse_page(
            r#"<meta property="og:image" content="https://cdn.example/still.webp">
               <meta property="og:image:type" content="image/webp">
               <meta property="og:image" content="https://cdn.example/anim.gif">
               <meta property="og:image:type" content="image/gif">"#,
        );
        assert_eq!(page.media_urls(), vec!["https://cdn.example/anim.gif"]);
        assert_eq!(page.media[0].kind, MediaKind::Image);
    }

    #[test]
    fn drops_a_manifest_by_extension_even_with_a_video_mime() {
        let page = parse_page(
            r#"<meta property="og:type" content="video.other">
               <meta property="og:video" content="https://cdn.example/live/master.m3u8">
               <meta property="og:video:type" content="video/mp4">"#,
        );
        assert!(page.is_empty());
    }

    #[test]
    fn ytdlp_fallback_is_limited_to_known_media_hosts() {
        assert!(allows_ytdlp_fallback("https://vimeo.com/76979871"));
        assert!(allows_ytdlp_fallback("https://player.vimeo.com/video/1"));
        assert!(allows_ytdlp_fallback("https://WWW.Reddit.com/r/aww/"));
        assert!(allows_ytdlp_fallback("https://www.medal.tv/games/1"));
        assert!(allows_ytdlp_fallback("https://vimeo.com./76979871"));
        assert!(!allows_ytdlp_fallback("https://notvimeo.com/video/1"));
        assert!(!allows_ytdlp_fallback("https://vimeo.com.evil.example/1"));
        assert!(!allows_ytdlp_fallback("https://vimeo.com@evil.example/1"));
        assert!(!allows_ytdlp_fallback("https://127.0.0.1/video"));
        assert!(!allows_ytdlp_fallback("https://example.com/article"));
        assert!(!allows_ytdlp_fallback("ftp://vimeo.com/76979871"));
        assert!(!allows_ytdlp_fallback("file:///C:/clip.mp4"));
    }

    #[test]
    fn title_cleaning_strips_the_gif_reference_suffix() {
        assert_eq!(
            clean_title("Jonathan Drouin Halifax Mooseheads GIF - Discover &amp; Share GIFs")
                .as_deref(),
            Some("Jonathan Drouin Halifax Mooseheads")
        );
        assert_eq!(
            clean_title("Will Smith Mib GIF - Find &amp; Share on GIPHY").as_deref(),
            Some("Will Smith Mib")
        );
        assert_eq!(
            clean_title("Un titre simple").as_deref(),
            Some("Un titre simple")
        );
    }
}
