//! Chargement typé et validation de la configuration d'environnement.
//!
//! ## Source de vérité
//!
//! Les noms de variables et les valeurs par défaut reproduisent
//! `server/src/services/env.ts` (legacy) et l'annexe C de
//! `docs/rust/migration-plan.md`. Deux assouplissements volontaires, rendus
//! nécessaires par les exigences du serveur 3.0 :
//!
//! - `DATABASE_URL` et le couple Discord (`DISCORD_TOKEN` + `DISCORD_CLIENT_ID`)
//!   sont **facultatifs**. Le legacy refuse de démarrer sans eux (cf.
//!   `docs/rust/audit-dynamique.md`) ; le serveur Rust doit pouvoir démarrer sans
//!   Discord, et une absence de base est exposée en `/health` (`db = "disabled"`).
//!   Quand ils sont fournis, ils sont validés et réellement utilisés.
//! - Le chargement se fait à partir de l'environnement du processus uniquement :
//!   aucun fichier `.env` n'est lu (le coordinateur de processus ou le service
//!   d'installation fournit les variables, comme documenté dans
//!   `docs/rust/server-core.md`).
//!
//! ## Contrat
//!
//! [`Config::from_pairs`] est l'entrée testable (aucun accès à l'environnement
//! global) ; [`Config::from_env`] se contente de lui passer les variables du
//! processus. Aucune de ces fonctions ne panique : chaque valeur invalide produit
//! une [`ConfigError`] qui nomme la variable et la valeur fautive (sans jamais
//! révéler un secret).
//!
//! Les champs inutilisés par la Phase 3a (média, Twitter, yt-dlp…) sont
//! volontairement parsés et validés dès maintenant : ils constituent la
//! configuration de référence que les Phases 3b à 6 consommeront.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use thiserror::Error;
use url::Url;

/// Environnement d'exécution par défaut (parité legacy).
pub const DEFAULT_NODE_ENV: &str = "production";

/// Valeur de `NODE_ENV` qui active le mode développement.
pub const DEVELOPMENT_NODE_ENV: &str = "development";

/// Port d'écoute HTTP par défaut (parité legacy).
pub const DEFAULT_PORT: u16 = 3000;

/// Chemin de stockage média par défaut (relatif au répertoire de travail).
pub const DEFAULT_MEDIA_STORAGE_DIR: &str = "./data/media";

/// Format `yt-dlp` par défaut (parité legacy, H.264/AAC ≤ 1080p).
pub const DEFAULT_YTDLP_FORMAT: &str = "bv*[vcodec^=avc1][ext=mp4][height<=1080]+ba[ext=m4a]/b[vcodec^=avc1][ext=mp4][height<=1080]/bv*[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4][height<=1080]/bv*[height<=1080]+ba/b[height<=1080]/best";

/// Niveau de journalisation (`LOG` du legacy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// `info` (défaut legacy).
    Info,
    /// `debug`.
    Debug,
    /// `error`.
    Error,
    /// `silent` : aucune sortie.
    Silent,
    /// `warning`.
    Warning,
}

impl LogLevel {
    /// Analyse une valeur de `LOG` (insensible à la casse, espaces ignorés).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "error" => Some(Self::Error),
            "silent" => Some(Self::Silent),
            "warning" => Some(Self::Warning),
            _ => None,
        }
    }

    /// Nom legacy du niveau (valeur brute acceptée par `LOG`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Error => "error",
            Self::Silent => "silent",
            Self::Warning => "warning",
        }
    }

    /// Directive `tracing` équivalente (`silent` devient `off`).
    pub fn directive(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Error => "error",
            Self::Silent => "off",
            Self::Warning => "warn",
        }
    }
}

/// Encodeur vidéo autorisé (`MEDIA_VIDEO_ENCODER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    /// Détection automatique (NVENC si disponible, sinon libx264).
    Auto,
    /// `libx264` (CPU).
    LibX264,
    /// `h264_nvenc` (GPU NVIDIA).
    H264Nvenc,
}

impl VideoEncoder {
    /// Analyse une valeur de `MEDIA_VIDEO_ENCODER` (valeurs legacy exactes).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "auto" => Some(Self::Auto),
            "libx264" => Some(Self::LibX264),
            "h264_nvenc" => Some(Self::H264Nvenc),
            _ => None,
        }
    }

    /// Nom legacy de l'encodeur.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::LibX264 => "libx264",
            Self::H264Nvenc => "h264_nvenc",
        }
    }
}

/// Secret d'authentification affiché comme `<redacted>` dans les logs.
///
/// Le type n'expose pas la valeur via `Debug`/`Display` : seuls les modules qui
/// en ont besoin appellent [`Secret::expose_secret`]. Aucune implémentation de
/// `serde::Serialize` n'est fournie pour éviter une fuite accidentelle.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    /// Construit un secret à partir de sa valeur en clair.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Accès explicite à la valeur en clair.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// `true` si le secret est vide (valeur par défaut du legacy).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(<redacted>)")
    }
}

/// Erreur de configuration : nomme toujours la variable concernée.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Variable requise absente (ou vide).
    #[error("variable d'environnement requise absente : {name}")]
    Missing {
        /// Nom de la variable.
        name: &'static str,
    },

    /// Valeur illisible pour une variable non secrète.
    #[error("valeur invalide pour {name} : {value:?} (attendu : {expected})")]
    Invalid {
        /// Nom de la variable.
        name: &'static str,
        /// Valeur fautive (jamais un secret).
        value: String,
        /// Format attendu, en français.
        expected: &'static str,
    },

    /// URL malformée ou non exploitable comme origine.
    #[error("URL invalide pour {name} : {value:?} ({reason})")]
    InvalidUrl {
        /// Nom de la variable.
        name: &'static str,
        /// Valeur fautive (jamais un secret).
        value: String,
        /// Raison du refus.
        reason: String,
    },

    /// `DISCORD_TOKEN` et `DISCORD_CLIENT_ID` doivent être fournis ensemble.
    #[error(
        "configuration Discord incomplète : {missing} doit être définie en même temps que {provided}"
    )]
    PartialDiscord {
        /// Variable manquante.
        missing: &'static str,
        /// Variable fournie seule.
        provided: &'static str,
    },
}

