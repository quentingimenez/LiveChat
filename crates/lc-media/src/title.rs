//! Titre « source » d'un média ajouté par lien.
//!
//! Quand un membre ajoute un média au board depuis un lien sans saisir de
//! titre, l'élément doit afficher le vrai titre de la publication (titre de la
//! vidéo YouTube, texte du tweet, nom du fichier d'une URL directe…) et non
//! plus le nom d'hôte du lien. Ce module calcule ce titre, de la meilleure
//! source disponible à la moins bonne :
//!
//! 1. le titre résolu par la plateforme (oEmbed, page TikTok, syndication
//!    Twitter) — pour un tweet sans texte, « Post de <auteur> » ;
//! 2. le titre imprimé par `yt-dlp` pendant le téléchargement (sources que
//!    seul yt-dlp sait décrire : Instagram, Reddit, clips Twitch, Vimeo…) ;
//! 3. pour une URL de fichier directe, le nom du fichier dans l'URL.
//!
//! Tout candidat est normalisé : trim, espaces collapseés, entités HTML
//! décodées, tronqué à [`TITLE_MAX_CHARS`] caractères.

use url::Url;

use crate::html;
use crate::resolve::ResolvedMedia;
use crate::source::{MediaSource, SourceKind};

/// Limite héritée du board pour le titre d'un élément (`TITLE_MAX_CHARS`).
pub const TITLE_MAX_CHARS: usize = 240;

/// Titre source d'un média ingéré, de la meilleure source disponible.
///
/// `ytdlp_title` est le titre extrait de la sortie `--print` de yt-dlp pendant
/// le téléchargement, s'il y en a un. Renvoie `None` quand rien n'est connu :
/// le board garde alors son repli actuel (nom d'hôte du lien).
pub fn source_title(
    source: &MediaSource,
    resolved: Option<&ResolvedMedia>,
    ytdlp_title: Option<&str>,
) -> Option<String> {
    match source.source_kind {
        // Une carte de commentaire porte le texte du commentaire dans le titre
        // du téléchargement : il prime sur la description de la vidéo.
        SourceKind::TikTok if crate::source::tiktok_comment_id(&source.canonical_url).is_some() => {
            normalized(ytdlp_title)
                .or_else(|| normalized(resolved.and_then(|resolved| resolved.title.as_deref())))
        }
        // Le résolveur de la plateforme a déjà le titre (oEmbed YouTube,
        // page TikTok) ; sinon le téléchargement yt-dlp l'a peut-être imprimé.
        SourceKind::YouTube | SourceKind::TikTok => {
            normalized(resolved.and_then(|resolved| resolved.title.as_deref()))
                .or_else(|| normalized(ytdlp_title))
        }
        // Tenor n'a que le titre Open Graph lu par le résolveur.
        SourceKind::Tenor => normalized(resolved.and_then(|resolved| resolved.title.as_deref())),
        // Un tweet n'a pas de titre : le début de son texte, sinon l'auteur.
        SourceKind::Twitter => resolved
            .and_then(tweet_title)
            .or_else(|| normalized(ytdlp_title)),
        // Fichier direct : le nom du fichier dans l'URL.
        SourceKind::DirectMedia => direct_media_title(&source.canonical_url),
        // Téléversements (le nom de fichier est déjà le titre demandé),
        // synthèse vocale et sources refusées : aucun titre source.
        SourceKind::LocalFile | SourceKind::Gtts | SourceKind::Unknown => None,
    }
}

/// Titre d'un tweet : début du texte de la publication quand le résolveur l'a
/// récupéré, sinon « Post de <auteur> ».
pub fn tweet_title(resolved: &ResolvedMedia) -> Option<String> {
    if let Some(title) = resolved
        .title
        .as_deref()
        .map(strip_trailing_short_links)
        .and_then(normalize_title)
    {
        return Some(title);
    }
    resolved
        .author
        .as_deref()
        .and_then(|author| normalize_title(&format!("Post de {author}")))
}

/// Retire les liens `t.co` qui terminent le texte d'un tweet (lien du média
/// ou de la publication citée) : ils n'apportent rien à un titre.
fn strip_trailing_short_links(text: &str) -> &str {
    let mut rest = text.trim_end();
    while let Some((head, last)) = rest.rsplit_once(char::is_whitespace) {
        if !last.starts_with("https://t.co/") {
            break;
        }
        rest = head.trim_end();
    }
    if rest.starts_with("https://t.co/") && !rest.contains(char::is_whitespace) {
        return "";
    }
    rest
}

