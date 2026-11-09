//! Contexte et backends factices pour les tests (aucun réseau, aucune I/O).
//!
//! [`FakeBotContext`] s'appuie sur une base `lc-db` **en mémoire** réelle (les
//! repositories et les requêtes SQL sont donc exercés) et sur des backends qui
//! enregistrent les demandes reçues.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{Clock, FixedClock, GuildId, MediaAsset, MediaAssetId, MediaKind, MediaStatus};

use crate::context::{
    Author, BotConfig, BotContext, ConnectedOverlay, GuildSettingsPatch, MediaBackend,
    MediaRequest, MemeAddRequest, OverlayDirectory, PlaybackBackend, PlaybackJobRequest,
};
use crate::error::{BotError, MediaError, MediaErrorCode, SpeechFailure};
use crate::i18n::Lang;

/// Instant de référence des tests : `2024-01-02T03:04:05.678Z`.
pub const SAMPLE_MS: i64 = 1_704_164_645_678;

/// Date fixe des tests, construite sans panique possible.
pub fn sample_now() -> DateTime<Utc> {
    match lc_domain::time::ms_to_datetime(SAMPLE_MS) {
        Some(at) => at,
        None => DateTime::<Utc>::UNIX_EPOCH,
    }
}

/// Construit un média prêt à lire, daté de [`sample_now`].
pub fn sample_asset(kind: MediaKind) -> MediaAsset {
    let created_at = sample_now();
    let expires_at = match created_at.checked_add_signed(ChronoDuration::hours(12)) {
        Some(at) => at,
        None => created_at,
    };
    let (mime, duration_sec) = match kind {
        MediaKind::Image => ("image/png", None),
        MediaKind::Audio => ("audio/mpeg", Some(30)),
        MediaKind::Video => ("video/mp4", Some(30)),
    };
    MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: "fake-source-hash".to_owned(),
        source_url: "https://example.com/media".to_owned(),
        kind,
        mime: mime.to_owned(),
        duration_sec,
        width: Some(1_920),
        height: Some(1_080),
        is_vertical: false,
        storage_path: "data/media/fake.mp4".to_owned(),
        size_bytes: 1_024,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: created_at,
        expires_at,
        created_at,
    }
}

/// Backend média factice.
#[derive(Default)]
pub struct FakeMediaBackend {
    requests: Mutex<Vec<MediaRequest>>,
    speech_requests: Mutex<Vec<(Lang, String)>>,
    source_response: Mutex<Option<Result<Option<MediaAsset>, MediaError>>>,
    speech_response: Mutex<Option<Result<MediaAsset, SpeechFailure>>>,
}

impl FakeMediaBackend {
    /// Backend par défaut : ingestions sans média (`Ok(None)`), TTS en échec.
    pub fn new() -> Self {
        Self::default()
    }

    /// Backend dont l'ingestion renvoie toujours l'asset donné.
    pub fn with_asset(asset: MediaAsset) -> Self {
        let backend = Self::default();
        backend.set_source_result(Ok(Some(asset.clone())));
        backend.set_speech_result(Ok(asset));
        backend
    }

    /// Configure la réponse d'ingestion distante.
    pub fn set_source_result(&self, result: Result<Option<MediaAsset>, MediaError>) {
        if let Ok(mut slot) = self.source_response.lock() {
            *slot = Some(result);
        }
    }

    /// Configure la réponse de synthèse vocale.
    pub fn set_speech_result(&self, result: Result<MediaAsset, SpeechFailure>) {
        if let Ok(mut slot) = self.speech_response.lock() {
            *slot = Some(result);
        }
    }

    /// Demandes d'ingestion reçues (dans l'ordre).
    pub fn requests(&self) -> Vec<MediaRequest> {
        match self.requests.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }

    /// Demandes de synthèse vocale reçues.
    pub fn speech_requests(&self) -> Vec<(Lang, String)> {
        match self.speech_requests.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }
}

#[async_trait]
impl MediaBackend for FakeMediaBackend {
    async fn ingest_from_source(
        &self,
        request: MediaRequest,
    ) -> Result<Option<MediaAsset>, MediaError> {
        if let Ok(mut guard) = self.requests.lock() {
            guard.push(request);
        }
        match self.source_response.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(result) => result.clone(),
                None => Ok(None),
            },
            Err(_) => Err(MediaError::new(MediaErrorCode::DownloadFailed)),
        }
    }

    async fn ingest_speech(&self, lang: Lang, text: &str) -> Result<MediaAsset, SpeechFailure> {
        if let Ok(mut guard) = self.speech_requests.lock() {
            guard.push((lang, text.to_owned()));
        }
        match self.speech_response.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(result) => result.clone(),
                None => Err(SpeechFailure::GenerationFailed),
            },
            Err(_) => Err(SpeechFailure::GenerationFailed),
        }
    }
}

