//! Registre des connexions WebSocket v2 et adaptation vers `lc-playback`.
//!
//! Un [`Hub`] unique est partagé par toutes les connexions `GET /ws` :
//!
//! - il maintient le registre `connexion -> guilde` (rooms
//!   `overlay-guild-{guildId}`) et le canal d'écriture de chaque socket ;
//! - il construit paresseusement le [`PlaybackEngine`] au premier `hello`
//!   réussi et implémente [`PlaybackSink`] pour relier le moteur aux sockets :
//!   `play` en unicast (URL signée par client) et `stop` en broadcast ;
//! - il expose l'API d'administration consommée par `http::admin::hub`.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use lc_db::Db;
use lc_domain::{
    now_ms, GuildId, OverlayClient, OverlayClientId, OverlaySessionMode, PlaybackJobId, SystemClock,
};
use lc_playback::{PlaybackConfig, PlaybackEngine, PlaybackSink};
use lc_protocol::{
    close_code, close_reason, CurrentJobPayload, MediaKind as ProtocolMediaKind, MediaPayload,
    PeerInfo, PeersPayload, PlayPayload, ServerEnvelope, ServerMessage, SessionMode,
    SettingsPayload, StopPayload, StopReason,
};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, OnceCell};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::app::AppState;
use crate::config::Config;
use crate::media_url;

/// Nombre maximal de pairs renvoyés dans un payload `peers`.
pub const MAX_PEERS: usize = 8;

/// Nombre maximal de sockets simultanées pour un même client overlay.
///
/// Au-delà, la nouvelle connexion est fermée `4409` avec la raison
/// `too_many_connections` (cf. `docs/rust/server-ws.md` §6).
pub const MAX_SOCKETS_PER_CLIENT: usize = 3;

/// Nombre maximal de sockets simultanées pour une même guilde.
pub const MAX_SOCKETS_PER_GUILD: usize = 16;

/// Raison de fermeture `4409` lorsqu'un plafond de connexions est atteint.
pub(crate) const TOO_MANY_CONNECTIONS_REASON: &str = "too_many_connections";

/// Période de la purge de rétention exécutée par le hub.
const PURGE_INTERVAL: Duration = Duration::from_secs(3_600);

/// Moteur de lecture partagé par le hub (type concret pour les tests/API).
pub type HubEngine = PlaybackEngine<HubSink, SystemClock>;

/// Erreur d'initialisation ou d'utilisation du hub.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HubError {
    /// Le serveur tourne sans base de données : le hub ne peut pas démarrer.
    #[error("base de données indisponible pour le hub WebSocket")]
    DatabaseUnavailable,
}

/// Motif de refus d'une nouvelle connexion (plafonds anti-DoS, COR-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterError {
    /// Le client overlay a déjà [`MAX_SOCKETS_PER_CLIENT`] sockets.
    ClientLimit,
    /// La guilde a déjà [`MAX_SOCKETS_PER_GUILD`] sockets.
    GuildLimit,
}

/// Trame sortante en attente d'écriture sur un socket.
pub(crate) enum OutFrame {
    /// Texte JSON à envoyer tel quel.
    Text(String),
    /// Fermeture de la connexion (`code`, raison lisible).
    Close(u16, String),
}

/// Connexion WebSocket authentifiée et enregistrée dans le hub.
struct Connection {
    guild_id: GuildId,
    client_id: OverlayClientId,
    client_id_str: String,
    client_uuid: Uuid,
    peer_label: String,
    session_mode: SessionMode,
    sender: mpsc::UnboundedSender<OutFrame>,
}

/// Registre interne (verrou court, jamais tenu à travers un `await`).
#[derive(Default)]
struct HubInner {
    connections: HashMap<Uuid, Connection>,
    guilds: HashMap<GuildId, BTreeSet<Uuid>>,
}

