//! Client WebSocket v2 de l'overlay.
//!
//! Dialogue avec `{serverUrl}/ws` selon `docs/rust/protocol-v2.md` :
//! handshake `hello`/`welcome`, heartbeat applicatif toutes les 15 s,
//! réception `play`/`stop`/`settings`/`peers`/`ack`/`error`/`media.ingestion`/
//! `board.changed`, envoi `playback.state`, `playback.ended`, `stop.request`,
//! `media.trigger`, `peers.request`, reconnexion avec backoff (1 s → 3 s,
//! 4409 plafonné à 30 s), et gestion terminale des closes 4401 (jeton invalide
//! → réappairage), 4402 (révoqué) et 4403 (overlay obsolète).
//!
//! La logique de session ([`run_session`]) est générique sur [`SessionEffects`]
//! afin d'être testable contre un serveur WebSocket mock ; l'implémentation
//! Tauri ([`AppEffects`]) branche les événements sur le renderer et le tray.

use std::collections::HashMap;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use lc_protocol::{
    AckPayload, BoardChangedPayload, ClientEnvelope, ClientMessage, HeartbeatPayload, HelloPayload,
    MediaIngestionPayload, PeersPayload, PlayPayload, PlaybackState, PlaybackStatePayload,
    ServerEnvelope, ServerErrorPayload, ServerMessage, SessionMode, SettingsPayload, StopPayload,
    WelcomePayload,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async_tls_with_config, Connector};
use url::Url;
use uuid::Uuid;

use crate::config::{Membership, OverlayConfigView};
use crate::events::{self, OverlayEvent};
use crate::net::tls;
use crate::state::{AppState, GuildRemoval, GuildRemovalReason, QueuedPlay};
use crate::tray;
use crate::windows;

/// Délai maximal d'établissement de la connexion TCP/TLS/WebSocket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Cadence du heartbeat applicatif (contrat v2 : 15 s).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
/// Cadence du keep-alive des `play` en file locale (avant leur affichage).
const QUEUED_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(3);
/// Every server message type known by this client.
const KNOWN_SERVER_MESSAGE_TYPES: [&str; 9] = [
    lc_protocol::msg_type::WELCOME,
    lc_protocol::msg_type::SETTINGS,
    lc_protocol::msg_type::PEERS,
    lc_protocol::msg_type::PLAY,
    lc_protocol::msg_type::STOP,
    lc_protocol::msg_type::ACK,
    lc_protocol::msg_type::ERROR,
    lc_protocol::msg_type::MEDIA_INGESTION,
    lc_protocol::msg_type::BOARD_CHANGED,
];

/// Statut de connexion affiché dans le tray et exposé au bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Disabled,
    NotPaired,
    Connecting,
    Connected,
    Reconnecting,
    Error,
    Revoked,
    Outdated,
}

impl ConnectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "désactivé",
            Self::NotPaired => "non appairé",
            Self::Connecting => "connexion…",
            Self::Connected => "connecté",
            Self::Reconnecting => "reconnexion…",
            Self::Error => "erreur",
            Self::Revoked => "appareil révoqué",
            Self::Outdated => "overlay obsolète",
        }
    }
}

/// Instantané de connexion partagé avec le tray et les commandes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetSnapshot {
    pub status: ConnectionStatus,
    pub reason: Option<String>,
    pub server_version: Option<String>,
    pub min_version: Option<String>,
    pub guild_id: Option<String>,
    pub client_id: Option<String>,
    pub session_mode: Option<String>,
    pub peers: Vec<String>,
    pub last_error: Option<String>,
}

impl Default for NetSnapshot {
    fn default() -> Self {
        Self {
            status: ConnectionStatus::Disabled,
            reason: None,
            server_version: None,
            min_version: None,
            guild_id: None,
            client_id: None,
            session_mode: None,
            peers: Vec::new(),
            last_error: None,
        }
    }
}

/// Commandes envoyées par les commandes Tauri/le tray vers la tâche réseau.
#[derive(Debug)]
pub enum NetCommand {
    Send(ClientMessage),
    /// Sends a frame and routes the matching `ack` back to the caller.
    SendWithAck {
        message: ClientMessage,
        reply: oneshot::Sender<AckPayload>,
    },
    Reconnect,
    Shutdown,
}

/// Session prête à être exécutée (construite depuis la config persistée).
struct SessionConfig {
    ws_url: Url,
    hello: HelloPayload,
    insecure_tls: bool,
    /// Cadence du keep-alive `playback.state` des jobs en file locale.
    queued_keepalive_interval: Duration,
}

/// Effets d'une session WS, abstraits pour permettre les tests sans Tauri.
pub trait SessionEffects: Send + Sync {
    fn on_welcome(&self, payload: &WelcomePayload);
    fn on_settings(&self, payload: &SettingsPayload);
    fn on_play(&self, payload: &PlayPayload);
    fn on_stop(&self, payload: &StopPayload);
    fn on_peers(&self, payload: &PeersPayload);
    fn on_ack(&self, payload: &AckPayload);
    fn on_server_error(&self, payload: &ServerErrorPayload);
    /// Result of the asynchronous media ingestion this client requested.
    fn on_media_ingestion(&self, payload: &MediaIngestionPayload);
    /// The guild's media board changed.
    fn on_board_changed(&self, payload: &BoardChangedPayload);
    /// `(job_id, durée ms)` des `play` de cette guilde en file locale.
    fn queued_jobs(&self) -> Vec<(String, u64)>;
}

/// Fin d'une session, interprétée par le superviseur.
#[derive(Debug)]
pub enum SessionOutcome {
    Closed { code: Option<u16>, reason: String },
    ConnectFailed(String),
    Failed(String),
    Shutdown,
    ReconnectRequested,
}

/// Décision du superviseur après une session.
#[derive(Debug)]
enum SupervisorStep {
    Retry {
        rate_limited: bool,
        reason: String,
    },
    Terminal {
        status: ConnectionStatus,
        reason: String,
        /// Some when the guild must be removed from the local config.
        purge: Option<GuildRemovalReason>,
    },
}

#[derive(Default)]
struct Backoff {
    attempt: u32,
}

impl Backoff {
    fn reset(&mut self) {
        self.attempt = 0;
    }

    fn delay(&mut self, rate_limited: bool) -> Duration {
        let attempt = self.attempt;
        self.attempt = self.attempt.saturating_add(1);
        let seconds = if rate_limited {
            (1u64 << attempt.min(5)).min(30)
        } else {
            match attempt {
                0 => 1,
                1 => 2,
                _ => 3,
            }
        };
        Duration::from_secs(seconds)
    }
}

/// Démarre un superviseur par guilde rejointe (depuis `setup`).
pub fn start(app: &AppHandle<Wry>) {
    let guilds: Vec<String> = app
        .state::<AppState>()
        .config_snapshot()
        .memberships
        .iter()
        .map(|membership| membership.guild_id.clone())
        .collect();
    for guild_id in guilds {
        ensure_supervisor(app, &guild_id);
    }
    refresh_tray_from_any_thread(app);
}

/// Démarre le superviseur d'une guilde s'il ne tourne pas déjà.
pub fn ensure_supervisor(app: &AppHandle<Wry>, guild_id: &str) {
    let state = app.state::<AppState>();
    if state.has_supervisor(guild_id) {
        return;
    }
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
    state.set_net_command_sender(guild_id, sender);
    let handle = app.clone();
    let guild_id = guild_id.to_string();
    tauri::async_runtime::spawn(async move {
        supervise(handle.clone(), guild_id.clone(), receiver).await;
        handle.state::<AppState>().remove_net(&guild_id);
        refresh_tray_from_any_thread(&handle);
        tracing::info!(guild_id = %guild_id, "superviseur WebSocket arrêté");
    });
}

/// Demande une reconnexion de toutes les guildes (config modifiée, tray…) et
/// démarre les superviseurs manquants (guilde tout juste rejointe).
pub fn request_reconnect(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    let guilds: Vec<String> = state
        .config_snapshot()
        .memberships
        .iter()
        .map(|membership| membership.guild_id.clone())
        .collect();
    for guild_id in &guilds {
        if state.has_supervisor(guild_id) {
            state.send_net_command_to(guild_id, NetCommand::Reconnect);
        } else {
            ensure_supervisor(app, guild_id);
        }
    }
    // Un superviseur dont la guilde a disparu se réveille et s'arrête.
    for guild_id in state.supervised_guilds() {
        if !guilds.contains(&guild_id) {
            state.send_net_command_to(&guild_id, NetCommand::Reconnect);
        }
    }
}

/// Envoie une trame au serveur de la guilde active si la connexion est établie.
pub fn send_command(app: &AppHandle<Wry>, command: NetCommand) -> bool {
    app.state::<AppState>().send_net_command(command)
}