/// Configuration complète du serveur, validée au démarrage.
#[derive(Debug, Clone)]
pub struct Config {
    /// `NODE_ENV` (défaut `production`).
    pub node_env: String,
    /// `LOG` (défaut `info`).
    pub log_level: LogLevel,
    /// `RUST_LOG` : filtre `tracing` avancé, prioritaire sur `LOG` s'il est fourni.
    pub rust_log: Option<String>,
    /// `PORT` (défaut `3000` ; `0` = port éphémère choisi par l'OS).
    pub port: u16,
    /// `I18N` (défaut `fr`).
    pub i18n: String,
    /// `API_URL` (obligatoire) : origine utilisée pour le CORS et les liens publics.
    pub api_url: Url,
    /// `ADMIN_UI_TOKEN` (défaut vide).
    pub admin_ui_token: Secret,
    /// `ADMIN_UI_LOCAL_ONLY` (défaut `true`).
    pub admin_ui_local_only: bool,
    /// `ADMIN_TRUST_FORWARDED_FOR` (défaut `false`) : lit l'IP annoncée par le
    /// proxy de confiance (`X-Real-IP`, puis dernier `X-Forwarded-For`) pour le
    /// contrôle local-only admin.
    pub admin_trust_forwarded_for: bool,
    /// `DISCORD_TOKEN` (facultatif ; absent = Discord désactivé).
    pub discord_token: Option<Secret>,
    /// `DISCORD_CLIENT_ID` (facultatif ; absent = Discord désactivé).
    pub discord_client_id: Option<String>,
    /// `INGEST_API_TOKEN` (défaut vide).
    pub ingest_api_token: Secret,
    /// `DATABASE_URL` (facultatif ; absent ou vide = base désactivée).
    pub database_url: Option<String>,
    /// `HIDE_COMMANDS_DISABLED` (défaut `false`).
    pub hide_commands_disabled: bool,
    /// `DEFAULT_DURATION` en secondes (défaut `5`).
    pub default_duration: u64,
    /// `PLAYBACK_JOB_RETENTION_HOURS` (défaut `72`).
    pub playback_job_retention_hours: u64,
    /// `MEDIA_STORAGE_DIR` (défaut `./data/media`).
    pub media_storage_dir: PathBuf,
    /// `MEDIA_CACHE_TTL_HOURS` (défaut `12`).
    pub media_cache_ttl_hours: u64,
    /// `MEDIA_CACHE_MAX_TOTAL_MB` (défaut `5120`).
    pub media_cache_max_total_mb: u64,
    /// `MEDIA_BOARD_MAX_TOTAL_MB` (défaut `15360`).
    pub media_board_max_total_mb: u64,
    /// `MEDIA_DOWNLOAD_TIMEOUT_MS` (défaut `180000`).
    pub media_download_timeout_ms: u64,
    /// `MEDIA_MAX_SIZE_MB` (défaut `2048`).
    pub media_max_size_mb: u64,
    /// `MEDIA_VIDEO_MAX_HEIGHT` (défaut `1080`).
    pub media_video_max_height: u32,
    /// `MEDIA_VIDEO_ENCODER` (défaut `auto`).
    pub media_video_encoder: VideoEncoder,
    /// `MEDIA_VIDEO_NVENC_PRESET` (défaut `p4`).
    pub media_video_nvenc_preset: String,
    /// `MEDIA_VIDEO_PRESET` (défaut `ultrafast`).
    pub media_video_preset: String,
    /// `MEDIA_AUDIO_NORMALIZE_ENABLED` (défaut `true`).
    pub media_audio_normalize_enabled: bool,
    /// `MEDIA_AUDIO_LOUDNORM_I` (défaut `-18`).
    pub media_audio_loudnorm_i: f64,
    /// `MEDIA_AUDIO_LOUDNORM_LRA` (défaut `11`).
    pub media_audio_loudnorm_lra: f64,
    /// `MEDIA_AUDIO_LOUDNORM_TP` (défaut `-1.5`).
    pub media_audio_loudnorm_tp: f64,
    /// `MEDIA_MAX_CONCURRENT_INGESTIONS` (défaut `2`, annexe C).
    pub media_max_concurrent_ingestions: u32,
    /// `MEDIA_URL_TTL_SEC` (défaut `300`, plafonné par `MEDIA_URL_TTL_MAX_SEC`).
    pub media_url_ttl_sec: u64,
    /// `MEDIA_URL_SECRET` (défaut vide : signatures média impossibles tant que non fourni).
    pub media_url_secret: Secret,
    /// `TWITTER_OEMBED_ENDPOINT`.
    pub twitter_oembed_endpoint: Url,
    /// `TWITTER_VIDEO_RESOLVER_BASE_URL`.
    pub twitter_video_resolver_base_url: Url,
    /// `TWITTER_CARD_DURATION_SEC` (défaut `15`).
    pub twitter_card_duration_sec: u64,
    /// `PAIRING_CODE_TTL_MINUTES` (défaut `5`).
    pub pairing_code_ttl_minutes: u64,
    /// `YTDLP_BINARY` (défaut `yt-dlp`).
    pub ytdlp_binary: String,
    /// `YTDLP_CONCURRENT_FRAGMENTS` (défaut `1`).
    pub ytdlp_concurrent_fragments: u32,
    /// `YTDLP_FORMAT`.
    pub ytdlp_format: String,
    /// `TIKTOK_COOKIE` (défaut vide).
    pub tiktok_cookie: Secret,
    /// `FFMPEG_BINARY` (défaut `ffmpeg`).
    pub ffmpeg_binary: String,
    /// `FFPROBE_BINARY` (défaut `ffprobe`).
    pub ffprobe_binary: String,
    /// `ADDITIONAL_CORS_ORIGINS` : liste séparée par des virgules (défaut vide).
    pub additional_cors_origins: Vec<Url>,
    /// `MIN_OVERLAY_VERSION` (défaut `3.0.0`).
    pub min_overlay_version: String,
    /// `TRUST_PROXY_HEADERS` (défaut `false`) : le rate limit lit `X-Real-IP`,
    /// puis le dernier `X-Forwarded-For` (voir `http::proxy`).
    pub trust_proxy_headers: bool,
    /// `WS_HEARTBEAT_TIMEOUT_SEC` (défaut `45`, utilisé en Phase 4).
    pub ws_heartbeat_timeout_sec: u64,
}