/// Hub WebSocket v2 : registre des overlays, diffusion et API admin.
pub struct Hub {
    inner: Mutex<HubInner>,
    engine: OnceCell<Arc<HubEngine>>,
    db: OnceCell<Db>,
    config: OnceCell<Arc<Config>>,
    ids: AtomicU64,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    /// Crée un hub vide (le moteur est construit au premier `hello`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HubInner::default()),
            engine: OnceCell::new(),
            db: OnceCell::new(),
            config: OnceCell::new(),
            ids: AtomicU64::new(0),
        }
    }

    /// Verrouille le registre en tolérant un éventuel empoisonnement.
    fn lock_inner(&self) -> MutexGuard<'_, HubInner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Génère un identifiant de trame serveur (`srv-{prefix}-{n}`).
    #[must_use]
    pub(crate) fn next_message_id(&self, prefix: &str) -> String {
        let sequence = self.ids.fetch_add(1, Ordering::Relaxed) + 1;
        format!("srv-{prefix}-{sequence:06}")
    }

    /// Enregistre une connexion authentifiée et retourne son identifiant.
    ///
    /// Applique les plafonds anti-DoS [`MAX_SOCKETS_PER_CLIENT`] et
    /// [`MAX_SOCKETS_PER_GUILD`] sous le verrou du registre ; au-delà,
    /// [`RegisterError`] est renvoyé et l'appelant ferme la socket `4409`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register(
        &self,
        guild_id: GuildId,
        client_id: OverlayClientId,
        client_id_str: String,
        label: String,
        session_mode: SessionMode,
        author_name: Option<String>,
        _author_image: Option<String>,
        sender: mpsc::UnboundedSender<OutFrame>,
    ) -> Result<Uuid, RegisterError> {
        let mut inner = self.lock_inner();
        let client_sockets = inner
            .connections
            .values()
            .filter(|connection| connection.client_id == client_id)
            .count();
        if client_sockets >= MAX_SOCKETS_PER_CLIENT {
            return Err(RegisterError::ClientLimit);
        }
        let guild_sockets = inner.guilds.get(&guild_id).map_or(0, BTreeSet::len);
        if guild_sockets >= MAX_SOCKETS_PER_GUILD {
            return Err(RegisterError::GuildLimit);
        }
        let connection_id = Uuid::new_v4();
        let peer_label = peer_label(author_name.as_deref(), &label);
        let connection = Connection {
            guild_id: guild_id.clone(),
            client_id,
            client_id_str,
            client_uuid: client_id.as_uuid(),
            peer_label,
            session_mode,
            sender,
        };
        // `_author_image` n'est pas stocké : il est déjà utilisé par la session
        // pour construire les jobs `meme.trigger` (le hub n'en a pas besoin).
        inner
            .guilds
            .entry(guild_id)
            .or_default()
            .insert(connection_id);
        inner.connections.insert(connection_id, connection);
        Ok(connection_id)
    }

    /// Retire une connexion du registre ; `true` si elle existait.
    pub(crate) fn unregister(&self, connection_id: Uuid) -> bool {
        let mut inner = self.lock_inner();
        let Some(connection) = inner.connections.remove(&connection_id) else {
            return false;
        };
        let mut empty_room = false;
        if let Some(room) = inner.guilds.get_mut(&connection.guild_id) {
            room.remove(&connection_id);
            empty_room = room.is_empty();
        }
        if empty_room {
            inner.guilds.remove(&connection.guild_id);
        }
        true
    }

    /// Envoie une trame à une connexion ; `false` si le canal est fermé.
    pub(crate) fn send(&self, connection_id: Uuid, frame: OutFrame) -> bool {
        let inner = self.lock_inner();
        inner
            .connections
            .get(&connection_id)
            .is_some_and(|connection| connection.sender.send(frame).is_ok())
    }

    /// Construit la liste des pairs d'une guilde (dédupliquée par client,
    /// triée par libellé puis `clientId`, tronquée à [`MAX_PEERS`]).
    pub(crate) fn peer_list(&self, guild_id: &GuildId) -> Vec<PeerInfo> {
        let inner = self.lock_inner();
        let Some(room) = inner.guilds.get(guild_id) else {
            return Vec::new();
        };
        let mut by_client: HashMap<OverlayClientId, PeerInfo> = HashMap::new();
        for connection_id in room {
            let Some(connection) = inner.connections.get(connection_id) else {
                continue;
            };
            by_client
                .entry(connection.client_id)
                .or_insert_with(|| PeerInfo {
                    client_id: connection.client_id_str.clone(),
                    label: connection.peer_label.clone(),
                });
        }
        let mut peers: Vec<PeerInfo> = by_client.into_values().collect();
        peers.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.client_id.cmp(&right.client_id))
        });
        peers.truncate(MAX_PEERS);
        peers
    }

    /// Diffuse un `peers` à toute la room d'une guilde ; retourne le nombre de
    /// sockets servis.
    pub(crate) fn broadcast_peers(&self, guild_id: &GuildId) -> usize {
        let peers = self.peer_list(guild_id);
        if peers.is_empty() {
            return 0;
        }
        let payload = PeersPayload {
            guild_id: guild_id.as_str().to_owned(),
            peers,
        };
        self.broadcast_message(guild_id, ServerMessage::Peers(payload))
    }

    /// Diffuse un message texte à la room d'une guilde.
    pub(crate) fn broadcast_message(&self, guild_id: &GuildId, message: ServerMessage) -> usize {
        let envelope = ServerEnvelope::new(
            self.next_message_id(message_prefix(&message)),
            now_ms(),
            message,
        );
        let text = match serde_json::to_string(&envelope) {
            Ok(text) => text,
            Err(error) => {
                warn!(error = %error, "sérialisation d'un message serveur impossible");
                return 0;
            }
        };
        self.send_text_to_guild(guild_id, text)
    }

    /// Envoie un texte JSON déjà sérialisé à toutes les connexions d'une room.
    fn send_text_to_guild(&self, guild_id: &GuildId, text: String) -> usize {
        let inner = self.lock_inner();
        let Some(room) = inner.guilds.get(guild_id) else {
            return 0;
        };
        let mut delivered = 0;
        for connection_id in room {
            let Some(connection) = inner.connections.get(connection_id) else {
                continue;
            };
            if connection.sender.send(OutFrame::Text(text.clone())).is_ok() {
                delivered += 1;
            }
        }
        delivered
    }

    /// Émet un `play` en unicast pour chaque socket de la guilde, avec une URL
    /// média signée pour le `clientId` de la connexion ; retourne le nombre de
    /// sockets servis.
    pub(crate) fn unicast_play(&self, guild_id: &GuildId, payload: &PlayPayload) -> usize {
        let config = self.config.get();
        let now_sec = current_unix_seconds();
        let inner = self.lock_inner();
        let Some(room) = inner.guilds.get(guild_id) else {
            return 0;
        };
        let mut delivered = 0;
        for connection_id in room {
            let Some(connection) = inner.connections.get(connection_id) else {
                continue;
            };
            // Job texte-seul (`media == null`) : aucun média à signer, le
            // payload est envoyé tel quel mais la socket est bien servie
            // (AUD-A1-001).
            let signed = if payload.media.is_none() {
                payload.clone()
            } else {
                let Some(config) = config else {
                    warn!("configuration absente : play média impossible");
                    return 0;
                };
                let Some(signed) =
                    signed_play_payload(payload, &connection.client_id_str, config, now_sec)
                else {
                    continue;
                };
                signed
            };
            let envelope = ServerEnvelope::new(
                self.next_message_id("play"),
                now_ms(),
                ServerMessage::Play(signed),
            );
            match serde_json::to_string(&envelope) {
                Ok(text) => {
                    if connection.sender.send(OutFrame::Text(text)).is_ok() {
                        delivered += 1;
                    }
                }
                Err(error) => warn!(error = %error, "sérialisation d'un play impossible"),
            }
        }
        delivered
    }

    /// Diffuse un `stop` à la room ; retourne le nombre de sockets servis.
    pub(crate) fn broadcast_stop_frame(&self, guild_id: &GuildId, payload: StopPayload) -> usize {
        self.broadcast_message(guild_id, ServerMessage::Stop(payload))
    }

    /// Construit le payload `welcome.currentJob` en re-signant l'URL média pour
    /// le client d'une connexion.
    pub(crate) fn signed_current_job(
        &self,
        current_job: &CurrentJobPayload,
        client_id: &str,
        now_sec: u64,
    ) -> Option<CurrentJobPayload> {
        // Job texte-seul : aucun média à signer, le payload part tel quel.
        if current_job.play.media.is_none() {
            return Some(current_job.clone());
        }
        let config = self.config.get()?;
        let play = signed_play_payload(&current_job.play, client_id, config, now_sec)?;
        Some(CurrentJobPayload {
            play,
            started_at: current_job.started_at,
            remaining_ms: current_job.remaining_ms,
        })
    }

    /// Valeurs de configuration de guilde utilisées par `welcome`/`settings`.
    pub(crate) async fn guild_defaults(&self, guild_id: &GuildId) -> (bool, u64, Option<u64>) {
        let fallback = self
            .config
            .get()
            .map_or(default_media_time_fallback(), |config| {
                config.default_duration
            });
        let Some(db) = self.db.get() else {
            return (true, fallback, None);
        };
        match db.guilds().get(guild_id).await {
            Ok(Some(guild)) => (
                true,
                guild
                    .default_media_time
                    .and_then(|value| u64::try_from(value).ok())
                    .unwrap_or(fallback),
                guild
                    .max_media_time
                    .and_then(|value| u64::try_from(value).ok()),
            ),
            Ok(None) => (true, fallback, None),
            Err(error) => {
                warn!(error = %error, guild = %guild_id, "lecture de la guilde impossible");
                (true, fallback, None)
            }
        }
    }

    /// Diffuse un `settings` à la room avec les valeurs fournies.
    pub(crate) fn broadcast_settings_values(
        &self,
        guild_id: &GuildId,
        show_text_default: bool,
        default_media_time: u64,
        max_media_time: Option<u64>,
    ) -> usize {
        let inner = self.lock_inner();
        let Some(room) = inner.guilds.get(guild_id) else {
            return 0;
        };
        let mut delivered = 0;
        for connection_id in room {
            let Some(connection) = inner.connections.get(connection_id) else {
                continue;
            };
            let payload = SettingsPayload {
                guild_id: guild_id.as_str().to_owned(),
                session_mode: connection.session_mode,
                show_text_default,
                default_media_time,
                max_media_time,
            };
            let envelope = ServerEnvelope::new(
                self.next_message_id("settings"),
                now_ms(),
                ServerMessage::Settings(payload),
            );
            match serde_json::to_string(&envelope) {
                Ok(text) => {
                    if connection.sender.send(OutFrame::Text(text)).is_ok() {
                        delivered += 1;
                    }
                }
                Err(error) => warn!(error = %error, "sérialisation d'un settings impossible"),
            }
        }
        delivered
    }

    /// Construit paresseusement le moteur de lecture partagé.
    ///
    /// Le moteur n'est créé qu'une fois (au premier `hello`) puis réutilisé ;
    /// ses tâches de fond (boucle `tick` et purge de rétention) sont arrêtées
    /// par `AppState::shutdown`.
    pub async fn ensure_runtime(
        self: &Arc<Self>,
        state: &AppState,
    ) -> Result<Arc<HubEngine>, HubError> {
        if let Some(engine) = self.engine.get() {
            return Ok(Arc::clone(engine));
        }
        let db = state.db.clone().ok_or(HubError::DatabaseUnavailable)?;
        let config = Arc::clone(&state.config);
        let shutdown = state.shutdown.clone();
        let weak_hub = Arc::downgrade(self);
        let engine_db = db.clone();
        let engine_config = Arc::clone(&config);
        let engine = self
            .engine
            .get_or_try_init(|| async move {
                let playback_config = PlaybackConfig::default()
                    .with_api_base_url(
                        engine_config
                            .api_url
                            .as_str()
                            .trim_end_matches('/')
                            .to_owned(),
                    )
                    .with_default_duration_sec(
                        i64::try_from(engine_config.default_duration).unwrap_or(i64::MAX),
                    )
                    .with_retention(chrono::Duration::hours(
                        i64::try_from(engine_config.playback_job_retention_hours)
                            .unwrap_or(i64::MAX),
                    ));
                let engine = Arc::new(HubEngine::new(
                    engine_db,
                    playback_config,
                    HubSink::new(weak_hub),
                    SystemClock,
                ));

                let run_engine = Arc::clone(&engine);
                let run_shutdown = shutdown.subscribe();
                tokio::spawn(async move {
                    if let Err(error) = run_engine.run(run_shutdown.wait()).await {
                        tracing::error!(error = %error, "moteur de lecture arrêté sur erreur");
                    }
                });

                let purge_engine = Arc::clone(&engine);
                let purge_shutdown = shutdown.subscribe();
                tokio::spawn(async move {
                    let shutdown = purge_shutdown.wait();
                    tokio::pin!(shutdown);
                    let mut ticker = interval(PURGE_INTERVAL);
                    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
                    ticker.tick().await;
                    loop {
                        tokio::select! {
                            () = &mut shutdown => break,
                            _ = ticker.tick() => {
                                let now = chrono::Utc::now();
                                match purge_engine.purge(now).await {
                                    Ok(report) => debug!(
                                        playback_jobs = report.playback_jobs,
                                        pairing_codes = report.pairing_codes,
                                        "purge de rétention effectuée"
                                    ),
                                    Err(error) => warn!(error = %error, "échec de la purge de rétention"),
                                }
                            }
                        }
                    }
                });

                Ok::<Arc<HubEngine>, HubError>(engine)
            })
            .await?
            .clone();
        drop(self.db.set(db));
        drop(self.config.set(config));
        info!("moteur de lecture initialisé par le hub WebSocket");
        Ok(engine)
    }

    /// Moteur de lecture, si le hub a déjà servi au moins un `hello`.
    #[must_use]
    pub fn engine(&self) -> Option<Arc<HubEngine>> {
        self.engine.get().cloned()
    }

    /// Base de données du hub, si le moteur a été initialisé.
    #[must_use]
    pub fn db(&self) -> Option<Db> {
        self.db.get().cloned()
    }

    /// Nombre de sockets actuellement enregistrés.
    #[must_use]
    pub fn connected_count(&self) -> usize {
        self.lock_inner().connections.len()
    }

    /// Nombre de sockets enregistrés pour une guilde.
    #[must_use]
    pub fn connected_for_guild(&self, guild_id: &GuildId) -> usize {
        self.lock_inner()
            .guilds
            .get(guild_id)
            .map_or(0, BTreeSet::len)
    }

    /// `true` si au moins un socket de ce client est connecté.
    #[must_use]
    pub fn is_connected(&self, client_id: &Uuid) -> bool {
        self.lock_inner()
            .connections
            .values()
            .any(|connection| connection.client_uuid == *client_id)
    }

    /// Ferme toutes les connexions d'un client avec le code donné.
    ///
    /// Pour une révocation (`4402`), le hub diffuse d'abord
    /// `stop {reason:"revoked", jobId:null}` (protocole §6.3/§10) puis envoie
    /// les close — l'ordre est garanti par le canal FIFO de chaque connexion.
    /// Retourne `true` si au moins un socket a été ciblé.
    pub fn close_client(&self, client_id: &Uuid, code: u16) -> bool {
        let reason = close_reason_for_code(code).to_owned();
        let revoked = code == close_code::REVOKED;
        let (connection_ids, guilds): (Vec<Uuid>, BTreeSet<GuildId>) = {
            let inner = self.lock_inner();
            let mut connection_ids = Vec::new();
            let mut guilds = BTreeSet::new();
            for (connection_id, connection) in &inner.connections {
                if connection.client_uuid == *client_id {
                    connection_ids.push(*connection_id);
                    guilds.insert(connection.guild_id.clone());
                }
            }
            (connection_ids, guilds)
        };
        if connection_ids.is_empty() {
            return false;
        }
        if revoked {
            let payload = StopPayload {
                reason: StopReason::Revoked,
                job_id: None,
            };
            for guild_id in &guilds {
                self.broadcast_stop_frame(guild_id, payload.clone());
            }
        }
        let mut closed = false;
        for connection_id in connection_ids {
            if self.send(connection_id, OutFrame::Close(code, reason.clone())) {
                closed = true;
            }
        }
        if revoked {
            self.spawn_release_after_revocation(&guilds);
        }
        closed
    }

    /// Réconcilie l'état des jobs après révocation sans rediffuser de `stop`
    /// (le payload a déjà été diffusé en amont par [`Self::close_client`]).
    fn spawn_release_after_revocation(&self, guilds: &BTreeSet<GuildId>) {
        let Some(engine) = self.engine.get().cloned() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            warn!("révocation hors runtime Tokio : jobs PLAYING non réconciliés");
            return;
        };
        for guild_id in guilds {
            let engine = Arc::clone(&engine);
            let guild_id = guild_id.clone();
            handle.spawn(async move {
                if let Err(error) = engine
                    .release_playing_without_stop(&guild_id, None, StopReason::Revoked)
                    .await
                {
                    warn!(error = %error, guild = %guild_id, "libération des jobs après révocation impossible");
                }
            });
        }
    }

    /// Arrête tous les jobs de la guilde (reason `manual`) ; retourne le nombre
    /// de jobs effectivement arrêtés.
    pub async fn broadcast_stop(&self, guild_id: &GuildId) -> usize {
        self.broadcast_stop_with_reason(guild_id, None, StopReason::Manual)
            .await
    }

    /// Arrête des jobs avec une raison protocolaire explicite.
    ///
    /// `job_id` ciblé produit un `stop {jobId}` ; `None` arrête toute la file
    /// (le `jobId` du payload suit `lc-playback` : l'identifiant unique si un
    /// seul job est libéré, sinon `null`).
    pub async fn broadcast_stop_with_reason(
        &self,
        guild_id: &GuildId,
        job_id: Option<PlaybackJobId>,
        reason: StopReason,
    ) -> usize {
        let Some(engine) = self.engine.get().cloned() else {
            return 0;
        };
        match engine.stop_with_reason(guild_id, job_id, reason).await {
            Ok(outcome) => outcome.stopped.len(),
            Err(error) => {
                warn!(error = %error, guild = %guild_id, "arrêt de guilde impossible");
                0
            }
        }
    }

    /// Purge administrateur : `stop {reason:"purge", jobId:null}` dédié.
    pub async fn broadcast_purge(&self, guild_id: &GuildId) -> usize {
        let Some(engine) = self.engine.get().cloned() else {
            return 0;
        };
        match engine
            .stop_all_with_reason(guild_id, StopReason::Purge)
            .await
        {
            Ok(outcome) => outcome.stopped.len(),
            Err(error) => {
                warn!(error = %error, guild = %guild_id, "purge de guilde impossible");
                0
            }
        }
    }

    /// Diffuse un `settings` à jour à la room de la guilde ; retourne le nombre
    /// de sockets servis.
    pub async fn broadcast_settings(&self, guild_id: &GuildId) -> usize {
        let (show_text_default, default_media_time, max_media_time) =
            self.guild_defaults(guild_id).await;
        self.broadcast_settings_values(
            guild_id,
            show_text_default,
            default_media_time,
            max_media_time,
        )
    }
}

