//! Téléchargement des sources média : HTTP streaming borné, `yt-dlp`, et
//! assemblage des carrousels TikTok.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lc_domain::MediaKind;
use tokio::io::AsyncWriteExt;

use crate::config::{MediaConfig, BYTES_PER_MB, YOUTUBE_RELAXED_YTDLP_FORMAT};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::guard;
use crate::http;
use crate::process;
use crate::resolve::tenor;
use crate::resolve::tiktok;
use crate::resolve::twitter;
use crate::source::{
    classify_source, extension_from_mime, is_tiktok_url, is_youtube_url, kind_from_mime,
    kind_from_url, mime_from_extension, tiktok_comment_id, tweet_status_id, MediaSource,
    SourceKind,
};
use crate::storage::MediaStorage;
use crate::tiktok_comment;
use crate::transcode;
use crate::translate;
use crate::tweet_card::{self, CardVideoTarget};
use crate::tweet_card_video;
use crate::web;

static DOWNLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Résultat d'un téléchargement (fichier local prêt à être normalisé).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadResult {
    /// Chemin du fichier téléchargé.
    pub path: PathBuf,
    /// Type de média déduit du contenu.
    pub kind: Option<MediaKind>,
    /// Type MIME annoncé, si connu.
    pub mime: Option<String>,
    /// Taille du fichier en octets.
    pub size_bytes: u64,
    /// URL source effectivement téléchargée.
    pub source_url: String,
    /// Titre de la source annoncé pendant le téléchargement : page Open Graph
    /// pour une page web, sortie `yt-dlp` pour les sources qu'il extrait.
    pub title: Option<String>,
    /// Nombre de caractères visibles quand le téléchargement est une carte
    /// texte (tweet) : texte principal et tweet cité. Sert à calculer la durée
    /// d'affichage sans dépendre du titre tronqué à
    /// [`crate::title::TITLE_MAX_CHARS`].
    pub text_chars: Option<usize>,
}

/// Options de lancement de `yt-dlp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YtdlpOptions {
    /// Sélecteur de format explicite.
    pub format_override: Option<String>,
    /// Client YouTube forcé (`android`, …).
    pub youtube_player_client: Option<String>,
    /// Fusionner la sortie en MP4.
    pub merge: bool,
}

impl Default for YtdlpOptions {
    fn default() -> Self {
        Self {
            format_override: None,
            youtube_player_client: None,
            merge: true,
        }
    }
}

/// Télécharge une source selon sa nature.
pub async fn download_source(
    source: &MediaSource,
    resolved: Option<&crate::resolve::ResolvedMedia>,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    ensure_download_allowed(source).await?;
    match source.source_kind {
        SourceKind::YouTube => download_youtube(config, &source.canonical_url, temp_dir).await,
        SourceKind::TikTok => download_tiktok(source, config, temp_dir).await,
        SourceKind::Twitter => download_twitter(source, resolved, config, temp_dir).await,
        SourceKind::Tenor => download_tenor(source, resolved, config, temp_dir).await,
        SourceKind::LocalFile => Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "les sources locales (file:) sont refusées par la politique média",
        )),
        SourceKind::Gtts => Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "la source gtts n'est pas prise en charge par lc-media (Phase 5b)",
        )),
        SourceKind::DirectMedia | SourceKind::Unknown => {
            download_web(source, config, temp_dir).await
        }
    }
}

/// Refuse les sources locales et valide la cible réseau (garde SSRF).
async fn ensure_download_allowed(source: &MediaSource) -> MediaResult<()> {
    match source.source_kind {
        SourceKind::LocalFile => Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "les sources locales (file:) sont refusées par la politique média",
        )),
        SourceKind::Gtts => Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "la synthèse vocale gtts: n'est pas prise en charge par le pipeline média",
        )),
        SourceKind::Unknown => Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            format!("source média non supportée : {}", source.canonical_url),
        )),
        SourceKind::YouTube
        | SourceKind::TikTok
        | SourceKind::Twitter
        | SourceKind::Tenor
        | SourceKind::DirectMedia => {
            guard::validate_target(&source.canonical_url).await?;
            Ok(())
        }
    }
}

/// Choisit l'erreur à remonter quand la page HTML n'a livré aucun média et que
/// `yt-dlp` a échoué : à priorité égale, l'échec de `yt-dlp` est plus utile que
/// « page HTML », car il explique pourquoi le repli n'a rien produit.
fn web_fallback_error(page_error: MediaError, ytdlp_error: MediaError) -> MediaError {
    ytdlp_error.most_relevant(page_error)
}

/// Télécharge une URL web inconnue : fichier direct, média Open Graph, puis
/// `yt-dlp` pour les hôtes de médias connus.
///
/// La première requête sert à la fois à télécharger un fichier direct et à lire
/// la page quand elle répond du HTML : une page Giphy coûte une requête, pas
/// deux. Les candidats Open Graph sont ensuite essayés dans l'ordre du parseur
/// (vidéo avant image), et `yt-dlp` ne prend le relais que si aucun ne donne un
/// fichier — c'est le cas des lecteurs HTML comme Vimeo.
async fn download_web(
    source: &MediaSource,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    let page_url = source.canonical_url.as_str();
    let (body, status) = match download_http_response(config, page_url, temp_dir, &[]).await? {
        DirectResponse::Media(download) => return Ok(download),
        DirectResponse::Html { body, status, .. } => (body, status),
    };
    let page = web::parse_page(&body);
    let mut last_error: Option<MediaError> = None;
    for candidate in &page.media {
        match download_http(config, &candidate.url, temp_dir, &[]).await {
            Ok(mut download) => {
                if download.title.is_none() {
                    download.title = page.title.clone();
                }
                return Ok(download);
            }
            Err(error) => {
                if error.is(MediaErrorCode::FileTooLarge) {
                    return Err(error);
                }
                last_error = Some(match last_error {
                    Some(previous) => previous.most_relevant(error),
                    None => error,
                });
            }
        }
    }

    let page_error = match last_error {
        Some(error) => error.most_relevant(html_page_error(status)),
        None => html_page_error(status),
    };
    if !web::allows_ytdlp_fallback(page_url) {
        return Err(page_error);
    }
    match download_with_ytdlp(config, page_url, temp_dir, &YtdlpOptions::default()).await {
        Ok(mut download) => {
            if download.title.is_none() {
                download.title = page.title.clone();
            }
            Ok(download)
        }
        // À priorité égale, l'échec de `yt-dlp` est plus utile que « page HTML » :
        // c'est lui qui explique pourquoi le repli n'a pas produit de média.
        Err(error) => Err(web_fallback_error(page_error, error)),
    }
}