impl Config {
    /// Charge la configuration depuis l'environnement du processus.
    ///
    /// Les variables dont le nom ou la valeur n'est pas de l'UTF-8 valide sont
    /// ignorées (jamais de panique) ; si l'une d'elles était requise, l'erreur
    /// [`ConfigError::Missing`] le signale.
    pub fn from_env() -> Result<Self, ConfigError> {
        let pairs = std::env::vars_os().filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let value = value.into_string().ok()?;
            Some((name, value))
        });
        Self::from_pairs(pairs)
    }

    /// Construit la configuration à partir de paires `(NOM, valeur)`.
    ///
    /// Variante testable de [`Config::from_env`] : aucun accès à l'environnement
    /// global, donc utilisable sans sérialiser les tests.
    pub fn from_pairs<I>(pairs: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let env = EnvReader::new(pairs);

        let node_env = env.string("NODE_ENV", DEFAULT_NODE_ENV);
        let log_level = env.parsed(
            "LOG",
            LogLevel::Info,
            "info, debug, error, silent ou warning",
            LogLevel::parse,
        )?;
        let rust_log = env.optional_string("RUST_LOG");
        let port = env.parsed("PORT", DEFAULT_PORT, "entier entre 0 et 65535", parse_u16)?;
        let i18n = env.string("I18N", "fr");
        let api_url = env.required_url("API_URL")?;
        let admin_ui_token = env.secret("ADMIN_UI_TOKEN");
        let admin_ui_local_only =
            env.parsed("ADMIN_UI_LOCAL_ONLY", true, "true ou false", parse_bool)?;
        let admin_trust_forwarded_for = env.parsed(
            "ADMIN_TRUST_FORWARDED_FOR",
            false,
            "true ou false",
            parse_bool,
        )?;
        let discord_token = env.optional_secret("DISCORD_TOKEN");
        let discord_client_id = env.optional_string("DISCORD_CLIENT_ID");
        match (&discord_token, &discord_client_id) {
            (Some(_), None) => {
                return Err(ConfigError::PartialDiscord {
                    missing: "DISCORD_CLIENT_ID",
                    provided: "DISCORD_TOKEN",
                })
            }
            (None, Some(_)) => {
                return Err(ConfigError::PartialDiscord {
                    missing: "DISCORD_TOKEN",
                    provided: "DISCORD_CLIENT_ID",
                })
            }
            _ => {}
        }
        let ingest_api_token = env.secret("INGEST_API_TOKEN");
        let database_url = env.optional_string("DATABASE_URL");
        let hide_commands_disabled =
            env.parsed("HIDE_COMMANDS_DISABLED", false, "true ou false", parse_bool)?;
        let default_duration = env.parsed("DEFAULT_DURATION", 5, "entier positif", parse_u64)?;
        let playback_job_retention_hours = env.parsed(
            "PLAYBACK_JOB_RETENTION_HOURS",
            72,
            "entier positif",
            parse_u64,
        )?;
        let media_storage_dir =
            PathBuf::from(env.string("MEDIA_STORAGE_DIR", DEFAULT_MEDIA_STORAGE_DIR));
        let media_cache_ttl_hours =
            env.parsed("MEDIA_CACHE_TTL_HOURS", 12, "entier positif", parse_u64)?;
        let media_cache_max_total_mb = env.parsed(
            "MEDIA_CACHE_MAX_TOTAL_MB",
            5120,
            "entier positif",
            parse_u64,
        )?;
        let media_board_max_total_mb = env.parsed(
            "MEDIA_BOARD_MAX_TOTAL_MB",
            15360,
            "entier positif",
            parse_u64,
        )?;
        let media_download_timeout_ms = env.parsed(
            "MEDIA_DOWNLOAD_TIMEOUT_MS",
            180_000,
            "entier positif",
            parse_u64,
        )?;
        let media_max_size_mb =
            env.parsed("MEDIA_MAX_SIZE_MB", 2048, "entier positif", parse_u64)?;
        let media_video_max_height =
            env.parsed("MEDIA_VIDEO_MAX_HEIGHT", 1080, "entier positif", parse_u32)?;
        let media_video_encoder = env.parsed(
            "MEDIA_VIDEO_ENCODER",
            VideoEncoder::Auto,
            "auto, libx264 ou h264_nvenc",
            VideoEncoder::parse,
        )?;
        let media_video_nvenc_preset = env.string("MEDIA_VIDEO_NVENC_PRESET", "p4");
        let media_video_preset = env.string("MEDIA_VIDEO_PRESET", "ultrafast");
        let media_audio_normalize_enabled = env.parsed(
            "MEDIA_AUDIO_NORMALIZE_ENABLED",
            true,
            "true ou false",
            parse_bool,
        )?;
        let media_audio_loudnorm_i =
            env.parsed("MEDIA_AUDIO_LOUDNORM_I", -18.0, "nombre décimal", parse_f64)?;
        let media_audio_loudnorm_lra = env.parsed(
            "MEDIA_AUDIO_LOUDNORM_LRA",
            11.0,
            "nombre décimal",
            parse_f64,
        )?;
        let media_audio_loudnorm_tp =
            env.parsed("MEDIA_AUDIO_LOUDNORM_TP", -1.5, "nombre décimal", parse_f64)?;
        let media_max_concurrent_ingestions = env.parsed(
            "MEDIA_MAX_CONCURRENT_INGESTIONS",
            2,
            "entier positif",
            parse_u32,
        )?;
        let media_url_ttl_sec = env.parsed(
            "MEDIA_URL_TTL_SEC",
            lc_protocol::MEDIA_URL_TTL_SEC,
            "entier positif (secondes)",
            parse_u64,
        )?;
        if media_url_ttl_sec > lc_protocol::MEDIA_URL_TTL_MAX_SEC {
            return Err(ConfigError::Invalid {
                name: "MEDIA_URL_TTL_SEC",
                value: media_url_ttl_sec.to_string(),
                expected: "durée inférieure ou égale à 3600 secondes (MEDIA_URL_TTL_MAX_SEC)",
            });
        }
        let media_url_secret = env.secret("MEDIA_URL_SECRET");
        let twitter_oembed_endpoint = env.url(
            "TWITTER_OEMBED_ENDPOINT",
            "https://publish.twitter.com/oembed",
        )?;
        let twitter_video_resolver_base_url =
            env.url("TWITTER_VIDEO_RESOLVER_BASE_URL", "https://vxtwitter.com")?;
        let twitter_card_duration_sec =
            env.parsed("TWITTER_CARD_DURATION_SEC", 15, "entier positif", parse_u64)?;
        let pairing_code_ttl_minutes =
            env.parsed("PAIRING_CODE_TTL_MINUTES", 5, "entier positif", parse_u64)?;
        let ytdlp_binary = env.string("YTDLP_BINARY", "yt-dlp");
        let ytdlp_concurrent_fragments =
            env.parsed("YTDLP_CONCURRENT_FRAGMENTS", 1, "entier positif", parse_u32)?;
        let ytdlp_format = env.string("YTDLP_FORMAT", DEFAULT_YTDLP_FORMAT);
        let tiktok_cookie = env.secret("TIKTOK_COOKIE");
        let ffmpeg_binary = env.string("FFMPEG_BINARY", "ffmpeg");
        let ffprobe_binary = env.string("FFPROBE_BINARY", "ffprobe");
        let additional_cors_origins = env.origin_list("ADDITIONAL_CORS_ORIGINS")?;
        let min_overlay_version = env.string(
            "MIN_OVERLAY_VERSION",
            lc_protocol::DEFAULT_MIN_OVERLAY_VERSION,
        );
        if lc_protocol::parse_simple_version(&min_overlay_version).is_none() {
            return Err(ConfigError::Invalid {
                name: "MIN_OVERLAY_VERSION",
                value: min_overlay_version,
                expected: "version majeure.mineure.correctif (ex. 3.0.0)",
            });
        }
        let trust_proxy_headers =
            env.parsed("TRUST_PROXY_HEADERS", false, "true ou false", parse_bool)?;
        let ws_heartbeat_timeout_sec =
            env.parsed("WS_HEARTBEAT_TIMEOUT_SEC", 45, "entier positif", parse_u64)?;

        Ok(Self {
            node_env,
            log_level,
            rust_log,
            port,
            i18n,
            api_url,
            admin_ui_token,
            admin_ui_local_only,
            admin_trust_forwarded_for,
            discord_token,
            discord_client_id,
            ingest_api_token,
            database_url,
            hide_commands_disabled,
            default_duration,
            playback_job_retention_hours,
            media_storage_dir,
            media_cache_ttl_hours,
            media_cache_max_total_mb,
            media_board_max_total_mb,
            media_download_timeout_ms,
            media_max_size_mb,
            media_video_max_height,
            media_video_encoder,
            media_video_nvenc_preset,
            media_video_preset,
            media_audio_normalize_enabled,
            media_audio_loudnorm_i,
            media_audio_loudnorm_lra,
            media_audio_loudnorm_tp,
            media_max_concurrent_ingestions,
            media_url_ttl_sec,
            media_url_secret,
            twitter_oembed_endpoint,
            twitter_video_resolver_base_url,
            twitter_card_duration_sec,
            pairing_code_ttl_minutes,
            ytdlp_binary,
            ytdlp_concurrent_fragments,
            ytdlp_format,
            tiktok_cookie,
            ffmpeg_binary,
            ffprobe_binary,
            additional_cors_origins,
            min_overlay_version,
            trust_proxy_headers,
            ws_heartbeat_timeout_sec,
        })
    }

    /// `true` si `NODE_ENV` vaut exactement `development`.
    pub fn is_development(&self) -> bool {
        self.node_env == DEVELOPMENT_NODE_ENV
    }

    /// Origine CORS dérivée de `API_URL` (schéma + hôte + port, sans chemin).
    pub fn api_origin(&self) -> String {
        self.api_url.origin().ascii_serialization()
    }
}