/// Adaptateur entre `lc-playback` et le hub WebSocket.
pub struct HubSink {
    hub: Weak<Hub>,
}

impl HubSink {
    fn new(hub: Weak<Hub>) -> Self {
        Self { hub }
    }
}

impl PlaybackSink for HubSink {
    fn on_play(
        &self,
        guild_id: &GuildId,
        payload: PlayPayload,
    ) -> impl std::future::Future<Output = usize> + Send {
        let hub = self.hub.clone();
        let guild_id = guild_id.clone();
        async move {
            match hub.upgrade() {
                Some(hub) => hub.unicast_play(&guild_id, &payload),
                None => 0,
            }
        }
    }

    fn on_stop(
        &self,
        guild_id: &GuildId,
        payload: StopPayload,
    ) -> impl std::future::Future<Output = ()> + Send {
        let hub = self.hub.clone();
        let guild_id = guild_id.clone();
        async move {
            if let Some(hub) = hub.upgrade() {
                hub.broadcast_stop_frame(&guild_id, payload);
            }
        }
    }
}

/// Résultat de l'authentification d'un token d'appairage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthError {
    /// Token absent ou vide.
    MissingToken,
    /// Hash inconnu.
    InvalidToken,
    /// Client révoqué ou supprimé.
    Revoked,
    /// Erreur interne (base de données).
    Internal(String),
}

