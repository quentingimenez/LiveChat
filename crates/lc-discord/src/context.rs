//! Contexte d'exécution injecté dans les handlers.
//!
//! Les handlers ne connaissent ni serenity, ni `lc-media`, ni `lc-playback`
//! (dont les API sont encore en cours de construction en parallèle) : ils
//! dépendent uniquement de ce module. La production fournit
//! [`DbBotContext`] ; les tests fournissent `FakeBotContext` (voir le module
//! [`crate::testing`]).
//!
//! Hypothèse Phase 6a documentée dans `docs/rust/lc-discord.md` : les traits
//! [`MediaBackend`], [`PlaybackBackend`], [`MemeBackend`] et
//! [`OverlayDirectory`] sont les points d'intégration qui seront implémentés
//! par les adaptateurs réels en Phase 6b (respectivement au-dessus de
//! `lc-media`, `lc-playback` et des dépôts `lc-db`).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lc_db::Db;
use lc_domain::{Clock, Guild, GuildId, MediaAsset, MediaAssetId, PairingCode, SystemClock};

use crate::error::{BotError, MediaError, MediaLimits, SpeechFailure};
use crate::i18n::Lang;

/// Alphabet des codes d'appairage (identique au legacy).
pub const PAIRING_CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Longueur d'un code d'appairage (identique au legacy).
pub const PAIRING_CODE_LENGTH: usize = 6;
/// Nombre d'essais de génération d'un code unique (identique au legacy).
pub const PAIRING_CODE_MAX_ATTEMPTS: usize = 5;

/// Auteur d'une interaction Discord (déjà résolu par la glue).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Author {
    /// Snowflake Discord de l'auteur (`interaction.user.id`).
    pub id: String,
    /// `interaction.user.username` (utilisé par `/alive`, `/send`, `/talk`).
    pub username: String,
    /// Nom affiché résolu : pseudo de serveur, sinon nom global, sinon
    /// `username` (`resolveDiscordAuthorName` legacy).
    pub name: String,
    /// URL d'avatar résolue (`resolveDiscordAuthorImage` legacy), si connue.
    pub image: Option<String>,
    /// `true` si l'auteur possède la permission Administrateur dans la guilde.
    pub is_admin: bool,
}

impl Author {
    /// Auteur minimal à partir du nom d'utilisateur.
    pub fn new(username: impl Into<String>) -> Self {
        let username = username.into();
        Self {
            name: username.clone(),
            username,
            ..Self::default()
        }
    }
}

/// Configuration du bot (valeurs issues de `server/src/services/env.ts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotConfig {
    /// `env.API_URL` (affichée par `/client`).
    pub api_url: String,
    /// `env.I18N` (fr par défaut).
    pub lang: Lang,
    /// `env.PAIRING_CODE_TTL_MINUTES` (5 par défaut).
    pub pairing_code_ttl_minutes: i64,
    /// `env.DEFAULT_DURATION` (5 par défaut).
    pub default_duration_sec: i64,
    /// `env.HIDE_COMMANDS_DISABLED` (faux par défaut).
    pub hide_commands_disabled: bool,
    /// Limites de stockage pour les messages d'erreur média.
    pub media: MediaLimits,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            lang: Lang::Fr,
            pairing_code_ttl_minutes: 5,
            default_duration_sec: 5,
            hide_commands_disabled: false,
            media: MediaLimits::default(),
        }
    }
}

impl BotConfig {
    /// Configuration avec l'URL d'ingestion donnée et les autres valeurs
    /// legacy par défaut.
    pub fn new(api_url: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
            ..Self::default()
        }
    }
}

/// Mise à jour des réglages d'une guilde (`/config-defaut`, `/config-max`,
/// `/config-displayfull`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuildSettingsPatch {
    /// Durée par défaut des médias, `None` pour revenir au défaut global.
    pub default_media_time: Option<i64>,
    /// Durée maximale des médias, `None` pour aucune limite explicite.
    pub max_media_time: Option<i64>,
    /// Afficher les médias en plein écran.
    pub display_media_full: bool,
}

/// Overlay actuellement connecté (vue simplifiée pour `/overlays`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectedOverlay {
    /// Identifiant client affiché.
    pub client_id: String,
    /// Libellé affiché.
    pub label: String,
}

/// Demande d'ingestion média (`ingestMediaFromSource` legacy).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaRequest {
    /// URL fournie via l'option `url`.
    pub url: Option<String>,
    /// URL de la pièce jointe (`attachment.url || attachment.proxyURL`).
    pub media: Option<String>,
    /// Force le rafraîchissement du cache.
    pub force_refresh: bool,
}

impl MediaRequest {
    /// Construit une demande depuis les options de commande.
    pub fn new(url: Option<String>, media: Option<String>, force_refresh: bool) -> Self {
        Self {
            url,
            media,
            force_refresh,
        }
    }

    /// `true` si une source (URL ou pièce jointe) est fournie.
    pub fn has_source(&self) -> bool {
        self.url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || self
                .media
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
    }
}