/// Télécharge le média d'un tweet : la carte du tweet (texte, photos ou vidéo,
/// tweet cité compris), avec repli sur la vidéo brute puis `yt-dlp`.
///
/// `yt-dlp` n'est appelé qu'en dernier recours, quand aucune variante vidéo n'a
/// pu être téléchargée (payload indisponible ou flux inaccessibles) : il ne
/// sait ni télécharger une photo ni rendre une carte.
async fn download_twitter(
    source: &MediaSource,
    resolved: Option<&crate::resolve::ResolvedMedia>,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    // 1. Le payload de syndication vient du résolveur : le relire ici coûtait
    //    une requête réseau (~169 ms) à chaque tweet.
    let resolved_payload = resolved.and_then(|resolved| resolved.syndication.clone());
    let payload = match resolved_payload {
        Some(value) => Some(value),
        None => match tweet_status_id(&source.canonical_url) {
            Some(status_id) => twitter::fetch_syndication_payload(config, &status_id).await,
            None => None,
        },
    };
    if let Some(value) = payload.as_ref() {
        let variants = twitter::tweet_video_variants(value);
        let has_video = !variants.is_empty();
        let video = match twitter::pick_variant(&variants) {
            Some(best) => match download_http(config, &best.url, temp_dir, &[]).await {
                Ok(download) => Some(download),
                Err(error) => {
                    tracing::debug!(url = %best.url, "téléchargement de la vidéo du tweet échoué : {error}");
                    None
                }
            },
            None => None,
        };
        if has_video && video.is_none() {
            tracing::debug!("vidéo du tweet inaccessible, repli yt-dlp");
        } else {
            match download_tweet_card(
                value,
                config,
                temp_dir,
                &source.canonical_url,
                video.as_ref(),
            )
            .await
            {
                Ok(download) => return Ok(download),
                Err(error) => {
                    tracing::debug!("rendu de la carte tweet impossible : {error}");
                    if let Some(video) = video {
                        return Ok(video);
                    }
                    if let Some(photo) = twitter::tweet_photos(value).first() {
                        return download_tweet_photo(config, &photo.source_url, temp_dir).await;
                    }
                }
            }
        }
    }
    // 2. Média déjà annoncé par le résolveur (vidéo).
    if let Some(media_url) = resolved.and_then(|resolved| resolved.media_url.clone()) {
        match download_http(config, &media_url, temp_dir, &[]).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                tracing::debug!(%media_url, "téléchargement direct du tweet échoué : {error}");
            }
        }
    }
    // 3. Vidéo dont aucune variante n'a pu être téléchargée, ou payload
    //    indisponible : comportement historique (yt-dlp sait télécharger une
    //    vidéo).
    tracing::debug!(
        source = %source.canonical_url,
        "aucun média du tweet téléchargeable directement, repli yt-dlp"
    );
    let options = YtdlpOptions::default();
    download_with_ytdlp(config, &source.canonical_url, temp_dir, &options).await
}

/// Télécharge le média d'une page Tenor : le MP4 annoncé par la page, puis le
/// GIF quand le MP4 est absent ou inaccessible.
///
/// Le résolveur a déjà lu la page ; si son URL a échoué (CDN, expiration), la
/// page est relue une fois pour récupérer les URLs courantes.
async fn download_tenor(
    source: &MediaSource,
    resolved: Option<&crate::resolve::ResolvedMedia>,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    let mut tried: Vec<String> = Vec::new();
    let mut primary_error: Option<MediaError> = None;
    if let Some(media_url) = resolved.and_then(|resolved| resolved.media_url.as_deref()) {
        tried.push(media_url.to_string());
        match download_http(config, media_url, temp_dir, &[]).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                tracing::debug!(
                    url = media_url,
                    "média Tenor annoncé inaccessible : {error}"
                );
                primary_error = Some(error);
            }
        }
    }
    let page = match tenor::fetch_tenor_page(config, &source.canonical_url).await {
        Ok(page) => page,
        // L'échec du média annoncé explique mieux le problème qu'une relecture
        // de page qui échoue pour la même raison réseau.
        Err(error) => return Err(primary_error.unwrap_or(error)),
    };
    for media_url in page.media_urls() {
        if tried.iter().any(|candidate| candidate == media_url) {
            continue;
        }
        match download_http(config, media_url, temp_dir, &[]).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                tracing::debug!(url = media_url, "média Tenor inaccessible : {error}");
                if primary_error.is_none() {
                    primary_error = Some(error);
                }
            }
        }
    }
    Err(primary_error.unwrap_or_else(|| {
        MediaError::new(
            MediaErrorCode::MediaNotFound,
            "la page Tenor ne contient aucun média téléchargeable",
        )
    }))
}

/// Télécharge une photo de tweet en taille originale, repli sur l'URL nue.
async fn download_tweet_photo(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    let original = twitter::photo_original_url(url);
    if original != url {
        match download_http(config, &original, temp_dir, &[]).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                tracing::debug!(
                    url = %original,
                    "photo de tweet en taille originale indisponible : {error}"
                );
            }
        }
    }
    download_http(config, url, temp_dir, &[]).await
}

/// Rend la carte d'un tweet (texte, photos, vidéo, tweet cité) et l'écrit dans
/// le dossier temporaire : PNG seul, ou MP4 quand une vidéo est incrustée.
async fn download_tweet_card(
    value: &serde_json::Value,
    config: &MediaConfig,
    temp_dir: &Path,
    canonical_url: &str,
    parent_video: Option<&DownloadResult>,
) -> MediaResult<DownloadResult> {
    let mut content = twitter::tweet_card_content(value);
    translate::localize_tweet_card(config, &mut content).await;
    content.avatar = download_tweet_avatar(config, value, temp_dir).await;
    if parent_video.is_none() {
        // Aucune vidéo téléchargée : pas de zone noire dans la carte.
        content.video = None;
    }
    for (index, photo) in content.photos.iter_mut().enumerate() {
        photo.data = download_tweet_photo_png(
            config,
            &photo.source_url,
            temp_dir,
            &format!("tweet-photo-{index}"),
        )
        .await;
    }
    let mut quoted_video: Option<PathBuf> = None;
    if let Some(quoted) = content.quoted.as_mut() {
        for (index, image) in quoted.images.iter_mut().enumerate() {
            image.data = download_tweet_photo_png(
                config,
                &image.source_url,
                temp_dir,
                &format!("tweet-quote-{index}"),
            )
            .await;
        }
        if quoted.video.is_some() {
            quoted_video = download_tweet_quoted_video(value, config, temp_dir).await;
            if quoted_video.is_none() {
                // Vidéo citée inaccessible : le bloc garde ses photos ou son
                // poster, sans zone noire.
                quoted.video = None;
            }
        }
    }
    if let Some(parent) = content.reply_parent.as_mut() {
        for (index, image) in parent.images.iter_mut().enumerate() {
            // Échec non bloquant : l'image est ignorée par le rendu.
            image.data = download_tweet_photo_png(
                config,
                &image.source_url,
                temp_dir,
                &format!("tweet-reply-{index}"),
            )
            .await;
        }
    }
    let card = tweet_card::render_tweet_card_with_slots(&content)?;
    let text_chars = Some(tweet_card::card_text_chars(&content));
    let has_parent = card
        .video_slots
        .iter()
        .any(|slot| slot.target == CardVideoTarget::Parent);
    let has_quoted = card
        .video_slots
        .iter()
        .any(|slot| slot.target == CardVideoTarget::Quoted);
    if !has_parent && !has_quoted {
        return write_tweet_card_png(temp_dir, &card.png, canonical_url, text_chars);
    }
    match tweet_card_video::compose_tweet_card_video(
        config,
        temp_dir,
        &card,
        parent_video.map(|video| video.path.as_path()),
        quoted_video.as_deref(),
    )
    .await
    {
        Ok(path) => {
            let size_bytes = MediaStorage::file_size(&path)?;
            Ok(DownloadResult {
                path,
                kind: Some(MediaKind::Video),
                mime: Some("video/mp4".to_string()),
                size_bytes,
                source_url: canonical_url.to_string(),
                title: None,
                text_chars,
            })
        }
        // Seule la vidéo citée manquait : la carte PNG reste valable.
        Err(error) if !has_parent => {
            tracing::debug!("composition de la vidéo citée impossible : {error}");
            if let Some(quoted) = content.quoted.as_mut() {
                quoted.video = None;
            }
            let card = tweet_card::render_tweet_card_with_slots(&content)?;
            write_tweet_card_png(temp_dir, &card.png, canonical_url, text_chars)
        }
        Err(error) => Err(error),
    }
}

