//! Adaptateurs entre [`crate::AppState`] et les traits [`lc_discord`].
//!
//! | trait | adaptateur |
//! |---|---|
//! | [`lc_discord::MediaBackend`] | [`ServerMediaBackend`] : [`MediaService`] (ingestion) and [`crate::media::tts`] (speech) |
//! | [`lc_discord::PlaybackBackend`] | [`ServerPlaybackBackend`] : `playback_jobs` + engine notification, admin stop |
//! | [`lc_discord::MediaBoardBackend`] | [`DbMediaBoardBackend`] : [`crate::media::board::add_to_board`] (quota, pin) |
//! | [`lc_discord::OverlayDirectory`] | [`DbOverlayDirectory`] : active `overlay_clients` with a live socket |

use std::sync::Arc;

use async_trait::async_trait;
use lc_db::Db;
use lc_discord::context::{
    BotConfigSource, ConnectedOverlay, MediaAddRequest, MediaBackend, MediaBoardBackend,
    MediaRequest, OverlayDirectory, PlaybackBackend, PlaybackJobRequest,
};
use lc_discord::error::MediaLimits;
use lc_discord::{BotConfig, BotError, Lang, MediaError, MediaErrorCode, SpeechFailure};
use lc_domain::entities::PlaybackJob;
use lc_domain::enums::JobStatus;
use lc_domain::ids::PlaybackJobId;
use lc_domain::{Clock, GuildId, MediaAsset, OverlayClient};
use lc_media::config::MediaConfig;
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};

use crate::app::AppState;
use crate::http::admin::hub::{AdminHub, NoopAdminHub};
use crate::media::errors::MediaServiceError;
use crate::media::service::MediaService;
use crate::runtime::Runtime;
use crate::Config;

/// Nombre maximal d'overlays listés par `/overlays` (borne défensive : la
/// réponse Discord est limitée à 4096 caractères par embed).
pub const MAX_OVERLAYS_LISTED: usize = 25;

/// Builds the [`BotConfig`] of the `lc-discord` crate: the frozen [`Config`]
/// fields plus the current values of the hot [`Runtime`] store.
pub fn bot_config_from(config: &Config, runtime: &Runtime) -> BotConfig {
    BotConfig {
        api_url: config.api_url.to_string(),
        lang: Lang::parse(&config.i18n),
        pairing_code_ttl_minutes: saturating_i64(config.pairing_code_ttl_minutes),
        default_duration_sec: saturating_i64(runtime.default_duration()),
        hide_commands_disabled: config.hide_commands_disabled,
        media: MediaLimits {
            max_size_mb: saturating_i64(runtime.media_max_size_mb()),
            max_cache_total_mb: saturating_i64(runtime.media_cache_max_total_mb()),
            max_board_total_mb: saturating_i64(runtime.media_board_max_total_mb()),
        },
    }
}

fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Live [`BotConfigSource`] for the Discord runtime: the non-hot fields come
/// from the server [`Config`], the hot ones are read from the [`Runtime`] store
/// on every command.
pub struct ServerBotConfigSource {
    state: AppState,
}

impl ServerBotConfigSource {
    /// Builds the source over the application state's config and runtime store.
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl BotConfigSource for ServerBotConfigSource {
    fn current(&self) -> BotConfig {
        bot_config_from(self.state.config.as_ref(), self.state.runtime())
    }
}

/// Adaptateurs [`lc_discord::BotContext`] du serveur.
#[derive(Clone)]
pub struct DiscordAdapters {
    media: Arc<dyn MediaBackend>,
    playback: Arc<dyn PlaybackBackend>,
    media_board: Arc<dyn MediaBoardBackend>,
    overlays: Arc<dyn OverlayDirectory>,
}

impl DiscordAdapters {
    /// Assemble des adaptateurs explicites.
    pub fn new(
        media: Arc<dyn MediaBackend>,
        playback: Arc<dyn PlaybackBackend>,
        media_board: Arc<dyn MediaBoardBackend>,
        overlays: Arc<dyn OverlayDirectory>,
    ) -> Self {
        Self {
            media,
            playback,
            media_board,
            overlays,
        }
    }