/// Envoie une trame au serveur d'une guilde donnée.
pub fn send_command_to(app: &AppHandle<Wry>, guild_id: &str, command: NetCommand) -> bool {
    let state = app.state::<AppState>();
    if !is_deliverable(state.net_snapshot_for(guild_id).status, &command) {
        return false;
    }
    state.send_net_command_to(guild_id, command)
}

/// A `Send` frame is only deliverable through a live session: a muted,
/// disabled or reconnecting supervisor would drop it while the caller
/// believed it was sent.
fn is_deliverable(status: ConnectionStatus, command: &NetCommand) -> bool {
    match command {
        NetCommand::Send(_) | NetCommand::SendWithAck { .. } => {
            status == ConnectionStatus::Connected
        }
        NetCommand::Reconnect | NetCommand::Shutdown => true,
    }
}

/// Ferme proprement toutes les tâches réseau (sortie de l'application).
pub fn request_shutdown(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    for guild_id in state.supervised_guilds() {
        let sent = state.send_net_command_to(&guild_id, NetCommand::Shutdown);
        tracing::debug!(sent, guild_id = %guild_id, "arrêt demandé au superviseur WebSocket");
    }
}

/// Rafraîchit le tray depuis n'importe quel thread (les menus exigent le
/// thread principal sous Windows).
pub fn refresh_tray_from_any_thread(app: &AppHandle<Wry>) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        tray::refresh_tray(&handle);
    }) {
        tracing::debug!(%error, "run_on_main_thread indisponible pour le tray");
    }
    if let Err(error) = events::emit_guilds_changed(app) {
        tracing::debug!(%error, "échec d'émission de l'état des guildes");
    }
}

async fn supervise(
    app: AppHandle<Wry>,
    guild_id: String,
    mut commands: UnboundedReceiver<NetCommand>,
) {
    let mut backoff = Backoff::default();
    loop {
        let config = app.state::<AppState>().config_snapshot();
        // Guilde quittée ou purgée : le superviseur n'a plus de raison d'être.
        let Some(membership) = config.membership(&guild_id).cloned() else {
            return;
        };
        if !config.enabled {
            publish(
                &app,
                &guild_id,
                ConnectionStatus::Disabled,
                Some("overlay désactivé".to_string()),
            );
            if !wait_for_command(&mut commands, &app).await {
                return;
            }
            continue;
        }
        if membership.muted {
            publish(
                &app,
                &guild_id,
                ConnectionStatus::Disabled,
                Some("guilde en sourdine".to_string()),
            );
            if !wait_for_command(&mut commands, &app).await {
                return;
            }
            continue;
        }
        let session = match build_session_config(&membership) {
            Ok(session) => session,
            Err(error) => {
                publish(&app, &guild_id, ConnectionStatus::Error, Some(error));
                if !wait_for_command(&mut commands, &app).await {
                    return;
                }
                continue;
            }
        };
        let status = if backoff.attempt == 0 {
            ConnectionStatus::Connecting
        } else {
            ConnectionStatus::Reconnecting
        };
        publish(&app, &guild_id, status, None);
        let effects = AppEffects {
            app: app.clone(),
            guild_id: guild_id.clone(),
        };
        let outcome = run_session(&effects, &session, &mut commands).await;
        // La connexion est tombée : ce que cette guilde affichait ou faisait
        // attendre ne peut plus être piloté par son serveur.
        release_guild_playback(&app, &guild_id);
        let step = match outcome {
            SessionOutcome::Shutdown => return,
            SessionOutcome::ReconnectRequested => {
                backoff.reset();
                publish(
                    &app,
                    &guild_id,
                    ConnectionStatus::Connecting,
                    Some("reconnexion demandée".to_string()),
                );
                continue;
            }
            SessionOutcome::ConnectFailed(reason) => SupervisorStep::Retry {
                rate_limited: false,
                reason,
            },
            SessionOutcome::Failed(reason) => SupervisorStep::Retry {
                rate_limited: false,
                reason,
            },
            SessionOutcome::Closed { code, reason } => close_step(code, &reason),
        };
        match step {
            SupervisorStep::Retry {
                rate_limited,
                reason,
            } => {
                let delay = backoff.delay(rate_limited);
                tracing::info!(
                    delay_ms = delay.as_millis(),
                    reason,
                    guild_id = %guild_id,
                    "nouvelle tentative de connexion WebSocket"
                );
                publish(
                    &app,
                    &guild_id,
                    ConnectionStatus::Reconnecting,
                    Some(reason),
                );
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    command = commands.recv() => match command {
                        None | Some(NetCommand::Shutdown) => return,
                        Some(NetCommand::Reconnect) => backoff.reset(),
                        Some(NetCommand::Send(_) | NetCommand::SendWithAck { .. }) => {
                            tracing::debug!("trame ignorée pendant le backoff de reconnexion");
                        }
                    }
                }
            }
            SupervisorStep::Terminal {
                status,
                reason,
                purge,
            } => {
                if let Some(purge_reason) = purge {
                    // Jeton invalide ou révoqué : la guilde est retirée ; le
                    // superviseur s'arrête au tour suivant.
                    let remaining = purge_pairing(&app, &guild_id, purge_reason);
                    if remaining == 0 {
                        windows::show_board(&app);
                    }
                    tracing::info!(guild_id = %guild_id, reason, "adhésion retirée");
                    return;
                }
                publish(&app, &guild_id, status, Some(reason));
                backoff.reset();
                if !wait_for_command(&mut commands, &app).await {
                    return;
                }
            }
        }
    }
}

/// Attend la prochaine commande significative (reconnexion ou sortie).
async fn wait_for_command(
    commands: &mut UnboundedReceiver<NetCommand>,
    _app: &AppHandle<Wry>,
) -> bool {
    loop {
        match commands.recv().await {
            None | Some(NetCommand::Shutdown) => return false,
            Some(NetCommand::Reconnect) => return true,
            Some(NetCommand::Send(_) | NetCommand::SendWithAck { .. }) => {
                tracing::debug!("trame ignorée : aucune connexion WebSocket active");
            }
        }
    }
}

fn publish(app: &AppHandle<Wry>, guild_id: &str, status: ConnectionStatus, reason: Option<String>) {
    app.state::<AppState>()
        .set_net_status(guild_id, status, reason);
    refresh_tray_from_any_thread(app);
}

/// Retire l'adhésion d'une guilde ; retourne le nombre d'adhésions restantes.
fn purge_pairing(app: &AppHandle<Wry>, guild_id: &str, reason: GuildRemovalReason) -> usize {
    let state = app.state::<AppState>();
    // Read the display name before the membership disappears.
    let guild_name = state
        .config_snapshot()
        .membership(guild_id)
        .and_then(|membership| membership.guild_name.clone());
    let removal = GuildRemoval {
        guild_id: guild_id.to_string(),
        guild_name,
        reason,
    };
    state.push_guild_removal(removal.clone());
    if let Err(error) = events::emit_guild_removed(app, &removal) {
        tracing::debug!(%error, "échec d'émission de l'événement de retrait de guilde");
    }
    let remaining = match state.update_config(|config| {
        config.leave(guild_id);
    }) {
        Ok(config) => config.memberships.len(),
        Err(error) => {
            tracing::warn!(%error, "échec de retrait de l'adhésion");
            state.config_snapshot().memberships.len()
        }
    };
    crate::shortcuts::reapply_for_session(app);
    if let Err(error) = events::emit_settings(app, &state.view()) {
        tracing::debug!(%error, "échec d'émission des settings après purge");
    }
    if let Err(error) = events::emit_device_settings(app) {
        tracing::debug!(%error, "échec d'émission de overlay://device-settings après purge");
    }
    remaining
}

/// Libère l'écran et la file locale de ce que tenait une guilde, puis passe au
/// `play` suivant en attente.
fn release_guild_playback(app: &AppHandle<Wry>, guild_id: &str) {
    let state = app.state::<AppState>();
    state.drop_queued(guild_id, None);
    if state.playback_guild().as_deref() == Some(guild_id) {
        state.clear_playback();
        events::stop_overlay_now(app, "disconnected");
        advance_local_queue(app);
    }
}

/// Affiche le prochain `play` en attente, s'il y en a un.
pub fn advance_local_queue(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    if state.playback_job().is_some() {
        return;
    }
    if let Some(next) = state.dequeue_play() {
        tracing::info!(job_id = %next.job_id, guild_id = %next.guild_id, "play en attente affiché");
        state.set_playback(&next.guild_id, next.job_id);
        events::deliver_or_buffer(app, state.overlay_ready(), OverlayEvent::play(next.payload));
    }
}

