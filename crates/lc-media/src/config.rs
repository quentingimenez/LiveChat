//! Configuration typée du pipeline média.
//!
//! Toutes les valeurs viennent de variables d'environnement (mêmes noms que
//! l'ancien serveur Node). Les fonctions de test utilisent [`MediaConfig::from_pairs`]
//! pour ne jamais dépendre de l'environnement global du processus.
//!
//! Les durées sont exposées en [`std::time::Duration`], les tailles en octets.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Taille d'un mégaoctet utilisée par les quotas (convention legacy : 1 MiB).
pub const BYTES_PER_MB: u64 = 1024 * 1024;

/// Répertoire de stockage par défaut (relatif au répertoire de travail).
pub const DEFAULT_STORAGE_DIR: &str = "./data/media";

/// Format `yt-dlp` compatible H.264/AAC (sélecteur legacy historique).
pub const DEFAULT_YTDLP_FORMAT: &str = "bv*[vcodec^=avc1][ext=mp4][height<=1080]+ba[ext=m4a]/b[vcodec^=avc1][ext=mp4][height<=1080]/bv*[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4][height<=1080]/bv*[height<=1080]+ba/b[height<=1080]/best";

/// Format `yt-dlp` relâché utilisé pour les nouvelles tentatives YouTube.
pub const YOUTUBE_RELAXED_YTDLP_FORMAT: &str = "b/bv*+ba/best";

/// User-agent utilisé pour récupérer les pages TikTok (desktop Chrome récent).
pub const TIKTOK_PAGE_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36";

/// Timeout de résolution des URL TikTok courtes (constante historique).
pub const SHORT_URL_RESOLVE_TIMEOUT: Duration = Duration::from_secs(8);

/// Anciens sélecteurs de format, remplacés par [`DEFAULT_YTDLP_FORMAT`].
const LEGACY_YTDLP_FORMATS: &[&str] = &[
    "bv*[height<=1080]+ba/b[height<=1080]/best",
    "bv*[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4][height<=1080]/bv*[height<=1080]+ba/b[height<=1080]/best",
    "b[ext=mp4][height<=1080]/b[height<=1080]/best",
];

/// Sous-configuration du stockage et des quotas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageConfig {
    /// Racine du stockage content-addressed (fichiers nommés `<sourceHash>.<ext>`).
    pub directory: PathBuf,
    /// Durée de vie d'un média en cache.
    pub cache_ttl: Duration,
    /// Quota total du cache non épinglé (médias absents du board).
    pub cache_max_total_bytes: u64,
    /// Quota total des médias épinglés sur un board.
    pub board_max_total_bytes: u64,
}

impl StorageConfig {
    /// Durée de vie exprimée en heures.
    pub fn cache_ttl_hours(&self) -> u64 {
        self.cache_ttl.as_secs() / 3600
    }
}

/// Sous-configuration du téléchargement HTTP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadConfig {
    /// Timeout global d'un téléchargement.
    pub timeout: Duration,
    /// Timeout de résolution d'une URL courte.
    pub short_url_timeout: Duration,
    /// Taille maximale d'un média téléchargé.
    pub max_size_bytes: u64,
    /// Nombre maximal d'ingestions simultanées (sémaphore du worker Phase 5b).
    pub max_concurrent_ingestions: usize,
    /// User-agent par défaut des requêtes HTTP.
    pub user_agent: String,
}

/// Encodeur vidéo demandé par la configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoEncoder {
    /// Détecte NVENC à l'exécution, retombe sur `libx264` sinon.
    #[default]
    Auto,
    /// Encodeur logiciel `libx264`.
    LibX264,
    /// Encodeur matériel `h264_nvenc`.
    H264Nvenc,
}

impl VideoEncoder {
    /// Analyse la variable `MEDIA_VIDEO_ENCODER` (`auto`, `libx264`, `h264_nvenc`).
    pub fn from_env_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "libx264" | "cpu" | "x264" => Self::LibX264,
            "h264_nvenc" | "nvenc" | "gpu" => Self::H264Nvenc,
            _ => Self::Auto,
        }
    }

    /// Représentation textuelle utilisée en configuration.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::LibX264 => "libx264",
            Self::H264Nvenc => "h264_nvenc",
        }
    }

    /// Nom de l'encodeur ffmpeg correspondant (`None` pour [`Self::Auto`]).
    pub const fn ffmpeg_name(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::LibX264 => Some("libx264"),
            Self::H264Nvenc => Some("h264_nvenc"),
        }
    }
}

