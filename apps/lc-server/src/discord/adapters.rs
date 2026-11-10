//! Adaptateurs entre [`crate::AppState`] et les traits [`lc_discord`].
//!
//! Les handlers du bot dépendent de traits injectés (pas de `lc-media` /
//! `lc-playback` en dur) :
//!
//! | trait | adaptateur réel | état Phase 6b |
//! |---|---|---|
//! | [`lc_discord::MemeBackend`] | [`DbMemeBackend`] (table `meme_board_items`) | branché |
//! | [`lc_discord::OverlayDirectory`] | [`DbOverlayDirectory`] (table `overlay_clients`) | branché (approximation du hub WS) |
//! | [`lc_discord::MediaBackend`] | [`UnavailableMediaBackend`] | placeholder : `lc-media` en cours (task-16) |
//! | [`lc_discord::PlaybackBackend`] | [`UnavailablePlaybackBackend`] | placeholder : `lc-playback` en cours |
//!
//! Les placeholders renvoient une erreur **typée et localisée** (jamais de
//! panic) : les commandes répondent proprement « erreur d'ingestion » tant que
//! les lanes média/playback n'exposent pas leur API. [`DiscordAdapters`]
//! accepte des implémentations réelles via [`DiscordAdapters::with_media`] et
//! [`DiscordAdapters::with_playback`] sans changer le runtime ; le point de
//! branchement exact est documenté dans `docs/rust/server-discord.md`.

use std::sync::Arc;

use async_trait::async_trait;
use lc_db::Db;
use lc_discord::context::{
    ConnectedOverlay, MediaBackend, MediaRequest, MemeAddRequest, MemeBackend, OverlayDirectory,
    PlaybackBackend, PlaybackJobRequest,
};
use lc_discord::error::MediaLimits;
use lc_discord::{BotConfig, BotError, Lang, MediaError, MediaErrorCode, SpeechFailure};
use lc_domain::{
    Clock, DomainError, GuildId, MediaAsset, MemeBoardItem, MemeBoardItemId, OverlayClient,
    SystemClock,
};

use crate::Config;

/// Nombre maximal d'overlays listés par `/overlays` (borne défensive : la
/// réponse Discord est limitée à 4096 caractères par embed).
pub const MAX_OVERLAYS_LISTED: usize = 25;

/// Construit la [`BotConfig`] du crate `lc-discord` à partir de la config
/// serveur [`Config`] (mêmes variables d'environnement que le legacy).
pub fn bot_config_from(config: &Config) -> BotConfig {
    BotConfig {
        api_url: config.api_url.to_string(),
        lang: Lang::parse(&config.i18n),
        pairing_code_ttl_minutes: saturating_i64(config.pairing_code_ttl_minutes),
        default_duration_sec: saturating_i64(config.default_duration),
        hide_commands_disabled: config.hide_commands_disabled,
        media: MediaLimits {
            max_size_mb: saturating_i64(config.media_max_size_mb),
            max_cache_total_mb: saturating_i64(config.media_cache_max_total_mb),
            max_board_total_mb: saturating_i64(config.media_board_max_total_mb),
        },
    }
}

fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Adaptateurs [`lc_discord::BotContext`] du serveur.
#[derive(Clone)]
pub struct DiscordAdapters {
    media: Arc<dyn MediaBackend>,
    playback: Arc<dyn PlaybackBackend>,
    meme: Arc<dyn MemeBackend>,
    overlays: Arc<dyn OverlayDirectory>,
}

impl DiscordAdapters {
    /// Assemble des adaptateurs explicites (point d'injection pour les lanes
    /// média et playback).
    pub fn new(
        media: Arc<dyn MediaBackend>,
        playback: Arc<dyn PlaybackBackend>,
        meme: Arc<dyn MemeBackend>,
        overlays: Arc<dyn OverlayDirectory>,
    ) -> Self {
        Self {
            media,
            playback,
            meme,
            overlays,
        }
    }

    /// Adaptateurs par défaut : meme board et overlays branchés sur `lc-db`,
    /// média et playback en attente des lanes Phase 6b.
    pub fn placeholders(db: Db) -> Self {
        Self::with_clock(db, Arc::new(SystemClock))
    }

    /// Comme [`DiscordAdapters::placeholders`], avec une horloge injectée
    /// (utile pour les tests déterministes).
    pub fn with_clock(db: Db, clock: Arc<dyn Clock>) -> Self {
        Self {
            media: Arc::new(UnavailableMediaBackend),
            playback: Arc::new(UnavailablePlaybackBackend),
            meme: Arc::new(DbMemeBackend::new(db.clone(), Arc::clone(&clock))),
            overlays: Arc::new(DbOverlayDirectory::new(db)),
        }
    }

