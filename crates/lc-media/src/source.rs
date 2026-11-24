//! Normalisation, classification et empreinte des sources média.
//!
//! La canonicalisation reproduit les règles de l'ancien serveur : YouTube est
//! ramené à `https://www.youtube.com/watch?v=<id>`, TikTok à
//! `https://www.tiktok.com/@_/<video|photo>/<id>`, une page Tenor à
//! `https://tenor.com/view/<id>`, les autres URL voient leur fragment retiré.
//! Le hash SHA-256 de cette URL canonique sert de clé de cache
//! content-addressed.

use sha2::{Digest, Sha256};
use url::Url;

use lc_domain::MediaKind;

use crate::config::MediaConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::guard;
use crate::http;

/// Hôtes courts TikTok résolus avant canonicalisation.
pub const TIKTOK_SHORT_HOSTS: &[&str] = &[
    "vm.tiktok.com",
    "www.vm.tiktok.com",
    "vt.tiktok.com",
    "www.vt.tiktok.com",
];

/// Hôtes Twitter/X reconnus.
pub const TWITTER_HOSTS: &[&str] = &[
    "x.com",
    "www.x.com",
    "twitter.com",
    "www.twitter.com",
    "mobile.twitter.com",
];

/// Hôtes des pages Tenor reconnues (le CDN `media.tenor.com` reste direct).
pub const TENOR_HOSTS: &[&str] = &["tenor.com", "www.tenor.com"];

/// Nature d'une source média.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// Vidéo YouTube (oEmbed + yt-dlp).
    YouTube,
    /// Vidéo ou carrousel TikTok (oEmbed + extraction HTML).
    TikTok,
    /// Tweet X/Twitter (syndication + carte).
    Twitter,
    /// Page Tenor (métadonnées Open Graph, GIF ou MP4).
    Tenor,
    /// Fichier média accessible en HTTP direct.
    DirectMedia,
    /// Fichier local (`file:`).
    LocalFile,
    /// Synthèse vocale (`gtts:`).
    Gtts,
    /// Source inconnue (tentative HTTP directe).
    Unknown,
}

impl SourceKind {
    /// Représentation textuelle stable (journaux, debug).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::YouTube => "youtube",
            Self::TikTok => "tiktok",
            Self::Twitter => "twitter",
            Self::Tenor => "tenor",
            Self::DirectMedia => "direct",
            Self::LocalFile => "file",
            Self::Gtts => "gtts",
            Self::Unknown => "unknown",
        }
    }
}

/// Source média résolue (avant récupération du contenu).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaSource {
    /// Saisie d'origine, trimmée.
    pub original: String,
    /// URL canonique utilisée pour le hash et la résolution.
    pub canonical_url: String,
    /// SHA-256 hex minuscule de `canonical_url`.
    pub source_hash: String,
    /// Nature de la source.
    pub source_kind: SourceKind,
    /// Offset de démarrage YouTube (`?t=`), s'il est valide.
    pub start_offset_sec: Option<u32>,
}

impl MediaSource {
    /// Construit une source depuis une URL canonique déjà normalisée.
    pub fn from_canonical(original: impl Into<String>, canonical_url: impl Into<String>) -> Self {
        let original = original.into();
        let canonical_url = canonical_url.into();
        let source_hash = source_hash(&canonical_url);
        let source_kind = classify_source(&canonical_url);
        Self {
            original,
            canonical_url,
            source_hash,
            source_kind,
            start_offset_sec: None,
        }
    }

    /// Builds the source of a locally uploaded file.
    ///
    /// The cache key is `upload:sha256:<hex>` (content digest supplied by the
    /// caller); `source_url` keeps the original filename for display and for
    /// the NOT NULL `media_assets.source_url` column.
    pub fn from_upload(original_filename: impl Into<String>, content_sha256: &str) -> Self {
        let original_filename = original_filename.into();
        let canonical_url = format!("upload:{original_filename}");
        Self {
            original: canonical_url.clone(),
            canonical_url,
            source_hash: format!("upload:sha256:{content_sha256}"),
            source_kind: SourceKind::LocalFile,
            start_offset_sec: None,
        }
    }

    /// Construit une source depuis une URL brute (sans résolution réseau).
    pub fn from_raw(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let canonical_url = canonicalize_source_url(trimmed);
        let source_kind = classify_source(&canonical_url);
        let start_offset_sec = if source_kind == SourceKind::YouTube {
            parse_youtube_start_offset(trimmed)
        } else {
            None
        };
        Some(Self {
            original: trimmed.to_string(),
            source_hash: source_hash(&canonical_url),
            canonical_url,
            source_kind,
            start_offset_sec,
        })
    }