/// Écrit la carte PNG et renvoie le téléchargement prêt à normaliser.
fn write_tweet_card_png(
    temp_dir: &Path,
    png: &[u8],
    canonical_url: &str,
    text_chars: Option<usize>,
) -> MediaResult<DownloadResult> {
    let path = temp_dir.join("tweet-card.png");
    fs::write(&path, png).map_err(|error| {
        MediaError::from_io(
            MediaErrorCode::DownloadFailed,
            "écriture de la carte tweet",
            &error,
        )
    })?;
    Ok(DownloadResult {
        path,
        kind: Some(MediaKind::Image),
        mime: Some("image/png".to_string()),
        size_bytes: png.len() as u64,
        source_url: canonical_url.to_string(),
        title: None,
        text_chars,
    })
}

/// Télécharge l'avatar de l'auteur et le convertit en PNG 192×192.
///
/// Twitter sert des JPEG (aucun décodeur JPEG n'est embarqué) : ffmpeg fait la
/// conversion. Toute erreur rend `None` (cercle avec l'initiale) et n'échoue
/// jamais le téléchargement.
async fn download_tweet_avatar(
    config: &MediaConfig,
    value: &serde_json::Value,
    temp_dir: &Path,
) -> Option<Vec<u8>> {
    let url = twitter::extract_avatar_url(value)?;
    let input = match download_http(config, &url, temp_dir, &[]).await {
        Ok(download) => download.path,
        Err(error) => {
            tracing::debug!(%url, "avatar du tweet indisponible : {error}");
            return None;
        }
    };
    let output = temp_dir.join("tweet-avatar.png");
    let args = vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-v".to_string(),
        "error".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-vf".to_string(),
        "scale=192:192".to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        output.display().to_string(),
    ];
    if let Err(error) = process::run_command(
        &config.tools.ffmpeg,
        &args,
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    {
        tracing::debug!("conversion de l'avatar du tweet impossible : {error}");
        return None;
    }
    match fs::read(&output) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::debug!("lecture de l'avatar du tweet impossible : {error}");
            None
        }
    }
}

/// Filtre ffmpeg de conversion des images de carte : au plus 1800 px par côté,
/// jamais agrandies.
///
/// `scale=1800:1800:force_original_aspect_ratio=decrease` agrandissait les
/// images plus petites que la boîte (une photo 1234×1326 devenait 1675×1800),
/// soit plusieurs mégaoctets de PNG pour une carte qui l'affiche sur 984 px de
/// large. Les bornes `min(iw,1800)`/`min(ih,1800)` laissent ffmpeg réduire les
/// grandes images et garder les petites à leur taille d'origine.
const CARD_IMAGE_SCALE_FILTER: &str =
    "scale=w='min(iw,1800)':h='min(ih,1800)':force_original_aspect_ratio=decrease";

/// Construit les arguments ffmpeg de conversion d'une image de carte en PNG
/// (fonction pure, testable).
fn card_image_png_args(input: &Path, output: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-hide_banner".to_string(),
        "-v".to_string(),
        "error".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-vf".to_string(),
        CARD_IMAGE_SCALE_FILTER.to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        output.display().to_string(),
    ]
}

/// Télécharge une image et la convertit en PNG pour une carte.
///
/// Aucun décodeur JPEG/WEBP n'est embarqué : ffmpeg fait la conversion, borne
/// l'image à 1800 px par côté et ne l'agrandit jamais. Toute erreur rend
/// `None` (image absente de la carte) et n'échoue jamais le téléchargement.
pub(crate) async fn download_card_image_png(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
    name: &str,
    headers: &[(&str, &str)],
) -> Option<Vec<u8>> {
    let input = match download_http(config, url, temp_dir, headers).await {
        Ok(download) => download.path,
        Err(error) => {
            tracing::debug!(%url, "image de carte indisponible : {error}");
            return None;
        }
    };
    let output = temp_dir.join(format!("{name}.png"));
    let args = card_image_png_args(&input, &output);
    if let Err(error) = process::run_command(
        &config.tools.ffmpeg,
        &args,
        config.download.timeout,
        MediaErrorCode::TranscodeFailed,
    )
    .await
    {
        tracing::debug!("conversion de l'image de carte impossible : {error}");
        return None;
    }
    match fs::read(&output) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::debug!("lecture de l'image de carte impossible : {error}");
            None
        }
    }
}

/// Télécharge une photo de tweet et la convertit en PNG pour la carte.
///
/// L'originale (`name=orig`) est demandée d'abord ; à défaut, l'URL telle
/// quelle. ffmpeg borne l'image à 1800 px par côté sans jamais l'agrandir
/// (aucun décodeur JPEG n'est embarqué). Toute erreur rend `None` (photo
/// absente de la carte) et n'échoue jamais le téléchargement.
async fn download_tweet_photo_png(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
    name: &str,
) -> Option<Vec<u8>> {
    let original = twitter::photo_original_url(url);
    if original != url {
        if let Some(png) = download_card_image_png(config, &original, temp_dir, name, &[]).await {
            return Some(png);
        }
    }
    download_card_image_png(config, url, temp_dir, name, &[]).await
}

/// Télécharge la vidéo du tweet cité, jouée dans le bloc de citation.
async fn download_tweet_quoted_video(
    value: &serde_json::Value,
    config: &MediaConfig,
    temp_dir: &Path,
) -> Option<PathBuf> {
    let variants = twitter::quoted_tweet_video_variants(value);
    let best = twitter::pick_variant(&variants)?;
    match download_http(config, &best.url, temp_dir, &[]).await {
        Ok(download) => Some(download.path),
        Err(error) => {
            tracing::debug!(
                url = %best.url,
                "téléchargement de la vidéo du tweet cité échoué : {error}"
            );
            None
        }
    }
}