impl AuthError {
    /// Code de fermeture WebSocket associé.
    pub(crate) fn code(&self) -> u16 {
        match self {
            Self::MissingToken | Self::InvalidToken => close_code::UNAUTHORIZED,
            Self::Revoked => close_code::REVOKED,
            Self::Internal(_) => close_code::INTERNAL_ERROR,
        }
    }

    /// Raison de fermeture associée.
    pub(crate) fn reason(&self) -> &'static str {
        match self {
            Self::MissingToken => close_reason::MISSING_TOKEN,
            Self::InvalidToken => close_reason::INVALID_TOKEN,
            Self::Revoked => close_reason::REVOKED,
            Self::Internal(_) => close_reason::INTERNAL_ERROR,
        }
    }
}

/// Authentifie un token d'appairage en clair.
///
/// Le token est haché en SHA-256 puis comparé au `token_hash` stocké par
/// `lc-db`. En cas d'échec, un SELECT ciblé distingue « inconnu » de
/// « révoqué » pour émettre respectivement `4401 invalid_token` et
/// `4402 revoked` (cf. `docs/rust/protocol-v2.md`).
pub(crate) async fn authenticate(db: &Db, token: &str) -> Result<OverlayClient, AuthError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(AuthError::MissingToken);
    }
    let token_hash = hash_token(token);
    match db.overlay_clients().get_by_token_hash(&token_hash).await {
        Ok(Some(client)) => Ok(client),
        Ok(None) => {
            let row: Option<(Option<i64>,)> =
                sqlx::query_as("SELECT revoked_at FROM overlay_clients WHERE token_hash = ?")
                    .bind(token_hash.as_str())
                    .fetch_optional(db.pool())
                    .await
                    .map_err(|error| AuthError::Internal(error.to_string()))?;
            match row {
                Some((Some(_),)) => Err(AuthError::Revoked),
                Some((None,)) | None => Err(AuthError::InvalidToken),
            }
        }
        Err(error) => Err(AuthError::Internal(error.to_string())),
    }
}