fn close_step(code: Option<u16>, reason: &str) -> SupervisorStep {
    match code {
        Some(1000) => SupervisorStep::Terminal {
            status: ConnectionStatus::Error,
            reason: "connexion fermée par le serveur".to_string(),
            purge: None,
        },
        Some(1003) => SupervisorStep::Terminal {
            status: ConnectionStatus::Error,
            reason: "trame non supportée par le serveur".to_string(),
            purge: None,
        },
        Some(4401) => SupervisorStep::Terminal {
            status: ConnectionStatus::Error,
            reason: "jeton invalide — réappairage nécessaire".to_string(),
            purge: Some(GuildRemovalReason::InvalidToken),
        },
        Some(4402) if reason == lc_protocol::close_reason::MEMBER_REMOVED => {
            SupervisorStep::Terminal {
                status: ConnectionStatus::Revoked,
                reason: "membre exclu de la guilde — réappairage nécessaire".to_string(),
                purge: Some(GuildRemovalReason::MemberRemoved),
            }
        }
        Some(4402) => SupervisorStep::Terminal {
            status: ConnectionStatus::Revoked,
            reason: "appareil révoqué — réappairage nécessaire".to_string(),
            purge: Some(GuildRemovalReason::Revoked),
        },
        Some(4403) => SupervisorStep::Terminal {
            status: ConnectionStatus::Outdated,
            reason: outdated_reason(reason),
            purge: None,
        },
        Some(4404) => SupervisorStep::Terminal {
            status: ConnectionStatus::Revoked,
            reason: "guilde supprimée par un admin".to_string(),
            purge: Some(GuildRemovalReason::GuildDeleted),
        },
        Some(4409) => SupervisorStep::Retry {
            rate_limited: true,
            reason: "limite de débit du serveur".to_string(),
        },
        Some(4408) => SupervisorStep::Retry {
            rate_limited: false,
            reason: "trame trop volumineuse".to_string(),
        },
        Some(_) | None => SupervisorStep::Retry {
            rate_limited: false,
            reason: if reason.trim().is_empty() {
                "connexion interrompue".to_string()
            } else {
                reason.to_string()
            },
        },
    }
}

fn outdated_reason(raw_reason: &str) -> String {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OutdatedClose {
        min_version: Option<String>,
    }
    match serde_json::from_str::<OutdatedClose>(raw_reason) {
        Ok(close) => match close.min_version {
            Some(version) => format!("mise à jour requise (version minimale {version})"),
            None => "mise à jour de l'overlay requise".to_string(),
        },
        Err(_) => "mise à jour de l'overlay requise".to_string(),
    }
}

fn build_session_config(membership: &Membership) -> Result<SessionConfig, String> {
    let server_url = membership.server_url.as_str();
    let token = Some(membership.client_token.as_str())
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "clientToken absent de la configuration".to_string())?;
    let insecure_tls = tls::insecure_tls_enabled();
    let ws_url = ws_url_from_server_url(server_url)?;
    let session_mode = if membership.guest_mode {
        SessionMode::InviteReadOnly
    } else {
        SessionMode::Normal
    };
    Ok(SessionConfig {
        ws_url,
        hello: HelloPayload {
            token: token.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            client_id: membership.client_id.clone(),
            session_mode: Some(session_mode),
        },
        insecure_tls,
        queued_keepalive_interval: QUEUED_KEEPALIVE_INTERVAL,
    })
}

/// Convertit `{API_URL}` en URL WebSocket `ws(s)://…/ws`.
fn ws_url_from_server_url(server_url: &str) -> Result<Url, String> {
    ws_url_from_server_url_with(server_url, tls::insecure_tls_enabled())
}

/// Variante testable : `allow_insecure` autorise `http://`/`ws://` (dev only).
fn ws_url_from_server_url_with(server_url: &str, allow_insecure: bool) -> Result<Url, String> {
    let trimmed = server_url.trim();
    if trimmed.is_empty() {
        return Err("serverUrl vide".to_string());
    }
    let mut url = Url::parse(trimmed).map_err(|error| format!("serverUrl invalide : {error}"))?;
    let scheme = match url.scheme() {
        "http" | "ws" => {
            if !allow_insecure {
                return Err(
                    "connexion WebSocket non sécurisée refusée : utilisez https:// (ou \
                     LIVECHAT_INSECURE_TLS=1 en build debug pour le développement local)"
                        .to_string(),
                );
            }
            "ws"
        }
        "https" | "wss" => "wss",
        other => return Err(format!("schéma serverUrl non supporté : {other}")),
    };
    url.set_scheme(scheme)
        .map_err(|_| "schéma WebSocket invalide".to_string())?;
    url.set_path(lc_protocol::WS_PATH);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn build_connector(insecure_tls: bool) -> Result<Option<Connector>, String> {
    if !insecure_tls {
        return Ok(None);
    }
    let connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|error| format!("configuration TLS insecure impossible : {error}"))?;
    Ok(Some(Connector::NativeTls(connector)))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// Enveloppe id used for a client frame (unique per connection).
fn new_envelope_id() -> String {
    format!("cli-{}", Uuid::new_v4())
}

/// Envoie une trame cliente enveloppée avec un id déjà connu de l'appelant.
async fn send_message_with_id<S>(
    sink: &mut S,
    id: &str,
    message: ClientMessage,
) -> Result<(), String>
where
    S: futures_util::Sink<Message, Error = tungstenite::Error> + Unpin,
{
    let envelope = ClientEnvelope::new(id.to_string(), now_ms(), message);
    let text = serde_json::to_string(&envelope)
        .map_err(|error| format!("sérialisation de trame impossible : {error}"))?;
    sink.send(Message::Text(text.into()))
        .await
        .map_err(|error| error.to_string())
}

/// Envoie une trame cliente enveloppée (id/timestamp générés) et retourne
/// l'identifiant de l'enveloppe.
async fn send_message<S>(sink: &mut S, message: ClientMessage) -> Result<String, String>
where
    S: futures_util::Sink<Message, Error = tungstenite::Error> + Unpin,
{
    let id = new_envelope_id();
    send_message_with_id(sink, &id, message).await?;
    Ok(id)
}

/// Returns the `type` of a text frame when it names no known server message.
///
/// Unknown message types are ignored so a newer server can add frames without
/// breaking older overlays; anything else is a protocol error.
fn unknown_server_message_type(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let message_type = value.as_object()?.get("type")?.as_str()?;
    if KNOWN_SERVER_MESSAGE_TYPES.contains(&message_type) {
        None
    } else {
        Some(message_type.to_string())
    }
}

/// Exécute une session WebSocket complète jusqu'à fermeture ou erreur.
async fn run_session<E: SessionEffects>(
    effects: &E,
    config: &SessionConfig,
    commands: &mut UnboundedReceiver<NetCommand>,
) -> SessionOutcome {
    let request = match config.ws_url.as_str().into_client_request() {
        Ok(request) => request,
        Err(error) => {
            return SessionOutcome::ConnectFailed(format!("requête WebSocket invalide : {error}"))
        }
    };
    let connector = match build_connector(config.insecure_tls) {
        Ok(connector) => connector,
        Err(error) => return SessionOutcome::ConnectFailed(error),
    };
    let connection = connect_async_tls_with_config(request, None, false, connector);
    let (socket, _response) = match tokio::time::timeout(CONNECT_TIMEOUT, connection).await {
        Err(_) => return SessionOutcome::ConnectFailed("délai de connexion dépassé".to_string()),
        Ok(Err(error)) => {
            return SessionOutcome::ConnectFailed(format!("connexion échouée : {error}"))
        }
        Ok(Ok(pair)) => pair,
    };
    let (mut sink, mut stream) = socket.split();

    if let Err(error) = send_message(&mut sink, ClientMessage::Hello(config.hello.clone())).await {
        return SessionOutcome::ConnectFailed(format!("envoi du hello impossible : {error}"));
    }

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut heartbeat_frame: Option<ClientMessage> = None;
    let mut queued_keepalive = tokio::time::interval(config.queued_keepalive_interval);
    queued_keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    queued_keepalive.tick().await;
    // Acks expected by `SendWithAck` callers, keyed by the sent envelope id.
    // Dropped with the session: callers observe a closed reply channel.
    let mut pending_acks: HashMap<String, oneshot::Sender<AckPayload>> = HashMap::new();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    None => {
                        return SessionOutcome::Closed {
                            code: None,
                            reason: "flux WebSocket fermé".to_string(),
                        };
                    }
                    Some(Err(error)) => {
                        return SessionOutcome::Closed {
                            code: None,
                            reason: error.to_string(),
                        };
                    }
                    Some(Ok(message)) => {
                        match message {
                            Message::Text(text) => {
                                let envelope = match serde_json::from_str::<ServerEnvelope>(text.as_str()) {
                                    Ok(envelope) => envelope,
                                    Err(error) => {
                                        let unknown_type =
                                            unknown_server_message_type(text.as_str());
                                        if let Some(message_type) = unknown_type {
                                            tracing::warn!(
                                                message_type = %message_type,
                                                "type de message serveur inconnu ignoré"
                                            );
                                            continue;
                                        }
                                        return SessionOutcome::Failed(format!(
                                            "trame serveur invalide : {error}"
                                        ));
                                    }
                                };
                                match envelope.message {
                                    ServerMessage::Welcome(payload) => {
                                        effects.on_welcome(&payload);
                                        let frame = ClientMessage::Heartbeat(HeartbeatPayload {
                                            client_id: payload.client_id.clone(),
                                            guild_id: payload.guild_id.clone(),
                                            app_version: config.hello.app_version.clone(),
                                            session_mode: Some(payload.session_mode),
                                        });
                                        heartbeat_frame = Some(frame.clone());
                                        if let Err(error) = send_message(&mut sink, frame).await {
                                            return SessionOutcome::Closed {
                                                code: None,
                                                reason: format!("envoi du heartbeat impossible : {error}"),
                                            };
                                        }
                                        if let Some(current) = &payload.current_job {
                                            tracing::info!(
                                                job_id = %current.play.job_id,
                                                remaining_ms = current.remaining_ms,
                                                "job courant adopté après welcome"
                                            );
                                        }
                                    }
                                    ServerMessage::Settings(payload) => effects.on_settings(&payload),
                                    ServerMessage::Play(payload) => effects.on_play(&payload),
                                    ServerMessage::Stop(payload) => effects.on_stop(&payload),
                                    ServerMessage::Peers(payload) => effects.on_peers(&payload),
                                    ServerMessage::Ack(payload) => {
                                        match pending_acks.remove(&payload.ref_id) {
                                            Some(reply) => {
                                                if reply.send(payload).is_err() {
                                                    tracing::debug!(
                                                        "ack attendu abandonné : appelant disparu"
                                                    );
                                                }
                                            }
                                            None => effects.on_ack(&payload),
                                        }
                                    }
                                    ServerMessage::Error(payload) => effects.on_server_error(&payload),
                                    ServerMessage::MediaIngestion(payload) => {
                                        effects.on_media_ingestion(&payload)
                                    }
                                    ServerMessage::BoardChanged(payload) => {
                                        effects.on_board_changed(&payload)
                                    }
                                }
                            }
                            Message::Close(frame) => {
                                let (code, reason) = match frame {
                                    Some(frame) => {
                                        (Some(u16::from(frame.code)), frame.reason.to_string())
                                    }
                                    None => (None, String::new()),
                                };
                                return SessionOutcome::Closed { code, reason };
                            }
                            Message::Ping(payload) => {
                                if let Err(error) = sink.send(Message::Pong(payload)).await {
                                    return SessionOutcome::Closed {
                                        code: None,
                                        reason: format!("envoi du pong impossible : {error}"),
                                    };
                                }
                            }
                            Message::Pong(_) => {}
                            Message::Binary(_) => {
                                return SessionOutcome::Failed(
                                    "trame binaire inattendue (texte UTF-8 attendu)".to_string(),
                                );
                            }
                            Message::Frame(_) => {}
                        }
                    }
                }
            }
            command = commands.recv() => {
                match command {
                    None | Some(NetCommand::Shutdown) => return SessionOutcome::Shutdown,
                    Some(NetCommand::Reconnect) => return SessionOutcome::ReconnectRequested,
                    Some(NetCommand::Send(message)) => {
                        if let Err(error) = send_message(&mut sink, message).await {
                            return SessionOutcome::Closed {
                                code: None,
                                reason: format!("envoi de trame impossible : {error}"),
                            };
                        }
                    }
                    Some(NetCommand::SendWithAck { message, reply }) => {
                        let id = new_envelope_id();
                        match send_message_with_id(&mut sink, &id, message).await {
                            Ok(()) => {
                                pending_acks.insert(id, reply);
                            }
                            Err(error) => {
                                return SessionOutcome::Closed {
                                    code: None,
                                    reason: format!("envoi de trame impossible : {error}"),
                                };
                            }
                        }
                    }
                }
            }
            _ = heartbeat.tick() => {
                if let Some(frame) = heartbeat_frame.clone() {
                    if let Err(error) = send_message(&mut sink, frame).await {
                        return SessionOutcome::Closed {
                            code: None,
                            reason: format!("envoi du heartbeat impossible : {error}"),
                        };
                    }
                }
            }
            _ = queued_keepalive.tick() => {
                // La file locale n'est annoncée qu'après le `welcome` : sans lui
                // le serveur ne connaît pas encore la session.
                if heartbeat_frame.is_some() {
                    for (job_id, duration_ms) in effects.queued_jobs() {
                        tracing::debug!(
                            job_id = %job_id,
                            duration_ms,
                            "keep-alive playback.state d'un play en file locale"
                        );
                        let frame = ClientMessage::PlaybackState(PlaybackStatePayload {
                            job_id,
                            state: PlaybackState::Paused,
                            remaining_ms: Some(duration_ms),
                        });
                        if let Err(error) = send_message(&mut sink, frame).await {
                            return SessionOutcome::Closed {
                                code: None,
                                reason: format!(
                                    "envoi du keep-alive playback.state impossible : {error}"
                                ),
                            };
                        }
                    }
                }
            }
        }
    }
}