/// Backend de lecture factice.
#[derive(Default)]
pub struct FakePlaybackBackend {
    requests: Mutex<Vec<PlaybackJobRequest>>,
    stop_calls: Mutex<Vec<GuildId>>,
    enqueue_fails: Mutex<bool>,
    stop_fails: Mutex<bool>,
    released: Mutex<u64>,
}

impl FakePlaybackBackend {
    /// Backend par défaut : enqueue et stop réussissent.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fait échouer les enqueues avec `BotError::Playback`.
    pub fn set_enqueue_failure(&self, fails: bool) {
        if let Ok(mut guard) = self.enqueue_fails.lock() {
            *guard = fails;
        }
    }

    /// Fait échouer les stops avec `BotError::Playback`.
    pub fn set_stop_failure(&self, fails: bool) {
        if let Ok(mut guard) = self.stop_fails.lock() {
            *guard = fails;
        }
    }

    /// Nombre de jobs libérés renvoyé par `stop_guild`.
    pub fn set_released(&self, released: u64) {
        if let Ok(mut guard) = self.released.lock() {
            *guard = released;
        }
    }

    /// Jobs enfilés (dans l'ordre).
    pub fn requests(&self) -> Vec<PlaybackJobRequest> {
        match self.requests.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }

    /// Guildes arrêtées.
    pub fn stop_calls(&self) -> Vec<GuildId> {
        match self.stop_calls.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }
}

#[async_trait]
impl PlaybackBackend for FakePlaybackBackend {
    async fn enqueue(&self, request: PlaybackJobRequest) -> Result<(), BotError> {
        let fails = match self.enqueue_fails.lock() {
            Ok(guard) => *guard,
            Err(_) => false,
        };
        if fails {
            return Err(BotError::Playback("échec simulé".to_owned()));
        }
        if let Ok(mut guard) = self.requests.lock() {
            guard.push(request);
        }
        Ok(())
    }

    async fn stop_guild(&self, guild_id: &GuildId) -> Result<u64, BotError> {
        let fails = match self.stop_fails.lock() {
            Ok(guard) => *guard,
            Err(_) => false,
        };
        if fails {
            return Err(BotError::Playback("échec simulé".to_owned()));
        }
        if let Ok(mut guard) = self.stop_calls.lock() {
            guard.push(guild_id.clone());
        }
        Ok(match self.released.lock() {
            Ok(guard) => *guard,
            Err(_) => 0,
        })
    }
}

/// Backend meme board factice.
#[derive(Default)]
pub struct FakeMemeBackend {
    requests: Mutex<Vec<MemeAddRequest>>,
    created: Mutex<bool>,
    failure: Mutex<Option<MediaErrorCode>>,
}

impl FakeMemeBackend {
    /// Backend par défaut : le premier ajout crée l'élément.
    pub fn new() -> Self {
        Self::default()
    }

    /// Indique si l'ajout doit être signalé comme créé ou déjà présent.
    pub fn set_created(&self, created: bool) {
        if let Ok(mut guard) = self.created.lock() {
            *guard = created;
        }
    }

    /// Fait échouer les ajouts avec l'erreur média donnée.
    pub fn set_failure(&self, code: MediaErrorCode) {
        if let Ok(mut guard) = self.failure.lock() {
            *guard = Some(code);
        }
    }

    /// Demandes d'ajout reçues.
    pub fn requests(&self) -> Vec<MemeAddRequest> {
        match self.requests.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }
}

#[async_trait]
impl crate::context::MemeBackend for FakeMemeBackend {
    async fn add_to_board(&self, request: MemeAddRequest) -> Result<bool, BotError> {
        if let Ok(mut guard) = self.requests.lock() {
            guard.push(request);
        }
        let failure = match self.failure.lock() {
            Ok(guard) => *guard,
            Err(_) => None,
        };
        if let Some(code) = failure {
            return Err(BotError::Media(MediaError::new(code)));
        }
        Ok(match self.created.lock() {
            Ok(guard) => *guard,
            Err(_) => false,
        })
    }
}

/// Annuaire d'overlays factice.
#[derive(Default)]
pub struct FakeOverlayDirectory {
    overlays: Mutex<Vec<ConnectedOverlay>>,
    guilds: Mutex<Vec<GuildId>>,
    fails: Mutex<bool>,
}

impl FakeOverlayDirectory {
    /// Annuaire vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Définit les overlays connectés renvoyés.
    pub fn set_overlays(&self, overlays: Vec<ConnectedOverlay>) {
        if let Ok(mut guard) = self.overlays.lock() {
            *guard = overlays;
        }
    }