/// Calcule l'empreinte SHA-256 hexadécimale d'un token d'appairage.
#[must_use]
pub(crate) fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

/// Convertit le mode de session du domaine vers le mode protocolaire.
#[must_use]
pub(crate) fn protocol_session_mode(mode: OverlaySessionMode) -> SessionMode {
    match mode {
        OverlaySessionMode::Normal => SessionMode::Normal,
        OverlaySessionMode::InviteReadOnly => SessionMode::InviteReadOnly,
    }
}

/// Préfixe d'identifiant de trame pour un message serveur.
#[must_use]
pub(crate) fn message_prefix(message: &ServerMessage) -> &'static str {
    match message {
        ServerMessage::Welcome(_) => "welcome",
        ServerMessage::Settings(_) => "settings",
        ServerMessage::Peers(_) => "peers",
        ServerMessage::Play(_) => "play",
        ServerMessage::Stop(_) => "stop",
        ServerMessage::Ack(_) => "ack",
        ServerMessage::Error(_) => "error",
    }
}

/// Raison de fermeture associée à un code utilisé par l'administration.
#[must_use]
pub(crate) fn close_reason_for_code(code: u16) -> &'static str {
    match code {
        close_code::NORMAL => close_reason::CLIENT_SHUTDOWN,
        close_code::GOING_AWAY => close_reason::SERVER_SHUTDOWN,
        close_code::UNAUTHORIZED => close_reason::INVALID_TOKEN,
        close_code::REVOKED => close_reason::REVOKED,
        close_code::OUTDATED_PROTOCOL => close_reason::OUTDATED_PROTOCOL,
        close_code::FRAME_TOO_LARGE => close_reason::FRAME_TOO_LARGE,
        close_code::RATE_LIMITED => close_reason::RATE_LIMITED,
        _ => close_reason::INTERNAL_ERROR,
    }
}