/// Lecture typée des variables, sans accès à l'environnement global.
struct EnvReader {
    vars: HashMap<String, String>,
}

impl EnvReader {
    fn new<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        Self {
            vars: pairs.into_iter().collect(),
        }
    }

    /// Valeur brute non vide (`None` si absente, vide ou composée d'espaces).
    fn opt_raw(&self, name: &'static str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
    }

    fn string(&self, name: &'static str, default: &str) -> String {
        self.opt_raw(name)
            .map(str::to_owned)
            .unwrap_or_else(|| default.to_owned())
    }

    fn optional_string(&self, name: &'static str) -> Option<String> {
        self.opt_raw(name).map(str::to_owned)
    }

    fn secret(&self, name: &'static str) -> Secret {
        Secret::new(self.opt_raw(name).unwrap_or_default())
    }

    fn optional_secret(&self, name: &'static str) -> Option<Secret> {
        self.opt_raw(name).map(Secret::new)
    }

    fn parsed<T>(
        &self,
        name: &'static str,
        default: T,
        expected: &'static str,
        parse: impl Fn(&str) -> Option<T>,
    ) -> Result<T, ConfigError> {
        match self.opt_raw(name) {
            Some(raw) => parse(raw.trim()).ok_or_else(|| ConfigError::Invalid {
                name,
                value: raw.to_owned(),
                expected,
            }),
            None => Ok(default),
        }
    }

    fn required_url(&self, name: &'static str) -> Result<Url, ConfigError> {
        let raw = self.opt_raw(name).ok_or(ConfigError::Missing { name })?;
        parse_http_url(name, raw)
    }

    fn url(&self, name: &'static str, default: &str) -> Result<Url, ConfigError> {
        parse_http_url(name, self.opt_raw(name).unwrap_or(default))
    }

    fn origin_list(&self, name: &'static str) -> Result<Vec<Url>, ConfigError> {
        let Some(raw) = self.opt_raw(name) else {
            return Ok(Vec::new());
        };
        raw.split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(|origin| parse_origin(name, origin))
            .collect()
    }
}