    /// The server's adapters: media service, playback queue and WebSocket hub
    /// of `state`.
    pub fn for_state(state: &AppState, db: Db) -> Self {
        let hub = Arc::clone(&state.admin_hub);
        Self {
            media: Arc::new(ServerMediaBackend::new(state.clone())),
            playback: Arc::new(ServerPlaybackBackend::new(state.clone(), db.clone())),
            media_board: Arc::new(
                DbMediaBoardBackend::new(db.clone(), Arc::clone(&state.clock))
                    .with_admin_hub(Arc::clone(&hub))
                    .with_runtime_state(state.clone()),
            ),
            overlays: Arc::new(DbOverlayDirectory::new(db).with_admin_hub(hub)),
        }
    }

    /// Adaptateur média courant.
    pub fn media(&self) -> Arc<dyn MediaBackend> {
        Arc::clone(&self.media)
    }

    /// Adaptateur playback courant.
    pub fn playback(&self) -> Arc<dyn PlaybackBackend> {
        Arc::clone(&self.playback)
    }

    /// Adaptateur board média courant.
    pub fn media_board(&self) -> Arc<dyn MediaBoardBackend> {
        Arc::clone(&self.media_board)
    }

    /// Annuaire des overlays connectés.
    pub fn overlays(&self) -> Arc<dyn OverlayDirectory> {
        Arc::clone(&self.overlays)
    }
}

/// Converts a media service error to the bot's localized error.
fn bot_media_error(error: &MediaServiceError) -> MediaError {
    match error.media_error() {
        Some(media_error) => {
            let wanted = media_error.code().as_str();
            let code = MediaErrorCode::ALL
                .iter()
                .copied()
                .find(|code| code.as_str() == wanted)
                .unwrap_or(MediaErrorCode::DownloadFailed);
            MediaError::with_detail(code, media_error.message())
        }
        None => MediaError::with_detail(MediaErrorCode::DownloadFailed, error.to_string()),
    }
}

/// Ingestion through the server's [`MediaService`].
pub struct ServerMediaBackend {
    state: AppState,
}

impl ServerMediaBackend {
    /// Builds the adapter.
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl MediaBackend for ServerMediaBackend {
    async fn ingest_from_source(
        &self,
        request: MediaRequest,
    ) -> Result<Option<MediaAsset>, MediaError> {
        if !request.has_source() {
            return Ok(None);
        }
        let service =
            MediaService::for_state(&self.state).map_err(|error| bot_media_error(&error))?;
        match service
            .ingest(
                request.url.as_deref(),
                request.media.as_deref(),
                request.force_refresh,
            )
            .await
        {
            Ok(asset) => Ok(Some(asset)),
            Err(error) => {
                tracing::warn!(%error, "ingestion d'un média Discord échouée");
                Err(bot_media_error(&error))
            }
        }
    }

    async fn ingest_speech(&self, lang: Lang, text: &str) -> Result<MediaAsset, SpeechFailure> {
        crate::media::tts::ingest_speech(&self.state, lang, text).await
    }
}

/// Playback queue of the server: the job is stored and the engine notified,
/// exactly like `POST /ingest`.
pub struct ServerPlaybackBackend {
    state: AppState,
    db: Db,
}

impl ServerPlaybackBackend {
    /// Builds the adapter.
    pub fn new(state: AppState, db: Db) -> Self {
        Self { state, db }
    }
}

#[async_trait]
impl PlaybackBackend for ServerPlaybackBackend {
    async fn enqueue(&self, request: PlaybackJobRequest) -> Result<(), BotError> {
        let now = self.state.clock.now();
        let job = PlaybackJob {
            id: PlaybackJobId::new_v4(),
            guild_id: request.guild_id,
            media_asset_id: request.media_asset_id,
            text: request.text,
            show_text: request.show_text,
            author_name: request.author_name,
            author_image: request.author_image,
            duration_sec: request.duration_sec.max(1),
            priority: i64::from(request.priority),
            status: JobStatus::Pending,
            resumes_after_job_id: None,
            resume_offset_sec: request.resume_offset_sec.max(0),
            remaining_ms_snapshot: None,
            last_playback_state_at: None,
            submission_date: now,
            execution_date: now,
            scheduled_at: now,
            started_at: None,
            finished_at: None,
        };
        self.db.playback_jobs().create(&job).await?;
        crate::media::hook::notify_ingested(&job).await;
        tracing::info!(
            job_id = %job.id.to_canonical(),
            guild_id = job.guild_id.as_str(),
            source = %request.source,
            "job de lecture créé par une commande Discord"
        );
        Ok(())
    }