/// Construit un payload `play` avec une URL média signée pour un client.
///
/// La signature est déléguée à `crate::media_url` (`HMAC-SHA256` sur
/// `assetId|clientId|exp`) et l'offset de reprise est conservé à l'identique
/// dans la query (`startOffsetSec`) et le fragment (`#t=`).
pub(crate) fn signed_play_payload(
    payload: &PlayPayload,
    client_id: &str,
    config: &Config,
    now_sec: u64,
) -> Option<PlayPayload> {
    // Job texte-seul (`media == null`) : il n'y a aucune URL à signer, le
    // payload `play` part tel quel (COR-07 / AUD-A1-001).
    if payload.media.is_none() {
        return Some(payload.clone());
    }
    let media = payload.media.as_ref()?;
    let secret = config.media_url_secret.expose_secret();
    if secret.is_empty() {
        warn!("MEDIA_URL_SECRET absent : impossible de signer l'URL média d'un play");
        return None;
    }
    let expires_at = media_url::expiry_for(media.kind, media.duration_sec, now_sec);
    let signature = media_url::sign(
        secret.as_bytes(),
        media.asset_id.as_str(),
        client_id,
        expires_at,
    )
    .ok()?;
    let mut url = format!(
        "{}/overlay/media/{}?clientId={client_id}&exp={expires_at}&sig={signature}",
        config.api_url.as_str().trim_end_matches('/'),
        media.asset_id,
    );
    if let Some(start_offset_sec) = media.start_offset_sec {
        let seekable = matches!(
            media.kind,
            ProtocolMediaKind::Audio | ProtocolMediaKind::Video
        );
        if start_offset_sec > 0 && seekable {
            url.push_str(&format!(
                "&startOffsetSec={start_offset_sec}#t={start_offset_sec}"
            ));
        }
    }
    let mut signed = payload.clone();
    signed.media = Some(MediaPayload {
        url,
        ..media.clone()
    });
    Some(signed)
}

/// Nombre maximal de pairs (voir [`MAX_PEERS`]).
/// Convertit un libellé d'auteur/appareil en libellé de pair.
fn peer_label(author_name: Option<&str>, device_label: &str) -> String {
    if let Some(author_name) = author_name {
        let trimmed = author_name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    let trimmed = device_label.trim();
    if trimmed.is_empty() {
        "unknown-device".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Valeur par défaut de `defaultMediaTime` (legacy `DEFAULT_DURATION=5`).
fn default_media_time_fallback() -> u64 {
    5
}

/// Heure murale en secondes Unix (signature des URLs média).
fn current_unix_seconds() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}