/// Assemble des images en vidéo de carrousel et renvoie le téléchargement prêt.
async fn build_carousel_download(
    config: &MediaConfig,
    temp_dir: &Path,
    images: &[PathBuf],
    audio: Option<&Path>,
    is_vertical: bool,
    source_url: String,
    title: Option<String>,
) -> MediaResult<DownloadResult> {
    let output = temp_dir.join("carousel.mp4");
    let dimensions = transcode::carousel_dimensions(config.video.max_height, is_vertical);
    transcode::build_carousel_video(
        &config.tools.ffmpeg,
        images,
        audio,
        &output,
        dimensions,
        config,
    )
    .await?;
    let size_bytes = MediaStorage::file_size(&output)?;
    Ok(DownloadResult {
        path: output,
        kind: Some(MediaKind::Video),
        mime: Some("video/mp4".to_string()),
        size_bytes,
        source_url,
        title,
        text_chars: None,
    })
}

/// Télécharge via `yt-dlp` avec les replis YouTube.
pub async fn download_youtube(
    config: &MediaConfig,
    canonical_url: &str,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    let first =
        download_with_ytdlp(config, canonical_url, temp_dir, &YtdlpOptions::default()).await;
    let first_error = match first {
        Ok(download) => return Ok(download),
        Err(error) => error,
    };
    let lowered = first_error
        .raw_message()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let auth_related = first_error.is(MediaErrorCode::PrivateOrAuthRequired)
        || lowered.contains("sign in")
        || lowered.contains("bot")
        || lowered.contains("login");
    if auth_related {
        let android = YtdlpOptions {
            youtube_player_client: Some("android".to_string()),
            ..YtdlpOptions::default()
        };
        match download_with_ytdlp(config, canonical_url, temp_dir, &android).await {
            Ok(download) => return Ok(download),
            Err(error) => tracing::debug!("repli YouTube android échoué : {error}"),
        }
    }
    let format_error = first_error.is(MediaErrorCode::DownloadFailed)
        || lowered.contains("requested format")
        || lowered.contains("format is not available");
    if format_error {
        let relaxed = YtdlpOptions {
            format_override: Some(YOUTUBE_RELAXED_YTDLP_FORMAT.to_string()),
            merge: false,
            ..YtdlpOptions::default()
        };
        match download_with_ytdlp(config, canonical_url, temp_dir, &relaxed).await {
            Ok(download) => return Ok(download),
            Err(error) => return Err(first_error.most_relevant(error)),
        }
    }
    Err(first_error)
}

/// Construit les arguments `yt-dlp` (hors cookies et sortie).
pub fn build_ytdlp_args(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
    options: &YtdlpOptions,
) -> Vec<String> {
    let format = options
        .format_override
        .clone()
        .unwrap_or_else(|| config.tools.ytdlp_format.clone());
    let max_megabytes = config.download.max_size_bytes / BYTES_PER_MB;
    let mut args = vec![
        "--no-playlist".to_string(),
        "--no-progress".to_string(),
        "--no-warnings".to_string(),
        // Le titre est imprimé AVANT le chemin : le parseur du chemin lit la
        // sortie en partant de la fin, la ligne de chemin reste donc la
        // dernière candidate, et le titre est ce qui précède.
        "--print".to_string(),
        "after_move:%(title)s".to_string(),
        "--print".to_string(),
        "after_move:filepath".to_string(),
        "--max-filesize".to_string(),
        format!("{max_megabytes}M"),
        "--format".to_string(),
        format,
    ];
    if options.merge {
        args.push("--merge-output-format".to_string());
        args.push("mp4".to_string());
    }
    if config.tools.ytdlp_concurrent_fragments > 1 {
        args.push("--concurrent-fragments".to_string());
        args.push(config.tools.ytdlp_concurrent_fragments.to_string());
    }
    if let Some(client_name) = &options.youtube_player_client {
        args.push("--extractor-args".to_string());
        args.push(format!("youtube:player_client={client_name}"));
    }
    args.push("-o".to_string());
    args.push(temp_dir.join("download.%(ext)s").display().to_string());
    // Option terminator: guarantees the url that follows can never be read
    // as a yt-dlp option, even when it starts with a dash.
    args.push("--".to_string());
    args.push(url.to_string());
    args
}

/// Sorties `--print` analysées d'un téléchargement `yt-dlp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YtdlpOutput {
    /// Chemin du fichier téléchargé, imprimé par `yt-dlp`.
    pub path: Option<PathBuf>,
    /// Titre de la source imprimé par `yt-dlp` (`NA` ou vide → `None`).
    pub title: Option<String>,
}

/// Analyse la sortie `--print` de `yt-dlp` : le titre puis le chemin, dans cet
/// ordre (voir [`build_ytdlp_args`]).
///
/// Le chemin est reconnu en partant de la fin (dernière ligne non technique
/// qui ressemble à un chemin), donc une ligne de titre ressemblant à un chemin
/// ne peut pas le détourner ; le titre est l'ensemble des lignes non techniques
/// qui précèdent la ligne de chemin — un titre contenant des retours à la
/// ligne est réassemblé.
pub fn parse_ytdlp_output(stdout: &str, temp_dir: &Path) -> YtdlpOutput {
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    let mut path = None;
    let mut title = None;
    for (index, line) in lines.iter().enumerate().rev() {
        if is_ytdlp_junk_line(line) {
            continue;
        }
        if let Some(recognized) = ytdlp_path_from_line(line, temp_dir) {
            path = Some(recognized);
            title = ytdlp_title_from_lines(&lines[..index]);
            break;
        }
    }
    YtdlpOutput { path, title }
}

/// Parse le chemin de sortie imprimé par `yt-dlp`.
pub fn parse_ytdlp_output_path(stdout: &str, temp_dir: &Path) -> Option<PathBuf> {
    parse_ytdlp_output(stdout, temp_dir).path
}