    async fn stop_guild(&self, guild_id: &GuildId) -> Result<u64, BotError> {
        let stop = crate::http::admin::stop_guild_playback(&self.state, &self.db, guild_id, false)
            .await
            .map_err(|error| BotError::Playback(format!("{error:?}")))?;
        Ok(stop.released_playing_jobs)
    }
}

/// Adaptateur board média (`/media-add`) : same rules as the board's HTTP
/// import (quota, pinned media, embedded cover).
pub struct DbMediaBoardBackend {
    db: Db,
    clock: Arc<dyn Clock>,
    hub: Arc<dyn AdminHub>,
    media_config: MediaConfig,
    /// Live application state; when set, the media config is rebuilt on every
    /// use so the hot limits apply without restarting the bot.
    state: Option<AppState>,
}

impl DbMediaBoardBackend {
    /// Construit l'adaptateur (hub inerte, configuration média par défaut).
    pub fn new(db: Db, clock: Arc<dyn Clock>) -> Self {
        Self {
            db,
            clock,
            hub: Arc::new(NoopAdminHub),
            media_config: MediaConfig::default(),
            state: None,
        }
    }

    /// Injects the real WebSocket hub used to broadcast `board.changed`.
    #[must_use]
    pub fn with_admin_hub(mut self, hub: Arc<dyn AdminHub>) -> Self {
        self.hub = hub;
        self
    }

    /// Media configuration (board quota).
    #[must_use]
    pub fn with_media_config(mut self, media_config: MediaConfig) -> Self {
        self.media_config = media_config;
        self
    }

    /// Rebuilds the media configuration from this live state on every use.
    #[must_use]
    pub fn with_runtime_state(mut self, state: AppState) -> Self {
        self.state = Some(state);
        self
    }

    fn current_media_config(&self) -> MediaConfig {
        self.state.as_ref().map_or_else(
            || self.media_config.clone(),
            crate::media::settings::media_config_for_state,
        )
    }
}

#[async_trait]
impl MediaBoardBackend for DbMediaBoardBackend {
    async fn add_to_board(&self, request: MediaAddRequest) -> Result<bool, BotError> {
        let Some(asset) = self.db.media_assets().get(request.media_asset_id).await? else {
            return Err(BotError::Media(MediaError::new(
                MediaErrorCode::MediaNotFound,
            )));
        };
        let media_config = self.current_media_config();
        let result = crate::media::board::add_to_board(
            &self.db,
            &media_config,
            &request.guild_id,
            &asset,
            request.title.as_deref(),
            request.message.as_deref(),
            request.created_by_discord_user_id.as_deref(),
            request.created_by_name.as_deref(),
            request.display_duration_sec,
            self.clock.now(),
        )
        .await
        .map_err(|error| match error {
            MediaServiceError::Database(db_error) => BotError::Db(db_error),
            other => BotError::Media(bot_media_error(&other)),
        })?;
        if result.created {
            self.hub.broadcast_to_guild(
                &result.item.guild_id,
                ServerMessage::BoardChanged(BoardChangedPayload {
                    change: BoardChange::Added,
                    item_id: result.item.id.to_canonical(),
                }),
            );
        }
        Ok(result.created)
    }
}

/// Adaptateur [`OverlayDirectory`] : active overlay clients of the guild that
/// have a live WebSocket on the hub. Without a hub (tests) every active client
/// is listed.
pub struct DbOverlayDirectory {
    db: Db,
    hub: Option<Arc<dyn AdminHub>>,
}

impl DbOverlayDirectory {
    /// Construit l'annuaire.
    pub fn new(db: Db) -> Self {
        Self { db, hub: None }
    }