/// Cibles du filtre `loudnorm` (EBU R128).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnormTargets {
    /// Loudness intégrée cible en LUFS (`MEDIA_AUDIO_LOUDNORM_I`, défaut `-18`).
    pub integrated_lufs: f64,
    /// Plage loudness cible en LU (`MEDIA_AUDIO_LOUDNORM_LRA`, défaut `11`).
    pub range_lu: f64,
    /// True peak cible en dBTP (`MEDIA_AUDIO_LOUDNORM_TP`, défaut `-1.5`).
    pub true_peak_dbtp: f64,
}

impl LoudnormTargets {
    /// Filtre ffmpeg `loudnorm=I=…:LRA=…:TP=…`.
    pub fn filter(&self) -> String {
        format!(
            "loudnorm=I={}:LRA={}:TP={}",
            format_number(self.integrated_lufs),
            format_number(self.range_lu),
            format_number(self.true_peak_dbtp)
        )
    }
}

impl Default for LoudnormTargets {
    fn default() -> Self {
        Self {
            integrated_lufs: -18.0,
            range_lu: 11.0,
            true_peak_dbtp: -1.5,
        }
    }
}

/// Sous-configuration du transcodage vidéo/audio.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoConfig {
    /// Hauteur maximale conservée (`MEDIA_VIDEO_MAX_HEIGHT`, défaut `1080`).
    pub max_height: u32,
    /// Encodeur demandé (`MEDIA_VIDEO_ENCODER`, défaut `auto`).
    pub encoder: VideoEncoder,
    /// Preset x264 (`MEDIA_VIDEO_PRESET`, défaut `ultrafast`).
    pub cpu_preset: String,
    /// Preset NVENC (`MEDIA_VIDEO_NVENC_PRESET`, défaut `p4`).
    pub nvenc_preset: String,
    /// Active le filtre `loudnorm` (`MEDIA_AUDIO_NORMALIZE_ENABLED`, défaut `true`).
    pub audio_normalize: bool,
    /// Cibles `loudnorm`.
    pub loudnorm: LoudnormTargets,
    /// Durée maximale optionnelle (`MEDIA_VIDEO_MAX_DURATION_SEC`).
    pub max_duration_sec: Option<u32>,
}

/// Sous-configuration des binaires externes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolsConfig {
    /// Binaire ffmpeg (`FFMPEG_BINARY`, défaut `ffmpeg`).
    pub ffmpeg: PathBuf,
    /// Binaire ffprobe (`FFPROBE_BINARY`, défaut `ffprobe`).
    pub ffprobe: PathBuf,
    /// Binaire yt-dlp (`YTDLP_BINARY`, défaut `yt-dlp`).
    pub ytdlp: PathBuf,
    /// Fragments téléchargés en parallèle par yt-dlp (`YTDLP_CONCURRENT_FRAGMENTS`, défaut `1`).
    pub ytdlp_concurrent_fragments: u32,
    /// Sélecteur de format yt-dlp (`YTDLP_FORMAT`).
    pub ytdlp_format: String,
    /// Cookie TikTok (`TIKTOK_COOKIE`), sans le préfixe `cookie:`.
    pub tiktok_cookie: Option<String>,
}

/// Sous-configuration de la résolution de sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveConfig {
    /// Endpoint oEmbed YouTube.
    pub youtube_oembed_endpoint: String,
    /// Endpoint oEmbed TikTok.
    pub tiktok_oembed_endpoint: String,
    /// Endpoint oEmbed Twitter/X.
    pub twitter_oembed_endpoint: String,
    /// Base du résolveur de secours X (vxtwitter).
    pub twitter_video_resolver_base_url: String,
    /// Durée de la carte tweet utilisée par le front.
    pub twitter_card_duration: Duration,
    /// Timeout des appels oEmbed (clampé entre 5 s et 12 s).
    pub oembed_timeout: Duration,
    /// Langue utilisée pour les cartes de tweet (`fr` ou `en`).
    pub language: String,
}

/// Configuration complète du pipeline média.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaConfig {
    /// Stockage et quotas.
    pub storage: StorageConfig,
    /// Téléchargement.
    pub download: DownloadConfig,
    /// Transcodage.
    pub video: VideoConfig,
    /// Binaires externes.
    pub tools: ToolsConfig,
    /// Résolution de sources.
    pub resolve: ResolveConfig,
}