    /// Fait échouer la lecture de l'annuaire.
    pub fn set_failure(&self, fails: bool) {
        if let Ok(mut guard) = self.fails.lock() {
            *guard = fails;
        }
    }

    /// Guildes interrogées.
    pub fn queried_guilds(&self) -> Vec<GuildId> {
        match self.guilds.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }
}

#[async_trait]
impl OverlayDirectory for FakeOverlayDirectory {
    async fn list_connected(&self, guild_id: &GuildId) -> Result<Vec<ConnectedOverlay>, BotError> {
        if let Ok(mut guard) = self.guilds.lock() {
            guard.push(guild_id.clone());
        }
        let fails = match self.fails.lock() {
            Ok(guard) => *guard,
            Err(_) => false,
        };
        if fails {
            return Err(BotError::Playback("annuaire en échec".to_owned()));
        }
        Ok(match self.overlays.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        })
    }
}

/// Contexte complet pour les tests : base mémoire + horloge figée + backends
/// factices.
pub struct FakeBotContext {
    db: Db,
    clock: FixedClock,
    config: BotConfig,
    media: FakeMediaBackend,
    playback: FakePlaybackBackend,
    meme: FakeMemeBackend,
    overlays: FakeOverlayDirectory,
}

impl FakeBotContext {
    /// Ouvre une base `lc-db` en mémoire migrée, horloge à [`sample_now`].
    pub async fn new() -> Result<Self, lc_db::DbError> {
        let db = Db::open_in_memory(&DbConfig::default()).await?;
        Ok(Self {
            db,
            clock: FixedClock::new(sample_now()),
            config: BotConfig::new("http://localhost:3333"),
            media: FakeMediaBackend::new(),
            playback: FakePlaybackBackend::new(),
            meme: FakeMemeBackend::new(),
            overlays: FakeOverlayDirectory::new(),
        })
    }

    /// Contexte dont l'ingestion renvoie `asset`.
    pub async fn with_media_asset(asset: MediaAsset) -> Result<Self, lc_db::DbError> {
        let mut context = Self::new().await?;
        context.media = FakeMediaBackend::with_asset(asset);
        Ok(context)
    }

    /// Remplace la configuration.
    pub fn set_config(&mut self, config: BotConfig) {
        self.config = config;
    }

    /// Accès mutable au backend média.
    pub fn media_mut(&mut self) -> &mut FakeMediaBackend {
        &mut self.media
    }

    /// Accès mutable au backend de lecture.
    pub fn playback_mut(&mut self) -> &mut FakePlaybackBackend {
        &mut self.playback
    }

    /// Accès mutable au backend meme board.
    pub fn meme_mut(&mut self) -> &mut FakeMemeBackend {
        &mut self.meme
    }

    /// Accès mutable à l'annuaire d'overlays.
    pub fn overlays_mut(&mut self) -> &mut FakeOverlayDirectory {
        &mut self.overlays
    }

    /// Horloge figée du contexte.
    pub fn fixed_clock(&self) -> &FixedClock {
        &self.clock
    }
}

#[async_trait]
impl BotContext for FakeBotContext {
    fn db(&self) -> &Db {
        &self.db
    }

    fn clock(&self) -> &dyn Clock {
        &self.clock
    }

    fn config(&self) -> &BotConfig {
        &self.config
    }

    fn media(&self) -> &dyn MediaBackend {
        &self.media
    }

    fn playback(&self) -> &dyn PlaybackBackend {
        &self.playback
    }

    fn meme(&self) -> &dyn crate::context::MemeBackend {
        &self.meme
    }

    async fn list_connected_overlays(
        &self,
        guild_id: &GuildId,
    ) -> Result<Vec<ConnectedOverlay>, BotError> {
        self.overlays.list_connected(guild_id).await
    }

    async fn update_guild_settings(
        &self,
        guild_id: &GuildId,
        patch: GuildSettingsPatch,
    ) -> Result<(), BotError> {
        self.db
            .guilds()
            .update_settings(
                guild_id,
                patch.default_media_time,
                patch.max_media_time,
                patch.display_media_full,
            )
            .await?;
        Ok(())
    }
}

/// Construit un auteur administrateur de test.
pub fn admin_author() -> Author {
    Author {
        id: "424242424242424242".to_owned(),
        username: "Admin".to_owned(),
        name: "Admin".to_owned(),
        image: None,
        is_admin: true,
    }
}

/// Construit un auteur non administrateur de test.
pub fn member_author() -> Author {
    Author {
        id: "515151515151515151".to_owned(),
        username: "john_doe".to_owned(),
        name: "John".to_owned(),
        image: Some("https://cdn.discordapp.com/avatars/1/abc.png?size=256".to_owned()),
        is_admin: false,
    }
}