    /// `true` si un offset de démarrage a été détecté.
    pub fn has_start_offset(&self) -> bool {
        self.start_offset_sec.is_some()
    }
}

/// Empreinte SHA-256 hex minuscule d'une chaîne.
pub fn source_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Canonicalise une URL de source selon les règles legacy.
pub fn canonicalize_source_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Les URL locales et de synthèse vocale ne sont pas réécrites.
    if trimmed.starts_with("file:") || trimmed.starts_with("gtts:") {
        return trimmed.to_string();
    }
    let Ok(parsed) = Url::parse(trimmed) else {
        return trimmed.to_string();
    };
    if youtube_host(&parsed) {
        normalize_youtube_url(&parsed)
    } else if tiktok_host(&parsed) {
        normalize_tiktok_url(&parsed)
    } else if let Some(tenor_id) = tenor_page_id(&parsed) {
        format!("https://tenor.com/view/{tenor_id}")
    } else {
        let mut canonical = parsed;
        canonical.set_fragment(None);
        canonical.to_string()
    }
}

/// Ramène une URL YouTube à `https://www.youtube.com/watch?v=<id>`.
pub fn normalize_youtube_url(parsed: &Url) -> String {
    let mut url = parsed.clone();
    let original_host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let short_host = original_host == "youtu.be";
    let segments: Vec<&str> = url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut video_id = url
        .query_pairs()
        .find(|(key, _)| key == "v")
        .map(|(_, value)| value.into_owned());
    if short_host {
        if let Some(segment) = segments.first() {
            video_id = Some((*segment).to_string());
        }
    }
    if video_id.is_none() {
        if let Some(prefix) = segments.first() {
            if matches!(*prefix, "shorts" | "embed" | "live" | "v" | "e") {
                if let Some(identifier) = segments.get(1) {
                    video_id = Some((*identifier).to_string());
                }
            }
        }
    }
    let target_host = if short_host
        || original_host == "youtube.com"
        || original_host.ends_with(".youtube.com")
    {
        "www.youtube.com".to_string()
    } else {
        original_host
    };
    if url.set_host(Some(&target_host)).is_err() {
        tracing::debug!(host = target_host, "hôte YouTube impossible à normaliser");
    }
    url.set_path("/watch");
    url.set_query(None);
    if let Some(identifier) = video_id {
        url.query_pairs_mut().append_pair("v", &identifier);
    }
    url.set_fragment(None);
    url.to_string()
}

/// Ramène une URL TikTok `/video/<id>` ou `/photo/<id>` à `/@_/<kind>/<id>`.
///
/// Un partage de commentaire (application mobile) ajoute `share_comment_id` à
/// l'URL de la vidéo. L'identifiant est conservé dans l'URL canonique : il
/// distingue deux partages de commentaires de la même vidéo dans le cache et
/// permet au téléchargement de rendre la carte du commentaire.
pub fn normalize_tiktok_url(parsed: &Url) -> String {
    let mut url = parsed.clone();
    let comment_id = tiktok_comment_query(&url);
    if let Some(path) = tiktok_media_path(&url) {
        url.set_path(&format!("/@_/{path}"));
    }
    url.set_query(None);
    if let Some(comment_id) = comment_id {
        url.query_pairs_mut()
            .append_pair("share_comment_id", &comment_id);
    }
    url.set_fragment(None);
    url.to_string()
}

/// Type de publication TikTok porté par l'URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TikTokPostKind {
    /// Publication vidéo (`/video/<id>`).
    Video,
    /// Publication photo ou carrousel (`/photo/<id>`).
    Photo,
}

/// Type de publication TikTok déduit du chemin de l'URL.
///
/// Reconnaît `/video/<id>`, `/photo/<id>` et leurs variantes avec pseudo
/// (`/@pseudo/video/<id>`), y compris l'URL canonique `/@_/...`. Une URL de
/// musique, de profil ou courte ne désigne pas une publication visuelle et
/// renvoie `None`.
pub fn tiktok_post_kind(raw: &str) -> Option<TikTokPostKind> {
    let parsed = Url::parse(raw.trim()).ok()?;
    if !tiktok_host(&parsed) {
        return None;
    }
    let segments: Vec<&str> = parsed
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let (kind, identifier) = match segments.as_slice() {
        ["video", identifier] => (TikTokPostKind::Video, *identifier),
        ["photo", identifier] => (TikTokPostKind::Photo, *identifier),
        [handle, "video", identifier] if handle.starts_with('@') => {
            (TikTokPostKind::Video, *identifier)
        }
        [handle, "photo", identifier] if handle.starts_with('@') => {
            (TikTokPostKind::Photo, *identifier)
        }
        _ => return None,
    };
    (!identifier.is_empty()
        && identifier.len() <= 32
        && identifier
            .chars()
            .all(|character| character.is_ascii_digit()))
    .then_some(kind)
}