impl MediaConfig {
    /// Charge la configuration depuis l'environnement du processus.
    pub fn from_env() -> Self {
        Self::from_pairs(std::env::vars())
    }

    /// Alias explicite de [`Self::from_env`].
    pub fn load() -> Self {
        Self::from_env()
    }

    /// Construit une configuration depuis une liste clé/valeur (tests, CLI, …).
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let vars: HashMap<String, String> = pairs
            .into_iter()
            .map(|(key, value)| (key.into().trim().to_ascii_uppercase(), value.into()))
            .collect();

        let storage = StorageConfig {
            directory: path_var(&vars, "MEDIA_STORAGE_DIR", DEFAULT_STORAGE_DIR),
            cache_ttl: Duration::from_secs(
                u64_var(&vars, "MEDIA_CACHE_TTL_HOURS", 12).max(1) * 3600,
            ),
            cache_max_total_bytes: u64_var(&vars, "MEDIA_CACHE_MAX_TOTAL_MB", 5120).max(1)
                * BYTES_PER_MB,
            board_max_total_bytes: u64_var(&vars, "MEDIA_BOARD_MAX_TOTAL_MB", 15360).max(1)
                * BYTES_PER_MB,
        };

        let timeout =
            Duration::from_millis(u64_var(&vars, "MEDIA_DOWNLOAD_TIMEOUT_MS", 180_000).max(1));
        let download = DownloadConfig {
            timeout,
            short_url_timeout: SHORT_URL_RESOLVE_TIMEOUT,
            max_size_bytes: u64_var(&vars, "MEDIA_MAX_SIZE_MB", 2048).max(1) * BYTES_PER_MB,
            max_concurrent_ingestions: usize::try_from(
                u64_var(&vars, "MEDIA_MAX_CONCURRENT_INGESTIONS", 2).max(1),
            )
            .unwrap_or(2),
            user_agent: string_var(&vars, "MEDIA_USER_AGENT", TIKTOK_PAGE_USER_AGENT),
        };

        let video = VideoConfig {
            max_height: u32::try_from(u64_var(&vars, "MEDIA_VIDEO_MAX_HEIGHT", 1080))
                .unwrap_or(1080),
            encoder: value(&vars, "MEDIA_VIDEO_ENCODER")
                .map(VideoEncoder::from_env_str)
                .unwrap_or_default(),
            cpu_preset: normalize_cpu_preset(&string_var(&vars, "MEDIA_VIDEO_PRESET", "ultrafast")),
            nvenc_preset: string_var(&vars, "MEDIA_VIDEO_NVENC_PRESET", "p4"),
            audio_normalize: bool_var(&vars, "MEDIA_AUDIO_NORMALIZE_ENABLED", true),
            loudnorm: LoudnormTargets {
                integrated_lufs: f64_var(&vars, "MEDIA_AUDIO_LOUDNORM_I", -18.0),
                range_lu: f64_var(&vars, "MEDIA_AUDIO_LOUDNORM_LRA", 11.0),
                true_peak_dbtp: f64_var(&vars, "MEDIA_AUDIO_LOUDNORM_TP", -1.5),
            },
            max_duration_sec: u32_var_optional(&vars, "MEDIA_VIDEO_MAX_DURATION_SEC"),
        };

        let tools = ToolsConfig {
            ffmpeg: path_var(&vars, "FFMPEG_BINARY", "ffmpeg"),
            ffprobe: path_var(&vars, "FFPROBE_BINARY", "ffprobe"),
            ytdlp: path_var(&vars, "YTDLP_BINARY", "yt-dlp"),
            ytdlp_concurrent_fragments: u32::try_from(
                u64_var(&vars, "YTDLP_CONCURRENT_FRAGMENTS", 1).max(1),
            )
            .unwrap_or(1),
            ytdlp_format: normalize_ytdlp_format(&string_var(
                &vars,
                "YTDLP_FORMAT",
                DEFAULT_YTDLP_FORMAT,
            )),
            tiktok_cookie: value(&vars, "TIKTOK_COOKIE")
                .map(str::trim)
                .filter(|cookie| !cookie.is_empty())
                .map(strip_cookie_prefix)
                .filter(|cookie| !cookie.is_empty()),
        };