/// Parse `true`/`false` (parité legacy : pas de `1`/`0`/`yes`).
fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_u16(raw: &str) -> Option<u16> {
    raw.trim().parse::<u16>().ok()
}

fn parse_u32(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok()
}

fn parse_u64(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
}

fn parse_f64(raw: &str) -> Option<f64> {
    raw.trim().parse::<f64>().ok()
}

/// Parse une URL HTTP(S) absolue (utilisable comme origine).
fn parse_http_url(name: &'static str, raw: &str) -> Result<Url, ConfigError> {
    let trimmed = raw.trim();
    let url = Url::parse(trimmed).map_err(|error| ConfigError::InvalidUrl {
        name,
        value: trimmed.to_owned(),
        reason: error.to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ConfigError::InvalidUrl {
            name,
            value: trimmed.to_owned(),
            reason: "schéma attendu : http ou https".to_owned(),
        });
    }
    if url.host().is_none() {
        return Err(ConfigError::InvalidUrl {
            name,
            value: trimmed.to_owned(),
            reason: "hôte manquant".to_owned(),
        });
    }
    Ok(url)
}

/// Parse une origine CORS (schéma + hôte + port, sans chemin ni identifiants).
fn parse_origin(name: &'static str, raw: &str) -> Result<Url, ConfigError> {
    let url = parse_http_url(name, raw)?;
    if url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ConfigError::InvalidUrl {
            name,
            value: raw.trim().to_owned(),
            reason: "une origine (schéma://hôte[:port]) sans chemin, requête ni identifiants est attendue"
                .to_owned(),
        });
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn minimal() -> Vec<(String, String)> {
        pairs(&[("API_URL", "http://localhost:3000")])
    }

    fn config() -> Config {
        Config::from_pairs(minimal()).expect("configuration minimale valide")
    }

    fn config_with(values: &[(&str, &str)]) -> Config {
        Config::from_pairs(pairs(values)).expect("configuration de test valide")
    }

    #[test]
    fn minimal_config_applies_legacy_defaults() {
        let config = config();
        assert_eq!(config.node_env, DEFAULT_NODE_ENV);
        assert!(!config.is_development());
        assert_eq!(config.log_level, LogLevel::Info);
        assert_eq!(config.log_level.as_str(), "info");
        assert_eq!(config.rust_log, None);
        assert_eq!(config.port, 3000);
        assert_eq!(config.i18n, "fr");
        assert_eq!(config.api_origin(), "http://localhost:3000");
        assert!(config.admin_ui_token.is_empty());
        assert!(config.admin_ui_local_only);
        assert!(config.discord_token.is_none());
        assert!(config.discord_client_id.is_none());
        assert!(config.ingest_api_token.is_empty());
        assert_eq!(config.database_url, None);
        assert!(!config.hide_commands_disabled);
        assert_eq!(config.default_duration, 5);
        assert_eq!(config.playback_job_retention_hours, 72);
        assert_eq!(
            config.media_storage_dir,
            PathBuf::from(DEFAULT_MEDIA_STORAGE_DIR)
        );
        assert_eq!(config.media_cache_ttl_hours, 12);
        assert_eq!(config.media_cache_max_total_mb, 5120);
        assert_eq!(config.media_board_max_total_mb, 15360);
        assert_eq!(config.media_download_timeout_ms, 180_000);
        assert_eq!(config.media_max_size_mb, 2048);
        assert_eq!(config.media_video_max_height, 1080);
        assert_eq!(config.media_video_encoder, VideoEncoder::Auto);
        assert_eq!(config.media_video_nvenc_preset, "p4");
        assert_eq!(config.media_video_preset, "ultrafast");
        assert!(config.media_audio_normalize_enabled);
        assert!((config.media_audio_loudnorm_i - (-18.0)).abs() < f64::EPSILON);
        assert!((config.media_audio_loudnorm_lra - 11.0).abs() < f64::EPSILON);
        assert!((config.media_audio_loudnorm_tp - (-1.5)).abs() < f64::EPSILON);
        assert_eq!(config.media_max_concurrent_ingestions, 2);
        assert_eq!(config.media_url_ttl_sec, 300);
        assert!(config.media_url_secret.is_empty());
        assert_eq!(
            config.twitter_oembed_endpoint.as_str(),
            "https://publish.twitter.com/oembed"
        );
        assert_eq!(
            config.twitter_video_resolver_base_url.as_str(),
            "https://vxtwitter.com/"
        );
        assert_eq!(config.twitter_card_duration_sec, 15);
        assert_eq!(config.pairing_code_ttl_minutes, 5);
        assert_eq!(config.ytdlp_binary, "yt-dlp");
        assert_eq!(config.ytdlp_concurrent_fragments, 1);
        assert_eq!(config.ytdlp_format, DEFAULT_YTDLP_FORMAT);
        assert!(config.tiktok_cookie.is_empty());
        assert_eq!(config.ffmpeg_binary, "ffmpeg");
        assert_eq!(config.ffprobe_binary, "ffprobe");
        assert!(config.additional_cors_origins.is_empty());
        assert_eq!(config.min_overlay_version, "3.0.0");
        assert!(!config.trust_proxy_headers);
        assert_eq!(config.ws_heartbeat_timeout_sec, 45);
    }

    #[test]
    fn explicit_values_override_defaults() {
        let config = Config::from_pairs(pairs(&[
            ("NODE_ENV", "development"),
            ("LOG", "debug"),
            ("RUST_LOG", "lc_server=debug,tower_http=info"),
            ("PORT", "39123"),
            ("I18N", "en"),
            ("API_URL", "https://livechat.example.com/app"),
            ("ADMIN_UI_TOKEN", "admin-secret"),
            ("ADMIN_UI_LOCAL_ONLY", "false"),
            ("DISCORD_TOKEN", "discord-secret"),
            ("DISCORD_CLIENT_ID", "123456789012345678"),
            ("INGEST_API_TOKEN", "ingest-secret"),
            ("DATABASE_URL", "file:./data/livechat.db"),
            ("HIDE_COMMANDS_DISABLED", "true"),
            ("DEFAULT_DURATION", "10"),
            ("PLAYBACK_JOB_RETENTION_HOURS", "48"),
            ("MEDIA_STORAGE_DIR", "D:/livechat/media"),
            ("MEDIA_CACHE_TTL_HOURS", "24"),
            ("MEDIA_CACHE_MAX_TOTAL_MB", "1024"),
            ("MEDIA_BOARD_MAX_TOTAL_MB", "2048"),
            ("MEDIA_DOWNLOAD_TIMEOUT_MS", "60000"),
            ("MEDIA_MAX_SIZE_MB", "512"),
            ("MEDIA_VIDEO_MAX_HEIGHT", "720"),
            ("MEDIA_VIDEO_ENCODER", "h264_nvenc"),
            ("MEDIA_VIDEO_NVENC_PRESET", "p1"),
            ("MEDIA_VIDEO_PRESET", "veryfast"),
            ("MEDIA_AUDIO_NORMALIZE_ENABLED", "false"),
            ("MEDIA_AUDIO_LOUDNORM_I", "-16"),
            ("MEDIA_AUDIO_LOUDNORM_LRA", "9"),
            ("MEDIA_AUDIO_LOUDNORM_TP", "-2"),
            ("MEDIA_MAX_CONCURRENT_INGESTIONS", "4"),
            ("MEDIA_URL_TTL_SEC", "600"),
            ("MEDIA_URL_SECRET", "url-secret"),
            ("TWITTER_OEMBED_ENDPOINT", "https://oembed.example.com"),
            ("TWITTER_VIDEO_RESOLVER_BASE_URL", "https://vx.example.com"),
            ("TWITTER_CARD_DURATION_SEC", "20"),
            ("PAIRING_CODE_TTL_MINUTES", "10"),
            ("YTDLP_BINARY", "C:/tools/yt-dlp.exe"),
            ("YTDLP_CONCURRENT_FRAGMENTS", "3"),
            ("YTDLP_FORMAT", "best"),
            ("TIKTOK_COOKIE", "sessionid=abc"),
            ("FFMPEG_BINARY", "C:/tools/ffmpeg.exe"),
            ("FFPROBE_BINARY", "C:/tools/ffprobe.exe"),
            (
                "ADDITIONAL_CORS_ORIGINS",
                "https://a.example, https://b.example:8443 ,",
            ),
            ("MIN_OVERLAY_VERSION", "3.1.0"),
            ("TRUST_PROXY_HEADERS", "true"),
            ("WS_HEARTBEAT_TIMEOUT_SEC", "30"),
        ]))
        .expect("configuration complète valide");

        assert_eq!(config.node_env, DEVELOPMENT_NODE_ENV);
        assert!(config.is_development());
        assert_eq!(config.log_level, LogLevel::Debug);
        assert_eq!(
            config.rust_log.as_deref(),
            Some("lc_server=debug,tower_http=info")
        );
        assert_eq!(config.port, 39123);
        assert_eq!(config.i18n, "en");
        assert_eq!(config.api_origin(), "https://livechat.example.com");
        assert_eq!(config.admin_ui_token.expose_secret(), "admin-secret");
        assert!(!config.admin_ui_local_only);
        assert_eq!(
            config.discord_token.as_ref().map(Secret::expose_secret),
            Some("discord-secret")
        );
        assert_eq!(
            config.discord_client_id.as_deref(),
            Some("123456789012345678")
        );
        assert_eq!(config.ingest_api_token.expose_secret(), "ingest-secret");
        assert_eq!(
            config.database_url.as_deref(),
            Some("file:./data/livechat.db")
        );
        assert!(config.hide_commands_disabled);
        assert_eq!(config.default_duration, 10);
        assert_eq!(config.playback_job_retention_hours, 48);
        assert_eq!(config.media_storage_dir, PathBuf::from("D:/livechat/media"));
        assert_eq!(config.media_cache_ttl_hours, 24);
        assert_eq!(config.media_cache_max_total_mb, 1024);
        assert_eq!(config.media_board_max_total_mb, 2048);
        assert_eq!(config.media_download_timeout_ms, 60_000);
        assert_eq!(config.media_max_size_mb, 512);
        assert_eq!(config.media_video_max_height, 720);
        assert_eq!(config.media_video_encoder, VideoEncoder::H264Nvenc);
        assert_eq!(config.media_video_nvenc_preset, "p1");
        assert_eq!(config.media_video_preset, "veryfast");
        assert!(!config.media_audio_normalize_enabled);
        assert!((config.media_audio_loudnorm_i - (-16.0)).abs() < f64::EPSILON);
        assert!((config.media_audio_loudnorm_lra - 9.0).abs() < f64::EPSILON);
        assert!((config.media_audio_loudnorm_tp - (-2.0)).abs() < f64::EPSILON);
        assert_eq!(config.media_max_concurrent_ingestions, 4);
        assert_eq!(config.media_url_ttl_sec, 600);
        assert_eq!(config.media_url_secret.expose_secret(), "url-secret");
        assert_eq!(
            config.twitter_oembed_endpoint.as_str(),
            "https://oembed.example.com/"
        );
        assert_eq!(
            config.twitter_video_resolver_base_url.as_str(),
            "https://vx.example.com/"
        );
        assert_eq!(config.twitter_card_duration_sec, 20);
        assert_eq!(config.pairing_code_ttl_minutes, 10);
        assert_eq!(config.ytdlp_binary, "C:/tools/yt-dlp.exe");
        assert_eq!(config.ytdlp_concurrent_fragments, 3);
        assert_eq!(config.ytdlp_format, "best");
        assert_eq!(config.tiktok_cookie.expose_secret(), "sessionid=abc");
        assert_eq!(config.ffmpeg_binary, "C:/tools/ffmpeg.exe");
        assert_eq!(config.ffprobe_binary, "C:/tools/ffprobe.exe");
        assert_eq!(config.additional_cors_origins.len(), 2);
        assert_eq!(
            config.additional_cors_origins[0].as_str(),
            "https://a.example/"
        );
        assert_eq!(
            config.additional_cors_origins[1].as_str(),
            "https://b.example:8443/"
        );
        assert_eq!(config.min_overlay_version, "3.1.0");
        assert!(config.trust_proxy_headers);
        assert_eq!(config.ws_heartbeat_timeout_sec, 30);
    }

    #[test]
    fn missing_api_url_is_reported() {
        let error =
            Config::from_pairs(pairs(&[("PORT", "3000")])).expect_err("API_URL est obligatoire");
        assert!(matches!(error, ConfigError::Missing { name: "API_URL" }));
        assert!(error.to_string().contains("API_URL"));
    }

    #[test]
    fn empty_api_url_is_reported_as_missing() {
        let error =
            Config::from_pairs(pairs(&[("API_URL", "   ")])).expect_err("API_URL vide refusée");
        assert!(matches!(error, ConfigError::Missing { name: "API_URL" }));
    }

    #[test]
    fn invalid_port_is_reported_with_variable_and_value() {
        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("PORT", "soixante"),
        ]))
        .expect_err("PORT invalide refusé");
        match error {
            ConfigError::Invalid { name, value, .. } => {
                assert_eq!(name, "PORT");
                assert_eq!(value, "soixante");
            }
            other => panic!("erreur inattendue : {other:?}"),
        }
    }

    #[test]
    fn port_zero_is_accepted_for_ephemeral_binding() {
        let config = config_with(&[("API_URL", "http://localhost:3000"), ("PORT", "0")]);
        assert_eq!(config.port, 0);
    }

    #[test]
    fn invalid_boolean_is_reported() {
        for name in [
            "ADMIN_UI_LOCAL_ONLY",
            "ADMIN_TRUST_FORWARDED_FOR",
            "HIDE_COMMANDS_DISABLED",
            "MEDIA_AUDIO_NORMALIZE_ENABLED",
            "TRUST_PROXY_HEADERS",
        ] {
            let error = Config::from_pairs(pairs(&[
                ("API_URL", "http://localhost:3000"),
                (name, "oui"),
            ]))
            .expect_err("booléen legacy strict");
            assert!(
                matches!(error, ConfigError::Invalid { name: reported, .. } if reported == name),
                "variable {name} : erreur inattendue {error:?}"
            );
        }
    }

    #[test]
    fn boolean_accepts_true_and_false_only() {
        let config = config_with(&[
            ("API_URL", "http://localhost:3000"),
            ("ADMIN_UI_LOCAL_ONLY", "false"),
        ]);
        assert!(!config.admin_ui_local_only);
        for invalid in ["1", "0", "yes", "on"] {
            assert_eq!(parse_bool(invalid), None, "valeur refusée : {invalid}");
        }
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool(" false "), Some(false));
    }

    #[test]
    fn invalid_log_level_is_reported() {
        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("LOG", "verbose"),
        ]))
        .expect_err("LOG invalide refusé");
        assert!(matches!(error, ConfigError::Invalid { name: "LOG", .. }));
    }

    #[test]
    fn log_level_maps_to_tracing_directive() {
        assert_eq!(LogLevel::Silent.directive(), "off");
        assert_eq!(LogLevel::Warning.directive(), "warn");
        assert_eq!(LogLevel::Error.directive(), "error");
        assert_eq!(LogLevel::parse("WARNING"), Some(LogLevel::Warning));
        assert_eq!(LogLevel::parse("verbose"), None);
    }

    #[test]
    fn video_encoder_accepts_legacy_values_only() {
        assert_eq!(VideoEncoder::parse("auto"), Some(VideoEncoder::Auto));
        assert_eq!(VideoEncoder::parse("libx264"), Some(VideoEncoder::LibX264));
        assert_eq!(
            VideoEncoder::parse("h264_nvenc"),
            Some(VideoEncoder::H264Nvenc)
        );
        assert_eq!(VideoEncoder::parse("vaapi"), None);
        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("MEDIA_VIDEO_ENCODER", "vaapi"),
        ]))
        .expect_err("encodeur inconnu refusé");
        assert!(matches!(
            error,
            ConfigError::Invalid {
                name: "MEDIA_VIDEO_ENCODER",
                ..
            }
        ));
    }

    #[test]
    fn api_url_requires_http_scheme() {
        let error = Config::from_pairs(pairs(&[("API_URL", "ftp://example.com")]))
            .expect_err("schéma non HTTP refusé");
        match error {
            ConfigError::InvalidUrl { name, reason, .. } => {
                assert_eq!(name, "API_URL");
                assert!(reason.contains("http"));
            }
            other => panic!("erreur inattendue : {other:?}"),
        }
    }

    #[test]
    fn api_url_must_be_absolute() {
        let error = Config::from_pairs(pairs(&[("API_URL", "localhost:3000")]))
            .expect_err("URL relative refusée");
        assert!(matches!(
            error,
            ConfigError::InvalidUrl {
                name: "API_URL",
                ..
            }
        ));
    }

    #[test]
    fn api_origin_drops_path_and_trailing_slash() {
        let config = config_with(&[("API_URL", "http://LOCALHOST:3000/")]);
        assert_eq!(config.api_origin(), "http://localhost:3000");
        let config = config_with(&[("API_URL", "https://example.com/app/")]);
        assert_eq!(config.api_origin(), "https://example.com");
    }

    #[test]
    fn additional_cors_origins_are_validated_and_trimmed() {
        let config = config_with(&[
            ("API_URL", "http://localhost:3000"),
            (
                "ADDITIONAL_CORS_ORIGINS",
                " https://a.example , ,http://127.0.0.1:5173,",
            ),
        ]);
        assert_eq!(config.additional_cors_origins.len(), 2);
        assert_eq!(
            config.additional_cors_origins[0].as_str(),
            "https://a.example/"
        );
        assert_eq!(
            config.additional_cors_origins[1].as_str(),
            "http://127.0.0.1:5173/"
        );
    }

    #[test]
    fn additional_cors_origins_reject_paths() {
        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("ADDITIONAL_CORS_ORIGINS", "https://a.example/chemin"),
        ]))
        .expect_err("origine avec chemin refusée");
        assert!(matches!(
            error,
            ConfigError::InvalidUrl {
                name: "ADDITIONAL_CORS_ORIGINS",
                ..
            }
        ));
    }

    #[test]
    fn min_overlay_version_must_be_semver_like() {
        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("MIN_OVERLAY_VERSION", "3.0"),
        ]))
        .expect_err("version illisible refusée");
        assert!(matches!(
            error,
            ConfigError::Invalid {
                name: "MIN_OVERLAY_VERSION",
                ..
            }
        ));
    }

    #[test]
    fn media_url_ttl_is_capped_by_protocol_maximum() {
        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("MEDIA_URL_TTL_SEC", "9999"),
        ]))
        .expect_err("TTL au-delà du maximum refusé");
        assert!(matches!(
            error,
            ConfigError::Invalid {
                name: "MEDIA_URL_TTL_SEC",
                ..
            }
        ));
    }

    #[test]
    fn discord_configuration_requires_both_variables() {
        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("DISCORD_TOKEN", "token"),
        ]))
        .expect_err("couple Discord incomplet");
        assert!(matches!(
            error,
            ConfigError::PartialDiscord {
                missing: "DISCORD_CLIENT_ID",
                ..
            }
        ));

        let error = Config::from_pairs(pairs(&[
            ("API_URL", "http://localhost:3000"),
            ("DISCORD_CLIENT_ID", "123"),
        ]))
        .expect_err("couple Discord incomplet");
        assert!(matches!(
            error,
            ConfigError::PartialDiscord {
                missing: "DISCORD_TOKEN",
                ..
            }
        ));
    }

    #[test]
    fn empty_optional_values_are_treated_as_absent() {
        let config = config_with(&[
            ("API_URL", "http://localhost:3000"),
            ("DATABASE_URL", "  "),
            ("DISCORD_TOKEN", ""),
            ("RUST_LOG", " "),
            ("ADDITIONAL_CORS_ORIGINS", ""),
        ]);
        assert_eq!(config.database_url, None);
        assert!(config.discord_token.is_none());
        assert_eq!(config.rust_log, None);
        assert!(config.additional_cors_origins.is_empty());
    }

    #[test]
    fn secrets_never_appear_in_debug_output() {
        let secret = Secret::new("super-secret-token");
        let debug = format!("{secret:?}");
        assert_eq!(debug, "Secret(<redacted>)");
        assert!(!debug.contains("super-secret-token"));

        let config = config_with(&[
            ("API_URL", "http://localhost:3000"),
            ("ADMIN_UI_TOKEN", "admin-secret"),
            ("DATABASE_URL", "file:./data/livechat.db"),
        ]);
        let debug = format!("{config:?}");
        assert!(!debug.contains("admin-secret"));
        assert!(debug.contains("Secret(<redacted>)"));
    }

    #[test]
    fn secret_access_is_explicit() {
        let secret = Secret::new("valeur");
        assert_eq!(secret.expose_secret(), "valeur");
        assert!(!secret.is_empty());
        assert!(Secret::default().is_empty());
    }

    #[test]
    fn invalid_numbers_are_reported() {
        for (name, value) in [
            ("DEFAULT_DURATION", "-1"),
            ("MEDIA_AUDIO_LOUDNORM_I", "fort"),
            ("MEDIA_MAX_SIZE_MB", "beaucoup"),
            ("WS_HEARTBEAT_TIMEOUT_SEC", "quarante-cinq"),
        ] {
            let error = Config::from_pairs(pairs(&[
                ("API_URL", "http://localhost:3000"),
                (name, value),
            ]))
            .expect_err("nombre invalide refusé");
            assert!(
                matches!(error, ConfigError::Invalid { name: reported, .. } if reported == name),
                "variable {name} : erreur inattendue {error:?}"
            );
        }
    }

    #[test]
    fn from_env_reads_process_environment() {
        // Test non destructif : on vérifie seulement que l'API existe et que ses
        // erreurs sont typées (le contenu de l'environnement du processus de test
        // n'est pas contrôlable de façon fiable sur toutes les plateformes).
        let result = Config::from_env();
        if let Err(error) = result {
            assert!(matches!(error, ConfigError::Missing { .. }));
        }
    }
}