/// Identifiant du commentaire partagé d'une URL TikTok, s'il est valide.
///
/// L'application mobile utilise `share_comment_id` (parfois `comment_id` sur
/// les anciennes versions). Seuls les identifiants numériques sont acceptés :
/// ils sont recopiés tel quel dans l'URL de l'API de commentaires.
pub fn tiktok_comment_id(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw.trim()).ok()?;
    if !tiktok_host(&parsed) {
        return None;
    }
    tiktok_comment_query(&parsed)
}

fn tiktok_comment_query(url: &Url) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == "share_comment_id" || key == "comment_id")
        .map(|(_, value)| value.into_owned())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 32
                && value.chars().all(|character| character.is_ascii_digit())
        })
}

/// URL canonique d'un tweet (`https://x.com/<handle>/status/<id>`).
pub fn normalize_tweet_status_url(raw: &str) -> Option<String> {
    let (handle, status_id) = tweet_path_parts(raw)?;
    Some(format!("https://x.com/{handle}/status/{status_id}"))
}

/// Identifiant de statut d'un tweet, si l'URL en est une.
pub fn tweet_status_id(raw: &str) -> Option<String> {
    tweet_path_parts(raw).map(|(_, status_id)| status_id)
}

/// `true` si l'URL est une vidéo YouTube.
pub fn is_youtube_url(raw: &str) -> bool {
    Url::parse(raw.trim()).is_ok_and(|parsed| youtube_host(&parsed))
}

/// `true` si l'URL appartient au domaine TikTok.
pub fn is_tiktok_url(raw: &str) -> bool {
    Url::parse(raw.trim()).is_ok_and(|parsed| tiktok_host(&parsed))
}

/// Pseudo de l'auteur d'une URL TikTok (`/@handle/video/<id>`).
///
/// Les URL canoniques utilisent `@_` : il n'y a alors pas d'auteur à afficher.
pub fn tiktok_author(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw.trim()).ok()?;
    if !tiktok_host(&parsed) {
        return None;
    }
    let handle = parsed.path_segments()?.next()?.strip_prefix('@')?;
    (!handle.is_empty() && handle != "_").then(|| handle.to_string())
}

/// `true` si l'URL est une URL courte TikTok (`vm.tiktok.com`, `vt.tiktok.com`, …).
pub fn is_tiktok_short_url(raw: &str) -> bool {
    Url::parse(raw.trim())
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| TIKTOK_SHORT_HOSTS.contains(&host.as_str()))
}

/// `true` si l'URL pointe un statut X/Twitter.
pub fn is_twitter_url(raw: &str) -> bool {
    tweet_status_id(raw).is_some()
}

/// Identifiant numérique d'une page Tenor (`/view/…`, `/embed/…`).
///
/// Accepte le préfixe de langue (`tenor.com/fr/view/…`), le slug complet
/// (`…-gif-<id>`) et la forme courte (`/view/<id>`). Une page de recherche ou
/// un fichier du CDN `media.tenor.com` ne donnent aucun identifiant.
pub fn tenor_view_id(raw: &str) -> Option<String> {
    let parsed = Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    tenor_page_id(&parsed)
}

/// Classe une URL canonique en [`SourceKind`].
pub fn classify_source(canonical_url: &str) -> SourceKind {
    let trimmed = canonical_url.trim();
    if trimmed.starts_with("file:") {
        return SourceKind::LocalFile;
    }
    if trimmed.starts_with("gtts:") {
        return SourceKind::Gtts;
    }
    let Ok(parsed) = Url::parse(trimmed) else {
        return SourceKind::Unknown;
    };
    if youtube_host(&parsed) {
        return SourceKind::YouTube;
    }
    if tiktok_host(&parsed) {
        return SourceKind::TikTok;
    }
    if tenor_page_id(&parsed).is_some() {
        return SourceKind::Tenor;
    }
    if tweet_path_parts(trimmed).is_some() {
        return SourceKind::Twitter;
    }
    match parsed.scheme() {
        "http" | "https" => SourceKind::DirectMedia,
        _ => SourceKind::Unknown,
    }
}

