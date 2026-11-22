//! Pont de configuration entre le serveur et [`lc_media`].
//!
//! Le serveur valide déjà toutes les variables `MEDIA_*` dans
//! [`crate::config::Config`] (mêmes noms et valeurs par défaut que le legacy).
//! Ce module les recopie dans le [`MediaConfig`] du crate média, sans relire
//! l'environnement une seconde fois : une seule source de vérité au démarrage.
//!
//! Les quelques champs que le legacy ne configurait pas côté serveur (points
//! d'entrée oEmbed YouTube/TikTok, timeout oEmbed) conservent les valeurs par
//! défaut de [`lc_media::config::MediaConfig`].

use std::path::PathBuf;
use std::time::Duration;

use lc_media::config::{
    sanitize_tiktok_cookie, DownloadConfig, LoudnormTargets, MediaConfig, ResolveConfig,
    StorageConfig, ToolsConfig, TranslateConfig, VideoConfig, VideoEncoder,
};

use crate::app::AppState;
use crate::config::{Config, VideoEncoder as ServerVideoEncoder};
use crate::runtime::Runtime;

/// Builds the media pipeline configuration from the application state,
/// applying the hot runtime overrides.
pub fn media_config_for_state(state: &AppState) -> MediaConfig {
    media_config_with_runtime(&state.config, state.runtime())
}

/// Builds the media pipeline configuration from the environment values, then
/// overrides the settings that may change while the server runs.
pub fn media_config_with_runtime(config: &Config, runtime: &Runtime) -> MediaConfig {
    let mut media = media_config(config);
    media.storage.cache_ttl = duration_from_hours(runtime.media_cache_ttl_hours());
    media.storage.cache_max_total_bytes = megabytes_to_bytes(runtime.media_cache_max_total_mb());
    media.storage.board_max_total_bytes = megabytes_to_bytes(runtime.media_board_max_total_mb());
    media.download.max_size_bytes = megabytes_to_bytes(runtime.media_max_size_mb());
    media.video.max_height = runtime.media_video_max_height();
    // Le cookie TikTok est aussi un réglage à chaud : la console admin le
    // persiste dans `admin_runtime.json`, et l'ingestion suivante doit
    // l'utiliser sans redémarrage. Un cookie vidé désactive le passage de
    // `--cookies` à yt-dlp au lieu de retomber silencieusement sur l'env.
    media.tools.tiktok_cookie = sanitize_tiktok_cookie(&runtime.tiktok_cookie());
    media
}

/// Construit la configuration du pipeline média depuis celle du serveur.
pub fn media_config(config: &Config) -> MediaConfig {
    let defaults = MediaConfig::default();

    MediaConfig {
        storage: StorageConfig {
            directory: config.media_storage_dir.clone(),
            cache_ttl: duration_from_hours(config.media_cache_ttl_hours),
            cache_max_total_bytes: megabytes_to_bytes(config.media_cache_max_total_mb),
            board_max_total_bytes: megabytes_to_bytes(config.media_board_max_total_mb),
        },
        download: DownloadConfig {
            timeout: Duration::from_millis(config.media_download_timeout_ms),
            short_url_timeout: defaults.download.short_url_timeout,
            max_size_bytes: megabytes_to_bytes(config.media_max_size_mb),
            max_concurrent_ingestions: usize::try_from(config.media_max_concurrent_ingestions)
                .unwrap_or(usize::MAX)
                .max(1),
            user_agent: defaults.download.user_agent,
        },
        translate: TranslateConfig {
            enabled: config.media_translate_enabled,
            endpoint: config.media_translate_endpoint.clone(),
            timeout: Duration::from_millis(config.media_translate_timeout_ms),
        },
        video: VideoConfig {
            max_height: config.media_video_max_height,
            encoder: video_encoder(config.media_video_encoder),
            cpu_preset: config.media_video_preset.clone(),
            nvenc_preset: config.media_video_nvenc_preset.clone(),
            audio_normalize: config.media_audio_normalize_enabled,
            loudnorm: LoudnormTargets {
                integrated_lufs: config.media_audio_loudnorm_i,
                range_lu: config.media_audio_loudnorm_lra,
                true_peak_dbtp: config.media_audio_loudnorm_tp,
            },
            max_duration_sec: defaults.video.max_duration_sec,
        },
        tools: ToolsConfig {
            ffmpeg: PathBuf::from(&config.ffmpeg_binary),
            ffprobe: PathBuf::from(&config.ffprobe_binary),
            ytdlp: PathBuf::from(&config.ytdlp_binary),
            ytdlp_concurrent_fragments: config.ytdlp_concurrent_fragments,
            ytdlp_format: config.ytdlp_format.clone(),
            tiktok_cookie: optional_secret(&config.tiktok_cookie),
        },
        resolve: ResolveConfig {
            youtube_oembed_endpoint: defaults.resolve.youtube_oembed_endpoint,
            tiktok_oembed_endpoint: defaults.resolve.tiktok_oembed_endpoint,
            tiktok_comment_endpoint: defaults.resolve.tiktok_comment_endpoint,
            twitter_oembed_endpoint: config.twitter_oembed_endpoint.to_string(),
            twitter_video_resolver_base_url: config.twitter_video_resolver_base_url.to_string(),
            twitter_card_duration: Duration::from_secs(config.twitter_card_duration_sec),
            oembed_timeout: defaults.resolve.oembed_timeout,
            language: config.i18n.clone(),
        },
    }
}

