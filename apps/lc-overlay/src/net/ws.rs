//! Client WebSocket v2 de l'overlay.
//!
//! Dialogue avec `{serverUrl}/ws` selon `docs/rust/protocol-v2.md` :
//! handshake `hello`/`welcome`, heartbeat applicatif toutes les 15 s,
//! réception `play`/`stop`/`settings`/`peers`/`ack`/`error`, envoi
//! `playback.state`, `playback.ended`, `stop.request`, `meme.trigger`,
//! `peers.request`, reconnexion avec backoff (1 s → 3 s, 4409 plafonné à 30 s),
//! et gestion terminale des closes 4401 (jeton invalide → réappairage),
//! 4402 (révoqué) et 4403 (overlay obsolète).
//!
//! La logique de session ([`run_session`]) est générique sur [`SessionEffects`]
//! afin d'être testable contre un serveur WebSocket mock ; l'implémentation
//! Tauri ([`AppEffects`]) branche les événements sur le renderer et le tray.

use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use lc_protocol::{
    AckPayload, ClientEnvelope, ClientMessage, HeartbeatPayload, HelloPayload, PeersPayload,
    PlayPayload, ServerEnvelope, ServerErrorPayload, ServerMessage, SessionMode, SettingsPayload,
    StopPayload, WelcomePayload,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, Wry};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async_tls_with_config, Connector};
use url::Url;
use uuid::Uuid;

use crate::config::{OverlayConfig, OverlayConfigView};
use crate::events::{self, OverlayEvent};
use crate::state::AppState;
use crate::tray;
use crate::windows;

/// Délai maximal d'établissement de la connexion TCP/TLS/WebSocket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Cadence du heartbeat applicatif (contrat v2 : 15 s).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
/// Variable d'environnement legacy : accepter un certificat TLS non vérifié.
const INSECURE_TLS_ENV: &str = "LIVECHAT_INSECURE_TLS";

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
    Reconnect,
    Shutdown,
}

/// Session prête à être exécutée (construite depuis la config persistée).
struct SessionConfig {
    ws_url: Url,
    hello: HelloPayload,
    insecure_tls: bool,
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
        purge: bool,
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

/// Démarre la tâche réseau (une seule fois, depuis `setup`).
pub fn start(app: &AppHandle<Wry>) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<NetCommand>();
    {
        let state = app.state::<AppState>();
        state.set_net_command_sender(sender);
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        supervise(handle, receiver).await;
        tracing::info!("superviseur WebSocket arrêté");
    });
}

/// Demande une reconnexion (appairage terminé, config modifiée, tray…).
pub fn request_reconnect(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    if !state.send_net_command(NetCommand::Reconnect) {
        tracing::debug!("superviseur WebSocket non démarré : reconnexion ignorée");
    }
}

/// Envoie une trame au serveur si la connexion est établie.
pub fn send_command(app: &AppHandle<Wry>, command: NetCommand) -> bool {
    app.state::<AppState>().send_net_command(command)
}

/// Ferme proprement la tâche réseau (sortie de l'application).
pub fn request_shutdown(app: &AppHandle<Wry>) {
    let sent = send_command(app, NetCommand::Shutdown);
    tracing::debug!(sent, "arrêt demandé au superviseur WebSocket");
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
}

