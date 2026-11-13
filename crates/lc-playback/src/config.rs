//! Configuration de l'ordonnanceur.

use std::time::Duration;

use chrono::Duration as ChronoDuration;

use crate::{
    DEFAULT_PLAYBACK_DURATION_SEC, DEFAULT_RETENTION_HOURS, GUILD_RUN_MAX_ITERATIONS,
    MEDIA_BOARD_JOB_PRIORITY, MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS, PLAYBACK_LOCK_PADDING_MS,
    PLAYBACK_STALE_RELEASE_GRACE_MS, PLAYBACK_STATE_MAX_AGE_MS,
};

/// Configuration complète d'un [`PlaybackEngine`](crate::PlaybackEngine).
///
/// Les valeurs par défaut reproduisent les constantes du scheduler legacy
/// (`server/src/services/playbackScheduler.ts`). L'URL de base des médias est
/// utilisée pour construire l'URL canonique du payload `play` ; le hub
/// WebSocket la remplace par une URL signée par client (cf.
/// `docs/rust/protocol-v2.md` §8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackConfig {
    /// Période de la boucle [`PlaybackEngine::tick`](crate::PlaybackEngine::tick).
    pub tick_interval: Duration,
    /// Durée de lecture par défaut quand un job n'a ni durée ni média.
    pub default_duration_sec: i64,
    /// Rétention des jobs terminés avant purge.
    pub retention: ChronoDuration,
    /// Priorité déclenchant une préemption (média du board).
    pub media_board_priority: i64,
    /// Base des URLs média (`{base}/overlay/media/{assetId}`).
    pub api_base_url: String,
    /// Débit minimal de persistance des checkpoints `playback.state`.
    pub state_persist_min_interval: Duration,
    /// Nombre maximal d'itérations par guilde et par passage.
    pub max_guild_iterations: usize,
    /// Marge ajoutée au verrou `busy_until`.
    pub lock_padding_ms: i64,
    /// Délai de grâce avant libération d'un job `PLAYING` dépassé.
    pub stale_release_grace_ms: i64,
    /// Verrou minimal après un `playback.state`.
    pub min_busy_lock_ms: i64,
    /// Âge maximal d'un snapshot `remaining_ms` avant de recalculer.
    pub state_snapshot_max_age_ms: i64,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_millis(500),
            default_duration_sec: DEFAULT_PLAYBACK_DURATION_SEC,
            retention: ChronoDuration::hours(DEFAULT_RETENTION_HOURS),
            media_board_priority: MEDIA_BOARD_JOB_PRIORITY,
            api_base_url: "http://127.0.0.1:3000".to_owned(),
            state_persist_min_interval: Duration::from_secs(1),
            max_guild_iterations: GUILD_RUN_MAX_ITERATIONS,
            lock_padding_ms: PLAYBACK_LOCK_PADDING_MS,
            stale_release_grace_ms: PLAYBACK_STALE_RELEASE_GRACE_MS,
            min_busy_lock_ms: MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS,
            state_snapshot_max_age_ms: PLAYBACK_STATE_MAX_AGE_MS,
        }
    }
}

impl PlaybackConfig {
    /// Remplace l'URL de base des médias.
    pub fn with_api_base_url(mut self, api_base_url: impl Into<String>) -> Self {
        self.api_base_url = api_base_url.into();
        self
    }

    /// Remplace la période de la boucle de fond.
    pub fn with_tick_interval(mut self, tick_interval: Duration) -> Self {
        self.tick_interval = tick_interval;
        self
    }

    /// Remplace la rétention des jobs terminés.
    pub fn with_retention(mut self, retention: ChronoDuration) -> Self {
        self.retention = retention;
        self
    }

    /// Remplace la durée de lecture par défaut.
    pub fn with_default_duration_sec(mut self, default_duration_sec: i64) -> Self {
        self.default_duration_sec = default_duration_sec.max(1);
        self
    }
}