/// Démarre la résolution d'une source (suit au besoin les URL courtes TikTok).
///
/// Renvoie `Ok(None)` pour une entrée vide et refuse toute source qui n'est
/// pas une URL `http`/`https` publique (voir [`validate_ingest_source`]).
pub async fn resolve_source(config: &MediaConfig, raw: &str) -> MediaResult<Option<MediaSource>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let candidate = if is_tiktok_short_url(trimmed) {
        http::resolve_short_url(&config.download, trimmed, config.download.short_url_timeout).await
    } else {
        trimmed.to_string()
    };
    validate_ingest_source(&candidate).await.map(Some)
}

/// Valide une entrée d'ingestion avant toute connexion réseau ou insertion en
/// base.
///
/// Refuse les schémas faibles (`file:`, `gtts:`, UNC, `ftp:`, `data:`…), les
/// URL inconnues, puis applique la garde SSRF de [`crate::guard`] : résolution
/// DNS et refus de toute adresse IP interne. Le serveur appelle cette fonction
/// avant `MediaService::ingest` pour ne créer aucun asset pour une source
/// refusée.
pub async fn validate_ingest_source(raw: &str) -> MediaResult<MediaSource> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "URL média vide",
        ));
    }
    let canonical_url = canonicalize_source_url(trimmed);
    let source_kind = classify_source(&canonical_url);
    match source_kind {
        SourceKind::LocalFile => {
            return Err(MediaError::new(
                MediaErrorCode::UnsupportedSource,
                "les sources locales (file:) sont refusées par la politique média",
            ));
        }
        SourceKind::Gtts => {
            return Err(MediaError::new(
                MediaErrorCode::UnsupportedSource,
                "la synthèse vocale gtts: n'est pas prise en charge par le pipeline média",
            ));
        }
        SourceKind::Unknown => {
            return Err(MediaError::new(
                MediaErrorCode::UnsupportedSource,
                format!("source média non supportée : {canonical_url}"),
            ));
        }
        SourceKind::YouTube
        | SourceKind::TikTok
        | SourceKind::Twitter
        | SourceKind::Tenor
        | SourceKind::DirectMedia => {}
    }
    guard::validate_target(&canonical_url).await?;
    let start_offset_sec = if source_kind == SourceKind::YouTube {
        parse_youtube_start_offset(trimmed)
    } else {
        None
    };
    tracing::debug!(
        original = trimmed,
        canonical = canonical_url,
        kind = source_kind.as_str(),
        "source média validée"
    );
    Ok(MediaSource {
        original: trimmed.to_string(),
        source_hash: source_hash(&canonical_url),
        canonical_url,
        source_kind,
        start_offset_sec,
    })
}

/// Parse un offset de démarrage YouTube (`?t=`, `?start=`, fragment `#t=`).
pub fn parse_youtube_start_offset(raw: &str) -> Option<u32> {
    let parsed = Url::parse(raw.trim()).ok()?;
    let mut candidate = query_value(&parsed, "t").or_else(|| query_value(&parsed, "start"));
    if candidate.is_none() {
        if let Some(fragment) = parsed.fragment() {
            if let Ok(fragment_url) = Url::parse(&format!("https://fragment.invalid/?{fragment}")) {
                candidate =
                    query_value(&fragment_url, "t").or_else(|| query_value(&fragment_url, "start"));
            }
        }
    }
    candidate.as_deref().and_then(parse_timestamp)
}

/// Détecte le type de média à partir de l'extension d'une URL.
pub fn kind_from_url(url: &str) -> Option<MediaKind> {
    let parsed = Url::parse(url.trim()).ok()?;
    let extension = parsed
        .path_segments()?
        .next_back()?
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())?;
    match extension.as_str() {
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "ts" | "m3u8" => Some(MediaKind::Video),
        "mp3" | "m4a" | "aac" | "wav" | "ogg" | "opus" | "flac" => Some(MediaKind::Audio),
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "bmp" | "heic" => Some(MediaKind::Image),
        _ => None,
    }
}