/// Titre d'une ligne imprimée par `yt-dlp` : `NA` (champ absent) ou vide → `None`.
fn ytdlp_print_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed == "NA" {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Titre imprimé avant la ligne de chemin : lignes non techniques jointes
/// (réassemblage d'un titre multi-lignes), filtrées des valeurs vides.
fn ytdlp_title_from_lines(before: &[&str]) -> Option<String> {
    let joined = before
        .iter()
        .filter(|line| !is_ytdlp_junk_line(line))
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    ytdlp_print_title(&joined)
}

/// Ligne de sortie à ignorer (vide, ou journal `yt-dlp` en `[...]`).
fn is_ytdlp_junk_line(line: &str) -> bool {
    line.is_empty() || line.starts_with('[')
}

/// Reconnaît une ligne de chemin : JSON `_filename`, sortie `download.<ext>`
/// relative au dossier temporaire, ou chemin absolu/existant.
fn ytdlp_path_from_line(trimmed: &str, temp_dir: &Path) -> Option<PathBuf> {
    if trimmed.starts_with('{') {
        let value = serde_json::from_str::<serde_json::Value>(trimmed).ok()?;
        let path = value.get("_filename").and_then(|path| path.as_str())?;
        let path = PathBuf::from(path.trim());
        if path.is_absolute() {
            return Some(path);
        }
        return Some(temp_dir.join(path));
    }
    if trimmed.starts_with("download.") {
        return Some(temp_dir.join(trimmed));
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() || path.exists() {
        return Some(path);
    }
    None
}

/// Trouve le fichier téléchargé le plus récent dans un répertoire temporaire.
pub fn find_downloaded_file(dir: &Path) -> MediaResult<Option<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "lecture du dossier temporaire",
                &error,
            ));
        }
    };
    let mut candidates: Vec<(i64, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.ends_with(".part") || name.ends_with(".ytdl") || name.starts_with('.') {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or_default();
        candidates.push((modified, entry.path()));
    }
    candidates.sort_by_key(|(modified, _)| *modified);
    Ok(candidates.pop().map(|(_, path)| path))
}

/// Télécharge via `yt-dlp` (cookies TikTok inclus si configurés).
pub async fn download_with_ytdlp(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
    options: &YtdlpOptions,
) -> MediaResult<DownloadResult> {
    let mut args = build_ytdlp_args(config, url, temp_dir, options);
    if is_tiktok_url(url) {
        if let Some(cookie) = &config.tools.tiktok_cookie {
            if !cookie.trim().is_empty() {
                let cookie_path = write_cookie_file(temp_dir, cookie)?;
                if let Some(index) = args.iter().position(|argument| argument == "-o") {
                    args.insert(index, cookie_path.display().to_string());
                    args.insert(index, "--cookies".to_string());
                }
            }
        }
    }
    let output = crate::process::run_command(
        &config.tools.ytdlp,
        &args,
        config.download.timeout,
        MediaErrorCode::DownloadFailed,
    )
    .await?;
    let parsed = parse_ytdlp_output(&output.stdout, temp_dir);
    let path = match parsed.path {
        Some(path) if path.exists() => path,
        _ => find_downloaded_file(temp_dir)?.ok_or_else(|| {
            MediaError::new(
                MediaErrorCode::DownloadFailed,
                "yt-dlp n'a produit aucun fichier téléchargé",
            )
        })?,
    };
    let size_bytes = MediaStorage::file_size(&path)?;
    if size_bytes > config.download.max_size_bytes {
        let _removed = fs::remove_file(&path);
        return Err(MediaError::new(
            MediaErrorCode::FileTooLarge,
            "le média téléchargé dépasse la taille maximale",
        ));
    }
    let bytes = read_header_bytes(&path, 32);
    let kind = sniff_kind(&bytes).or_else(|| kind_from_url(&path.display().to_string()));
    let mime = path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(mime_from_extension)
        .map(str::to_string)
        .or_else(|| kind.map(|kind| default_mime(kind).to_string()));
    Ok(DownloadResult {
        path,
        kind,
        mime,
        size_bytes,
        source_url: url.to_string(),
        title: parsed.title,
        text_chars: None,
    })
}

/// Réponse d'une URL directe : un média enregistré, ou la page HTML qui le
/// décrit.
enum DirectResponse {
    Media(DownloadResult),
    Html { body: String, status: u16 },
}

/// Télécharge une URL directe en streaming, avec plafond de taille.
pub async fn download_http(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
    headers: &[(&str, &str)],
) -> MediaResult<DownloadResult> {
    match download_http_response(config, url, temp_dir, headers).await? {
        DirectResponse::Media(download) => Ok(download),
        DirectResponse::Html { status, .. } => Err(html_page_error(status)),
    }
}

/// Requête directe : un non-HTML est enregistré comme média, un HTML est lu
/// comme page (borné) pour que l'appelant en extraie les balises Open Graph.
async fn download_http_response(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
    headers: &[(&str, &str)],
) -> MediaResult<DirectResponse> {
    let (response, final_url) = http::send_guarded(
        &config.download,
        reqwest::Method::GET,
        url,
        config.download.timeout,
        headers,
    )
    .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(http::request_error_for_status(status.as_u16(), url));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if is_html_content_type(content_type.as_deref()) {
        let body = http::read_text(response).await?;
        return Ok(DirectResponse::Html {
            body,
            status: status.as_u16(),
        });
    }
    let extension = content_type
        .as_deref()
        .and_then(extension_from_mime)
        .unwrap_or("bin");
    let sequence = DOWNLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = temp_dir.join(format!("download-{sequence}.{extension}"));
    if let Some(parent) = path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "création du dossier temporaire",
                &error,
            ));
        }
    }
    let mut file = match tokio::fs::File::create(&path).await {
        Ok(file) => file,
        Err(error) => {
            return Err(MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "création du fichier temporaire",
                &error,
            ));
        }
    };
    let mut total: u64 = 0;
    let mut header = Vec::new();
    let mut response = response;
    let streaming = async {
        while let Some(chunk) = response.chunk().await.map_err(http::request_error)? {
            total = total.saturating_add(chunk.len() as u64);
            if total > config.download.max_size_bytes {
                return Err(MediaError::new(
                    MediaErrorCode::FileTooLarge,
                    "le média dépasse la taille maximale autorisée",
                ));
            }
            if header.len() < 32 {
                let take = chunk.len().min(32 - header.len());
                header.extend_from_slice(&chunk[..take]);
            }
            file.write_all(&chunk).await.map_err(|error| {
                MediaError::from_io(
                    MediaErrorCode::DownloadFailed,
                    "écriture du fichier téléchargé",
                    &error,
                )
            })?;
        }
        file.flush().await.map_err(|error| {
            MediaError::from_io(
                MediaErrorCode::DownloadFailed,
                "finalisation du fichier téléchargé",
                &error,
            )
        })?;
        Ok::<(), MediaError>(())
    }
    .await;
    if let Err(error) = streaming {
        let _removed = fs::remove_file(&path);
        return Err(error);
    }
    if total == 0 {
        let _removed = fs::remove_file(&path);
        return Err(MediaError::new(
            MediaErrorCode::InvalidMedia,
            "le téléchargement n'a produit aucun octet",
        ));
    }
    let kind = content_type
        .as_deref()
        .and_then(kind_from_mime)
        .or_else(|| sniff_kind(&header))
        .or_else(|| kind_from_url(url));
    let mime = content_type
        .clone()
        .or_else(|| kind.map(|kind| default_mime(kind).to_string()));
    Ok(DirectResponse::Media(DownloadResult {
        path,
        kind,
        mime,
        size_bytes: total,
        source_url: final_url.to_string(),
        title: None,
        text_chars: None,
    }))
}

/// Une page est du HTML si son type l'annonce ; `application/xhtml+xml` est du
/// HTML servi sous un autre nom MIME.
fn is_html_content_type(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|content_type| {
        let content_type = content_type.to_ascii_lowercase();
        content_type.contains("text/html") || content_type.contains("application/xhtml+xml")
    })
}