    /// Lists only the clients connected to this hub.
    #[must_use]
    pub fn with_admin_hub(mut self, hub: Arc<dyn AdminHub>) -> Self {
        self.hub = Some(hub);
        self
    }
}

#[async_trait]
impl OverlayDirectory for DbOverlayDirectory {
    async fn list_connected(&self, guild_id: &GuildId) -> Result<Vec<ConnectedOverlay>, BotError> {
        let clients: Vec<OverlayClient> = self.db.overlay_clients().list_by_guild(guild_id).await?;

        Ok(clients
            .into_iter()
            .filter(OverlayClient::is_active)
            .filter(|client| {
                self.hub
                    .as_ref()
                    .is_none_or(|hub| hub.is_connected(&client.id.as_uuid()))
            })
            .take(MAX_OVERLAYS_LISTED)
            .map(|client| ConnectedOverlay {
                client_id: client.id.to_canonical(),
                label: client.label,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use lc_db::{Db, DbConfig};
    use lc_discord::context::{MediaAddRequest, MediaBoardBackend, OverlayDirectory};
    use lc_discord::{BotConfigSource, BotContext, BotError, DbBotContext, MediaErrorCode};
    use lc_domain::{
        FixedClock, Guild, GuildId, MediaAsset, MediaAssetId, MediaKind, MediaStatus,
        OverlayClient, OverlayClientId, OverlaySessionMode,
    };
    use time::UtcDateTime;

    use crate::runtime::{Runtime, RuntimeOverrides};
    use crate::Config;

    use super::{
        bot_config_from, DbMediaBoardBackend, DbOverlayDirectory, DiscordAdapters,
        ServerBotConfigSource,
    };

    fn config_with(pairs: &[(&str, &str)]) -> Config {
        let mut entries = vec![("API_URL".to_owned(), "http://localhost:3333".to_owned())];
        entries.extend(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        Config::from_pairs(entries).unwrap()
    }

    fn sample_now() -> UtcDateTime {
        time::macros::utc_datetime!(2024-01-02 3:04:05)
    }

    fn guild_id() -> GuildId {
        GuildId::parse("424242424242424242").unwrap()
    }

    async fn insert_guild(db: &Db) {
        db.guilds()
            .upsert(&Guild {
                id: guild_id(),
                name: Some("Guilde test".to_owned()),
                busy_until: None,
                default_media_time: None,
                max_media_time: None,
                display_media_full: false,
            })
            .await
            .unwrap();
    }

    async fn insert_asset(db: &Db, at: UtcDateTime) -> MediaAssetId {
        let id = MediaAssetId::new_v4();
        db.media_assets()
            .insert(&MediaAsset {
                id,
                source_hash: format!("hash-{id}"),
                source_url: "https://example.test/media.png".to_owned(),
                source_title: None,
                kind: MediaKind::Image,
                mime: "image/png".to_owned(),
                duration_sec: None,
                width: Some(640),
                height: Some(480),
                is_vertical: false,
                storage_path: "media/media.png".to_owned(),
                size_bytes: 1024,
                status: MediaStatus::Ready,
                error: None,
                last_accessed_at: at,
                expires_at: at + time::Duration::days(1),
                created_at: at,
            })
            .await
            .unwrap();
        id
    }

    async fn insert_overlay(
        db: &Db,
        label: &str,
        mode: OverlaySessionMode,
        at: UtcDateTime,
    ) -> OverlayClientId {
        let client = OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: guild_id(),
            label: label.to_owned(),
            token_hash: format!("hash-{label}"),
            session_mode: mode,
            default_author_name: Some("Overlay".to_owned()),
            default_author_image: None,
            created_by_discord_user_id: Some("424242424242424242".to_owned()),
            last_seen_at: Some(at),
            created_at: at,
            revoked_at: None,
        };
        let id = client.id;
        db.overlay_clients().create(&client).await.unwrap();
        id
    }

    #[test]
    fn bot_config_reflete_les_variables_serveur() {
        let config = config_with(&[
            ("I18N", "en"),
            ("PAIRING_CODE_TTL_MINUTES", "12"),
            ("DEFAULT_DURATION", "9"),
            ("HIDE_COMMANDS_DISABLED", "true"),
            ("MEDIA_MAX_SIZE_MB", "143"),
            ("MEDIA_CACHE_MAX_TOTAL_MB", "2048"),
            ("MEDIA_BOARD_MAX_TOTAL_MB", "4096"),
        ]);
        let runtime = Runtime::new(Arc::new(config.clone()));
        let bot = bot_config_from(&config, &runtime);
        assert_eq!(bot.api_url, "http://localhost:3333/");
        assert_eq!(bot.lang, lc_discord::Lang::En);
        assert_eq!(bot.pairing_code_ttl_minutes, 12);
        assert_eq!(bot.default_duration_sec, 9);
        assert!(bot.hide_commands_disabled);
        assert_eq!(bot.media.max_size_mb, 143);
        assert_eq!(bot.media.max_cache_total_mb, 2048);
        assert_eq!(bot.media.max_board_total_mb, 4096);
    }

    #[test]
    fn bot_config_valeurs_par_defaut_legacy() {
        let config = config_with(&[]);
        let runtime = Runtime::new(Arc::new(config.clone()));
        let bot = bot_config_from(&config, &runtime);
        assert_eq!(bot.lang, lc_discord::Lang::Fr);
        assert_eq!(bot.pairing_code_ttl_minutes, 5);
        assert_eq!(bot.default_duration_sec, 5);
        assert!(!bot.hide_commands_disabled);
        assert_eq!(bot.media.max_size_mb, 2048);
        assert_eq!(bot.media.max_cache_total_mb, 5120);
        assert_eq!(bot.media.max_board_total_mb, 15360);
    }

    #[test]
    fn bot_config_follows_the_hot_runtime_values() {
        let config = config_with(&[]);
        let runtime = Runtime::new(Arc::new(config.clone()));
        runtime.apply(RuntimeOverrides {
            default_duration: Some(17),
            media_max_size_mb: Some(11),
            media_cache_max_total_mb: Some(22),
            media_board_max_total_mb: Some(33),
            ..RuntimeOverrides::default()
        });

        let bot = bot_config_from(&config, &runtime);
        assert_eq!(bot.default_duration_sec, 17);
        assert_eq!(bot.media.max_size_mb, 11);
        assert_eq!(bot.media.max_cache_total_mb, 22);
        assert_eq!(bot.media.max_board_total_mb, 33);
    }

    #[tokio::test]
    async fn server_bot_config_source_reads_the_hot_values_on_every_use() {
        let (shutdown, _signal) = crate::shutdown::Shutdown::new();
        let state = crate::app::AppState::new(Arc::new(config_with(&[])), None, shutdown);
        let source = ServerBotConfigSource::new(state.clone());
        assert_eq!(source.current().default_duration_sec, 5);

        state.runtime().apply(RuntimeOverrides {
            default_duration: Some(21),
            media_board_max_total_mb: Some(44),
            ..RuntimeOverrides::default()
        });
        let bot = source.current();
        assert_eq!(bot.default_duration_sec, 21);
        assert_eq!(bot.media.max_board_total_mb, 44);
    }

    #[tokio::test]
    async fn media_board_cree_puis_detecte_le_doublon() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        insert_guild(&db).await;
        let asset = insert_asset(&db, sample_now()).await;
        let backend = DbMediaBoardBackend::new(db.clone(), Arc::new(FixedClock::new(sample_now())));
        let request = MediaAddRequest {
            guild_id: guild_id(),
            media_asset_id: asset,
            title: Some("Titre".to_owned()),
            message: None,
            created_by_discord_user_id: Some("424242424242424242".to_owned()),
            created_by_name: Some("Alice".to_owned()),
            display_duration_sec: None,
        };
        assert!(backend.add_to_board(request.clone()).await.unwrap());
        assert!(!backend.add_to_board(request).await.unwrap());
        let items = db.media_board().list_by_guild(&guild_id()).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title.as_deref(), Some("Titre"));
        assert_eq!(items[0].created_by_name.as_deref(), Some("Alice"));
        assert_eq!(items[0].created_at, sample_now());
    }

    #[tokio::test]
    async fn media_board_backend_follows_the_hot_board_quota() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        insert_guild(&db).await;
        let asset = insert_asset(&db, sample_now()).await;
        let state = test_state(&db);
        let adapters = DiscordAdapters::for_state(&state, db.clone());
        let backend = adapters.media_board();
        let request = MediaAddRequest {
            guild_id: guild_id(),
            media_asset_id: asset,
            title: None,
            message: None,
            created_by_discord_user_id: None,
            created_by_name: Some("Alice".to_owned()),
            display_duration_sec: None,
        };

        // No guild quota: the server-wide hot limit applies. Zero megabytes
        // leaves no room for the 1 KiB asset.
        state.runtime().apply(RuntimeOverrides {
            media_board_max_total_mb: Some(0),
            ..RuntimeOverrides::default()
        });
        let error = backend.add_to_board(request).await.unwrap_err();
        let BotError::Media(media) = error else {
            panic!("media error expected, got {error:?}");
        };
        assert_eq!(media.code, MediaErrorCode::BoardStorageLimitReached);
    }

    #[tokio::test]
    async fn overlay_directory_ignore_les_clients_revoques() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        insert_guild(&db).await;
        let at = sample_now();
        let active = insert_overlay(&db, "Salon", OverlaySessionMode::Normal, at).await;
        let revoked = insert_overlay(&db, "Vieux", OverlaySessionMode::InviteReadOnly, at).await;
        db.overlay_clients().revoke(revoked, at).await.unwrap();

        let directory = DbOverlayDirectory::new(db.clone());
        let listed = directory.list_connected(&guild_id()).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].client_id, active.to_canonical());
        assert_eq!(listed[0].label, "Salon");
    }

