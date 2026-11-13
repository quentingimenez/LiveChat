//! Helpers partagés des tests d'intégration de `lc-playback`.
//!
//! Fournit une base SQLite en mémoire, des fabriques de jobs/médias, une
//! horloge partagée (`FixedClock` derrière `Arc`) et un `FakeSink` qui
//! enregistre les payloads émis.

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{
    ms_to_datetime, Clock, FixedClock, Guild, GuildId, JobStatus, MediaAsset, MediaAssetId,
    MediaKind, MediaStatus, PlaybackJob, PlaybackJobId,
};
use lc_playback::{PlaybackConfig, PlaybackEngine, PlaybackSink};
use lc_protocol::{PlayPayload, StopPayload};

/// Instant de référence des tests (2023-11-14T22:13:20Z).
pub const NOW_MS: i64 = 1_700_000_000_000;

/// Instant de référence.
pub fn now() -> DateTime<Utc> {
    ms_to_datetime(NOW_MS).unwrap()
}

/// Date depuis un timestamp en millisecondes.
pub fn date(ms: i64) -> DateTime<Utc> {
    ms_to_datetime(ms).unwrap()
}

/// Identifiant de guilde de test.
pub fn guild_id() -> GuildId {
    GuildId::parse("123456789012345678").unwrap()
}

/// Base SQLite en mémoire, migrations appliquées.
pub async fn memory_db() -> Db {
    Db::open_in_memory(&DbConfig::default()).await.unwrap()
}

/// Guilde de test (sans surcharge de durée).
pub fn sample_guild(id: GuildId) -> Guild {
    Guild {
        id,
        name: Some("Guilde de test".to_owned()),
        busy_until: None,
        default_media_time: None,
        max_media_time: None,
        display_media_full: false,
    }
}

/// Média vidéo prêt à être lu (durée 12 s).
pub fn sample_media_asset(now: DateTime<Utc>) -> MediaAsset {
    let id = MediaAssetId::new_v4();
    MediaAsset {
        id,
        source_hash: format!("hash-{}", id.to_canonical()),
        source_url: "https://example.test/media".to_owned(),
        kind: MediaKind::Video,
        mime: "video/mp4".to_owned(),
        duration_sec: Some(12),
        width: Some(1920),
        height: Some(1080),
        is_vertical: false,
        storage_path: format!("media/{}.mp4", id.to_canonical()),
        size_bytes: 1_024,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: now,
        expires_at: now + ChronoDuration::hours(24),
        created_at: now,
    }
}

/// Job texte seul (5 s, priorité 0).
pub fn sample_job(guild_id: GuildId, submission: DateTime<Utc>) -> PlaybackJob {
    PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id,
        media_asset_id: None,
        text: Some("média de test".to_owned()),
        show_text: true,
        author_name: Some("Auteur".to_owned()),
        author_image: None,
        duration_sec: 5,
        priority: 0,
        status: JobStatus::Pending,
        resumes_after_job_id: None,
        resume_offset_sec: 0,
        remaining_ms_snapshot: None,
        last_playback_state_at: None,
        submission_date: submission,
        execution_date: submission,
        scheduled_at: submission,
        started_at: None,
        finished_at: None,
    }
}

/// Job média (durée demandée en secondes).
pub fn sample_media_job(
    guild_id: GuildId,
    submission: DateTime<Utc>,
    media_asset_id: MediaAssetId,
    duration_sec: i64,
) -> PlaybackJob {
    let mut job = sample_job(guild_id, submission);
    job.media_asset_id = Some(media_asset_id);
    job.duration_sec = duration_sec;
    job.text = None;
    job.show_text = false;
    job
}

/// Horloge figée partageable entre le moteur et le test.
#[derive(Clone, Debug)]
pub struct SharedClock(Arc<FixedClock>);

impl SharedClock {
    /// Fige l'horloge à `at`.
    pub fn new(at: DateTime<Utc>) -> Self {
        Self(Arc::new(FixedClock::new(at)))
    }

    /// Fixe l'instant courant.
    pub fn set(&self, at: DateTime<Utc>) {
        self.0.set(at);
    }

    /// Avance l'horloge.
    pub fn advance(&self, delta: ChronoDuration) {
        self.0.advance(delta);
    }
}