fn html_page_error(status: u16) -> MediaError {
    MediaError::new(
        MediaErrorCode::DownloadFailed,
        format!("la source a renvoyé une page HTML au lieu d'un média (HTTP {status})"),
    )
}

/// Télécharge une publication TikTok (vidéo, image ou carrousel assemblé).
///
/// Les URL signées du CDN exigent les cookies de session de la page et un
/// Referer, et le CDN répond parfois 403 à un candidat valide : chaque
/// candidat est essayé, puis `yt-dlp` sert de repli pour les vidéos.
pub async fn download_tiktok(
    source: &MediaSource,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    if let Some(comment_id) = tiktok_comment_id(&source.canonical_url) {
        match tiktok_comment::download_tiktok_comment(source, &comment_id, config, temp_dir).await {
            Ok(download) => return Ok(download),
            Err(error) => tracing::warn!(
                %comment_id,
                "carte du commentaire TikTok indisponible, repli sur la vidéo : {error}"
            ),
        }
    }
    download_tiktok_media(source, config, temp_dir, false).await
}

/// Télécharge le média d'une page TikTok (vidéo, yt-dlp ou carrousel).
///
/// `prefer_video` force un carrousel photo en vidéo (diaporama), pour pouvoir
/// l'incruster dans une carte de commentaire.
pub(crate) async fn download_tiktok_media(
    source: &MediaSource,
    config: &MediaConfig,
    temp_dir: &Path,
    prefer_video: bool,
) -> MediaResult<DownloadResult> {
    let page = tiktok::fetch_tiktok_page(config, &source.canonical_url).await?;
    let mut last_error: Option<MediaError> = None;
    for candidate in &page.video_candidates {
        let owned_headers = tiktok::media_headers(config, &page, candidate);
        let headers = tiktok::header_pairs(&owned_headers);
        match download_http(config, &candidate.url, temp_dir, &headers).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                if error.is(MediaErrorCode::FileTooLarge) {
                    return Err(error);
                }
                last_error = Some(match last_error {
                    Some(previous) => previous.most_relevant(error),
                    None => error,
                });
            }
        }
    }
    if !page.photo_slides.is_empty() {
        match download_tiktok_carousel(&page, config, temp_dir, prefer_video).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                last_error = Some(match last_error {
                    Some(previous) => previous.most_relevant(error),
                    None => error,
                });
            }
        }
    } else {
        let options = YtdlpOptions::default();
        match download_with_ytdlp(config, &source.canonical_url, temp_dir, &options).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                tracing::debug!("repli yt-dlp TikTok échoué : {error}");
                last_error = Some(match last_error {
                    Some(previous) => previous.most_relevant(error),
                    None => error,
                });
            }
        }
    }
    if page.auth_required {
        return Err(MediaError::new(
            MediaErrorCode::PrivateOrAuthRequired,
            "contenu TikTok privé ou nécessitant une connexion",
        ));
    }
    Err(classify_public_tiktok_failure(last_error.unwrap_or_else(
        || {
            MediaError::new(
                MediaErrorCode::DownloadFailed,
                "aucun média TikTok téléchargeable",
            )
        },
    )))
}

/// Reclasse l'échec d'une page publique dont le CDN a refusé l'URL signée.
///
/// Un `401`/`403` HTTP du CDN donne `PrivateOrAuthRequired` par défaut ; sur
/// une page publique ce n'est pas un contenu privé mais un téléchargement
/// refusé (cookies de session absents ou invalides). Le message conserve le
/// statut HTTP pour les journaux. Un contenu réellement privé signalé par
/// `yt-dlp` (« Private video ») n'est pas un refus HTTP et reste inchangé.
fn classify_public_tiktok_failure(error: MediaError) -> MediaError {
    let http_refusal = error.message().contains("HTTP 401") || error.message().contains("HTTP 403");
    if !error.is(MediaErrorCode::PrivateOrAuthRequired) || !http_refusal {
        return error;
    }
    MediaError::with_raw(
        MediaErrorCode::DownloadFailed,
        error.message().to_string(),
        error.raw_message().unwrap_or_default(),
    )
}

async fn download_tiktok_carousel(
    page: &tiktok::TikTokPage,
    config: &MediaConfig,
    temp_dir: &Path,
    force_video: bool,
) -> MediaResult<DownloadResult> {
    let mut images: Vec<PathBuf> = Vec::new();
    let mut last_error: Option<MediaError> = None;
    for slide in &page.photo_slides {
        if images.len() >= tiktok::MAX_CAROUSEL_IMAGES {
            break;
        }
        for candidate in slide {
            let owned_headers = tiktok::media_headers(config, page, candidate);
            let headers = tiktok::header_pairs(&owned_headers);
            match download_http(config, &candidate.url, temp_dir, &headers).await {
                Ok(download) => {
                    images.push(download.path);
                    break;
                }
                Err(error) => {
                    last_error = Some(match last_error {
                        Some(previous) => previous.most_relevant(error),
                        None => error,
                    });
                }
            }
        }
    }
    if images.is_empty() {
        return Err(last_error.unwrap_or_else(|| {
            MediaError::new(
                MediaErrorCode::DownloadFailed,
                "aucune image de carrousel TikTok téléchargeable",
            )
        }));
    }
    if images.len() == 1 && page.audio_candidates.is_empty() && !force_video {
        let path = images.remove(0);
        let size_bytes = MediaStorage::file_size(&path)?;
        let header = read_header_bytes(&path, 32);
        return Ok(DownloadResult {
            path,
            kind: sniff_kind(&header).or(Some(MediaKind::Image)),
            mime: Some("image/jpeg".to_string()),
            size_bytes,
            source_url: page
                .canonical_url
                .clone()
                .unwrap_or_else(|| "tiktok".to_string()),
            title: None,
            text_chars: None,
        });
    }
    let mut audio: Option<PathBuf> = None;
    for candidate in &page.audio_candidates {
        let owned_headers = tiktok::media_headers(config, page, candidate);
        let headers = tiktok::header_pairs(&owned_headers);
        match download_http(config, &candidate.url, temp_dir, &headers).await {
            Ok(download) => {
                audio = Some(download.path);
                break;
            }
            Err(error) => {
                tracing::debug!("piste audio TikTok indisponible : {error}");
            }
        }
    }
    build_carousel_download(
        config,
        temp_dir,
        &images,
        audio.as_deref(),
        true,
        page.canonical_url
            .clone()
            .unwrap_or_else(|| "tiktok".to_string()),
        None,
    )
    .await
}

/// Écrit un fichier de cookies Netscape pour `yt-dlp` à partir d'un en-tête Cookie.
pub fn write_cookie_file(temp_dir: &Path, cookie_header: &str) -> MediaResult<PathBuf> {
    let path = temp_dir.join("cookies.txt");
    let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];
    for (name, value) in parse_cookie_pairs(cookie_header) {
        lines.push(format!(
            ".tiktok.com\tTRUE\t/\tTRUE\t2147483647\t{name}\t{value}"
        ));
    }
    fs::write(&path, lines.join("\n")).map_err(|error| {
        MediaError::from_io(
            MediaErrorCode::DownloadFailed,
            "écriture du fichier de cookies",
            &error,
        )
    })?;
    Ok(path)
}