/// Calcule le reprise d'un job après reconnexion (protocole §6.1.1).
fn adopt_current_job(play: PlayPayload, remaining_ms: u64) -> PlayPayload {
    let mut play = play;
    let remaining_sec = remaining_ms.div_ceil(1000);
    let elapsed_sec = play.duration_sec.saturating_sub(remaining_sec);
    if let Some(media) = play.media.as_mut() {
        let base = media.start_offset_sec.unwrap_or(0);
        media.start_offset_sec = Some(base.saturating_add(elapsed_sec));
    }
    play
}

/// Guild identity fields refreshed by a `welcome`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentityUpdate {
    client_id: String,
    guest_mode: bool,
    /// New display name, when it differs from the stored one.
    guild_name: Option<String>,
    /// `Some` = apply this icon value (`None` clears it), `None` = no change.
    guild_icon_url: Option<Option<String>>,
}

impl IdentityUpdate {
    fn apply(&self, membership: &mut Membership) {
        membership.client_id = Some(self.client_id.clone());
        membership.guest_mode = self.guest_mode;
        if let Some(guild_name) = &self.guild_name {
            membership.guild_name = Some(guild_name.clone());
        }
        if let Some(guild_icon_url) = &self.guild_icon_url {
            membership.guild_icon_url = guild_icon_url.clone();
        }
    }
}

/// Icons are only trusted from Discord's CDN.
const GUILD_ICON_PREFIX: &str = "https://cdn.discordapp.com/";

/// Computes the membership update carried by a `welcome`, or `None` when the
/// stored identity and the welcome agree.
fn identity_from_welcome(
    membership: &Membership,
    payload: &WelcomePayload,
) -> Option<IdentityUpdate> {
    let guest_mode = payload.session_mode == SessionMode::InviteReadOnly;
    let client_changed = membership.client_id.as_deref() != Some(payload.client_id.as_str());
    let guest_mode_changed = membership.guest_mode != guest_mode;

    let welcome_name = payload
        .guild_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let guild_name = welcome_name
        .filter(|name| membership.guild_name.as_deref() != Some(*name))
        .map(ToString::to_string);

    let welcome_icon = payload
        .guild_icon_url
        .as_deref()
        .map(str::trim)
        .filter(|url| url.starts_with(GUILD_ICON_PREFIX))
        .map(ToString::to_string);
    let guild_icon_url = match welcome_icon {
        Some(icon) => Some(Some(icon)),
        // A modern welcome without an icon means a guild without an icon.
        None if welcome_name.is_some() => Some(None),
        // An old welcome carries no identity: leave the icon untouched.
        None => None,
    };
    let icon_changed = match &guild_icon_url {
        Some(Some(icon)) => membership.guild_icon_url.as_deref() != Some(icon.as_str()),
        Some(None) => membership.guild_icon_url.is_some(),
        None => false,
    };

    if !client_changed && !guest_mode_changed && guild_name.is_none() && !icon_changed {
        return None;
    }
    Some(IdentityUpdate {
        client_id: payload.client_id.clone(),
        guest_mode,
        guild_name,
        guild_icon_url,
    })
}

/// Effets Tauri : dispatcher vers le renderer, le tray et l'état partagé.
struct AppEffects {
    app: AppHandle<Wry>,
    /// Guilde de la session (clé de l'adhésion).
    guild_id: String,
}

impl SessionEffects for AppEffects {
    fn on_welcome(&self, payload: &WelcomePayload) {
        let app = &self.app;
        let guild_id = self.guild_id.as_str();
        let state = app.state::<AppState>();
        let config = state.config_snapshot();
        let identity = config
            .membership(guild_id)
            .and_then(|membership| identity_from_welcome(membership, payload));
        if let Some(identity) = identity {
            if let Err(error) = state.update_config(|config| {
                config.update_membership(guild_id, |membership| identity.apply(membership));
            }) {
                tracing::warn!(%error, "échec de persistance de l'identité serveur");
            }
            crate::shortcuts::reapply_for_session(app);
        }
        state.set_net_welcome(guild_id, payload);
        tracing::info!(
            server_version = %payload.server_version,
            client_id = %payload.client_id,
            guild_id = %guild_id,
            session_mode = ?payload.session_mode,
            "welcome reçu"
        );
        refresh_tray_from_any_thread(app);
        if state.overlay_ready() {
            if let Err(error) = events::emit_settings(app, &state.view()) {
                tracing::debug!(%error, "échec d'émission des settings après welcome");
            }
        }
        if let Some(current) = &payload.current_job {
            let adopted = adopt_current_job(current.play.clone(), current.remaining_ms);
            self.present_play(&adopted);
        }
    }