    /// Remplace l'adaptateur média (branchement `lc-media`, task-16).
    pub fn with_media(mut self, media: Arc<dyn MediaBackend>) -> Self {
        self.media = media;
        self
    }

    /// Remplace l'adaptateur playback (branchement `lc-playback`).
    pub fn with_playback(mut self, playback: Arc<dyn PlaybackBackend>) -> Self {
        self.playback = playback;
        self
    }

    /// Adaptateur média courant.
    pub fn media(&self) -> Arc<dyn MediaBackend> {
        Arc::clone(&self.media)
    }

    /// Adaptateur playback courant.
    pub fn playback(&self) -> Arc<dyn PlaybackBackend> {
        Arc::clone(&self.playback)
    }

    /// Adaptateur meme board courant.
    pub fn meme(&self) -> Arc<dyn MemeBackend> {
        Arc::clone(&self.meme)
    }

    /// Annuaire des overlays connectés.
    pub fn overlays(&self) -> Arc<dyn OverlayDirectory> {
        Arc::clone(&self.overlays)
    }
}

/// Adaptateur média indisponible (lane `lc-media` en cours).
pub struct UnavailableMediaBackend;

#[async_trait]
impl MediaBackend for UnavailableMediaBackend {
    async fn ingest_from_source(
        &self,
        _request: MediaRequest,
    ) -> Result<Option<MediaAsset>, MediaError> {
        Err(MediaError::with_detail(
            MediaErrorCode::UnsupportedSource,
            "adaptateur média non branché (lc-media, task-16)",
        ))
    }

    async fn ingest_speech(&self, _lang: Lang, _text: &str) -> Result<MediaAsset, SpeechFailure> {
        Err(SpeechFailure::GenerationFailed)
    }
}

/// Adaptateur playback indisponible (lane `lc-playback` en cours).
pub struct UnavailablePlaybackBackend;

#[async_trait]
impl PlaybackBackend for UnavailablePlaybackBackend {
    async fn enqueue(&self, _request: PlaybackJobRequest) -> Result<(), BotError> {
        Err(BotError::Playback(
            "adaptateur playback non branché (lc-playback)".to_owned(),
        ))
    }

    async fn stop_guild(&self, _guild_id: &GuildId) -> Result<u64, BotError> {
        Err(BotError::Playback(
            "adaptateur playback non branché (lc-playback)".to_owned(),
        ))
    }
}

/// Adaptateur meme board sur la table `meme_board_items` de `lc-db`.
pub struct DbMemeBackend {
    db: Db,
    clock: Arc<dyn Clock>,
}

impl DbMemeBackend {
    /// Construit l'adaptateur.
    pub fn new(db: Db, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }
}

#[async_trait]
impl MemeBackend for DbMemeBackend {
    async fn add_to_board(&self, request: MemeAddRequest) -> Result<bool, BotError> {
        if self
            .db
            .meme_board()
            .get_by_guild_and_asset(&request.guild_id, request.media_asset_id)
            .await?
            .is_some()
        {
            return Ok(false);
        }

        let item = MemeBoardItem {
            id: MemeBoardItemId::new_v4(),
            guild_id: request.guild_id,
            media_asset_id: request.media_asset_id,
            title: request.title,
            message: request.message,
            created_by_discord_user_id: request.created_by_discord_user_id,
            created_by_name: request.created_by_name,
            created_at: self.clock.now(),
        };

        match self.db.meme_board().create(&item).await {
            Ok(()) => Ok(true),
            Err(error) if is_conflict(&error) => Ok(false),
            Err(error) => Err(BotError::Db(error)),
        }
    }
}

/// `true` si l'erreur `lc-db` est une violation d'unicité (course entre deux
/// ajouts simultanés du même média, comme le `P2002` traité par le legacy).
///
/// `lc-db` convertit les violations `UNIQUE`/`PRIMARY KEY` en
/// [`lc_domain::DomainError::Conflict`] au moment de la requête.
fn is_conflict(error: &lc_db::DbError) -> bool {
    matches!(error, lc_db::DbError::Domain(DomainError::Conflict(_)))
}

/// Adaptateur [`OverlayDirectory`] sur la table `overlay_clients` de `lc-db`.
///
/// Approximation fidèle en attendant le hub WebSocket (task-17) : un overlay est
/// « connecté » s'il a une ligne **active** (non révoquée) dans la base. Le label
/// vient de `overlay_clients.label`.
pub struct DbOverlayDirectory {
    db: Db,
}