    #[tokio::test]
    async fn db_bot_context_utilise_les_adaptateurs() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let (shutdown, _signal) = crate::shutdown::Shutdown::new();
        let state =
            crate::app::AppState::new(Arc::new(config_with(&[])), Some(db.clone()), shutdown)
                .with_clock(Arc::new(FixedClock::new(sample_now())));
        let adapters = DiscordAdapters::for_state(&state, db.clone());
        let source = Arc::new(ServerBotConfigSource::new(state));
        let context = DbBotContext::with_clock(
            db,
            FixedClock::new(sample_now()),
            source,
            adapters.media(),
            adapters.playback(),
            adapters.media_board(),
            adapters.overlays(),
            Arc::new(lc_domain::rights::RightsTable::new()),
        );
        assert_eq!(context.config().pairing_code_ttl_minutes, 5);
        assert_eq!(context.clock().now(), sample_now());
        assert_eq!(context.config().media.max_board_total_mb, 15360);
    }

    fn test_state(db: &Db) -> crate::app::AppState {
        let (shutdown, _signal) = crate::shutdown::Shutdown::new();
        crate::app::AppState::new(Arc::new(config_with(&[])), Some(db.clone()), shutdown)
            .with_clock(Arc::new(FixedClock::new(sample_now())))
    }

    #[tokio::test]
    async fn playback_backend_queues_a_pending_job() {
        use lc_discord::context::{PlaybackBackend, PlaybackJobRequest};

        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        insert_guild(&db).await;
        let backend = super::ServerPlaybackBackend::new(test_state(&db), db.clone());
        backend
            .enqueue(PlaybackJobRequest {
                guild_id: guild_id(),
                media_asset_id: None,
                text: Some("salut".to_owned()),
                show_text: true,
                author_name: Some("john".to_owned()),
                author_image: None,
                duration_sec: 7,
                priority: 0,
                resume_offset_sec: 0,
                source: "discord_send_command".to_owned(),
            })
            .await
            .unwrap();
        let pending = db
            .playback_jobs()
            .list_pending_by_guild(&guild_id())
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text.as_deref(), Some("salut"));
        assert_eq!(pending[0].duration_sec, 7);
        assert_eq!(pending[0].author_name.as_deref(), Some("john"));
        assert_eq!(backend.stop_guild(&guild_id()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn overlay_directory_lists_only_connected_clients_with_a_hub() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        insert_guild(&db).await;
        insert_overlay(&db, "Salon", OverlaySessionMode::Normal, sample_now()).await;
        let directory = DbOverlayDirectory::new(db.clone())
            .with_admin_hub(Arc::new(crate::http::admin::hub::NoopAdminHub));
        assert!(directory
            .list_connected(&guild_id())
            .await
            .unwrap()
            .is_empty());
    }
}