impl Clock for SharedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0.now()
    }
}

/// Appel enregistré par [`FakeSink`].
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SinkCall {
    /// Émission `play`.
    Play {
        /// Guilde ciblée.
        guild: String,
        /// Payload émis.
        payload: PlayPayload,
    },
    /// Émission `stop`.
    Stop {
        /// Guilde ciblée.
        guild: String,
        /// Payload émis.
        payload: StopPayload,
    },
}

/// Sink de test : enregistre les appels et renvoie un nombre de destinataires
/// configurable.
#[derive(Clone, Debug, Default)]
pub struct FakeSink {
    inner: Arc<FakeSinkInner>,
}

#[derive(Debug, Default)]
struct FakeSinkInner {
    calls: Mutex<Vec<SinkCall>>,
    recipients: AtomicUsize,
}

impl FakeSink {
    /// Sink qui annonce `recipients` clients connectés à chaque `play`.
    pub fn new(recipients: usize) -> Self {
        let sink = Self::default();
        sink.set_recipients(recipients);
        sink
    }

    /// Change le nombre de destinataires renvoyé par `on_play`.
    pub fn set_recipients(&self, recipients: usize) {
        self.inner.recipients.store(recipients, Ordering::SeqCst);
    }

    /// Tous les appels, dans l'ordre.
    pub fn calls(&self) -> Vec<SinkCall> {
        self.inner.calls.lock().unwrap().clone()
    }

    /// Payloads `play` émis, dans l'ordre.
    pub fn play_calls(&self) -> Vec<PlayPayload> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                SinkCall::Play { payload, .. } => Some(payload),
                SinkCall::Stop { .. } => None,
            })
            .collect()
    }

    /// Payloads `stop` émis, dans l'ordre.
    pub fn stop_calls(&self) -> Vec<StopPayload> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                SinkCall::Play { .. } => None,
                SinkCall::Stop { payload, .. } => Some(payload),
            })
            .collect()
    }
}

impl PlaybackSink for FakeSink {
    fn on_play(
        &self,
        guild_id: &GuildId,
        payload: PlayPayload,
    ) -> impl std::future::Future<Output = usize> + Send {
        let inner = Arc::clone(&self.inner);
        let guild = guild_id.as_str().to_owned();
        async move {
            inner
                .calls
                .lock()
                .unwrap()
                .push(SinkCall::Play { guild, payload });
            inner.recipients.load(Ordering::SeqCst)
        }
    }

    fn on_stop(
        &self,
        guild_id: &GuildId,
        payload: StopPayload,
    ) -> impl std::future::Future<Output = ()> + Send {
        let inner = Arc::clone(&self.inner);
        let guild = guild_id.as_str().to_owned();
        async move {
            inner
                .calls
                .lock()
                .unwrap()
                .push(SinkCall::Stop { guild, payload });
        }
    }
}

/// Moteur de test prêt à l'emploi (base mémoire + FakeSink + horloge partagée).
pub struct Harness {
    /// Base SQLite en mémoire.
    pub db: Db,
    /// Sink d'observation.
    pub sink: FakeSink,
    /// Horloge partagée.
    pub clock: SharedClock,
    /// Moteur sous test.
    pub engine: PlaybackEngine<FakeSink, SharedClock>,
    /// Guilde de test.
    pub guild_id: GuildId,
}

/// Construit un [`Harness`] avec `recipients` overlays simulés.
pub async fn harness(recipients: usize) -> Harness {
    let db = memory_db().await;
    let sink = FakeSink::new(recipients);
    let clock = SharedClock::new(now());
    let engine = PlaybackEngine::new(
        db.clone(),
        PlaybackConfig::default(),
        sink.clone(),
        clock.clone(),
    );
    Harness {
        db,
        sink,
        clock,
        engine,
        guild_id: guild_id(),
    }
}

impl Harness {
    /// Avance l'horloge de `ms` millisecondes.
    pub fn advance(&self, ms: i64) {
        self.clock.advance(ChronoDuration::milliseconds(ms));
    }

    /// Relit un job depuis la base.
    pub async fn stored_job(&self, id: PlaybackJobId) -> PlaybackJob {
        self.db.playback_jobs().get(id).await.unwrap().unwrap()
    }
}