impl DbOverlayDirectory {
    /// Construit l'annuaire.
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl OverlayDirectory for DbOverlayDirectory {
    async fn list_connected(&self, guild_id: &GuildId) -> Result<Vec<ConnectedOverlay>, BotError> {
        let clients: Vec<OverlayClient> = self.db.overlay_clients().list_by_guild(guild_id).await?;

        Ok(clients
            .into_iter()
            .filter(OverlayClient::is_active)
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

    use chrono::{DateTime, TimeZone, Utc};
    use lc_db::{Db, DbConfig};
    use lc_discord::context::{MediaRequest, MemeAddRequest, MemeBackend, OverlayDirectory};
    use lc_discord::{BotContext, DbBotContext};
    use lc_domain::{
        FixedClock, Guild, GuildId, MediaAsset, MediaAssetId, MediaKind, MediaStatus,
        OverlayClient, OverlayClientId, OverlaySessionMode,
    };

    use crate::Config;

    use super::{
        bot_config_from, DbMemeBackend, DbOverlayDirectory, DiscordAdapters, MAX_OVERLAYS_LISTED,
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

    fn sample_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap()
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

    async fn insert_asset(db: &Db, at: DateTime<Utc>) -> MediaAssetId {
        let id = MediaAssetId::new_v4();
        db.media_assets()
            .insert(&MediaAsset {
                id,
                source_hash: format!("hash-{id}"),
                source_url: "https://example.test/meme.png".to_owned(),
                kind: MediaKind::Image,
                mime: "image/png".to_owned(),
                duration_sec: None,
                width: Some(640),
                height: Some(480),
                is_vertical: false,
                storage_path: "media/meme.png".to_owned(),
                size_bytes: 1024,
                status: MediaStatus::Ready,
                error: None,
                last_accessed_at: at,
                expires_at: at + chrono::Duration::days(1),
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
        at: DateTime<Utc>,
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
        let bot = bot_config_from(&config);
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
        let bot = bot_config_from(&config);
        assert_eq!(bot.lang, lc_discord::Lang::Fr);
        assert_eq!(bot.pairing_code_ttl_minutes, 5);
        assert_eq!(bot.default_duration_sec, 5);
        assert!(!bot.hide_commands_disabled);
        assert_eq!(bot.media.max_size_mb, 2048);
        assert_eq!(bot.media.max_cache_total_mb, 5120);
        assert_eq!(bot.media.max_board_total_mb, 15360);
    }

    #[tokio::test]
    async fn adapters_placeholders_renvoient_des_erreurs_typees() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let adapters = DiscordAdapters::placeholders(db);
        let media = adapters.media();
        let error = media
            .ingest_from_source(MediaRequest::new(
                Some("https://example.test/x".to_owned()),
                None,
                false,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code, lc_discord::MediaErrorCode::UnsupportedSource);
        let playback = adapters.playback();
        assert!(playback.stop_guild(&guild_id()).await.is_err());
        assert_eq!(MAX_OVERLAYS_LISTED, 25);
    }

    #[tokio::test]
    async fn meme_board_cree_puis_detecte_le_doublon() {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        insert_guild(&db).await;
        let asset = insert_asset(&db, sample_now()).await;
        let backend = DbMemeBackend::new(db.clone(), Arc::new(FixedClock::new(sample_now())));
        let request = MemeAddRequest {
            guild_id: guild_id(),
            media_asset_id: asset,
            title: Some("Titre".to_owned()),
            message: None,
            created_by_discord_user_id: Some("424242424242424242".to_owned()),
            created_by_name: Some("Alice".to_owned()),
        };
        assert!(backend.add_to_board(request.clone()).await.unwrap());
        assert!(!backend.add_to_board(request).await.unwrap());
        let items = db.meme_board().list_by_guild(&guild_id()).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title.as_deref(), Some("Titre"));
        assert_eq!(items[0].created_by_name.as_deref(), Some("Alice"));
        assert_eq!(items[0].created_at, sample_now());
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
        let adapters =
            DiscordAdapters::with_clock(db.clone(), Arc::new(FixedClock::new(sample_now())));
        let context = DbBotContext::with_clock(
            db,
            FixedClock::new(sample_now()),
            bot_config_from(&config_with(&[])),
            adapters.media(),
            adapters.playback(),
            adapters.meme(),
            adapters.overlays(),
        );
        assert_eq!(context.config().pairing_code_ttl_minutes, 5);
        assert_eq!(context.clock().now(), sample_now());
        assert_eq!(context.config().media.max_board_total_mb, 15360);
    }
}