/// Titre d'un média direct : nom de fichier de l'URL, pourcent-décodé, sans
/// extension, `_` et `-` remplacés par des espaces.
///
/// `https://x.y/media/Chat%20qui_tombe.mp4` donne « Chat qui tombe ».
pub fn direct_media_title(url: &str) -> Option<String> {
    let parsed = Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let segment = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .rfind(|segment| !segment.is_empty())?;
    let decoded = percent_decode(segment);
    let stem = strip_extension(&decoded);
    normalize_title(&stem.replace(['_', '-'], " "))
}

/// Normalise un candidat titre : entités HTML décodées, trim, espaces
/// collapseés, tronqué à [`TITLE_MAX_CHARS`] caractères ; vide → `None`.
pub fn normalize_title(raw: &str) -> Option<String> {
    let decoded = html::decode_html_entities(raw);
    let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(collapsed.chars().take(TITLE_MAX_CHARS).collect())
}

/// `normalize_title` sur une valeur optionnelle.
fn normalized(raw: Option<&str>) -> Option<String> {
    raw.and_then(normalize_title)
}

/// Retire l'extension d'un nom de fichier (dernier `.` et ce qui suit).
///
/// Un nom sans point, ou réduit à son extension (`.env`), est conservé tel
/// quel.
fn strip_extension(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// Décode les séquences `%XX` d'un segment d'URL (octets, dégradation UTF-8).
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_digit(bytes[index + 1]);
            let low = hex_digit(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                decoded.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Valeur d'un chiffre hexadécimal ASCII.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_titles() {
        assert_eq!(
            normalize_title("  Chat   qui\ttombe \n").as_deref(),
            Some("Chat qui tombe")
        );
        assert_eq!(
            normalize_title("Nuit &amp; jour &#38; &#x26; brume").as_deref(),
            Some("Nuit & jour & & brume")
        );
        assert_eq!(normalize_title("   ").as_deref(), None);
        assert_eq!(normalize_title("").as_deref(), None);
        let long = "é".repeat(300);
        assert_eq!(
            normalize_title(&long).map(|value| value.chars().count()),
            Some(TITLE_MAX_CHARS)
        );
    }

    #[test]
    fn extracts_direct_media_titles_from_urls() {
        let cases = [
            (
                "https://x.y/media/Chat%20qui_tombe.mp4",
                Some("Chat qui tombe"),
            ),
            (
                "https://cdn.test/videos/mon-super-clip.mp4?token=1",
                Some("mon super clip"),
            ),
            (
                "https://cdn.test/a/b/son%20du%20soir.mp3",
                Some("son du soir"),
            ),
            ("https://cdn.test/image.JPEG", Some("image")),
            ("https://cdn.test/sans-extension", Some("sans extension")),
            ("https://cdn.test/.env", Some(".env")),
            ("https://cdn.test/dernier-segment/", Some("dernier segment")),
        ];
        for (url, expected) in cases {
            assert_eq!(direct_media_title(url).as_deref(), expected, "url={url}");
        }
    }

    #[test]
    fn direct_media_title_rejects_non_http_urls() {
        assert_eq!(direct_media_title("file:C:\\media\\clip.mp4"), None);
        assert_eq!(direct_media_title("pas une url"), None);
        assert_eq!(direct_media_title("https://cdn.test/"), None);
    }

    #[test]
    fn builds_tweet_titles() {
        let source = MediaSource::from_raw("https://x.com/a/status/1").unwrap();
        let mut resolved = ResolvedMedia::from_source(&source);
        resolved.title = Some("Regardez ce chat trop mignon !".to_owned());
        resolved.author = Some("Jeanne".to_owned());
        assert_eq!(
            tweet_title(&resolved).as_deref(),
            Some("Regardez ce chat trop mignon !")
        );

        // Les liens t.co de fin de texte sont retirés.
        resolved.title = Some("Regardez ça https://t.co/abc https://t.co/def".to_owned());
        assert_eq!(tweet_title(&resolved).as_deref(), Some("Regardez ça"));
        resolved.title = Some("https://t.co/abc".to_owned());
        assert_eq!(tweet_title(&resolved).as_deref(), Some("Post de Jeanne"));

        resolved.title = None;
        assert_eq!(tweet_title(&resolved).as_deref(), Some("Post de Jeanne"));

        resolved.author = None;
        assert_eq!(tweet_title(&resolved), None);

        // Un texte tout blanc compte comme absent.
        resolved.title = Some("   ".to_owned());
        resolved.author = Some("Jeanne".to_owned());
        assert_eq!(tweet_title(&resolved).as_deref(), Some("Post de Jeanne"));
    }

    fn resolved(kind: SourceKind, title: Option<&str>) -> ResolvedMedia {
        let raw = match kind {
            SourceKind::YouTube => "https://www.youtube.com/watch?v=abc",
            SourceKind::TikTok => "https://www.tiktok.com/@_/video/1",
            SourceKind::Tenor => "https://tenor.com/view/1",
            _ => "https://x.com/a/status/1",
        };
        let source = MediaSource::from_raw(raw).unwrap();
        ResolvedMedia {
            title: title.map(str::to_owned),
            ..ResolvedMedia::from_source(&source)
        }
    }

    #[test]
    fn tiktok_comment_title_prefers_the_comment_text() {
        let comment = MediaSource::from_raw(
            "https://www.tiktok.com/@bingo83865/video/7707202086697487646?share_comment_id=7707256588990661390",
        )
        .unwrap();
        let video = resolved(SourceKind::TikTok, Some("Description de la vidéo"));
        assert_eq!(
            source_title(
                &comment,
                Some(&video),
                Some("When i accidentally step on my dogs tail at 3am")
            )
            .as_deref(),
            Some("When i accidentally step on my dogs tail at 3am")
        );
        // Carte indisponible : repli sur la description de la vidéo.
        assert_eq!(
            source_title(&comment, Some(&video), None).as_deref(),
            Some("Description de la vidéo")
        );
        // Une vidéo TikTok normale garde la priorité au résolveur.
        let plain = MediaSource::from_raw("https://www.tiktok.com/@_/video/1").unwrap();
        assert_eq!(
            source_title(&plain, Some(&video), Some("Titre yt-dlp")).as_deref(),
            Some("Description de la vidéo")
        );
    }

    #[test]
    fn picks_the_best_source_title_per_kind() {
        // YouTube : le titre résolu gagne sur celui de yt-dlp.
        let source = MediaSource::from_raw("https://youtu.be/abc").unwrap();
        let youtube = resolved(SourceKind::YouTube, Some("Titre oEmbed"));
        assert_eq!(
            source_title(&source, Some(&youtube), Some("Titre yt-dlp")).as_deref(),
            Some("Titre oEmbed")
        );
        // Résolution échouée mais téléchargement yt-dlp réussi.
        assert_eq!(
            source_title(&source, None, Some("Titre yt-dlp")).as_deref(),
            Some("Titre yt-dlp")
        );
        assert_eq!(
            source_title(&source, Some(&youtube), None).as_deref(),
            Some("Titre oEmbed")
        );

        // Tenor : le titre Open Graph du résolveur, sans repli yt-dlp.
        let tenor = MediaSource::from_raw("https://tenor.com/view/1").unwrap();
        let tenor_media = resolved(
            SourceKind::Tenor,
            Some("Jonathan Drouin Halifax Mooseheads"),
        );
        assert_eq!(
            source_title(&tenor, Some(&tenor_media), Some("Titre yt-dlp")).as_deref(),
            Some("Jonathan Drouin Halifax Mooseheads")
        );

        // Média direct : nom de fichier de l'URL.
        let direct = MediaSource::from_raw("https://cdn.test/Chat%20qui_tombe.mp4").unwrap();
        assert_eq!(
            source_title(&direct, None, None).as_deref(),
            Some("Chat qui tombe")
        );

        // Tweet : texte, sinon « Post de … », sinon le titre yt-dlp.
        let tweet = MediaSource::from_raw("https://x.com/a/status/1").unwrap();
        let mut no_text = resolved(SourceKind::Twitter, None);
        no_text.author = Some("Jeanne".to_owned());
        assert_eq!(
            source_title(&tweet, Some(&no_text), Some("Texte yt-dlp")).as_deref(),
            Some("Post de Jeanne")
        );
        assert_eq!(
            source_title(&tweet, None, Some("Texte yt-dlp")).as_deref(),
            Some("Texte yt-dlp")
        );

        // Téléversement local : aucun titre source (le nom de fichier est déjà
        // le titre demandé).
        let upload = MediaSource::from_upload("photo.png", "deadbeef");
        assert_eq!(source_title(&upload, None, None), None);
    }

    #[test]
    fn strips_extensions_from_file_names() {
        assert_eq!(strip_extension("clip.final.mp4"), "clip.final");
        assert_eq!(strip_extension("son"), "son");
        assert_eq!(strip_extension(".env"), ".env");
        assert_eq!(strip_extension("video."), "video");
    }

    #[test]
    fn percent_decodes_url_segments() {
        assert_eq!(percent_decode("Chat%20qui%5Ftombe"), "Chat qui_tombe");
        assert_eq!(percent_decode("100%25%20sur"), "100% sur");
        // Séquences invalides conservées telles quelles.
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz"), "a%zz");
        assert_eq!(percent_decode("%C3%A9"), "é");
    }
}