/// Parse un en-tête `Cookie` en paires nom/valeur (dernier gagne).
pub fn parse_cookie_pairs(cookie_header: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for part in cookie_header.split(';') {
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let value = value.trim();
        match pairs.iter_mut().find(|(known, _)| known == name) {
            Some(existing) => existing.1 = value.to_string(),
            None => pairs.push((name.to_string(), value.to_string())),
        }
    }
    pairs
}

/// Détecte le type de média à partir des octets d'en-tête (magic bytes).
pub fn sniff_kind(bytes: &[u8]) -> Option<MediaKind> {
    if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some(MediaKind::Image);
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some(MediaKind::Image);
    }
    if bytes.len() >= 4 && (&bytes[..4] == b"GIF8") {
        return Some(MediaKind::Image);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(MediaKind::Image);
    }
    if bytes.len() >= 2 && &bytes[..2] == b"BM" {
        return Some(MediaKind::Image);
    }
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return Some(MediaKind::Video);
    }
    if bytes.len() >= 4 && bytes[..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return Some(MediaKind::Video);
    }
    if bytes.len() >= 4 && &bytes[..4] == b"OggS" {
        return Some(MediaKind::Audio);
    }
    if bytes.len() >= 4 && &bytes[..4] == b"fLaC" {
        return Some(MediaKind::Audio);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some(MediaKind::Audio);
    }
    if bytes.len() >= 3 && &bytes[..3] == b"ID3" {
        return Some(MediaKind::Audio);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
        return Some(MediaKind::Audio);
    }
    None
}

/// Type MIME par défaut d'un type de média.
pub fn default_mime(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image/jpeg",
        MediaKind::Audio => "audio/mpeg",
        MediaKind::Video => "video/mp4",
    }
}

/// Normalise la classification d'une URL en [`SourceKind`] (utilitaire).
pub fn classify(url: &str) -> SourceKind {
    classify_source(url)
}

/// `true` si l'URL est un média YouTube (utilitaire).
pub fn is_youtube(url: &str) -> bool {
    is_youtube_url(url)
}