async fn supervise(app: AppHandle<Wry>, mut commands: UnboundedReceiver<NetCommand>) {
    let mut backoff = Backoff::default();
    loop {
        let config = app.state::<AppState>().config_snapshot();
        if !config.enabled {
            publish(
                &app,
                ConnectionStatus::Disabled,
                Some("overlay désactivé".to_string()),
            );
            if !wait_for_command(&mut commands, &app).await {
                return;
            }
            continue;
        }
        if !config.has_pairing_config() {
            publish(
                &app,
                ConnectionStatus::NotPaired,
                Some("appairage requis".to_string()),
            );
            if !wait_for_command(&mut commands, &app).await {
                return;
            }
            continue;
        }
        let session = match build_session_config(&config) {
            Ok(session) => session,
            Err(error) => {
                publish(&app, ConnectionStatus::Error, Some(error));
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
        publish(&app, status, None);
        let effects = AppEffects { app: app.clone() };
        let outcome = run_session(&effects, &session, &mut commands).await;
        let step = match outcome {
            SessionOutcome::Shutdown => return,
            SessionOutcome::ReconnectRequested => {
                backoff.reset();
                publish(
                    &app,
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
                    "nouvelle tentative de connexion WebSocket"
                );
                publish(&app, ConnectionStatus::Reconnecting, Some(reason));
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    command = commands.recv() => match command {
                        None | Some(NetCommand::Shutdown) => return,
                        Some(NetCommand::Reconnect) => backoff.reset(),
                        Some(NetCommand::Send(_)) => {
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
                if purge {
                    purge_pairing(&app);
                    windows::show_pairing(&app);
                }
                publish(&app, status, Some(reason));
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
            Some(NetCommand::Send(_)) => {
                tracing::debug!("trame ignorée : aucune connexion WebSocket active");
            }
        }
    }
}

fn publish(app: &AppHandle<Wry>, status: ConnectionStatus, reason: Option<String>) {
    app.state::<AppState>().set_net_status(status, reason);
    refresh_tray_from_any_thread(app);
}

fn purge_pairing(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    if let Err(error) = state.update_config(|config| {
        config.client_token = None;
        config.client_id = None;
        config.guest_mode = false;
    }) {
        tracing::warn!(%error, "échec de purge du jeton d'appairage");
    }
    crate::shortcuts::reapply_for_session(app);
    if let Err(error) = events::emit_settings(app, &state.view()) {
        tracing::debug!(%error, "échec d'émission des settings après purge");
    }
}

fn close_step(code: Option<u16>, reason: &str) -> SupervisorStep {
    match code {
        Some(1000) => SupervisorStep::Terminal {
            status: ConnectionStatus::Error,
            reason: "connexion fermée par le serveur".to_string(),
            purge: false,
        },
        Some(1003) => SupervisorStep::Terminal {
            status: ConnectionStatus::Error,
            reason: "trame non supportée par le serveur".to_string(),
            purge: false,
        },
        Some(4401) => SupervisorStep::Terminal {
            status: ConnectionStatus::Error,
            reason: "jeton invalide — réappairage nécessaire".to_string(),
            purge: true,
        },
        Some(4402) => SupervisorStep::Terminal {
            status: ConnectionStatus::Revoked,
            reason: "appareil révoqué — réappairage nécessaire".to_string(),
            purge: true,
        },
        Some(4403) => SupervisorStep::Terminal {
            status: ConnectionStatus::Outdated,
            reason: outdated_reason(reason),
            purge: false,
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

fn build_session_config(config: &OverlayConfig) -> Result<SessionConfig, String> {
    let server_url = config
        .server_url
        .as_deref()
        .ok_or_else(|| "serverUrl absent de la configuration".to_string())?;
    let token = config
        .client_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "clientToken absent de la configuration".to_string())?;
    let ws_url = ws_url_from_server_url(server_url)?;
    let session_mode = if config.guest_mode {
        SessionMode::InviteReadOnly
    } else {
        SessionMode::Normal
    };
    Ok(SessionConfig {
        ws_url,
        hello: HelloPayload {
            token: token.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            client_id: config.client_id.clone(),
            session_mode: Some(session_mode),
        },
        insecure_tls: insecure_tls_enabled(),
    })
}

/// Convertit `{API_URL}` en URL WebSocket `ws(s)://…/ws`.
fn ws_url_from_server_url(server_url: &str) -> Result<Url, String> {
    let trimmed = server_url.trim();
    if trimmed.is_empty() {
        return Err("serverUrl vide".to_string());
    }
    let mut url = Url::parse(trimmed).map_err(|error| format!("serverUrl invalide : {error}"))?;
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
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

fn insecure_tls_enabled() -> bool {
    match std::env::var(INSECURE_TLS_ENV) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
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

/// Envoie une trame cliente enveloppée (id/timestamp générés).
async fn send_message<S>(sink: &mut S, message: ClientMessage) -> Result<(), String>
where
    S: futures_util::Sink<Message, Error = tungstenite::Error> + Unpin,
{
    let envelope = ClientEnvelope::new(format!("cli-{}", Uuid::new_v4()), now_ms(), message);
    let text = serde_json::to_string(&envelope)
        .map_err(|error| format!("sérialisation de trame impossible : {error}"))?;
    sink.send(Message::Text(text.into()))
        .await
        .map_err(|error| error.to_string())
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
                                    ServerMessage::Ack(payload) => effects.on_ack(&payload),
                                    ServerMessage::Error(payload) => effects.on_server_error(&payload),
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

/// Effets Tauri : dispatcher vers le renderer, le tray et l'état partagé.
struct AppEffects {
    app: AppHandle<Wry>,
}

impl SessionEffects for AppEffects {
    fn on_welcome(&self, payload: &WelcomePayload) {
        let app = &self.app;
        let state = app.state::<AppState>();
        let guest_mode = payload.session_mode == SessionMode::InviteReadOnly;
        let config = state.config_snapshot();
        let identity_changed = config.client_id.as_deref() != Some(payload.client_id.as_str())
            || config.guild_id.as_deref() != Some(payload.guild_id.as_str())
            || config.guest_mode != guest_mode;
        if identity_changed {
            if let Err(error) = state.update_config(|config| {
                config.client_id = Some(payload.client_id.clone());
                config.guild_id = Some(payload.guild_id.clone());
                config.guest_mode = guest_mode;
            }) {
                tracing::warn!(%error, "échec de persistance de l'identité serveur");
            }
            crate::shortcuts::reapply_for_session(app);
        }
        state.set_net_welcome(payload);
        tracing::info!(
            server_version = %payload.server_version,
            client_id = %payload.client_id,
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
            match serde_json::to_value(&adopted) {
                Ok(value) => {
                    state.set_playback(adopted.job_id);
                    events::deliver_or_buffer(
                        app,
                        state.overlay_ready(),
                        OverlayEvent::play(value),
                    );
                }
                Err(error) => tracing::warn!(%error, "currentJob non sérialisable"),
            }
        }
    }

    fn on_settings(&self, payload: &SettingsPayload) {
        let state = self.app.state::<AppState>();
        state.set_net_settings_summary(&payload.guild_id, payload.session_mode);
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
        let app = &self.app;
        let state = app.state::<AppState>();
        state.set_playback(payload.job_id.clone());
        match serde_json::to_value(payload) {
            Ok(value) => {
                tracing::info!(job_id = %payload.job_id, "play reçu");
                events::deliver_or_buffer(app, state.overlay_ready(), OverlayEvent::play(value));
            }
            Err(error) => tracing::warn!(%error, "play non sérialisable"),
        }
    }

    fn on_stop(&self, payload: &StopPayload) {
        let app = &self.app;
        let state = app.state::<AppState>();
        state.clear_playback();
        match serde_json::to_value(payload) {
            Ok(value) => {
                tracing::info!(reason = ?payload.reason, "stop reçu");
                events::deliver_or_buffer(app, state.overlay_ready(), OverlayEvent::stop(value));
            }
            Err(error) => tracing::warn!(%error, "stop non sérialisable"),
        }
    }

    fn on_peers(&self, payload: &PeersPayload) {
        let state = self.app.state::<AppState>();
        let labels = payload
            .peers
            .iter()
            .map(|peer| peer.label.clone())
            .collect::<Vec<_>>();
        state.set_net_peers(labels);
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
            self.app.state::<AppState>().set_net_error(error);
            refresh_tray_from_any_thread(&self.app);
        }
    }

    fn on_server_error(&self, payload: &ServerErrorPayload) {
        let message = format!("{}: {}", payload.code, payload.message);
        tracing::warn!(%message, job_id = ?payload.job_id, "erreur serveur non fatale");
        self.app.state::<AppState>().set_net_error(message);
        refresh_tray_from_any_thread(&self.app);
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

    #[derive(Default)]
    struct RecordingEffects {
        welcome: StdMutex<u32>,
        plays: StdMutex<Vec<String>>,
        stops: StdMutex<u32>,
        peers: StdMutex<u32>,
        settings: StdMutex<u32>,
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

        fn on_ack(&self, _payload: &AckPayload) {}

        fn on_server_error(&self, _payload: &ServerErrorPayload) {}
    }

    fn welcome_payload() -> WelcomePayload {
        WelcomePayload {
            protocol_version: 2,
            guild_id: "guild-1".to_string(),
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
        }
    }

    async fn next_client_message(
        socket: &mut WebSocketStream<tokio::net::TcpStream>,
    ) -> Option<ClientMessage> {
        loop {
            let message = socket.next().await?.ok()?;
            match message {
                Message::Text(text) => {
                    return serde_json::from_str::<ClientEnvelope>(text.as_str())
                        .ok()
                        .map(|envelope| envelope.message);
                }
                Message::Close(_) => return None,
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
            }
        }
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
            ws_url: ws_url_from_server_url(&format!("http://{address}"))
                .expect("url WebSocket de test"),
            hello: HelloPayload {
                token: "token-test".to_string(),
                app_version: "3.0.0".to_string(),
                client_id: Some("client-1".to_string()),
                session_mode: Some(SessionMode::Normal),
            },
            insecure_tls: false,
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

    #[test]
    fn close_codes_map_to_expected_actions() {
        assert!(matches!(
            close_step(Some(4401), ""),
            SupervisorStep::Terminal { purge: true, .. }
        ));
        assert!(matches!(
            close_step(Some(4402), ""),
            SupervisorStep::Terminal {
                status: ConnectionStatus::Revoked,
                purge: true,
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
            SupervisorStep::Terminal { .. }
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
        assert_eq!(
            ws_url_from_server_url("http://127.0.0.1:3000/")
                .expect("http")
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
}