    fn on_settings(&self, payload: &SettingsPayload) {
        let state = self.app.state::<AppState>();
        state.set_net_settings_summary(&self.guild_id, payload.session_mode);
        tracing::info!(
            guild_id = %payload.guild_id,
            show_text_default = payload.show_text_default,
            max_media_time = ?payload.max_media_time,
            "settings serveur reçus"
        );
        if state.overlay_ready() {
            if let Err(error) = events::emit_settings(&self.app, &state.view()) {
                tracing::debug!(%error, "échec d'émission des settings vers le renderer");
            }
        }
        refresh_tray_from_any_thread(&self.app);
    }

    fn on_play(&self, payload: &PlayPayload) {
        self.present_play(payload);
        if let Err(error) = events::emit_queue_hint(&self.app, &self.guild_id) {
            tracing::debug!(%error, "échec d'émission de l'indice de file d'attente");
        }
    }

    fn on_stop(&self, payload: &StopPayload) {
        if let Err(error) = events::emit_queue_hint(&self.app, &self.guild_id) {
            tracing::debug!(%error, "échec d'émission de l'indice de file d'attente");
        }
        let app = &self.app;
        let state = app.state::<AppState>();
        // Un `stop` ne concerne l'écran que si cette guilde l'occupe ; sinon il
        // annule seulement ce qu'elle faisait attendre.
        let dropped = state.drop_queued(&self.guild_id, payload.job_id.as_deref());
        let owner = state.playback_guild();
        if owner.is_some() && owner.as_deref() != Some(self.guild_id.as_str()) {
            tracing::info!(
                guild_id = %self.guild_id,
                dropped,
                "stop reçu d'une guilde qui n'occupe pas l'écran"
            );
            return;
        }
        state.clear_playback();
        match serde_json::to_value(payload) {
            Ok(value) => {
                tracing::info!(reason = ?payload.reason, guild_id = %self.guild_id, "stop reçu");
                events::deliver_or_buffer(app, state.overlay_ready(), OverlayEvent::stop(value));
            }
            Err(error) => tracing::warn!(%error, "stop non sérialisable"),
        }
        advance_local_queue(app);
    }

    fn on_peers(&self, payload: &PeersPayload) {
        let state = self.app.state::<AppState>();
        let labels = payload
            .peers
            .iter()
            .map(|peer| peer.label.clone())
            .collect::<Vec<_>>();
        state.set_net_peers(&self.guild_id, labels);
        refresh_tray_from_any_thread(&self.app);
    }

    fn on_ack(&self, payload: &AckPayload) {
        if payload.ok {
            tracing::debug!(reference = %payload.ref_id, "ack reçu");
        } else {
            let error = payload
                .error
                .as_ref()
                .map(|error| format!("{}: {}", error.code, error.message))
                .unwrap_or_else(|| "ack en erreur sans détail".to_string());
            tracing::warn!(reference = %payload.ref_id, %error, "ack en erreur");
            self.app
                .state::<AppState>()
                .set_net_error(&self.guild_id, error);
            refresh_tray_from_any_thread(&self.app);
        }
    }

    fn on_server_error(&self, payload: &ServerErrorPayload) {
        let message = format!("{}: {}", payload.code, payload.message);
        tracing::warn!(%message, job_id = ?payload.job_id, "erreur serveur non fatale");
        self.app
            .state::<AppState>()
            .set_net_error(&self.guild_id, message);
        refresh_tray_from_any_thread(&self.app);
    }

    fn on_media_ingestion(&self, payload: &MediaIngestionPayload) {
        if let Err(error) = events::emit_media_ingestion(&self.app, &self.guild_id, payload) {
            tracing::debug!(%error, "échec d'émission du résultat d'ingestion");
        }
    }

    fn on_board_changed(&self, payload: &BoardChangedPayload) {
        if let Err(error) = events::emit_board_changed(&self.app, &self.guild_id, payload) {
            tracing::debug!(%error, "échec d'émission du changement de board");
        }
    }

    fn queued_jobs(&self) -> Vec<(String, u64)> {
        self.app.state::<AppState>().queued_jobs_for(&self.guild_id)
    }
}

/// Effective renderer volume: general volume × the guild's own volume.
pub(crate) fn effective_volume(general_volume: f64, guild_volume: f64) -> f64 {
    crate::config::normalize_volume(general_volume) * crate::config::normalize_volume(guild_volume)
}

/// Serializes a `play` for the renderer and adds the computed effective volume.
pub(crate) fn renderer_play_payload(
    payload: &PlayPayload,
    general_volume: f64,
    guild_volume: f64,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut value = serde_json::to_value(payload)?;
    if let serde_json::Value::Object(object) = &mut value {
        object.insert(
            "volume".to_string(),
            serde_json::json!(effective_volume(general_volume, guild_volume)),
        );
    }
    Ok(value)
}

impl AppEffects {
    /// Affiche un `play`, ou le met en attente si une autre guilde occupe
    /// l'écran (un seul média à la fois, toutes guildes confondues).
    fn present_play(&self, payload: &PlayPayload) {
        let app = &self.app;
        let state = app.state::<AppState>();
        let config = state.config_snapshot();
        let guild_volume = config
            .membership(&self.guild_id)
            .map_or(1.0, |membership| membership.volume);
        let value = match renderer_play_payload(payload, config.volume, guild_volume) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error, "play non sérialisable");
                return;
            }
        };
        let owner = state.playback_guild();
        if owner.is_some() && owner.as_deref() != Some(self.guild_id.as_str()) {
            tracing::info!(
                job_id = %payload.job_id,
                guild_id = %self.guild_id,
                "play mis en attente : une autre guilde occupe l'écran"
            );
            state.enqueue_play(QueuedPlay {
                guild_id: self.guild_id.clone(),
                job_id: payload.job_id.clone(),
                payload: value,
            });
            return;
        }
        tracing::info!(job_id = %payload.job_id, guild_id = %self.guild_id, "play reçu");
        state.set_playback(&self.guild_id, payload.job_id.clone());
        events::deliver_or_buffer(app, state.overlay_ready(), OverlayEvent::play(value));
    }
}