fn read_header_bytes(path: &Path, length: usize) -> Vec<u8> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut buffer = vec![0_u8; length];
    match file.read(&mut buffer) {
        Ok(read) => {
            buffer.truncate(read);
            buffer
        }
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn card_image_conversion_keeps_small_images_at_their_size() {
        let args = card_image_png_args(Path::new("photo.jpg"), Path::new("photo.png"));
        let filter = args
            .windows(2)
            .find(|pair| pair[0] == "-vf")
            .map(|pair| pair[1].as_str())
            .unwrap();
        assert_eq!(filter, CARD_IMAGE_SCALE_FILTER);
        // Régression : `scale=1800:1800:force_original_aspect_ratio=decrease`
        // agrandissait les images plus petites que la boîte (1234×1326 devenait
        // 1675×1800). Les bornes doivent rester relatives à l'image source.
        assert!(filter.contains("min(iw,1800)") && filter.contains("min(ih,1800)"));
        assert!(
            !filter.contains("1800:1800"),
            "filtre agrandissant : {filter}"
        );
        assert_eq!(args.last().unwrap(), "photo.png");
    }

    #[test]
    fn builds_ytdlp_args_with_flags() {
        let config = MediaConfig::default();
        let temp_dir = Path::new("C:\\temp\\lc-media");
        let args = build_ytdlp_args(
            &config,
            "https://youtu.be/abc",
            temp_dir,
            &YtdlpOptions::default(),
        );
        assert!(args.contains(&"--no-playlist".to_string()));
        assert!(args.contains(&"--no-progress".to_string()));
        assert!(args.contains(&"--no-warnings".to_string()));
        // Le titre est imprimé avant le chemin : la recherche du chemin part
        // de la fin, la ligne de chemin reste donc la dernière candidate.
        let title_print = args
            .windows(2)
            .position(|pair| pair[0] == "--print" && pair[1] == "after_move:%(title)s")
            .expect("print du titre");
        let path_print = args
            .windows(2)
            .position(|pair| pair[0] == "--print" && pair[1] == "after_move:filepath")
            .expect("print du chemin");
        assert!(title_print < path_print, "ordre des --print : {args:?}");
        assert!(args.contains(&"--merge-output-format".to_string()));
        assert!(args.contains(&"2048M".to_string()));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-o" && pair[1].contains("download.%(ext)s")));
        assert_eq!(args.last().unwrap(), "https://youtu.be/abc");
        let android = YtdlpOptions {
            youtube_player_client: Some("android".to_string()),
            merge: false,
            ..YtdlpOptions::default()
        };
        let args = build_ytdlp_args(&config, "https://youtu.be/abc", temp_dir, &android);
        assert!(args.contains(&"youtube:player_client=android".to_string()));
        assert!(!args.contains(&"--merge-output-format".to_string()));
    }

    #[test]
    fn parses_ytdlp_output_paths() {
        // Absolute on every platform: a hard-coded `C:\…` path is relative on Linux.
        let temp_dir_buf = std::env::temp_dir().join("lc-media");
        let temp_dir = temp_dir_buf.as_path();
        let absolute = temp_dir.join("download.mp4");
        assert_eq!(
            parse_ytdlp_output_path(&absolute.to_string_lossy(), temp_dir),
            Some(absolute)
        );
        assert_eq!(
            parse_ytdlp_output_path("download.mkv\n", temp_dir),
            Some(temp_dir.join("download.mkv"))
        );
        let json_path = temp_dir.join("a.mp4");
        let json_line = serde_json::json!({ "_filename": json_path.to_string_lossy() }).to_string();
        assert_eq!(
            parse_ytdlp_output_path(&json_line, temp_dir),
            Some(json_path)
        );
        assert_eq!(
            parse_ytdlp_output_path("[youtube] extraction", temp_dir),
            None
        );
        assert_eq!(parse_ytdlp_output_path("", temp_dir), None);
    }

    #[test]
    fn parses_ytdlp_titles_with_paths() {
        let temp_dir_buf = std::env::temp_dir().join("lc-media");
        let temp_dir = temp_dir_buf.as_path();
        let download = temp_dir.join("download.mp4");

        // Sortie attendue : titre puis chemin.
        let stdout = format!("Chat qui tombe\n{}\n", download.to_string_lossy());
        assert_eq!(
            parse_ytdlp_output(&stdout, temp_dir),
            YtdlpOutput {
                path: Some(download.clone()),
                title: Some("Chat qui tombe".to_string()),
            }
        );

        // Titre absent (champ inconnu de yt-dlp) : « NA ».
        let stdout = format!("NA\n{}\n", download.to_string_lossy());
        assert_eq!(
            parse_ytdlp_output(&stdout, temp_dir),
            YtdlpOutput {
                path: Some(download),
                title: None,
            }
        );

        // Journal yt-dlp seul : ni chemin ni titre.
        assert_eq!(
            parse_ytdlp_output("[youtube] abc: downloading webpage\n", temp_dir),
            YtdlpOutput {
                path: None,
                title: None,
            }
        );

        // Chemin relatif `download.<ext>` : le titre est la ligne précédente.
        let output = parse_ytdlp_output("Mon titre\n\ndownload.webm\n", temp_dir);
        assert_eq!(output.path, Some(temp_dir.join("download.webm")));
        assert_eq!(output.title.as_deref(), Some("Mon titre"));
    }

    #[test]
    fn parses_ytdlp_titles_robustly() {
        let temp_dir_buf = std::env::temp_dir().join("lc-media");
        let temp_dir = temp_dir_buf.as_path();
        let download = temp_dir.join("download.mp4");

        // Un titre multi-ligne est réassemblé.
        let stdout = format!("ligne une\nligne deux\n{}\n", download.to_string_lossy());
        assert_eq!(
            parse_ytdlp_output(&stdout, temp_dir).title.as_deref(),
            Some("ligne une\nligne deux")
        );

        // Un titre ressemblant à un chemin ne détourne pas la recherche du
        // chemin, qui part de la fin.
        let stdout = format!("download.mp4\n{}\n", download.to_string_lossy());
        let output = parse_ytdlp_output(&stdout, temp_dir);
        assert_eq!(output.path, Some(download.clone()));
        assert_eq!(output.title.as_deref(), Some("download.mp4"));

        // Empty title then path: no title.
        let stdout = format!("\n{}\n", download.to_string_lossy());
        assert_eq!(parse_ytdlp_output(&stdout, temp_dir).title, None);
    }

    #[test]
    fn finds_newest_downloaded_file() {
        let directory = tempfile::TempDir::new().unwrap();
        assert_eq!(find_downloaded_file(directory.path()).unwrap(), None);
        std::fs::write(directory.path().join("download.part"), b"x").unwrap();
        std::fs::write(directory.path().join("download.mp4"), b"video").unwrap();
        std::fs::write(directory.path().join(".cache"), b"hidden").unwrap();
        let found = find_downloaded_file(directory.path()).unwrap();
        assert_eq!(found, Some(directory.path().join("download.mp4")));
    }

    #[test]
    fn sniffs_magic_bytes() {
        assert_eq!(sniff_kind(b"\x89PNG\r\n\x1a\nrest"), Some(MediaKind::Image));
        assert_eq!(
            sniff_kind(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            Some(MediaKind::Image)
        );
        assert_eq!(sniff_kind(b"GIF89a"), Some(MediaKind::Image));
        assert_eq!(
            sniff_kind(&[0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p']),
            Some(MediaKind::Video)
        );
        assert_eq!(
            sniff_kind(&[0x1A, 0x45, 0xDF, 0xA3, 0x00]),
            Some(MediaKind::Video)
        );
        assert_eq!(sniff_kind(b"ID3\x04\x00"), Some(MediaKind::Audio));
        assert_eq!(sniff_kind(b"OggS...."), Some(MediaKind::Audio));
        assert_eq!(sniff_kind(b"fLaC....."), Some(MediaKind::Audio));
        assert_eq!(sniff_kind(b"RIFFxxxxWAVE"), Some(MediaKind::Audio));
        assert_eq!(
            sniff_kind(&[0xFF, 0xFB, 0x90, 0x00]),
            Some(MediaKind::Audio)
        );
        assert_eq!(sniff_kind(b"pas un media"), None);
        assert_eq!(sniff_kind(&[]), None);
    }

    #[test]
    fn cookies_are_parsed_and_written() {
        let pairs = parse_cookie_pairs("a=1; b=2; a=3; invalid; c=");
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), "3".to_string()),
                ("b".to_string(), "2".to_string()),
                ("c".to_string(), String::new())
            ]
        );
        let directory = tempfile::TempDir::new().unwrap();
        let path = write_cookie_file(directory.path(), "sessionid=abc; sid_tt=def").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(".tiktok.com\tTRUE\t/\tTRUE\t2147483647\tsessionid\tabc"));
        assert!(content.contains("sid_tt\tdef"));
    }

    #[test]
    fn public_cdn_rejection_is_a_download_failure_with_its_status() {
        let forbidden = MediaError::with_raw(
            MediaErrorCode::PrivateOrAuthRequired,
            "la source a répondu HTTP 403 (https://v16.test/play.mp4)",
            "forbidden",
        );
        let classified = classify_public_tiktok_failure(forbidden);
        assert_eq!(classified.code(), MediaErrorCode::DownloadFailed);
        assert!(classified.message().contains("HTTP 403"));
        assert_eq!(classified.raw_message(), Some("forbidden"));

        let timeout = MediaError::new(MediaErrorCode::DownloadTimeout, "trop long");
        assert_eq!(classify_public_tiktok_failure(timeout.clone()), timeout);

        // Un contenu réellement privé signalé par yt-dlp reste classé privé.
        let private = MediaError::new(
            MediaErrorCode::PrivateOrAuthRequired,
            "ERROR: [tiktok] 123: Private video. Sign in if you've been granted access",
        );
        assert_eq!(classify_public_tiktok_failure(private.clone()), private);
    }

    #[test]
    fn keeps_the_ytdlp_failure_visible_when_the_page_has_no_media() {
        let page = html_page_error(200);
        let ytdlp = MediaError::new(
            MediaErrorCode::DownloadFailed,
            "impossible de lancer yt-dlp : programme introuvable",
        );
        let selected = web_fallback_error(page, ytdlp.clone());
        assert_eq!(selected.message(), ytdlp.message());

        // Une erreur de page plus prioritaire (fichier trop gros) reste remontée.
        let page = MediaError::new(MediaErrorCode::FileTooLarge, "trop gros");
        let selected = web_fallback_error(page.clone(), ytdlp);
        assert_eq!(selected, page);
    }

    #[test]
    fn recognizes_html_content_types_only() {
        assert!(is_html_content_type(Some("text/html; charset=utf-8")));
        assert!(is_html_content_type(Some("application/xhtml+xml")));
        assert!(!is_html_content_type(Some("image/jpeg")));
        assert!(!is_html_content_type(Some("video/mp4")));
        assert!(!is_html_content_type(None));
    }

    #[test]
    fn classifies_helpers() {
        assert_eq!(
            classify("https://www.youtube.com/watch?v=abc"),
            SourceKind::YouTube
        );
        assert!(is_youtube("https://youtu.be/abc"));
        assert_eq!(default_mime(MediaKind::Video), "video/mp4");
        assert_eq!(default_mime(MediaKind::Audio), "audio/mpeg");
        assert_eq!(default_mime(MediaKind::Image), "image/jpeg");
    }
}
