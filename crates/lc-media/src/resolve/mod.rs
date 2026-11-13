//! Résolution des métadonnées d'une source média.
//!
//! Chaque plateforme a son propre résolveur : YouTube (oEmbed puis yt-dlp),
//! TikTok (oEmbed + extraction HTML pour les carrousels) et X/Twitter
//! (syndication, carte oEmbed, repli vxtwitter). Les résolveurs ne téléchargent
//! pas le média : ils décrivent la source pour l'étape suivante.

pub mod tiktok;
pub mod twitter;
pub mod youtube;

use lc_domain::MediaKind;

use crate::config::MediaConfig;
use crate::error::MediaResult;
use crate::source::{kind_from_url, MediaSource, SourceKind};

/// Carte de prévisualisation d'un tweet (HTML assaini + auteur).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TweetCard {
    /// URL canonique du tweet.
    pub url: String,
    /// HTML oEmbed assaini (aucun script/iframe).
    pub html: String,
    /// Nom d'affichage de l'auteur, si connu.
    pub author_name: Option<String>,
    /// URL du profil de l'auteur, si connue.
    pub author_url: Option<String>,
}

/// Métadonnées d'une source résolue, avant téléchargement.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMedia {
    /// Nature de la source.
    pub source_kind: SourceKind,
    /// URL canonique (celle du hash).
    pub canonical_url: String,
    /// SHA-256 hex de `canonical_url`.
    pub source_hash: String,
    /// Titre de la publication, si connu.
    pub title: Option<String>,
    /// Auteur de la publication, si connu.
    pub author: Option<String>,
    /// Durée annoncée en secondes, si connue.
    pub duration_sec: Option<f64>,
    /// Largeur annoncée en pixels, si connue.
    pub width: Option<u32>,
    /// Hauteur annoncée en pixels, si connue.
    pub height: Option<u32>,
    /// `true` si la source est verticale.
    pub is_vertical: bool,
    /// Type de média attendu.
    pub kind: Option<MediaKind>,
    /// Carte oEmbed d'un tweet.
    pub tweet_card: Option<TweetCard>,
    /// URL directe à télécharger quand elle est connue.
    pub media_url: Option<String>,
    /// Type MIME annoncé pour `media_url`.
    pub mime: Option<String>,
}

impl ResolvedMedia {
    /// Métadonnées minimales déduites d'une source (sans réseau).
    pub fn from_source(source: &MediaSource) -> Self {
        Self {
            source_kind: source.source_kind,
            canonical_url: source.canonical_url.clone(),
            source_hash: source.source_hash.clone(),
            kind: kind_from_url(&source.canonical_url),
            is_vertical: false,
            title: None,
            author: None,
            duration_sec: None,
            width: None,
            height: None,
            tweet_card: None,
            media_url: None,
            mime: None,
        }
    }

    /// Renseigne dimensions et orientation à partir d'un couple largeur/hauteur.
    pub fn with_dimensions(mut self, width: Option<u32>, height: Option<u32>) -> Self {
        self.width = width;
        self.height = height;
        self.is_vertical = matches!((width, height), (Some(w), Some(h)) if w > 0 && h > w);
        self
    }
}

/// Résout les métadonnées d'une source selon sa nature.
pub async fn resolve_media(
    config: &MediaConfig,
    source: &MediaSource,
) -> MediaResult<ResolvedMedia> {
    match source.source_kind {
        SourceKind::YouTube => youtube::resolve_youtube(source, config).await,
        SourceKind::TikTok => tiktok::resolve_tiktok(source, config).await,
        SourceKind::Twitter => twitter::resolve_tweet(source, config).await,
        _ => Ok(ResolvedMedia::from_source(source)),
    }
}

/// Encode une valeur pour l'insérer dans une query string.
pub(crate) fn encode_query_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