/// Demande de création d'un job de lecture (`createPlaybackJob` legacy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackJobRequest {
    /// Guilde cible.
    pub guild_id: GuildId,
    /// Média à lire, si la lecture est basée sur un média.
    pub media_asset_id: Option<MediaAssetId>,
    /// Texte overlay (brut ou payload riche encodé).
    pub text: Option<String>,
    /// Afficher le texte sous le média.
    pub show_text: bool,
    /// Nom de l'auteur affiché par l'overlay.
    pub author_name: Option<String>,
    /// Image de l'auteur affichée par l'overlay.
    pub author_image: Option<String>,
    /// Durée effective calculée (déjà bornée par la guilde).
    pub duration_sec: i64,
    /// Priorité (0 par défaut dans le legacy).
    pub priority: i32,
    /// Offset de reprise en secondes.
    pub resume_offset_sec: i64,
    /// Source du job (`discord_send_command`, `discord_talk_command`…).
    pub source: String,
}

/// Demande d'ajout au meme board (`addToMemeBoard` legacy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemeAddRequest {
    /// Guilde du meme board.
    pub guild_id: GuildId,
    /// Média à épingler.
    pub media_asset_id: MediaAssetId,
    /// Titre optionnel (tronqué à 240 caractères).
    pub title: Option<String>,
    /// Message optionnel (tronqué à 500 caractères).
    pub message: Option<String>,
    /// Identifiant Discord de l'auteur.
    pub created_by_discord_user_id: Option<String>,
    /// Nom affiché de l'auteur.
    pub created_by_name: Option<String>,
}

/// Backend d'ingestion média (adaptateur `lc-media` en Phase 6b).
#[async_trait]
pub trait MediaBackend: Send + Sync {
    /// Ingère une source distante. `None` si aucun média n'a été produit
    /// (le handler répond alors `missing_media`, comme le legacy).
    async fn ingest_from_source(
        &self,
        request: MediaRequest,
    ) -> Result<Option<MediaAsset>, MediaError>;

    /// Synthétise un texte en MP3 et l'ingère (équivalent `gtts` +
    /// `ingestMediaFromLocalFile` du legacy).
    async fn ingest_speech(&self, lang: Lang, text: &str) -> Result<MediaAsset, SpeechFailure>;
}

/// Backend de lecture (adaptateur `lc-playback` en Phase 6b).
#[async_trait]
pub trait PlaybackBackend: Send + Sync {
    /// Enqueue un job de lecture.
    async fn enqueue(&self, request: PlaybackJobRequest) -> Result<(), BotError>;

    /// Arrête la lecture de la guilde et libère les jobs en cours
    /// (`executeManualStopForGuild` legacy). Retourne le nombre de jobs
    /// libérés.
    async fn stop_guild(&self, guild_id: &GuildId) -> Result<u64, BotError>;
}

/// Backend du meme board (dépôts `lc-db` + stockage en Phase 6b).
#[async_trait]
pub trait MemeBackend: Send + Sync {
    /// Ajoute (ou retrouve) un média sur le meme board.
    ///
    /// Retourne `true` si un nouvel élément a été créé. Une erreur de capacité
    /// doit être signalée via
    /// [`BotError::Media`] avec [`crate::MediaErrorCode::BoardStorageLimitReached`].
    async fn add_to_board(&self, request: MemeAddRequest) -> Result<bool, BotError>;
}

/// Annuaire des overlays connectés (socket.io `overlay-guild-*` en Phase 6b).
#[async_trait]
pub trait OverlayDirectory: Send + Sync {
    /// Overlays actuellement connectés pour la guilde.
    async fn list_connected(&self, guild_id: &GuildId) -> Result<Vec<ConnectedOverlay>, BotError>;
}

/// Dépendances injectées dans les handlers.
///
/// `BotContext` ne dépend pas de serenity : les tests fournissent une
/// implémentation en mémoire.
#[async_trait]
pub trait BotContext: Send + Sync {
    /// Accès à la base `lc-db`.
    fn db(&self) -> &Db;
    /// Horloge injectée (déterminisme des tests).
    fn clock(&self) -> &dyn Clock;
    /// Configuration du bot.
    fn config(&self) -> &BotConfig;
    /// Backend d'ingestion média.
    fn media(&self) -> &dyn MediaBackend;
    /// Backend de lecture.
    fn playback(&self) -> &dyn PlaybackBackend;
    /// Backend du meme board.
    fn meme(&self) -> &dyn MemeBackend;

    /// Overlays actuellement connectés pour la guilde.
    async fn list_connected_overlays(
        &self,
        guild_id: &GuildId,
    ) -> Result<Vec<ConnectedOverlay>, BotError>;

    /// Applique une mise à jour des réglages de la guilde.
    async fn update_guild_settings(
        &self,
        guild_id: &GuildId,
        patch: GuildSettingsPatch,
    ) -> Result<(), BotError>;