        let resolve = ResolveConfig {
            youtube_oembed_endpoint: string_var(
                &vars,
                "YOUTUBE_OEMBED_ENDPOINT",
                "https://www.youtube.com/oembed",
            ),
            tiktok_oembed_endpoint: string_var(
                &vars,
                "TIKTOK_OEMBED_ENDPOINT",
                "https://www.tiktok.com/oembed",
            ),
            twitter_oembed_endpoint: string_var(
                &vars,
                "TWITTER_OEMBED_ENDPOINT",
                "https://publish.twitter.com/oembed",
            ),
            twitter_video_resolver_base_url: string_var(
                &vars,
                "TWITTER_VIDEO_RESOLVER_BASE_URL",
                "https://vxtwitter.com",
            ),
            twitter_card_duration: Duration::from_secs(
                u64_var(&vars, "TWITTER_CARD_DURATION_SEC", 15).max(1),
            ),
            oembed_timeout: clamp_oembed_timeout(timeout),
            language: language_from_i18n(&string_var(&vars, "I18N", "fr")),
        };

        Self {
            storage,
            download,
            video,
            tools,
            resolve,
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self::from_pairs(std::iter::empty::<(String, String)>())
    }
}

/// Variables d'environnement reconnues (documentation et diagnostics).
pub const ENV_KEYS: &[&str] = &[
    "MEDIA_STORAGE_DIR",
    "MEDIA_CACHE_TTL_HOURS",
    "MEDIA_CACHE_MAX_TOTAL_MB",
    "MEDIA_BOARD_MAX_TOTAL_MB",
    "MEDIA_DOWNLOAD_TIMEOUT_MS",
    "MEDIA_MAX_SIZE_MB",
    "MEDIA_MAX_CONCURRENT_INGESTIONS",
    "MEDIA_USER_AGENT",
    "MEDIA_VIDEO_MAX_HEIGHT",
    "MEDIA_VIDEO_ENCODER",
    "MEDIA_VIDEO_PRESET",
    "MEDIA_VIDEO_NVENC_PRESET",
    "MEDIA_VIDEO_MAX_DURATION_SEC",
    "MEDIA_AUDIO_NORMALIZE_ENABLED",
    "MEDIA_AUDIO_LOUDNORM_I",
    "MEDIA_AUDIO_LOUDNORM_LRA",
    "MEDIA_AUDIO_LOUDNORM_TP",
    "FFMPEG_BINARY",
    "FFPROBE_BINARY",
    "YTDLP_BINARY",
    "YTDLP_CONCURRENT_FRAGMENTS",
    "YTDLP_FORMAT",
    "TIKTOK_COOKIE",
    "YOUTUBE_OEMBED_ENDPOINT",
    "TIKTOK_OEMBED_ENDPOINT",
    "TWITTER_OEMBED_ENDPOINT",
    "TWITTER_VIDEO_RESOLVER_BASE_URL",
    "TWITTER_CARD_DURATION_SEC",
    "I18N",
];

fn value<'a>(vars: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    vars.get(key)
        .map(|raw| raw.trim())
        .filter(|raw| !raw.is_empty())
}

fn string_var(vars: &HashMap<String, String>, key: &str, default: &str) -> String {
    value(vars, key)
        .map(str::to_string)
        .unwrap_or_else(|| default.to_string())
}

fn path_var(vars: &HashMap<String, String>, key: &str, default: &str) -> PathBuf {
    PathBuf::from(string_var(vars, key, default))
}

fn u64_var(vars: &HashMap<String, String>, key: &str, default: u64) -> u64 {
    match value(vars, key).map(str::parse::<u64>) {
        Some(Ok(parsed)) => parsed,
        Some(Err(error)) => {
            tracing::warn!(variable = key, %error, "valeur numérique invalide, valeur par défaut utilisée");
            default
        }
        None => default,
    }
}

fn u32_var_optional(vars: &HashMap<String, String>, key: &str) -> Option<u32> {
    match value(vars, key).map(str::parse::<u32>) {
        Some(Ok(parsed)) => Some(parsed),
        Some(Err(error)) => {
            tracing::warn!(variable = key, %error, "valeur numérique invalide, variable ignorée");
            None
        }
        None => None,
    }
}

fn f64_var(vars: &HashMap<String, String>, key: &str, default: f64) -> f64 {
    match value(vars, key).map(str::parse::<f64>) {
        Some(Ok(parsed)) if parsed.is_finite() => parsed,
        Some(Ok(_)) => default,
        Some(Err(error)) => {
            tracing::warn!(variable = key, %error, "valeur numérique invalide, valeur par défaut utilisée");
            default
        }
        None => default,
    }
}

