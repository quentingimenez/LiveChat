//! # lc-media
//!
//! Pipeline média de LiveChat v3 : résolution de sources sociales
//! (YouTube, TikTok, X/Twitter), téléchargement HTTP/`yt-dlp` borné,
//! probe `ffprobe`, transcodage `ffmpeg` et cache content-addressed.
//!
//! Le crate est **indépendant de la base de données** : il produit un
//! [`NormalizedMedia`] (fichier normalisé + métadonnées) que `lc-db` persiste
//! ensuite. Aucune fonction de bibliothèque ne lit l'environnement global :
//! toute la configuration passe par [`MediaConfig`].
//!
//! ## Exemple
//!
//! ```no_run
//! use lc_media::{MediaConfig, MediaPipeline};
//!
//! # async fn example() -> Result<(), lc_media::MediaError> {
//! let config = MediaConfig::from_env();
//! let pipeline = MediaPipeline::new(config)?;
//! let media = pipeline.ingest_url("https://youtu.be/dQw4w9WgXcQ").await?;
//! println!("{:?} -> {:?}", media.normalized.kind, media.normalized.storage_path);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub use lc_domain::MediaKind;

pub mod cache;
pub mod config;
pub mod download;
pub mod error;
pub mod guard;
pub mod html;
pub mod http;
pub mod ingest;
pub mod probe;
pub mod process;
pub mod resolve;
pub mod source;
pub mod storage;
pub mod transcode;

pub use crate::cache::{
    board_total_bytes, cache_total_bytes, plan_board_eviction, plan_cache_eviction,
    purge_candidates, CacheEntry, CacheEvictionPlan,
};
pub use crate::config::{
    DownloadConfig, LoudnormTargets, MediaConfig, ResolveConfig, StorageConfig, ToolsConfig,
    VideoConfig, VideoEncoder,
};
pub use crate::download::{
    build_ytdlp_args, download_http, download_source, download_tiktok, download_with_ytdlp,
    download_youtube, find_downloaded_file, parse_ytdlp_output_path, sniff_kind, DownloadResult,
    YtdlpOptions,
};
pub use crate::error::{
    classify_code, code_from_http_status, ApiErrorLabel, MediaError, MediaErrorCode, MediaResult,
};
pub use crate::guard::{
    ip_block_reason, parse_media_url, resolve_redirect, validate_target, ValidatedTarget,
    MAX_REDIRECTS,
};
pub use crate::ingest::{
    ingest_url, lock_source_hash, IngestLimiter, IngestedMedia, MediaPipeline,
};
pub use crate::probe::{parse_probe_json, probe, probe_args, ProbeInfo};
pub use crate::resolve::{resolve_media, ResolvedMedia, TweetCard};
pub use crate::source::{
    canonicalize_source_url, classify_source, source_hash, validate_ingest_source, MediaSource,
    SourceKind,
};
pub use crate::storage::{
    is_staged_file_name, resolve_storage_reference, staged_path, MediaStorage, StoredVariant,
};
pub use crate::transcode::{
    build_audio_args, build_carousel_video, build_carousel_video_args, build_video_transcode_args,
    carousel_dimensions, normalize_downloaded_media, normalize_local_media, resolve_video_encoder,
    NormalizedMedia,
};