/// Détecte le type de média à partir d'un en-tête `Content-Type`.
pub fn kind_from_mime(mime: &str) -> Option<MediaKind> {
    let essence = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    // Un SVG est un contenu actif : il n'est jamais assimilé à une image
    // téléchargeable (AUD-A4-004, défense en profondeur après `transcode`).
    if essence == "image/svg+xml" {
        return None;
    }
    if essence.starts_with("image/") {
        Some(MediaKind::Image)
    } else if essence.starts_with("audio/") {
        Some(MediaKind::Audio)
    } else if essence.starts_with("video/")
        || essence == "application/vnd.apple.mpegurl"
        || essence == "application/x-mpegurl"
    {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// Extension de fichier associée à un type MIME, si connue.
pub fn extension_from_mime(mime: &str) -> Option<&'static str> {
    let essence = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let extension = match essence.as_str() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/mp2t" => "ts",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/aac" => "aac",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/flac" | "audio/x-flac" => "flac",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        _ => return None,
    };
    Some(extension)
}

/// Type MIME associé à une extension de fichier, si connu.
pub fn mime_from_extension(extension: &str) -> Option<&'static str> {
    let normalized = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let mime = match normalized.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ts" => "video/mp2t",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        _ => return None,
    };
    Some(mime)
}

fn youtube_host(parsed: &Url) -> bool {
    parsed
        .host_str()
        .map(|host| {
            let host = host.to_ascii_lowercase();
            host == "youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com")
        })
        .unwrap_or(false)
}

fn tiktok_host(parsed: &Url) -> bool {
    parsed
        .host_str()
        .map(|host| {
            let host = host.to_ascii_lowercase();
            host == "tiktok.com" || host.ends_with(".tiktok.com")
        })
        .unwrap_or(false)
}