fn bool_var(vars: &HashMap<String, String>, key: &str, default: bool) -> bool {
    match value(vars, key).map(|raw| raw.to_ascii_lowercase()) {
        Some(raw) => match raw.as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        None => default,
    }
}

fn clamp_oembed_timeout(download_timeout: Duration) -> Duration {
    download_timeout.clamp(Duration::from_secs(5), Duration::from_secs(12))
}

fn language_from_i18n(i18n: &str) -> String {
    if i18n.trim().to_ascii_lowercase().starts_with("fr") {
        "fr".to_string()
    } else {
        "en".to_string()
    }
}

fn strip_cookie_prefix(cookie: &str) -> String {
    let trimmed = cookie.trim();
    let without_prefix = trimmed
        .strip_prefix("cookie:")
        .or_else(|| trimmed.strip_prefix("Cookie:"))
        .or_else(|| trimmed.strip_prefix("COOKIE:"))
        .unwrap_or(trimmed);
    without_prefix.trim().to_string()
}

fn normalize_cpu_preset(preset: &str) -> String {
    let trimmed = preset.trim();
    if trimmed.eq_ignore_ascii_case("superfast") {
        "ultrafast".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_ytdlp_format(format: &str) -> String {
    let trimmed = format.trim();
    if LEGACY_YTDLP_FORMATS.iter().any(|legacy| legacy == &trimmed) {
        DEFAULT_YTDLP_FORMAT.to_string()
    } else {
        trimmed.to_string()
    }
}

fn format_number(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{}", value.trunc())
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_legacy_environment() {
        let config = MediaConfig::default();
        assert_eq!(config.storage.directory, PathBuf::from(DEFAULT_STORAGE_DIR));
        assert_eq!(config.storage.cache_ttl, Duration::from_secs(12 * 3600));
        assert_eq!(config.storage.cache_max_total_bytes, 5120 * BYTES_PER_MB);
        assert_eq!(config.storage.board_max_total_bytes, 15360 * BYTES_PER_MB);
        assert_eq!(config.download.timeout, Duration::from_millis(180_000));
        assert_eq!(config.download.short_url_timeout, Duration::from_secs(8));
        assert_eq!(config.download.max_size_bytes, 2048 * BYTES_PER_MB);
        assert_eq!(config.download.max_concurrent_ingestions, 2);
        assert_eq!(config.video.max_height, 1080);
        assert_eq!(config.video.encoder, VideoEncoder::Auto);
        assert_eq!(config.video.cpu_preset, "ultrafast");
        assert_eq!(config.video.nvenc_preset, "p4");
        assert!(config.video.audio_normalize);
        assert_eq!(config.video.loudnorm, LoudnormTargets::default());
        assert_eq!(config.video.max_duration_sec, None);
        assert_eq!(config.tools.ffmpeg, PathBuf::from("ffmpeg"));
        assert_eq!(config.tools.ffprobe, PathBuf::from("ffprobe"));
        assert_eq!(config.tools.ytdlp, PathBuf::from("yt-dlp"));
        assert_eq!(config.tools.ytdlp_concurrent_fragments, 1);
        assert_eq!(config.tools.ytdlp_format, DEFAULT_YTDLP_FORMAT);
        assert_eq!(config.tools.tiktok_cookie, None);
        assert_eq!(
            config.resolve.twitter_oembed_endpoint,
            "https://publish.twitter.com/oembed"
        );
        assert_eq!(
            config.resolve.twitter_video_resolver_base_url,
            "https://vxtwitter.com"
        );
        assert_eq!(
            config.resolve.twitter_card_duration,
            Duration::from_secs(15)
        );
        assert_eq!(config.resolve.oembed_timeout, Duration::from_secs(12));
        assert_eq!(config.resolve.language, "fr");
    }

    #[test]
    fn overrides_are_parsed() {
        let config = MediaConfig::from_pairs([
            ("MEDIA_STORAGE_DIR", "D:/media"),
            ("MEDIA_CACHE_TTL_HOURS", "6"),
            ("MEDIA_CACHE_MAX_TOTAL_MB", "100"),
            ("MEDIA_BOARD_MAX_TOTAL_MB", "200"),
            ("MEDIA_DOWNLOAD_TIMEOUT_MS", "60000"),
            ("MEDIA_MAX_SIZE_MB", "512"),
            ("MEDIA_MAX_CONCURRENT_INGESTIONS", "4"),
            ("MEDIA_VIDEO_MAX_HEIGHT", "720"),
            ("MEDIA_VIDEO_ENCODER", "h264_nvenc"),
            ("MEDIA_VIDEO_PRESET", "superfast"),
            ("MEDIA_VIDEO_NVENC_PRESET", "p1"),
            ("MEDIA_VIDEO_MAX_DURATION_SEC", "90"),
            ("MEDIA_AUDIO_NORMALIZE_ENABLED", "0"),
            ("MEDIA_AUDIO_LOUDNORM_I", "-16"),
            ("MEDIA_AUDIO_LOUDNORM_LRA", "9"),
            ("MEDIA_AUDIO_LOUDNORM_TP", "-2"),
            ("FFMPEG_BINARY", "C:/tools/ffmpeg.exe"),
            ("FFPROBE_BINARY", "C:/tools/ffprobe.exe"),
            ("YTDLP_BINARY", "C:/tools/yt-dlp.exe"),
            ("YTDLP_CONCURRENT_FRAGMENTS", "3"),
            ("TIKTOK_COOKIE", "cookie: sessionid=abc; ttwid=xyz"),
            ("I18N", "en-US"),
        ]);
        assert_eq!(config.storage.directory, PathBuf::from("D:/media"));
        assert_eq!(config.storage.cache_ttl_hours(), 6);
        assert_eq!(config.storage.cache_max_total_bytes, 100 * BYTES_PER_MB);
        assert_eq!(config.storage.board_max_total_bytes, 200 * BYTES_PER_MB);
        assert_eq!(config.download.timeout, Duration::from_secs(60));
        assert_eq!(config.download.max_size_bytes, 512 * BYTES_PER_MB);
        assert_eq!(config.download.max_concurrent_ingestions, 4);
        assert_eq!(config.video.max_height, 720);
        assert_eq!(config.video.encoder, VideoEncoder::H264Nvenc);
        assert_eq!(config.video.cpu_preset, "ultrafast");
        assert_eq!(config.video.nvenc_preset, "p1");
        assert_eq!(config.video.max_duration_sec, Some(90));
        assert!(!config.video.audio_normalize);
        assert_eq!(config.video.loudnorm.integrated_lufs, -16.0);
        assert_eq!(config.video.loudnorm.range_lu, 9.0);
        assert_eq!(config.video.loudnorm.true_peak_dbtp, -2.0);
        assert_eq!(config.tools.ffmpeg, PathBuf::from("C:/tools/ffmpeg.exe"));
        assert_eq!(config.tools.ffprobe, PathBuf::from("C:/tools/ffprobe.exe"));
        assert_eq!(config.tools.ytdlp, PathBuf::from("C:/tools/yt-dlp.exe"));
        assert_eq!(config.tools.ytdlp_concurrent_fragments, 3);
        assert_eq!(
            config.tools.tiktok_cookie.as_deref(),
            Some("sessionid=abc; ttwid=xyz")
        );
        assert_eq!(config.resolve.language, "en");
        assert_eq!(config.resolve.oembed_timeout, Duration::from_secs(12));
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let config = MediaConfig::from_pairs([
            ("MEDIA_CACHE_TTL_HOURS", "beaucoup"),
            ("MEDIA_VIDEO_MAX_HEIGHT", "-1"),
            ("MEDIA_AUDIO_NORMALIZE_ENABLED", "peut-être"),
            ("MEDIA_VIDEO_ENCODER", "?!"),
            ("MEDIA_VIDEO_MAX_DURATION_SEC", "n/a"),
        ]);
        assert_eq!(config.storage.cache_ttl, Duration::from_secs(12 * 3600));
        assert_eq!(config.video.max_height, 1080);
        assert!(config.video.audio_normalize);
        assert_eq!(config.video.encoder, VideoEncoder::Auto);
        assert_eq!(config.video.max_duration_sec, None);
    }

    #[test]
    fn minimum_values_are_enforced() {
        let config = MediaConfig::from_pairs([
            ("MEDIA_CACHE_TTL_HOURS", "0"),
            ("MEDIA_CACHE_MAX_TOTAL_MB", "0"),
            ("MEDIA_BOARD_MAX_TOTAL_MB", "0"),
            ("MEDIA_DOWNLOAD_TIMEOUT_MS", "0"),
            ("MEDIA_MAX_SIZE_MB", "0"),
            ("MEDIA_MAX_CONCURRENT_INGESTIONS", "0"),
            ("YTDLP_CONCURRENT_FRAGMENTS", "0"),
            ("TWITTER_CARD_DURATION_SEC", "0"),
        ]);
        assert_eq!(config.storage.cache_ttl, Duration::from_secs(3600));
        assert_eq!(config.storage.cache_max_total_bytes, BYTES_PER_MB);
        assert_eq!(config.storage.board_max_total_bytes, BYTES_PER_MB);
        assert_eq!(config.download.timeout, Duration::from_millis(1));
        assert_eq!(config.download.max_size_bytes, BYTES_PER_MB);
        assert_eq!(config.download.max_concurrent_ingestions, 1);
        assert_eq!(config.tools.ytdlp_concurrent_fragments, 1);
        assert_eq!(config.resolve.twitter_card_duration, Duration::from_secs(1));
    }

    #[test]
    fn oembed_timeout_is_clamped() {
        let short = MediaConfig::from_pairs([("MEDIA_DOWNLOAD_TIMEOUT_MS", "1000")]);
        assert_eq!(short.resolve.oembed_timeout, Duration::from_secs(5));
        let long = MediaConfig::from_pairs([("MEDIA_DOWNLOAD_TIMEOUT_MS", "60000")]);
        assert_eq!(long.resolve.oembed_timeout, Duration::from_secs(12));
        let middle = MediaConfig::from_pairs([("MEDIA_DOWNLOAD_TIMEOUT_MS", "8000")]);
        assert_eq!(middle.resolve.oembed_timeout, Duration::from_secs(8));
    }

    #[test]
    fn legacy_ytdlp_format_is_replaced() {
        let config = MediaConfig::from_pairs([(
            "YTDLP_FORMAT",
            "bv*[height<=1080]+ba/b[height<=1080]/best",
        )]);
        assert_eq!(config.tools.ytdlp_format, DEFAULT_YTDLP_FORMAT);
        let custom = MediaConfig::from_pairs([("YTDLP_FORMAT", "bestvideo+bestaudio")]);
        assert_eq!(custom.tools.ytdlp_format, "bestvideo+bestaudio");
    }

    #[test]
    fn encoder_parsing_is_tolerant() {
        assert_eq!(VideoEncoder::from_env_str("AUTO"), VideoEncoder::Auto);
        assert_eq!(VideoEncoder::from_env_str("libx264"), VideoEncoder::LibX264);
        assert_eq!(VideoEncoder::from_env_str(" CPU "), VideoEncoder::LibX264);
        assert_eq!(
            VideoEncoder::from_env_str("h264_nvenc"),
            VideoEncoder::H264Nvenc
        );
        assert_eq!(VideoEncoder::from_env_str("nvenc"), VideoEncoder::H264Nvenc);
        assert_eq!(VideoEncoder::from_env_str("inconnu"), VideoEncoder::Auto);
        assert_eq!(VideoEncoder::LibX264.ffmpeg_name(), Some("libx264"));
        assert_eq!(VideoEncoder::H264Nvenc.ffmpeg_name(), Some("h264_nvenc"));
        assert_eq!(VideoEncoder::Auto.ffmpeg_name(), None);
    }

    #[test]
    fn loudnorm_filter_is_formatted_like_legacy() {
        let targets = LoudnormTargets::default();
        assert_eq!(targets.filter(), "loudnorm=I=-18:LRA=11:TP=-1.5");
        let custom = LoudnormTargets {
            integrated_lufs: -16.5,
            range_lu: 9.0,
            true_peak_dbtp: -2.0,
        };
        assert_eq!(custom.filter(), "loudnorm=I=-16.5:LRA=9:TP=-2");
    }

    #[test]
    fn cookie_prefix_is_stripped() {
        let config = MediaConfig::from_pairs([("TIKTOK_COOKIE", "Cookie: a=b")]);
        assert_eq!(config.tools.tiktok_cookie.as_deref(), Some("a=b"));
        let empty = MediaConfig::from_pairs([("TIKTOK_COOKIE", "cookie: ")]);
        assert_eq!(empty.tools.tiktok_cookie, None);
    }
}