    /// Génère un code d'appairage aléatoire (alphabet legacy, 6 caractères).
    fn generate_pairing_code(&self) -> PairingCode {
        use rand::Rng as _;

        let mut rng = rand::rng();
        loop {
            let candidate: String = (0..PAIRING_CODE_LENGTH)
                .map(|_| {
                    let index = rng.random_range(0..PAIRING_CODE_ALPHABET.len());
                    char::from(PAIRING_CODE_ALPHABET[index])
                })
                .collect();
            if let Ok(code) = PairingCode::parse(&candidate) {
                return code;
            }
        }
    }
}

/// Contexte de production : base `lc-db` réelle, horloge système et backends
/// injectés.
pub struct DbBotContext<C: Clock = SystemClock> {
    db: Db,
    clock: C,
    config: BotConfig,
    media: Arc<dyn MediaBackend>,
    playback: Arc<dyn PlaybackBackend>,
    meme: Arc<dyn MemeBackend>,
    overlays: Arc<dyn OverlayDirectory>,
}

impl DbBotContext<SystemClock> {
    /// Construit un contexte de production.
    pub fn new(
        db: Db,
        config: BotConfig,
        media: Arc<dyn MediaBackend>,
        playback: Arc<dyn PlaybackBackend>,
        meme: Arc<dyn MemeBackend>,
        overlays: Arc<dyn OverlayDirectory>,
    ) -> Self {
        Self::with_clock(db, SystemClock, config, media, playback, meme, overlays)
    }
}

impl<C: Clock> DbBotContext<C> {
    /// Construit un contexte avec une horloge explicite (tests d'intégration).
    pub fn with_clock(
        db: Db,
        clock: C,
        config: BotConfig,
        media: Arc<dyn MediaBackend>,
        playback: Arc<dyn PlaybackBackend>,
        meme: Arc<dyn MemeBackend>,
        overlays: Arc<dyn OverlayDirectory>,
    ) -> Self {
        Self {
            db,
            clock,
            config,
            media,
            playback,
            meme,
            overlays,
        }
    }

    /// Référence à la base.
    pub fn db_ref(&self) -> &Db {
        &self.db
    }

    /// Référence à l'horloge.
    pub fn clock_ref(&self) -> &C {
        &self.clock
    }

    /// Référence à la configuration.
    pub fn config_ref(&self) -> &BotConfig {
        &self.config
    }
}

#[async_trait]
impl<C: Clock + Send + Sync> BotContext for DbBotContext<C> {
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
        self.media.as_ref()
    }

    fn playback(&self) -> &dyn PlaybackBackend {
        self.playback.as_ref()
    }

    fn meme(&self) -> &dyn MemeBackend {
        self.meme.as_ref()
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

/// Lit les réglages de la guilde, `None` si elle n'est pas encore connue.
pub async fn load_guild(
    ctx: &dyn BotContext,
    guild_id: &GuildId,
) -> Result<Option<Guild>, BotError> {
    Ok(ctx.db().guilds().get(guild_id).await?)
}

/// Horodatage courant fourni par l'horloge injectée.
pub fn now(ctx: &dyn BotContext) -> DateTime<Utc> {
    ctx.clock().now()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Author, BotConfig, MediaRequest, PAIRING_CODE_ALPHABET, PAIRING_CODE_LENGTH};
    use crate::i18n::Lang;

    #[test]
    fn config_par_defaut_comme_le_legacy() {
        let config = BotConfig::default();
        assert_eq!(config.api_url, "");
        assert_eq!(config.lang, Lang::Fr);
        assert_eq!(config.pairing_code_ttl_minutes, 5);
        assert_eq!(config.default_duration_sec, 5);
        assert!(!config.hide_commands_disabled);
        assert_eq!(config.media.max_size_mb, 2_048);
        assert_eq!(config.media.max_cache_total_mb, 5_120);
        assert_eq!(config.media.max_board_total_mb, 15_360);

        let config = BotConfig::new("http://localhost:3333");
        assert_eq!(config.api_url, "http://localhost:3333");
        assert_eq!(config.lang, Lang::Fr);
    }

    #[test]
    fn auteur_minimal() {
        let author = Author::new("JohnDoe");
        assert_eq!(author.username, "JohnDoe");
        assert_eq!(author.name, "JohnDoe");
        assert_eq!(author.image, None);
        assert!(!author.is_admin);
    }

    #[test]
    fn detection_de_source() {
        assert!(!MediaRequest::default().has_source());
        assert!(!MediaRequest::new(Some("  ".to_owned()), None, false).has_source());
        assert!(MediaRequest::new(Some("https://x".to_owned()), None, false).has_source());
        assert!(MediaRequest::new(None, Some("https://y".to_owned()), true).has_source());
    }

    #[test]
    fn alphabet_des_codes() {
        assert_eq!(PAIRING_CODE_LENGTH, 6);
        assert_eq!(PAIRING_CODE_ALPHABET.len(), 32);
        assert_eq!(
            std::str::from_utf8(PAIRING_CODE_ALPHABET).unwrap(),
            "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
        );
    }
}