fn tenor_host(parsed: &Url) -> bool {
    parsed
        .host_str()
        .map(|host| TENOR_HOSTS.contains(&host.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Identifiant d'une page Tenor, à partir d'une URL déjà analysée.
fn tenor_page_id(url: &Url) -> Option<String> {
    if !tenor_host(url) {
        return None;
    }
    let segments: Vec<&str> = url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let kind_index = match segments.first().copied() {
        Some("view") | Some("embed") => 0,
        Some(locale) if is_tenor_locale(locale) => match segments.get(1).copied() {
            Some("view") | Some("embed") => 1,
            _ => return None,
        },
        _ => return None,
    };
    if segments.len() != kind_index + 2 {
        return None;
    }
    tenor_id_from_slug(segments[kind_index + 1])
}

fn is_tenor_locale(segment: &str) -> bool {
    segment.len() == 2
        && segment
            .chars()
            .all(|character| character.is_ascii_alphabetic())
}

/// Extrait l'identifiant qui termine un slug Tenor (`…-gif-<id>` ou `<id>`).
fn tenor_id_from_slug(slug: &str) -> Option<String> {
    let identifier = slug
        .rsplit_once('-')
        .map_or(slug, |(_, identifier)| identifier);
    (!identifier.is_empty()
        && identifier.len() <= 32
        && identifier
            .chars()
            .all(|character| character.is_ascii_digit()))
    .then(|| identifier.to_string())
}

fn tiktok_media_path(url: &Url) -> Option<String> {
    let segments: Vec<&str> = url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() != 2 || !matches!(segments[0], "video" | "photo") {
        return None;
    }
    if segments[1].is_empty() || !segments[1].chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}/{}", segments[0], segments[1]))
}

fn tweet_path_parts(raw: &str) -> Option<(String, String)> {
    let parsed = Url::parse(raw.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !TWITTER_HOSTS.contains(&host.as_str()) {
        return None;
    }
    let segments: Vec<&str> = parsed
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.len() < 3 || segments[1] != "status" {
        return None;
    }
    let handle = segments[0];
    let status_id = segments[2];
    if handle.is_empty()
        || handle.len() > 25
        || !handle
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }
    if status_id.is_empty() || !status_id.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    Some((handle.to_string(), status_id.to_string()))
}

fn query_value(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

fn parse_timestamp(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|character| character.is_ascii_digit()) {
        return trimmed.parse::<u32>().ok().filter(|seconds| *seconds > 0);
    }
    if let Some(stripped) = trimmed.strip_suffix('s') {
        if !stripped.is_empty() && stripped.chars().all(|character| character.is_ascii_digit()) {
            return stripped.parse::<u32>().ok().filter(|seconds| *seconds > 0);
        }
    }
    if trimmed.contains(':') {
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() == 2 || parts.len() == 3 {
            let mut seconds: u32 = 0;
            for (index, part) in parts.iter().enumerate() {
                let Ok(parsed) = part.parse::<u32>() else {
                    return None;
                };
                let weight = match parts.len() {
                    2 => {
                        if index == 0 {
                            60
                        } else {
                            1
                        }
                    }
                    3 => match index {
                        0 => 3600,
                        1 => 60,
                        _ => 1,
                    },
                    _ => return None,
                };
                seconds = seconds.checked_add(parsed.checked_mul(weight)?)?;
            }
            return (seconds > 0).then_some(seconds);
        }
        return None;
    }
    parse_compact_duration(trimmed)
}

fn parse_compact_duration(raw: &str) -> Option<u32> {
    let mut total: u32 = 0;
    let mut digits = String::new();
    let mut saw_unit = false;
    for character in raw.chars() {
        if character.is_ascii_digit() {
            digits.push(character);
            continue;
        }
        let weight = match character {
            'h' | 'H' => 3600,
            'm' | 'M' => 60,
            's' | 'S' => 1,
            _ => return None,
        };
        if digits.is_empty() {
            return None;
        }
        let value = digits.parse::<u32>().ok()?;
        digits.clear();
        saw_unit = true;
        total = total.checked_add(value.checked_mul(weight)?)?;
    }
    if digits.is_empty() && saw_unit && total > 0 {
        Some(total)
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_reference_vector() {
        assert_eq!(
            source_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            source_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn canonicalizes_youtube_variants() {
        let cases = [
            (
                "https://www.youtube.com/watch?v=abc123&t=15",
                "https://www.youtube.com/watch?v=abc123",
            ),
            (
                "https://youtube.com/watch?v=abc123",
                "https://www.youtube.com/watch?v=abc123",
            ),
            (
                "https://m.youtube.com/watch?v=abc123&list=x",
                "https://www.youtube.com/watch?v=abc123",
            ),
            (
                "https://youtu.be/abc123?t=42",
                "https://www.youtube.com/watch?v=abc123",
            ),
            (
                "https://www.youtube.com/shorts/Sh0rt",
                "https://www.youtube.com/watch?v=Sh0rt",
            ),
            (
                "https://www.youtube.com/embed/Embed",
                "https://www.youtube.com/watch?v=Embed",
            ),
            (
                "https://www.youtube.com/live/LiveId",
                "https://www.youtube.com/watch?v=LiveId",
            ),
            (
                "https://www.youtube.com/playlist?list=PL1",
                "https://www.youtube.com/watch",
            ),
            (
                "  https://WWW.YOUTUBE.COM/watch?v=Xyz  ",
                "https://www.youtube.com/watch?v=Xyz",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(canonicalize_source_url(input), expected, "input={input}");
        }
    }

    #[test]
    fn canonicalizes_tiktok_variants() {
        let cases = [
            (
                "https://www.tiktok.com/video/7311111111111111111?is_copy_url=1",
                "https://www.tiktok.com/@_/video/7311111111111111111",
            ),
            (
                "https://www.tiktok.com/photo/7311111111111111111/",
                "https://www.tiktok.com/@_/photo/7311111111111111111",
            ),
            (
                "https://www.tiktok.com/@user/video/7311111111111111111",
                "https://www.tiktok.com/@user/video/7311111111111111111",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(canonicalize_source_url(input), expected, "input={input}");
        }
    }

    #[test]
    fn detects_tiktok_visual_posts_from_the_url() {
        use TikTokPostKind::{Photo, Video};
        let cases = [
            (
                "https://www.tiktok.com/video/7311111111111111111?is_copy_url=1",
                Some(Video),
            ),
            (
                "https://www.tiktok.com/photo/7311111111111111112/",
                Some(Photo),
            ),
            (
                "https://www.tiktok.com/@user/video/7311111111111111111",
                Some(Video),
            ),
            (
                "https://www.tiktok.com/@_/photo/7311111111111111112",
                Some(Photo),
            ),
            (
                "https://www.tiktok.com/@user/video/7311111111111111111?share_comment_id=7311111111111111113",
                Some(Video),
            ),
            ("https://www.tiktok.com/music/original-sound-1", None),
            ("https://www.tiktok.com/@user", None),
            ("https://vm.tiktok.com/ZMabc/", None),
            ("https://www.tiktok.com/@user/video/abcd", None),
            ("https://example.com/@user/video/1", None),
        ];
        for (url, expected) in cases {
            assert_eq!(tiktok_post_kind(url), expected, "url={url}");
        }
    }

    #[test]
    fn keeps_the_shared_comment_id_in_the_tiktok_canonical_url() {
        let shared = "https://www.tiktok.com/@bingo83865/video/7707202086697487646?is_from_webapp=1&sender_device=pc&web_id=1&share_comment_id=7707256588990661390&share_item_id=7707202086697487646";
        assert_eq!(
            canonicalize_source_url(shared),
            "https://www.tiktok.com/@bingo83865/video/7707202086697487646?share_comment_id=7707256588990661390"
        );
        assert_eq!(
            tiktok_comment_id(shared).as_deref(),
            Some("7707256588990661390")
        );
        // Canonicaliser une URL déjà canonique est idempotent.
        let canonical = canonicalize_source_url(shared);
        assert_eq!(canonicalize_source_url(&canonical), canonical);
    }

    #[test]
    fn rejects_a_malformed_shared_comment_id() {
        assert_eq!(tiktok_comment_id("https://www.tiktok.com/@a/video/1"), None);
        assert_eq!(
            tiktok_comment_id("https://www.tiktok.com/@a/video/1?share_comment_id=abc"),
            None
        );
        assert_eq!(
            tiktok_comment_id("https://example.com/@a/video/1?share_comment_id=123"),
            None
        );
        assert_eq!(
            canonicalize_source_url(
                "https://www.tiktok.com/@a/video/1?share_comment_id=abc&keep=1"
            ),
            "https://www.tiktok.com/@a/video/1"
        );
    }

    #[test]
    fn gives_two_comment_shares_of_one_video_distinct_hashes() {
        let first = MediaSource::from_raw(
            "https://www.tiktok.com/@a/video/1?share_comment_id=111&share_item_id=1",
        )
        .expect("source");
        let second = MediaSource::from_raw(
            "https://www.tiktok.com/@a/video/1?share_comment_id=222&share_item_id=1",
        )
        .expect("source");
        assert_eq!(first.source_kind, SourceKind::TikTok);
        assert_ne!(first.source_hash, second.source_hash);
    }

    #[test]
    fn canonicalizes_other_urls_by_stripping_fragment_only() {
        assert_eq!(
            canonicalize_source_url("https://cdn.test/video.mp4?x=1#frag"),
            "https://cdn.test/video.mp4?x=1"
        );
        assert_eq!(
            canonicalize_source_url("file:C:\\media\\clip.mp4"),
            "file:C:\\media\\clip.mp4"
        );
        assert_eq!(canonicalize_source_url("gtts:bonjour"), "gtts:bonjour");
        assert_eq!(canonicalize_source_url("   "), "");
        assert_eq!(canonicalize_source_url("pas une url"), "pas une url");
    }

    #[test]
    fn normalizes_tweet_status_urls() {
        assert_eq!(
            normalize_tweet_status_url("https://twitter.com/SomeOne/status/1234567890?s=20")
                .as_deref(),
            Some("https://x.com/SomeOne/status/1234567890")
        );
        assert_eq!(
            normalize_tweet_status_url("https://mobile.twitter.com/a_1/status/42/photo/1")
                .as_deref(),
            Some("https://x.com/a_1/status/42")
        );
        assert_eq!(normalize_tweet_status_url("https://x.com/user"), None);
        assert_eq!(
            normalize_tweet_status_url("https://example.com/user/status/123"),
            None
        );
        assert_eq!(
            tweet_status_id("https://x.com/user/status/123").as_deref(),
            Some("123")
        );
    }

    #[test]
    fn parses_youtube_start_offsets() {
        let cases: &[(&str, Option<u32>)] = &[
            ("https://www.youtube.com/watch?v=abc&t=42", Some(42)),
            ("https://www.youtube.com/watch?v=abc&t=42s", Some(42)),
            ("https://www.youtube.com/watch?v=abc&t=1h2m3s", Some(3723)),
            ("https://www.youtube.com/watch?v=abc&t=2m03s", Some(123)),
            ("https://www.youtube.com/watch?v=abc&t=1:02", Some(62)),
            ("https://www.youtube.com/watch?v=abc&t=1:02:03", Some(3723)),
            ("https://youtu.be/abc?start=7", Some(7)),
            ("https://youtu.be/abc#t=10", Some(10)),
            ("https://www.youtube.com/watch?v=abc", None),
            ("https://www.youtube.com/watch?v=abc&t=0", None),
            ("https://www.youtube.com/watch?v=abc&t=-3", None),
            ("https://www.youtube.com/watch?v=abc&t=inconnu", None),
            ("https://www.youtube.com/watch?v=abc&t=1:2:3:4", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_youtube_start_offset(input),
                *expected,
                "input={input}"
            );
        }
    }

    #[test]
    fn classifies_sources() {
        assert_eq!(
            classify_source("https://www.youtube.com/watch?v=abc"),
            SourceKind::YouTube
        );
        assert_eq!(classify_source("https://youtu.be/abc"), SourceKind::YouTube);
        assert_eq!(
            classify_source("https://www.tiktok.com/@a/video/1"),
            SourceKind::TikTok
        );
        assert_eq!(
            classify_source("https://vm.tiktok.com/abc"),
            SourceKind::TikTok
        );
        assert_eq!(
            classify_source("https://x.com/a/status/1"),
            SourceKind::Twitter
        );
        assert_eq!(
            classify_source("https://cdn.test/v.mp4"),
            SourceKind::DirectMedia
        );
        assert_eq!(classify_source("file:clip.mp4"), SourceKind::LocalFile);
        assert_eq!(classify_source("gtts:salut"), SourceKind::Gtts);
        assert_eq!(classify_source("nope"), SourceKind::Unknown);
        assert!(is_youtube_url("https://youtu.be/x"));
        assert!(is_tiktok_url("https://www.tiktok.com/@a/video/1"));
        assert!(!is_tiktok_url("https://example.com"));
        assert!(is_tiktok_short_url("https://vm.tiktok.com/ZMabc/"));
        assert!(!is_tiktok_short_url("https://www.tiktok.com/@a/video/1"));
        assert!(is_twitter_url("https://twitter.com/a/status/1"));
        assert!(!is_twitter_url("https://twitter.com/a"));
    }

    #[test]
    fn media_source_from_raw_computes_hash_and_offset() {
        let source = MediaSource::from_raw("https://youtu.be/abc?t=30").unwrap();
        assert_eq!(source.canonical_url, "https://www.youtube.com/watch?v=abc");
        assert_eq!(
            source.source_hash,
            source_hash("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(source.source_kind, SourceKind::YouTube);
        assert_eq!(source.start_offset_sec, Some(30));
        assert!(source.has_start_offset());
        assert!(MediaSource::from_raw("   ").is_none());
        let direct =
            MediaSource::from_canonical("https://cdn.test/a.mp4", "https://cdn.test/a.mp4");
        assert_eq!(direct.source_kind, SourceKind::DirectMedia);
        assert_eq!(direct.start_offset_sec, None);
    }

    #[test]
    fn detects_kind_from_url_and_mime() {
        assert_eq!(
            kind_from_url("https://a.test/b.MP4"),
            Some(MediaKind::Video)
        );
        assert_eq!(
            kind_from_url("https://a.test/b.mp3?x=1"),
            Some(MediaKind::Audio)
        );
        assert_eq!(
            kind_from_url("https://a.test/b.jpeg"),
            Some(MediaKind::Image)
        );
        assert_eq!(kind_from_url("https://a.test/b.pdf"), None);
        assert_eq!(kind_from_url("pas-une-url"), None);
        assert_eq!(kind_from_mime("image/png"), Some(MediaKind::Image));
        assert_eq!(kind_from_mime("image/svg+xml"), None);
        assert_eq!(kind_from_mime("image/SVG+XML; charset=utf-8"), None);
        assert_eq!(
            kind_from_mime("audio/mp4; codecs=mp4a"),
            Some(MediaKind::Audio)
        );
        assert_eq!(kind_from_mime("video/webm"), Some(MediaKind::Video));
        assert_eq!(
            kind_from_mime("application/vnd.apple.mpegurl"),
            Some(MediaKind::Video)
        );
        assert_eq!(kind_from_mime("text/html"), None);
    }

    #[test]
    fn extension_and_mime_mappings_are_bidirectional() {
        for (mime, extension) in [
            ("video/mp4", "mp4"),
            ("audio/mpeg", "mp3"),
            ("image/jpeg", "jpg"),
            ("image/png", "png"),
        ] {
            assert_eq!(extension_from_mime(mime), Some(extension));
            assert_eq!(mime_from_extension(extension), Some(mime));
        }
        assert_eq!(extension_from_mime("application/octet-stream"), None);
        assert_eq!(mime_from_extension(".MP4"), Some("video/mp4"));
        assert_eq!(mime_from_extension("inconnu"), None);
    }

    #[test]
    fn tiktok_short_urls_are_temporarily_configured() {
        assert_eq!(
            crate::config::SHORT_URL_RESOLVE_TIMEOUT,
            std::time::Duration::from_secs(8)
        );
        assert!(TIKTOK_SHORT_HOSTS.contains(&"www.vt.tiktok.com"));
    }
}
