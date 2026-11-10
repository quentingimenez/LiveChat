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
use crate::resolve::tiktok;
use crate::source::{
    classify_source, extension_from_mime, is_tiktok_url, is_youtube_url, kind_from_mime,
    kind_from_url, mime_from_extension, MediaSource, SourceKind,
};
use crate::storage::MediaStorage;
use crate::transcode;

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
        SourceKind::LocalFile => Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "les sources locales (file:) sont refusées par la politique média",
        )),
        SourceKind::Gtts => Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "la source gtts n'est pas prise en charge par lc-media (Phase 5b)",
        )),
        SourceKind::DirectMedia | SourceKind::Unknown => {
            download_http(config, &source.canonical_url, temp_dir, &[]).await
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
        | SourceKind::DirectMedia => {
            guard::validate_target(&source.canonical_url).await?;
            Ok(())
        }
    }
}

async fn download_twitter(
    source: &MediaSource,
    resolved: Option<&crate::resolve::ResolvedMedia>,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    if let Some(media_url) = resolved.and_then(|resolved| resolved.media_url.clone()) {
        match download_http(config, &media_url, temp_dir, &[]).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                tracing::debug!(%media_url, "téléchargement direct du tweet échoué : {error}");
            }
        }
    }
    let options = YtdlpOptions::default();
    download_with_ytdlp(config, &source.canonical_url, temp_dir, &options).await
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
    args.push(url.to_string());
    args
}

/// Parse le chemin de sortie imprimé par `yt-dlp`.
pub fn parse_ytdlp_output_path(stdout: &str, temp_dir: &Path) -> Option<PathBuf> {
    for line in stdout.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('[') {
            continue;
        }
        if trimmed.starts_with('{') {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(path) = value.get("_filename").and_then(|path| path.as_str()) {
                    let path = PathBuf::from(path.trim());
                    if path.is_absolute() {
                        return Some(path);
                    }
                    return Some(temp_dir.join(path));
                }
            }
            continue;
        }
        if trimmed.starts_with("download.") {
            return Some(temp_dir.join(trimmed));
        }
        let path = PathBuf::from(trimmed);
        if path.is_absolute() || path.exists() {
            return Some(path);
        }
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
    let path = match parse_ytdlp_output_path(&output.stdout, temp_dir) {
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
    })
}

/// Télécharge une URL directe en streaming, avec plafond de taille.
pub async fn download_http(
    config: &MediaConfig,
    url: &str,
    temp_dir: &Path,
    headers: &[(&str, &str)],
) -> MediaResult<DownloadResult> {
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
    if content_type
        .as_deref()
        .is_some_and(|content_type| content_type.to_ascii_lowercase().contains("text/html"))
    {
        return Err(MediaError::new(
            MediaErrorCode::UnsupportedSource,
            "URL points to an HTML page, not a direct media file",
        ));
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
    Ok(DownloadResult {
        path,
        kind,
        mime,
        size_bytes: total,
        source_url: final_url.to_string(),
    })
}

/// Télécharge une publication TikTok (vidéo, image ou carrousel assemblé).
pub async fn download_tiktok(
    source: &MediaSource,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    let page = tiktok::fetch_tiktok_page(config, &source.canonical_url).await?;
    let owned_headers = tiktok::tiktok_headers(config);
    let headers = tiktok::header_pairs(&owned_headers);
    let mut last_error: Option<MediaError> = None;
    for candidate in &page.video_candidates {
        match download_http(config, &candidate.url, temp_dir, &headers).await {
            Ok(download) => return Ok(download),
            Err(error) => {
                if error.is(MediaErrorCode::FileTooLarge)
                    || error.is(MediaErrorCode::PrivateOrAuthRequired)
                {
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
        match download_tiktok_carousel(&page, config, temp_dir).await {
            Ok(download) => return Ok(download),
            Err(error) => {
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
    Err(last_error.unwrap_or_else(|| {
        MediaError::new(
            MediaErrorCode::DownloadFailed,
            "aucun média TikTok téléchargeable",
        )
    }))
}

async fn download_tiktok_carousel(
    page: &tiktok::TikTokPage,
    config: &MediaConfig,
    temp_dir: &Path,
) -> MediaResult<DownloadResult> {
    let owned_headers = tiktok::tiktok_headers(config);
    let headers = tiktok::header_pairs(&owned_headers);
    let mut images: Vec<PathBuf> = Vec::new();
    let mut last_error: Option<MediaError> = None;
    for slide in &page.photo_slides {
        if images.len() >= tiktok::MAX_CAROUSEL_IMAGES {
            break;
        }
        for candidate in slide {
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
    if images.len() == 1 && page.audio_candidates.is_empty() {
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
        });
    }
    let mut audio: Option<PathBuf> = None;
    for candidate in &page.audio_candidates {
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
    let output = temp_dir.join("carousel.mp4");
    let dimensions = transcode::carousel_dimensions(config.video.max_height, true);
    transcode::build_carousel_video(
        &config.tools.ffmpeg,
        &images,
        audio.as_deref(),
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
        source_url: page
            .canonical_url
            .clone()
            .unwrap_or_else(|| "tiktok".to_string()),
    })
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
        assert!(args.contains(&"after_move:filepath".to_string()));
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
        let temp_dir = Path::new("C:\\temp\\lc-media");
        assert_eq!(
            parse_ytdlp_output_path("C:\\temp\\lc-media\\download.mp4", temp_dir),
            Some(PathBuf::from("C:\\temp\\lc-media\\download.mp4"))
        );
        assert_eq!(
            parse_ytdlp_output_path("download.mkv\n", temp_dir),
            Some(temp_dir.join("download.mkv"))
        );
        assert_eq!(
            parse_ytdlp_output_path(
                "{\"_filename\": \"C:\\\\temp\\\\lc-media\\\\a.mp4\"}",
                temp_dir
            ),
            Some(PathBuf::from("C:\\temp\\lc-media\\a.mp4"))
        );
        assert_eq!(
            parse_ytdlp_output_path("[youtube] extraction", temp_dir),
            None
        );
        assert_eq!(parse_ytdlp_output_path("", temp_dir), None);
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