/// Convertit un `Secret` optionnel en `Option<String>` non vide.
fn optional_secret(secret: &crate::config::Secret) -> Option<String> {
    if secret.is_empty() {
        None
    } else {
        Some(secret.expose_secret().to_owned())
    }
}

/// Convertit l'énumération d'encodeur du serveur vers celle du crate média.
fn video_encoder(encoder: ServerVideoEncoder) -> VideoEncoder {
    match encoder {
        ServerVideoEncoder::Auto => VideoEncoder::Auto,
        ServerVideoEncoder::LibX264 => VideoEncoder::LibX264,
        ServerVideoEncoder::H264Nvenc => VideoEncoder::H264Nvenc,
    }
}

/// Durée en heures, saturée pour rester représentable.
fn duration_from_hours(hours: u64) -> Duration {
    Duration::from_secs(hours.saturating_mul(3600))
}

/// Conversion mégaoctets → octets (saturante, jamais de dépassement).
fn megabytes_to_bytes(megabytes: u64) -> u64 {
    megabytes.saturating_mul(1024 * 1024)
}

/// Conversion `u64` → `i64` saturante (les valeurs viennent de l'environnement
/// et peuvent être arbitrairement grandes).
#[cfg(test)]
fn clamp_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn config(values: &[(&str, &str)]) -> Config {
        Config::from_pairs(
            values
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
        )
        .expect("configuration de test valide")
    }

    #[test]
    fn maps_storage_and_download_settings() {
        let server = config(&[
            ("API_URL", "http://localhost:3000"),
            ("MEDIA_STORAGE_DIR", "./data/test-media"),
            ("MEDIA_CACHE_TTL_HOURS", "6"),
            ("MEDIA_CACHE_MAX_TOTAL_MB", "10"),
            ("MEDIA_BOARD_MAX_TOTAL_MB", "20"),
            ("MEDIA_DOWNLOAD_TIMEOUT_MS", "4200"),
            ("MEDIA_MAX_SIZE_MB", "7"),
            ("MEDIA_MAX_CONCURRENT_INGESTIONS", "3"),
        ]);
        let media = media_config(&server);

        assert_eq!(media.storage.directory, PathBuf::from("./data/test-media"));
        assert_eq!(media.storage.cache_ttl, Duration::from_secs(6 * 3600));
        assert_eq!(media.storage.cache_max_total_bytes, 10 * 1024 * 1024);
        assert_eq!(media.storage.board_max_total_bytes, 20 * 1024 * 1024);
        assert_eq!(media.download.timeout, Duration::from_millis(4200));
        assert_eq!(media.download.max_size_bytes, 7 * 1024 * 1024);
        assert_eq!(media.download.max_concurrent_ingestions, 3);
    }

    #[test]
    fn maps_translation_settings() {
        let server = config(&[
            ("API_URL", "http://localhost:3000"),
            ("MEDIA_TRANSLATE_ENABLED", "false"),
            ("MEDIA_TRANSLATE_ENDPOINT", "https://translate.example/gtx"),
            ("MEDIA_TRANSLATE_TIMEOUT_MS", "4200"),
        ]);
        let media = media_config(&server);

        assert!(!media.translate.enabled);
        assert_eq!(
            media.translate.endpoint.as_deref(),
            Some("https://translate.example/gtx")
        );
        assert_eq!(media.translate.timeout, Duration::from_millis(4200));
    }

    #[test]
    fn maps_video_and_tools_settings() {
        let server = config(&[
            ("API_URL", "http://localhost:3000"),
            ("MEDIA_VIDEO_ENCODER", "h264_nvenc"),
            ("MEDIA_VIDEO_MAX_HEIGHT", "720"),
            ("MEDIA_AUDIO_NORMALIZE_ENABLED", "false"),
            ("MEDIA_AUDIO_LOUDNORM_I", "-16"),
            ("FFMPEG_BINARY", "/opt/ffmpeg"),
            ("FFPROBE_BINARY", "/opt/ffprobe"),
            ("YTDLP_BINARY", "/opt/yt-dlp"),
            ("YTDLP_CONCURRENT_FRAGMENTS", "4"),
            ("TIKTOK_COOKIE", "session=abc"),
        ]);
        let media = media_config(&server);

        assert_eq!(media.video.encoder, VideoEncoder::H264Nvenc);
        assert_eq!(media.video.max_height, 720);
        assert!(!media.video.audio_normalize);
        assert_eq!(media.video.loudnorm.integrated_lufs, -16.0);
        assert_eq!(media.tools.ffmpeg, PathBuf::from("/opt/ffmpeg"));
        assert_eq!(media.tools.ffprobe, PathBuf::from("/opt/ffprobe"));
        assert_eq!(media.tools.ytdlp, PathBuf::from("/opt/yt-dlp"));
        assert_eq!(media.tools.ytdlp_concurrent_fragments, 4);
        assert_eq!(media.tools.tiktok_cookie.as_deref(), Some("session=abc"));
    }

    #[test]
    fn empty_tiktok_cookie_is_disabled() {
        let server = config(&[("API_URL", "http://localhost:3000")]);
        let media = media_config(&server);

        assert!(media.tools.tiktok_cookie.is_none());
    }

    #[test]
    fn runtime_overrides_replace_the_environment_values() {
        use std::sync::Arc;

        use crate::runtime::{Runtime, RuntimeOverrides};

        let server = config(&[
            ("API_URL", "http://localhost:3000"),
            ("MEDIA_CACHE_TTL_HOURS", "6"),
            ("MEDIA_CACHE_MAX_TOTAL_MB", "10"),
            ("MEDIA_BOARD_MAX_TOTAL_MB", "20"),
            ("MEDIA_MAX_SIZE_MB", "7"),
            ("MEDIA_VIDEO_MAX_HEIGHT", "1080"),
        ]);
        let runtime = Runtime::new(Arc::new(server.clone()));
        runtime.apply(RuntimeOverrides {
            media_cache_ttl_hours: Some(2),
            media_cache_max_total_mb: Some(3),
            media_board_max_total_mb: Some(4),
            media_max_size_mb: Some(5),
            media_video_max_height: Some(480),
            ..RuntimeOverrides::default()
        });

        let media = media_config_with_runtime(&server, &runtime);
        assert_eq!(media.storage.cache_ttl, Duration::from_secs(2 * 3600));
        assert_eq!(media.storage.cache_max_total_bytes, 3 * 1024 * 1024);
        assert_eq!(media.storage.board_max_total_bytes, 4 * 1024 * 1024);
        assert_eq!(media.download.max_size_bytes, 5 * 1024 * 1024);
        assert_eq!(media.video.max_height, 480);
    }

    #[test]
    fn runtime_tiktok_cookie_overrides_the_environment_value() {
        use std::sync::Arc;

        use crate::runtime::{Runtime, RuntimeOverrides};

        let server = config(&[
            ("API_URL", "http://localhost:3000"),
            ("TIKTOK_COOKIE", "session=environment"),
        ]);
        let runtime = Runtime::new(Arc::new(server.clone()));

        runtime.apply(RuntimeOverrides {
            tiktok_cookie: Some("session=runtime".to_owned()),
            ..RuntimeOverrides::default()
        });
        let media = media_config_with_runtime(&server, &runtime);
        assert_eq!(
            media.tools.tiktok_cookie.as_deref(),
            Some("session=runtime")
        );

        runtime.apply(RuntimeOverrides {
            tiktok_cookie: Some(String::new()),
            ..RuntimeOverrides::default()
        });
        let media = media_config_with_runtime(&server, &runtime);
        assert!(media.tools.tiktok_cookie.is_none());

        runtime.apply(RuntimeOverrides {
            tiktok_cookie: Some("session=runtime\ninjected=1".to_owned()),
            ..RuntimeOverrides::default()
        });
        let media = media_config_with_runtime(&server, &runtime);
        assert!(media.tools.tiktok_cookie.is_none());

        runtime.apply(RuntimeOverrides::default());
        let media = media_config_with_runtime(&server, &runtime);
        assert_eq!(
            media.tools.tiktok_cookie.as_deref(),
            Some("session=environment")
        );
    }

    #[test]
    fn maps_encoder_variants() {
        for (raw, expected) in [
            ("auto", VideoEncoder::Auto),
            ("libx264", VideoEncoder::LibX264),
            ("h264_nvenc", VideoEncoder::H264Nvenc),
        ] {
            let server = config(&[
                ("API_URL", "http://localhost:3000"),
                ("MEDIA_VIDEO_ENCODER", raw),
            ]);
            assert_eq!(media_config(&server).video.encoder, expected);
        }
    }

    #[test]
    fn maps_resolve_settings() {
        let server = config(&[
            ("API_URL", "http://localhost:3000"),
            ("I18N", "en"),
            ("TWITTER_OEMBED_ENDPOINT", "https://publish.example/oembed"),
            (
                "TWITTER_VIDEO_RESOLVER_BASE_URL",
                "https://resolver.example",
            ),
            ("TWITTER_CARD_DURATION_SEC", "20"),
        ]);
        let media = media_config(&server);

        assert_eq!(media.resolve.language, "en");
        assert_eq!(
            media.resolve.twitter_oembed_endpoint,
            "https://publish.example/oembed"
        );
        assert_eq!(
            media.resolve.twitter_video_resolver_base_url,
            "https://resolver.example/"
        );
        assert_eq!(media.resolve.twitter_card_duration, Duration::from_secs(20));
    }

    #[test]
    fn saturating_byte_conversions() {
        assert_eq!(megabytes_to_bytes(0), 0);
        assert_eq!(megabytes_to_bytes(u64::MAX), u64::MAX);
        assert_eq!(clamp_i64(u64::MAX), i64::MAX);
        assert_eq!(clamp_i64(0), 0);
    }
}