/// Entrée de test : minimise la surface (welcome/play/stop non instrumentés
/// ailleurs) tout en gardant le rendu réel.
#[allow(dead_code)]
pub(crate) fn emit_renderer_settings(app: &AppHandle<Wry>, view: &OverlayConfigView) {
    if let Err(error) = events::emit_settings(app, view) {
        tracing::debug!(%error, "échec d'émission des settings");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::WebSocketStream;

    use lc_protocol::{
        AckOutcome, BoardChange, BoardChangedPayload, MediaIngestionError, MediaIngestionPayload,
        MediaTrigger, MediaTriggerPayload,
    };

    #[derive(Default)]
    struct RecordingEffects {
        welcome: StdMutex<u32>,
        plays: StdMutex<Vec<String>>,
        stops: StdMutex<u32>,
        peers: StdMutex<u32>,
        settings: StdMutex<u32>,
        acks: StdMutex<u32>,
        media_ingestions: StdMutex<Vec<MediaIngestionPayload>>,
        board_changes: StdMutex<Vec<BoardChangedPayload>>,
        queued: StdMutex<Vec<(String, u64)>>,
    }

    impl SessionEffects for RecordingEffects {
        fn on_welcome(&self, _payload: &WelcomePayload) {
            if let Ok(mut counter) = self.welcome.lock() {
                *counter += 1;
            }
        }

        fn on_settings(&self, _payload: &SettingsPayload) {
            if let Ok(mut counter) = self.settings.lock() {
                *counter += 1;
            }
        }

        fn on_play(&self, payload: &PlayPayload) {
            if let Ok(mut plays) = self.plays.lock() {
                plays.push(payload.job_id.clone());
            }
        }

        fn on_stop(&self, _payload: &StopPayload) {
            if let Ok(mut counter) = self.stops.lock() {
                *counter += 1;
            }
        }

        fn on_peers(&self, _payload: &PeersPayload) {
            if let Ok(mut counter) = self.peers.lock() {
                *counter += 1;
            }
        }

        fn on_ack(&self, _payload: &AckPayload) {
            if let Ok(mut counter) = self.acks.lock() {
                *counter += 1;
            }
        }

        fn on_server_error(&self, _payload: &ServerErrorPayload) {}

        fn on_media_ingestion(&self, payload: &MediaIngestionPayload) {
            if let Ok(mut received) = self.media_ingestions.lock() {
                received.push(payload.clone());
            }
        }

        fn on_board_changed(&self, payload: &BoardChangedPayload) {
            if let Ok(mut received) = self.board_changes.lock() {
                received.push(payload.clone());
            }
        }

        fn queued_jobs(&self) -> Vec<(String, u64)> {
            self.queued
                .lock()
                .map(|queued| queued.clone())
                .unwrap_or_default()
        }
    }

    fn welcome_payload() -> WelcomePayload {
        WelcomePayload {
            protocol_version: 2,
            guild_id: "guild-1".to_string(),
            guild_name: None,
            guild_icon_url: None,
            client_id: "client-1".to_string(),
            session_mode: SessionMode::Normal,
            server_version: "0.1.0-test".to_string(),
            server_time: now_ms(),
            config: lc_protocol::WelcomeConfig {
                show_text_default: true,
                default_media_time: 10,
                max_media_time: None,
            },
            current_job: None,
        }
    }

    fn play_payload() -> PlayPayload {
        PlayPayload {
            job_id: "job-1".to_string(),
            media: Some(lc_protocol::MediaPayload {
                asset_id: "asset-1".to_string(),
                url: "http://127.0.0.1/media/asset-1.mp4".to_string(),
                mime: "video/mp4".to_string(),
                kind: lc_protocol::MediaKind::Video,
                duration_sec: Some(10),
                is_vertical: false,
                start_offset_sec: None,
            }),
            text: lc_protocol::TextPayload {
                value: String::new(),
                enabled: false,
            },
            author: lc_protocol::AuthorPayload {
                name: String::new(),
                image: None,
                enabled: false,
            },
            tweet_card: None,
            duration_sec: 10,
            ..Default::default()
        }
    }

    fn media_ingestion_payload() -> MediaIngestionPayload {
        MediaIngestionPayload {
            ingestion_id: "ing-1".to_string(),
            ok: false,
            created: None,
            item_id: None,
            error: Some(MediaIngestionError {
                code: "unsupported_media".to_string(),
                message: Some("format inconnu".to_string()),
            }),
        }
    }

    fn board_changed_payload() -> BoardChangedPayload {
        BoardChangedPayload {
            change: BoardChange::Added,
            item_id: "item-1".to_string(),
        }
    }

    fn session_config(address: std::net::SocketAddr) -> SessionConfig {
        SessionConfig {
            ws_url: ws_url_from_server_url_with(&format!("http://{address}"), true)
                .expect("url WebSocket de test"),
            hello: HelloPayload {
                token: "token-test".to_string(),
                app_version: "3.0.0".to_string(),
                client_id: Some("client-1".to_string()),
                session_mode: Some(SessionMode::Normal),
            },
            insecure_tls: false,
            queued_keepalive_interval: QUEUED_KEEPALIVE_INTERVAL,
        }
    }

    async fn next_client_envelope(
        socket: &mut WebSocketStream<tokio::net::TcpStream>,
    ) -> Option<ClientEnvelope> {
        loop {
            let message = socket.next().await?.ok()?;
            match message {
                Message::Text(text) => {
                    return serde_json::from_str::<ClientEnvelope>(text.as_str()).ok();
                }
                Message::Close(_) => return None,
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
            }
        }
    }

    async fn next_client_message(
        socket: &mut WebSocketStream<tokio::net::TcpStream>,
    ) -> Option<ClientMessage> {
        next_client_envelope(socket)
            .await
            .map(|envelope| envelope.message)
    }

    async fn send_server_message(
        socket: &mut WebSocketStream<tokio::net::TcpStream>,
        id: &str,
        message: ServerMessage,
    ) {
        let envelope = ServerEnvelope::new(id.to_string(), now_ms(), message);
        let text = serde_json::to_string(&envelope).expect("enveloppe serveur");
        socket
            .send(Message::Text(text.into()))
            .await
            .expect("envoi serveur");
    }

    async fn send_server_text(socket: &mut WebSocketStream<tokio::net::TcpStream>, text: &str) {
        socket
            .send(Message::Text(text.to_string().into()))
            .await
            .expect("envoi serveur brut");
    }

    #[tokio::test]
    async fn session_handshake_heartbeat_play_and_close() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("handshake serveur");
            let hello = next_client_message(&mut socket).await;
            assert!(matches!(hello, Some(ClientMessage::Hello(_))));
            send_server_message(
                &mut socket,
                "srv-welcome",
                ServerMessage::Welcome(welcome_payload()),
            )
            .await;
            let heartbeat = next_client_message(&mut socket).await;
            assert!(matches!(heartbeat, Some(ClientMessage::Heartbeat(_))));
            send_server_message(&mut socket, "srv-play", ServerMessage::Play(play_payload())).await;
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "bye".into(),
                })))
                .await
                .expect("close serveur");
            let _closed = socket.close(None).await;
        });

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
        let session = SessionConfig {
            ws_url: ws_url_from_server_url_with(&format!("http://{address}"), true)
                .expect("url WebSocket de test"),
            hello: HelloPayload {
                token: "token-test".to_string(),
                app_version: "3.0.0".to_string(),
                client_id: Some("client-1".to_string()),
                session_mode: Some(SessionMode::Normal),
            },
            insecure_tls: false,
            queued_keepalive_interval: QUEUED_KEEPALIVE_INTERVAL,
        };
        let effects = RecordingEffects::default();
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_session(&effects, &session, &mut receiver),
        )
        .await
        .expect("session non bloquée");
        match outcome {
            SessionOutcome::Closed { code, .. } => assert_eq!(code, Some(1000)),
            other => panic!("fin de session inattendue : {other:?}"),
        }
        assert_eq!(*effects.welcome.lock().expect("welcome"), 1);
        assert_eq!(effects.plays.lock().expect("plays").as_slice(), ["job-1"]);
        drop(sender);
        server.await.expect("serveur mock");
    }

    #[tokio::test]
    async fn session_dispatches_media_ingestion_and_board_changed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("handshake serveur");
            let hello = next_client_message(&mut socket).await;
            assert!(matches!(hello, Some(ClientMessage::Hello(_))));
            send_server_message(
                &mut socket,
                "srv-welcome",
                ServerMessage::Welcome(welcome_payload()),
            )
            .await;
            send_server_message(
                &mut socket,
                "srv-ingestion",
                ServerMessage::MediaIngestion(media_ingestion_payload()),
            )
            .await;
            send_server_message(
                &mut socket,
                "srv-board",
                ServerMessage::BoardChanged(board_changed_payload()),
            )
            .await;
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "bye".into(),
                })))
                .await
                .expect("close serveur");
            let _closed = socket.close(None).await;
        });

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
        let session = session_config(address);
        let effects = RecordingEffects::default();
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_session(&effects, &session, &mut receiver),
        )
        .await
        .expect("session non bloquée");
        match outcome {
            SessionOutcome::Closed { code, .. } => assert_eq!(code, Some(1000)),
            other => panic!("fin de session inattendue : {other:?}"),
        }
        assert_eq!(
            effects
                .media_ingestions
                .lock()
                .expect("ingestions")
                .as_slice(),
            [media_ingestion_payload()]
        );
        assert_eq!(
            effects
                .board_changes
                .lock()
                .expect("changements de board")
                .as_slice(),
            [board_changed_payload()]
        );
        drop(sender);
        server.await.expect("serveur mock");
    }

    #[tokio::test]
    async fn unknown_server_message_type_is_ignored_and_session_continues() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("handshake serveur");
            let hello = next_client_message(&mut socket).await;
            assert!(matches!(hello, Some(ClientMessage::Hello(_))));
            send_server_message(
                &mut socket,
                "srv-welcome",
                ServerMessage::Welcome(welcome_payload()),
            )
            .await;
            send_server_text(
                &mut socket,
                r#"{"v":2,"id":"srv-future","ts":0,"type":"future.event","payload":{"x":1}}"#,
            )
            .await;
            send_server_message(&mut socket, "srv-play", ServerMessage::Play(play_payload())).await;
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "bye".into(),
                })))
                .await
                .expect("close serveur");
            let _closed = socket.close(None).await;
        });

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
        let session = session_config(address);
        let effects = RecordingEffects::default();
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_session(&effects, &session, &mut receiver),
        )
        .await
        .expect("session non bloquée");
        match outcome {
            SessionOutcome::Closed { code, .. } => assert_eq!(code, Some(1000)),
            other => panic!("fin de session inattendue : {other:?}"),
        }
        assert_eq!(effects.plays.lock().expect("plays").as_slice(), ["job-1"]);
        drop(sender);
        server.await.expect("serveur mock");
    }

    #[tokio::test]
    async fn known_server_message_type_with_invalid_payload_fails_the_session() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("handshake serveur");
            let hello = next_client_message(&mut socket).await;
            assert!(matches!(hello, Some(ClientMessage::Hello(_))));
            send_server_message(
                &mut socket,
                "srv-welcome",
                ServerMessage::Welcome(welcome_payload()),
            )
            .await;
            send_server_text(
                &mut socket,
                r#"{"v":2,"id":"srv-bad","ts":0,"type":"play","payload":null}"#,
            )
            .await;
            let _closed = socket.close(None).await;
        });

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
        let session = session_config(address);
        let effects = RecordingEffects::default();
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_session(&effects, &session, &mut receiver),
        )
        .await
        .expect("session non bloquée");
        match outcome {
            SessionOutcome::Failed(reason) => {
                assert!(reason.starts_with("trame serveur invalide"), "{reason}");
            }
            other => panic!("fin de session inattendue : {other:?}"),
        }
        drop(sender);
        server.await.expect("serveur mock");
    }

    #[test]
    fn unknown_server_message_type_only_matches_unknown_types() {
        assert_eq!(
            unknown_server_message_type(r#"{"type":"future.event"}"#),
            Some("future.event".to_string())
        );
        assert_eq!(unknown_server_message_type(r#"{"type":"play"}"#), None);
        assert_eq!(unknown_server_message_type(r#"{"type":3}"#), None);
        assert_eq!(unknown_server_message_type("pas du json"), None);
    }

    #[tokio::test]
    async fn session_sends_keepalive_for_locally_queued_job() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("handshake serveur");
            let hello = next_client_message(&mut socket).await;
            assert!(matches!(hello, Some(ClientMessage::Hello(_))));
            send_server_message(
                &mut socket,
                "srv-welcome",
                ServerMessage::Welcome(welcome_payload()),
            )
            .await;
            let keepalive = loop {
                match next_client_message(&mut socket).await {
                    Some(ClientMessage::PlaybackState(payload))
                        if payload.job_id == "job-queue-1" =>
                    {
                        break payload;
                    }
                    Some(_) => continue,
                    None => panic!("flux fermé avant le keep-alive"),
                }
            };
            assert_eq!(keepalive.state, PlaybackState::Paused);
            assert_eq!(keepalive.remaining_ms, Some(7_000));
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "bye".into(),
                })))
                .await
                .expect("close serveur");
            let _closed = socket.close(None).await;
        });

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
        let session = SessionConfig {
            ws_url: ws_url_from_server_url_with(&format!("http://{address}"), true)
                .expect("url WebSocket de test"),
            hello: HelloPayload {
                token: "token-test".to_string(),
                app_version: "3.0.0".to_string(),
                client_id: Some("client-1".to_string()),
                session_mode: Some(SessionMode::Normal),
            },
            insecure_tls: false,
            queued_keepalive_interval: Duration::from_millis(50),
        };
        let effects = RecordingEffects::default();
        effects
            .queued
            .lock()
            .expect("queued")
            .push(("job-queue-1".to_string(), 7_000));
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_session(&effects, &session, &mut receiver),
        )
        .await
        .expect("session non bloquée");
        match outcome {
            SessionOutcome::Closed { code, .. } => assert_eq!(code, Some(1000)),
            other => panic!("fin de session inattendue : {other:?}"),
        }
        drop(sender);
        server.await.expect("serveur mock");
    }

    #[tokio::test]
    async fn send_with_ack_routes_the_matching_ack_to_the_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("handshake serveur");
            let hello = next_client_message(&mut socket).await;
            assert!(matches!(hello, Some(ClientMessage::Hello(_))));
            send_server_message(
                &mut socket,
                "srv-welcome",
                ServerMessage::Welcome(welcome_payload()),
            )
            .await;
            let envelope = loop {
                match next_client_envelope(&mut socket).await {
                    Some(envelope) => match envelope.message {
                        ClientMessage::MediaTrigger(_) => break envelope,
                        _ => continue,
                    },
                    None => panic!("flux fermé avant media.trigger"),
                }
            };
            send_server_message(
                &mut socket,
                "srv-ack",
                ServerMessage::Ack(AckPayload {
                    ref_id: envelope.id,
                    ok: true,
                    error: None,
                    outcome: Some(AckOutcome::Queued),
                    position: Some(2),
                    job_id: Some("job-42".to_string()),
                }),
            )
            .await;
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "bye".into(),
                })))
                .await
                .expect("close serveur");
            let _closed = socket.close(None).await;
        });

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
        let (reply, reply_receiver) = oneshot::channel();
        sender
            .send(NetCommand::SendWithAck {
                message: ClientMessage::MediaTrigger(MediaTriggerPayload {
                    item_id: "item-1".to_string(),
                    trigger: MediaTrigger::Ui,
                }),
                reply,
            })
            .expect("commande en file");
        let session = SessionConfig {
            ws_url: ws_url_from_server_url_with(&format!("http://{address}"), true)
                .expect("url WebSocket de test"),
            hello: HelloPayload {
                token: "token-test".to_string(),
                app_version: "3.0.0".to_string(),
                client_id: Some("client-1".to_string()),
                session_mode: Some(SessionMode::Normal),
            },
            insecure_tls: false,
            queued_keepalive_interval: QUEUED_KEEPALIVE_INTERVAL,
        };
        let effects = RecordingEffects::default();
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_session(&effects, &session, &mut receiver),
        )
        .await
        .expect("session non bloquée");
        match outcome {
            SessionOutcome::Closed { code, .. } => assert_eq!(code, Some(1000)),
            other => panic!("fin de session inattendue : {other:?}"),
        }

        let payload = reply_receiver.await.expect("ack routé vers le reply");
        assert!(payload.ok);
        assert_eq!(payload.outcome, Some(AckOutcome::Queued));
        assert_eq!(payload.position, Some(2));
        assert_eq!(payload.job_id.as_deref(), Some("job-42"));
        assert_eq!(*effects.acks.lock().expect("acks"), 0);
        drop(sender);
        server.await.expect("serveur mock");
    }

    #[tokio::test]
    async fn ack_without_expected_reference_is_dispatched_to_effects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept");
            let mut socket = accept_async(stream).await.expect("handshake serveur");
            let hello = next_client_message(&mut socket).await;
            assert!(matches!(hello, Some(ClientMessage::Hello(_))));
            send_server_message(
                &mut socket,
                "srv-welcome",
                ServerMessage::Welcome(welcome_payload()),
            )
            .await;
            send_server_message(
                &mut socket,
                "srv-ack",
                ServerMessage::Ack(AckPayload {
                    ref_id: "cli-not-expected".to_string(),
                    ok: true,
                    error: None,
                    outcome: None,
                    position: None,
                    job_id: None,
                }),
            )
            .await;
            socket
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "bye".into(),
                })))
                .await
                .expect("close serveur");
            let _closed = socket.close(None).await;
        });

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
        let session = SessionConfig {
            ws_url: ws_url_from_server_url_with(&format!("http://{address}"), true)
                .expect("url WebSocket de test"),
            hello: HelloPayload {
                token: "token-test".to_string(),
                app_version: "3.0.0".to_string(),
                client_id: Some("client-1".to_string()),
                session_mode: Some(SessionMode::Normal),
            },
            insecure_tls: false,
            queued_keepalive_interval: QUEUED_KEEPALIVE_INTERVAL,
        };
        let effects = RecordingEffects::default();
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_session(&effects, &session, &mut receiver),
        )
        .await
        .expect("session non bloquée");
        match outcome {
            SessionOutcome::Closed { code, .. } => assert_eq!(code, Some(1000)),
            other => panic!("fin de session inattendue : {other:?}"),
        }
        assert_eq!(*effects.acks.lock().expect("acks"), 1);
        drop(sender);
        server.await.expect("serveur mock");
    }

    #[test]
    fn close_codes_map_to_expected_actions() {
        assert!(matches!(
            close_step(Some(4401), ""),
            SupervisorStep::Terminal {
                purge: Some(GuildRemovalReason::InvalidToken),
                ..
            }
        ));
        assert!(matches!(
            close_step(Some(4402), "revoked"),
            SupervisorStep::Terminal {
                status: ConnectionStatus::Revoked,
                purge: Some(GuildRemovalReason::Revoked),
                ..
            }
        ));
        assert!(matches!(
            close_step(Some(4402), "member_removed"),
            SupervisorStep::Terminal {
                status: ConnectionStatus::Revoked,
                purge: Some(GuildRemovalReason::MemberRemoved),
                ..
            }
        ));
        assert!(matches!(
            close_step(Some(4403), "{\"minVersion\":\"3.1.0\"}"),
            SupervisorStep::Terminal {
                status: ConnectionStatus::Outdated,
                ..
            }
        ));
        assert!(matches!(
            close_step(Some(4404), "guild_deleted"),
            SupervisorStep::Terminal {
                status: ConnectionStatus::Revoked,
                purge: Some(GuildRemovalReason::GuildDeleted),
                reason,
            } if reason.contains("guilde supprimée")
        ));
        assert!(matches!(
            close_step(Some(4409), "rate_limited"),
            SupervisorStep::Retry {
                rate_limited: true,
                ..
            }
        ));
        assert!(matches!(
            close_step(None, "reset"),
            SupervisorStep::Retry {
                rate_limited: false,
                ..
            }
        ));
        assert!(matches!(
            close_step(Some(1000), ""),
            SupervisorStep::Terminal { purge: None, .. }
        ));
    }

    fn identity_membership() -> Membership {
        Membership {
            server_url: "https://example.test".to_string(),
            guild_id: "guild-1".to_string(),
            guild_name: Some("Ancienne guilde".to_string()),
            guild_icon_url: Some("https://cdn.discordapp.com/icons/old.png".to_string()),
            client_token: "token-test".to_string(),
            client_id: Some("client-1".to_string()),
            member_id: None,
            display_name: None,
            role: None,
            guest_mode: false,
            muted: false,
            volume: 1.0,
            media_bindings: Default::default(),
        }
    }

    #[test]
    fn identity_from_welcome_updates_name_and_icon() {
        let mut welcome = welcome_payload();
        welcome.guild_name = Some("  Nouvelle guilde  ".to_string());
        welcome.guild_icon_url = Some("https://cdn.discordapp.com/icons/new.png".to_string());
        let update = identity_from_welcome(&identity_membership(), &welcome).expect("update");
        assert_eq!(update.client_id, "client-1");
        assert_eq!(update.guild_name.as_deref(), Some("Nouvelle guilde"));
        assert_eq!(
            update.guild_icon_url,
            Some(Some("https://cdn.discordapp.com/icons/new.png".to_string()))
        );
    }

    #[test]
    fn identity_from_welcome_clears_a_modern_iconless_guild_icon() {
        let mut welcome = welcome_payload();
        welcome.guild_name = Some("Ancienne guilde".to_string());
        welcome.guild_icon_url = None;
        let update = identity_from_welcome(&identity_membership(), &welcome).expect("update");
        assert_eq!(update.guild_name, None);
        assert_eq!(update.guild_icon_url, Some(None));
    }

    #[test]
    fn identity_from_welcome_keeps_icon_for_a_legacy_welcome() {
        let membership = identity_membership();
        let welcome = welcome_payload();
        assert_eq!(identity_from_welcome(&membership, &welcome), None);

        let mut welcome = welcome_payload();
        welcome.client_id = "client-2".to_string();
        let update = identity_from_welcome(&membership, &welcome).expect("update");
        assert_eq!(update.client_id, "client-2");
        assert_eq!(update.guild_icon_url, None);
    }

    #[test]
    fn identity_from_welcome_treats_untrusted_icons_as_absent() {
        let mut welcome = welcome_payload();
        welcome.guild_name = Some("Ancienne guilde".to_string());
        welcome.guild_icon_url = Some("https://evil.test/icons/old.png".to_string());
        let update = identity_from_welcome(&identity_membership(), &welcome).expect("update");
        assert_eq!(update.guild_icon_url, Some(None));
    }

    #[test]
    fn identity_from_welcome_returns_none_when_nothing_changed() {
        let mut welcome = welcome_payload();
        welcome.guild_name = Some("Ancienne guilde".to_string());
        welcome.guild_icon_url = Some("https://cdn.discordapp.com/icons/old.png".to_string());
        assert_eq!(
            identity_from_welcome(&identity_membership(), &welcome),
            None
        );
    }

    fn test_send_with_ack() -> NetCommand {
        let (reply, _receiver) = oneshot::channel();
        NetCommand::SendWithAck {
            message: ClientMessage::PeersRequest(lc_protocol::PeersRequestPayload {}),
            reply,
        }
    }

    #[test]
    fn send_with_ack_is_deliverable_only_when_connected() {
        assert!(is_deliverable(
            ConnectionStatus::Connected,
            &test_send_with_ack()
        ));
        assert!(!is_deliverable(
            ConnectionStatus::Reconnecting,
            &test_send_with_ack()
        ));
        assert!(!is_deliverable(
            ConnectionStatus::Disabled,
            &test_send_with_ack()
        ));
    }

    #[test]
    fn backoff_is_one_second_then_three_second_cap() {
        let mut backoff = Backoff::default();
        assert_eq!(backoff.delay(false), Duration::from_secs(1));
        assert_eq!(backoff.delay(false), Duration::from_secs(2));
        assert_eq!(backoff.delay(false), Duration::from_secs(3));
        assert_eq!(backoff.delay(false), Duration::from_secs(3));
        backoff.reset();
        assert_eq!(backoff.delay(true), Duration::from_secs(1));
        assert_eq!(backoff.delay(true), Duration::from_secs(2));
        assert_eq!(backoff.delay(true), Duration::from_secs(4));
    }

    #[test]
    fn ws_url_conversion_matches_v2_contract() {
        assert!(
            ws_url_from_server_url("http://127.0.0.1:3000/").is_err(),
            "http:// refusé par défaut (AUD-A6-003)"
        );
        assert_eq!(
            ws_url_from_server_url_with("http://127.0.0.1:3000/", true)
                .expect("dérogation dev")
                .as_str(),
            "ws://127.0.0.1:3000/ws"
        );
        assert_eq!(
            ws_url_from_server_url("https://api.livechat.example/base")
                .expect("https")
                .as_str(),
            "wss://api.livechat.example/ws"
        );
        assert!(ws_url_from_server_url("ftp://example").is_err());
        assert!(ws_url_from_server_url("  ").is_err());
    }

    #[test]
    fn adopt_current_job_adds_elapsed_offset() {
        let mut play = play_payload();
        if let Some(media) = play.media.as_mut() {
            media.start_offset_sec = Some(2);
        }
        // duration 10 s, remaining 4 s -> elapsed 6 s, seek = 2 + 6 = 8 s.
        let adopted = adopt_current_job(play, 4_000);
        let media = adopted.media.expect("média conservé");
        assert_eq!(media.start_offset_sec, Some(8));
        assert_eq!(adopted.job_id, "job-1");
    }

    #[test]
    fn outdated_reason_extracts_min_version() {
        assert!(outdated_reason("{\"minVersion\":\"3.1.0\"}").contains("3.1.0"));
        assert!(outdated_reason("n'importe quoi").contains("mise à jour"));
    }

    #[test]
    fn outbound_envelope_is_flat_v2() {
        let envelope = ClientEnvelope::new(
            "cli-test",
            42,
            ClientMessage::PeersRequest(lc_protocol::PeersRequestPayload {}),
        );
        let json = serde_json::to_value(&envelope).expect("json");
        assert_eq!(json["v"], 2);
        assert_eq!(json["type"], "peers.request");
        assert_eq!(json["id"], "cli-test");
        assert_eq!(json["ts"], 42);
        // Le payload reste un objet (éventuellement vide) : le contrat v2 exige
        // un objet et tolère les champs vides.
        assert!(json.get("payload").is_none_or(serde_json::Value::is_object));
    }

    fn test_send() -> NetCommand {
        NetCommand::Send(ClientMessage::PeersRequest(
            lc_protocol::PeersRequestPayload {},
        ))
    }

    #[test]
    fn send_is_deliverable_when_connected() {
        assert!(is_deliverable(ConnectionStatus::Connected, &test_send()));
    }

    #[test]
    fn send_is_not_deliverable_when_disabled() {
        assert!(!is_deliverable(ConnectionStatus::Disabled, &test_send()));
    }

    #[test]
    fn send_is_not_deliverable_when_reconnecting() {
        assert!(!is_deliverable(
            ConnectionStatus::Reconnecting,
            &test_send()
        ));
    }

    #[test]
    fn reconnect_is_deliverable_when_disabled() {
        assert!(is_deliverable(
            ConnectionStatus::Disabled,
            &NetCommand::Reconnect
        ));
    }

    #[test]
    fn shutdown_is_deliverable_when_disabled() {
        assert!(is_deliverable(
            ConnectionStatus::Disabled,
            &NetCommand::Shutdown
        ));
    }

    #[test]
    fn effective_volume_multiplies_general_and_guild_volumes() {
        assert!((effective_volume(0.8, 0.5) - 0.4).abs() < f64::EPSILON);
        assert_eq!(effective_volume(2.0, 3.0), 1.0);
        assert_eq!(effective_volume(-1.0, 0.5), 0.0);
        assert_eq!(effective_volume(0.8, 1.0), 0.8);
    }

    #[test]
    fn renderer_play_payload_carries_the_effective_volume() {
        let play = play_payload();
        let value = renderer_play_payload(&play, 0.8, 0.5).expect("serialize");
        assert_eq!(value["jobId"], play.job_id);
        assert!((value["volume"].as_f64().expect("volume") - 0.4).abs() < f64::EPSILON);
    }
}
