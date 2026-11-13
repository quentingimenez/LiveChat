//! Session WebSocket v2 : handshake `hello`/`welcome`, boucle de lecture et
//! interprétation des messages client.
//!
//! Chaque connexion `GET /ws` suit le cycle de vie décrit par
//! `docs/rust/protocol-v2.md` :
//!
//! 1. le client doit envoyer `hello` sous 5 s, sinon `4401 auth_timeout` ;
//! 2. le serveur authentifie le token (SHA-256 comparé au `token_hash`),
//!    vérifie la version de protocole et la version minimale de l'overlay,
//!    puis répond `welcome` + `settings` et diffuse `peers` ;
//! 3. la boucle applique les limites (64 Kio, 20 trames/s), route les messages
//!    vers `lc-playback` et relaie les trames sortantes du hub.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::http::{header::ORIGIN, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use lc_db::Db;
use lc_domain::{
    now_ms, GuildId, JobStatus, MediaBoardItemId, OverlayClientId, PlaybackJob, PlaybackJobId,
};
use lc_playback::{
    PlaybackEndedEvent, PlaybackStateEvent, PlaybackStateOutcome, MEDIA_BOARD_JOB_PRIORITY,
};
use lc_protocol::{
    close_code, close_reason, is_version_below, outdated_protocol_reason, AckError, AckPayload,
    ClientEnvelope, ClientMessage, HelloPayload, PlaybackEndedPayload, PlaybackStatePayload,
    ServerEnvelope, ServerErrorPayload, ServerMessage, SessionMode, SettingsPayload,
    StopRequestSource, WelcomeConfig, WelcomePayload, HELLO_TIMEOUT_MS,
    MAX_CLIENT_MESSAGES_PER_SECOND, MAX_FRAME_BYTES, MAX_MEDIA_TEXT_CHARS,
    MAX_PLAYBACK_REMAINING_MS, PEERS_REQUEST_MIN_INTERVAL_MS, PING_INTERVAL_MS,
    PLAYBACK_STATE_MIN_INTERVAL_MS, PROTOCOL_VERSION,
};
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, MissedTickBehavior};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::hub::{self, AuthError, Hub, HubEngine, OutFrame, TOO_MANY_CONNECTIONS_REASON};

/// Borne dure de transport de l'upgrade : volontairement supérieure à la
/// limite applicative (64 Kio) afin de pouvoir répondre `4408` aux trames
/// intermédiaires au lieu de laisser axum couper en 1006/1009 (P2, AUD-COR-12).
const MAX_TRANSPORT_MESSAGE_BYTES: usize = 1024 * 1024;

/// Longueur maximale de l'`id` d'une trame client (protocole §4).
const MAX_REQUEST_ID_CHARS: usize = 64;

/// Fenêtre glissante du rate limiter client.
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);

/// Code close `1008` (policy violation) pour une origine refusée.
///
/// Le close est utilisé plutôt qu'une réponse HTTP 403 car la requête est déjà
/// un upgrade WebSocket ; la raison documentée est `origin_not_allowed`.
const POLICY_VIOLATION: u16 = 1008;

/// Raison de fermeture quand l'origine WebSocket n'est pas autorisée.
const ORIGIN_NOT_ALLOWED_REASON: &str = "origin_not_allowed";

/// Variable d'environnement d'allowlist d'origines WebSocket (séparées par `,`).
const WS_ALLOWED_ORIGINS_ENV: &str = "WS_ALLOWED_ORIGINS";

/// Raison de fermeture décidée par la session.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CloseInfo {
    code: u16,
    reason: String,
}

impl CloseInfo {
    fn new(code: u16, reason: &str) -> Self {
        Self {
            code,
            reason: reason.to_owned(),
        }
    }
}

/// Contexte immuable d'une session authentifiée.
struct SessionContext {
    guild_id: GuildId,
    client_id: OverlayClientId,
    client_id_str: String,
    session_mode: SessionMode,
    author_name: Option<String>,
    author_image: Option<String>,
}

/// Horodatages des derniers messages soumis à throttle (par connexion).
#[derive(Default)]
struct SessionTimers {
    last_playback_state: Option<Instant>,
    last_stop_request: Option<Instant>,
    last_peers_request: Option<Instant>,
}

/// Handler axum de `GET /ws`.
pub(crate) async fn ws_handler(
    State(state): State<crate::app::AppState>,
    headers: HeaderMap,
    Extension(hub): Extension<Arc<Hub>>,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let origin = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    match ws {
        Ok(upgrade) => upgrade
            .max_message_size(MAX_TRANSPORT_MESSAGE_BYTES)
            .max_frame_size(MAX_TRANSPORT_MESSAGE_BYTES)
            .on_upgrade(move |socket| handle_socket(socket, state, hub, origin)),
        Err(_rejection) => {
            debug!("upgrade WebSocket refusé : 426 websocket_required");
            (
                StatusCode::UPGRADE_REQUIRED,
                Json(serde_json::json!({ "error": "websocket_required" })),
            )
                .into_response()
        }
    }
}

/// Boucle de vie d'une connexion WebSocket.
async fn handle_socket(
    socket: WebSocket,
    state: crate::app::AppState,
    hub: Arc<Hub>,
    origin: Option<String>,
) {
    let (mut sink, mut stream) = socket.split();

    if !origin_allowed(origin.as_deref(), &state.config) {
        debug!(
            ?origin,
            "origine WebSocket refusée : 1008 origin_not_allowed"
        );
        let info = CloseInfo::new(POLICY_VIOLATION, ORIGIN_NOT_ALLOWED_REASON);
        close_sink(&mut sink, &info).await;
        return;
    }

    let hello = match wait_for_hello(&mut stream, state.config.min_overlay_version.as_str()).await {
        Ok(hello) => hello,
        Err(info) => {
            close_sink(&mut sink, &info).await;
            return;
        }
    };

    let Some(db) = state.db.clone() else {
        let info = CloseInfo::new(close_code::INTERNAL_ERROR, close_reason::INTERNAL_ERROR);
        warn!("connexion WebSocket sans base de données : fermeture 1011");
        close_sink(&mut sink, &info).await;
        return;
    };

    let client = match hub::authenticate(&db, hello.token.as_str()).await {
        Ok(client) => client,
        Err(error) => {
            if let AuthError::Internal(detail) = &error {
                warn!(error = %detail, "erreur interne pendant l'authentification WebSocket");
            }
            let info = CloseInfo::new(error.code(), error.reason());
            close_sink(&mut sink, &info).await;
            return;
        }
    };

    let client_id_str = client.id.to_canonical();
    if let Some(provided) = hello.client_id.as_deref() {
        let provided = provided.trim();
        if !provided.is_empty() && provided != client_id_str {
            let info = CloseInfo::new(close_code::UNAUTHORIZED, close_reason::INVALID_TOKEN);
            close_sink(&mut sink, &info).await;
            return;
        }
    }

    if is_version_below(
        hello.app_version.as_str(),
        state.config.min_overlay_version.as_str(),
    ) {
        let info = CloseInfo::new(
            close_code::OUTDATED_PROTOCOL,
            outdated_protocol_reason(state.config.min_overlay_version.as_str()).as_str(),
        );
        close_sink(&mut sink, &info).await;
        return;
    }

    let engine = match hub.ensure_runtime(&state).await {
        Ok(engine) => engine,
        Err(error) => {
            warn!(error = %error, "moteur de lecture indisponible pour le hub WebSocket");
            let info = CloseInfo::new(close_code::INTERNAL_ERROR, close_reason::INTERNAL_ERROR);
            close_sink(&mut sink, &info).await;
            return;
        }
    };

    let guild_id = client.guild_id.clone();
    let session_mode = hub::protocol_session_mode(client.session_mode);
    let context = SessionContext {
        guild_id: guild_id.clone(),
        client_id: client.id,
        client_id_str: client_id_str.clone(),
        session_mode,
        author_name: client.default_author_name.clone(),
        author_image: client.default_author_image.clone(),
    };

    let (sender, mut receiver) = mpsc::unbounded_channel::<OutFrame>();
    let connection_id = match hub.register(
        guild_id.clone(),
        client.id,
        client_id_str,
        client.label,
        session_mode,
        context.author_name.clone(),
        context.author_image.clone(),
        sender,
    ) {
        Ok(connection_id) => connection_id,
        Err(error) => {
            debug!(
                ?error,
                "connexion WebSocket refusée : plafond de sockets atteint"
            );
            let info = CloseInfo::new(close_code::RATE_LIMITED, TOO_MANY_CONNECTIONS_REASON);
            close_sink(&mut sink, &info).await;
            return;
        }
    };

    if !send_welcome(&mut sink, &hub, &engine, &context).await {
        hub.unregister(connection_id);
        return;
    }

    hub.broadcast_peers(&guild_id);
    info!(client = %context.client_id_str, guild = %guild_id, "overlay connecté au hub WebSocket");

    let mut timers = SessionTimers::default();
    let max_messages = usize::try_from(MAX_CLIENT_MESSAGES_PER_SECOND).unwrap_or(usize::MAX);
    let mut rate_window: VecDeque<Instant> =
        VecDeque::with_capacity(max_messages.saturating_add(1));
    let heartbeat_timeout = Duration::from_secs(state.config.ws_heartbeat_timeout_sec.max(1));
    let mut liveness = PongLiveness::new(heartbeat_timeout);
    let mut ping_interval = interval(Duration::from_millis(PING_INTERVAL_MS));
    ping_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ping_interval.tick().await;

    let mut close_info: Option<CloseInfo> = None;
    loop {
        tokio::select! {
            maybe_message = stream.next() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        let raw = text.as_str();
                        if raw.len() > MAX_FRAME_BYTES {
                            close_info = Some(CloseInfo::new(
                                close_code::FRAME_TOO_LARGE,
                                close_reason::FRAME_TOO_LARGE,
                            ));
                            break;
                        }
                        let now = Instant::now();
                        while rate_window
                            .front()
                            .is_some_and(|previous| now.duration_since(*previous) > RATE_LIMIT_WINDOW)
                        {
                            rate_window.pop_front();
                        }
                        rate_window.push_back(now);
                        if rate_window.len() > max_messages {
                            close_info = Some(CloseInfo::new(
                                close_code::RATE_LIMITED,
                                close_reason::RATE_LIMITED,
                            ));
                            break;
                        }
                        let envelope: ClientEnvelope = match serde_json::from_str(raw) {
                            Ok(envelope) => envelope,
                            Err(_) => {
                                close_info = Some(CloseInfo::new(
                                    close_code::PROTOCOL_ERROR,
                                    close_reason::PROTOCOL_ERROR,
                                ));
                                break;
                            }
                        };
                        if envelope.v != PROTOCOL_VERSION {
                            close_info = Some(CloseInfo::new(
                                close_code::OUTDATED_PROTOCOL,
                                outdated_protocol_reason(
                                    state.config.min_overlay_version.as_str(),
                                )
                                .as_str(),
                            ));
                            break;
                        }
                        let request_id = envelope.id;
                        if let Err(info) = validate_request_id(request_id.as_str()) {
                            close_info = Some(info);
                            break;
                        }
                        if let Err(info) = handle_client_message(
                            &state,
                            &hub,
                            &db,
                            &engine,
                            &context,
                            connection_id,
                            request_id,
                            envelope.message,
                            &mut timers,
                        )
                        .await
                        {
                            close_info = Some(info);
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        close_info = Some(CloseInfo::new(
                            close_code::UNSUPPORTED_FRAME,
                            close_reason::UNSUPPORTED_FRAME,
                        ));
                        break;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Pong(_))) => {
                        liveness.on_pong();
                    }
                    Some(Ok(Message::Ping(_))) => {}
                    Some(Err(error)) => {
                        debug!(error = %error, "lecture du socket WebSocket interrompue");
                        break;
                    }
                }
            }
            maybe_frame = receiver.recv() => {
                match maybe_frame {
                    Some(OutFrame::Text(text)) => {
                        if sink.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(OutFrame::Close(code, reason)) => {
                        close_info = Some(CloseInfo { code, reason });
                        break;
                    }
                    None => break,
                }
            }
            _ = ping_interval.tick() => {
                if liveness.is_expired() {
                    close_info = Some(CloseInfo::new(
                        close_code::GOING_AWAY,
                        close_reason::HEARTBEAT_TIMEOUT,
                    ));
                    break;
                }
                if sink.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
            _ = state.shutdown.subscribe().wait() => {
                close_info = Some(CloseInfo::new(
                    close_code::GOING_AWAY,
                    close_reason::SERVER_SHUTDOWN,
                ));
                break;
            }
        }
    }

    if let Some(info) = close_info {
        close_sink(&mut sink, &info).await;
    }
    hub.unregister(connection_id);
    hub.broadcast_peers(&guild_id);
    info!(client = %context.client_id_str, guild = %guild_id, "overlay déconnecté du hub WebSocket");
}

/// Vérifie qu'une origine WebSocket est autorisée (plafonds/anti-DoS COR-06).
///
/// L'absence d'en-tête `Origin` désigne un client natif (overlay Tauri) et est
/// acceptée. Sinon, l'origine doit appartenir à l'allowlist : origine de
/// `API_URL`, `tauri.localhost` (http et https), `ADDITIONAL_CORS_ORIGINS`,
/// variable `WS_ALLOWED_ORIGINS`, et loopback uniquement en développement.
fn origin_allowed(origin: Option<&str>, config: &crate::config::Config) -> bool {
    let Some(raw) = origin else {
        return true;
    };
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if !parsed.origin().is_tuple() {
        // `null` et origines opaques (`file:`, `data:`…) refusées.
        return false;
    }
    if config.is_development() && is_loopback_origin(&parsed) {
        return true;
    }
    let serialized = parsed.origin().ascii_serialization();
    let mut allowed = vec![
        config.api_origin(),
        "http://tauri.localhost".to_owned(),
        "https://tauri.localhost".to_owned(),
    ];
    allowed.extend(
        config
            .additional_cors_origins
            .iter()
            .map(|origin| origin.origin().ascii_serialization()),
    );
    if let Ok(extra) = std::env::var(WS_ALLOWED_ORIGINS_ENV) {
        for item in extra.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            match url::Url::parse(item) {
                Ok(url) if url.origin().is_tuple() => {
                    allowed.push(url.origin().ascii_serialization());
                }
                _ => warn!(value = %item, "origine invalide dans {WS_ALLOWED_ORIGINS_ENV}"),
            }
        }
    }
    allowed.iter().any(|candidate| candidate == &serialized)
}

/// Vrai si l'origine pointe vers la boucle locale.
fn is_loopback_origin(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

/// Attend la trame `hello` (5 s) et applique les validations amont.
async fn wait_for_hello(
    stream: &mut SplitStream<WebSocket>,
    min_overlay_version: &str,
) -> Result<HelloPayload, CloseInfo> {
    let deadline = sleep(Duration::from_millis(HELLO_TIMEOUT_MS));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => {
                return Err(CloseInfo::new(
                    close_code::UNAUTHORIZED,
                    close_reason::AUTH_TIMEOUT,
                ));
            }
            maybe_message = stream.next() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        let raw = text.as_str();
                        if raw.len() > MAX_FRAME_BYTES {
                            return Err(CloseInfo::new(
                                close_code::FRAME_TOO_LARGE,
                                close_reason::FRAME_TOO_LARGE,
                            ));
                        }
                        let envelope: ClientEnvelope = match serde_json::from_str(raw) {
                            Ok(envelope) => envelope,
                            Err(_) => {
                                return Err(CloseInfo::new(
                                    close_code::PROTOCOL_ERROR,
                                    close_reason::PROTOCOL_ERROR,
                                ));
                            }
                        };
                        if envelope.v != PROTOCOL_VERSION {
                            return Err(CloseInfo::new(
                                close_code::OUTDATED_PROTOCOL,
                                outdated_protocol_reason(min_overlay_version).as_str(),
                            ));
                        }
                        return match envelope.message {
                            ClientMessage::Hello(hello) => Ok(hello),
                            _ => Err(CloseInfo::new(
                                close_code::PROTOCOL_ERROR,
                                close_reason::PROTOCOL_ERROR,
                            )),
                        };
                    }
                    Some(Ok(Message::Binary(_))) => {
                        return Err(CloseInfo::new(
                            close_code::UNSUPPORTED_FRAME,
                            close_reason::UNSUPPORTED_FRAME,
                        ));
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_))) | None => {
                        return Err(CloseInfo::new(
                            close_code::NORMAL,
                            close_reason::CLIENT_SHUTDOWN,
                        ));
                    }
                    Some(Err(_)) => {
                        return Err(CloseInfo::new(
                            close_code::INTERNAL_ERROR,
                            close_reason::INTERNAL_ERROR,
                        ));
                    }
                }
            }
        }
    }
}

/// Envoie `welcome` puis `settings` (premières trames serveur de la session).
async fn send_welcome(
    sink: &mut SplitSink<WebSocket, Message>,
    hub: &Arc<Hub>,
    engine: &Arc<HubEngine>,
    context: &SessionContext,
) -> bool {
    let (show_text_default, default_media_time, max_media_time) =
        hub.guild_defaults(&context.guild_id).await;
    let current_job = match engine.current_job(&context.guild_id).await {
        Ok(Some(current_job)) => {
            let now_sec = u64::try_from(Utc::now().timestamp()).unwrap_or(0);
            hub.signed_current_job(&current_job, &context.client_id_str, now_sec)
        }
        Ok(None) => None,
        Err(error) => {
            warn!(error = %error, "lecture du job courant impossible au welcome");
            None
        }
    };
    let welcome = WelcomePayload {
        protocol_version: PROTOCOL_VERSION,
        guild_id: context.guild_id.as_str().to_owned(),
        client_id: context.client_id_str.clone(),
        session_mode: context.session_mode,
        server_version: env!("CARGO_PKG_VERSION").to_owned(),
        server_time: now_ms(),
        config: WelcomeConfig {
            show_text_default,
            default_media_time,
            max_media_time,
        },
        current_job,
    };
    if !send_server_message(sink, hub, ServerMessage::Welcome(welcome)).await {
        return false;
    }
    let settings = SettingsPayload {
        guild_id: context.guild_id.as_str().to_owned(),
        session_mode: context.session_mode,
        show_text_default,
        default_media_time,
        max_media_time,
    };
    send_server_message(sink, hub, ServerMessage::Settings(settings)).await
}

/// Sérialise et envoie un message serveur ; `false` si le socket est fermé.
async fn send_server_message(
    sink: &mut SplitSink<WebSocket, Message>,
    hub: &Arc<Hub>,
    message: ServerMessage,
) -> bool {
    let envelope = ServerEnvelope::new(
        hub.next_message_id(hub::message_prefix(&message)),
        now_ms(),
        message,
    );
    let text = match serde_json::to_string(&envelope) {
        Ok(text) => text,
        Err(error) => {
            warn!(error = %error, "sérialisation d'un message serveur impossible");
            return true;
        }
    };
    sink.send(Message::Text(text.into())).await.is_ok()
}

/// Envoie une trame de fermeture en ignorant les erreurs d'écriture.
async fn close_sink(sink: &mut SplitSink<WebSocket, Message>, info: &CloseInfo) {
    let frame = CloseFrame {
        code: info.code,
        reason: info.reason.clone().into(),
    };
    drop(sink.send(Message::Close(Some(frame))).await);
}

/// Traite un message client authentifié.
#[allow(clippy::too_many_arguments)]
async fn handle_client_message(
    state: &crate::app::AppState,
    hub: &Arc<Hub>,
    db: &Db,
    engine: &Arc<HubEngine>,
    context: &SessionContext,
    connection_id: Uuid,
    request_id: String,
    message: ClientMessage,
    timers: &mut SessionTimers,
) -> Result<(), CloseInfo> {
    match message {
        ClientMessage::Heartbeat(heartbeat) => {
            if let Err(error) = db
                .overlay_clients()
                .touch_last_seen(context.client_id, state.clock.now())
                .await
            {
                warn!(error = %error, client = %context.client_id_str, "mise à jour last_seen impossible");
            }
            if let Some(mode) = heartbeat.session_mode {
                if mode != context.session_mode {
                    debug!(
                        client = %context.client_id_str,
                        "sessionMode du heartbeat divergent du welcome (serveur autoritaire)"
                    );
                }
            }
        }
        ClientMessage::PlaybackState(payload) => {
            if !throttle(
                &mut timers.last_playback_state,
                Duration::from_millis(PLAYBACK_STATE_MIN_INTERVAL_MS),
            ) {
                return Ok(());
            }
            let job_id = parse_playback_job_id(&payload, close_reason::BAD_PLAYBACK_STATE)?;
            let remaining_ms = payload
                .remaining_ms
                .map(|remaining| remaining.min(MAX_PLAYBACK_REMAINING_MS));
            match engine
                .handle_playback_state(PlaybackStateEvent::new(
                    context.guild_id.clone(),
                    Some(job_id),
                    payload.state,
                    remaining_ms,
                ))
                .await
            {
                Ok(PlaybackStateOutcome::Persisted) => {
                    debug!(client = %context.client_id_str, "playback.state persisté");
                }
                Ok(outcome) => {
                    debug!(?outcome, client = %context.client_id_str, "playback.state non persisté");
                }
                Err(error) => {
                    warn!(error = %error, "échec du traitement de playback.state");
                }
            }
        }
        ClientMessage::PlaybackEnded(payload) => {
            let job_id = parse_playback_ended_job_id(&payload, close_reason::BAD_PLAYBACK_STATE)?;
            match engine
                .handle_playback_ended(PlaybackEndedEvent::new(
                    context.guild_id.clone(),
                    job_id,
                    payload.reason,
                ))
                .await
            {
                Ok(outcome) => debug!(
                    released = outcome.released,
                    next_started = outcome.next_started,
                    "playback.ended traité"
                ),
                Err(error) => warn!(error = %error, "échec du traitement de playback.ended"),
            }
        }
        ClientMessage::Error(payload) => {
            warn!(
                client = %context.client_id_str,
                job = %payload.job_id,
                code = %payload.code,
                message = %payload.message,
                "erreur signalée par l'overlay"
            );
        }
        ClientMessage::StopRequest(payload) => {
            if context.session_mode == SessionMode::InviteReadOnly {
                send_ack(
                    hub,
                    connection_id,
                    request_id.as_str(),
                    false,
                    Some(ack_error("read_only", "arrêt refusé en lecture seule")),
                );
                return Ok(());
            }
            if !throttle(
                &mut timers.last_stop_request,
                Duration::from_millis(PLAYBACK_STATE_MIN_INTERVAL_MS),
            ) {
                send_ack(
                    hub,
                    connection_id,
                    request_id.as_str(),
                    false,
                    Some(ack_error("rate_limited", "trop de stop.request")),
                );
                return Ok(());
            }
            let job_id = match payload.job_id.as_deref() {
                Some(raw) if !raw.trim().is_empty() => match PlaybackJobId::parse(raw.trim()) {
                    Ok(job_id) => Some(job_id),
                    Err(_) => {
                        send_ack(
                            hub,
                            connection_id,
                            request_id.as_str(),
                            false,
                            Some(ack_error("invalid_payload", "jobId invalide")),
                        );
                        return Ok(());
                    }
                },
                _ => None,
            };
            let source = payload.source.unwrap_or(StopRequestSource::User);
            send_ack(hub, connection_id, request_id.as_str(), true, None);
            match engine.stop(&context.guild_id, job_id, source).await {
                Ok(outcome) => debug!(stopped = outcome.stopped.len(), "stop.request appliqué"),
                Err(error) => {
                    warn!(error = %error, "échec de stop.request");
                    send_server_error(
                        hub,
                        connection_id,
                        "internal_error",
                        "échec de l'arrêt demandé",
                        job_id.map(|job_id| job_id.to_canonical()),
                    );
                }
            }
        }
        ClientMessage::MediaTrigger(payload) => {
            if context.session_mode == SessionMode::InviteReadOnly {
                send_ack(
                    hub,
                    connection_id,
                    request_id.as_str(),
                    false,
                    Some(ack_error("read_only", "média refusé en lecture seule")),
                );
                return Ok(());
            }
            let item_id = match MediaBoardItemId::parse(payload.item_id.as_str()) {
                Ok(item_id) => item_id,
                Err(_) => {
                    send_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        false,
                        Some(ack_error("invalid_payload", "itemId invalide")),
                    );
                    return Ok(());
                }
            };
            let item = match db.media_board().get(item_id).await {
                Ok(Some(item)) => item,
                Ok(None) => {
                    send_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        false,
                        Some(ack_error("item_not_found", "item inconnu")),
                    );
                    return Ok(());
                }
                Err(error) => {
                    warn!(error = %error, "lecture d'un item du board média impossible");
                    send_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        false,
                        Some(ack_error("internal_error", "lecture impossible")),
                    );
                    return Ok(());
                }
            };
            if item.guild_id != context.guild_id {
                send_ack(
                    hub,
                    connection_id,
                    request_id.as_str(),
                    false,
                    Some(ack_error("item_not_found", "item d'une autre guilde")),
                );
                return Ok(());
            }
            let media = match db.media_assets().get(item.media_asset_id).await {
                Ok(Some(media)) => media,
                Ok(None) => {
                    send_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        false,
                        Some(ack_error("media_not_ready", "média de l'item introuvable")),
                    );
                    return Ok(());
                }
                Err(error) => {
                    warn!(error = %error, "lecture du média d'un item du board impossible");
                    send_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        false,
                        Some(ack_error("internal_error", "lecture impossible")),
                    );
                    return Ok(());
                }
            };
            let now = state.clock.now();
            if !media.is_ready() || media.is_expired_at(now) {
                send_ack(
                    hub,
                    connection_id,
                    request_id.as_str(),
                    false,
                    Some(ack_error("media_not_ready", "média non prêt ou expiré")),
                );
                return Ok(());
            }
            let text = item
                .message
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(|text| text.chars().take(MAX_MEDIA_TEXT_CHARS).collect::<String>());
            let author_name = context
                .author_name
                .clone()
                .or_else(|| item.created_by_name.clone());
            let duration_sec = media
                .duration_sec
                .filter(|duration| *duration > 0)
                .unwrap_or(0);
            let job = PlaybackJob {
                id: PlaybackJobId::new_v4(),
                guild_id: context.guild_id.clone(),
                media_asset_id: Some(media.id),
                text: text.clone(),
                show_text: text.is_some(),
                author_name,
                author_image: context.author_image.clone(),
                duration_sec,
                priority: MEDIA_BOARD_JOB_PRIORITY,
                status: JobStatus::Pending,
                resumes_after_job_id: None,
                resume_offset_sec: 0,
                remaining_ms_snapshot: None,
                last_playback_state_at: None,
                submission_date: now,
                execution_date: now,
                scheduled_at: now,
                started_at: None,
                finished_at: None,
            };
            send_ack(hub, connection_id, request_id.as_str(), true, None);
            match engine.enqueue(&job).await {
                Ok(()) => debug!(
                    item = %payload.item_id,
                    "média déclenché",
                ),
                Err(error) => {
                    warn!(error = %error, "échec de l'enfilement d'un média");
                    send_server_error(
                        hub,
                        connection_id,
                        "internal_error",
                        "échec de l'enfilement du média",
                        Some(job.id.to_canonical()),
                    );
                }
            }
        }
        ClientMessage::PeersRequest(_payload) => {
            if !throttle(
                &mut timers.last_peers_request,
                Duration::from_millis(PEERS_REQUEST_MIN_INTERVAL_MS),
            ) {
                return Ok(());
            }
            send_ack(hub, connection_id, request_id.as_str(), true, None);
            hub.broadcast_peers(&context.guild_id);
        }
        ClientMessage::Hello(_) => {
            debug!(client = %context.client_id_str, "hello répété : fermeture 1002");
            return Err(CloseInfo::new(
                close_code::PROTOCOL_ERROR,
                close_reason::PROTOCOL_ERROR,
            ));
        }
    }
    Ok(())
}

/// Construit un payload `ack` et l'envoie via le canal de la connexion.
fn send_ack(
    hub: &Arc<Hub>,
    connection_id: Uuid,
    request_id: &str,
    ok: bool,
    error: Option<AckError>,
) {
    let payload = AckPayload {
        ref_id: request_id.to_owned(),
        ok,
        error,
    };
    let envelope = ServerEnvelope::new(
        hub.next_message_id("ack"),
        now_ms(),
        ServerMessage::Ack(payload),
    );
    if let Ok(text) = serde_json::to_string(&envelope) {
        hub.send(connection_id, OutFrame::Text(text));
    }
}

/// Construit une erreur d'`ack`.
fn ack_error(code: &str, message: &str) -> AckError {
    AckError {
        code: code.to_owned(),
        message: message.to_owned(),
    }
}

/// Envoie un message `error` serveur non fatal (P2, AUD-COR-12).
fn send_server_error(
    hub: &Arc<Hub>,
    connection_id: Uuid,
    code: &str,
    message: &str,
    job_id: Option<String>,
) {
    let payload = ServerErrorPayload {
        code: code.to_owned(),
        message: message.to_owned(),
        job_id,
    };
    let envelope = ServerEnvelope::new(
        hub.next_message_id("error"),
        now_ms(),
        ServerMessage::Error(payload),
    );
    if let Ok(text) = serde_json::to_string(&envelope) {
        hub.send(connection_id, OutFrame::Text(text));
    }
}

/// Parse le `jobId` d'un `playback.state` ; erreur protocolaire sinon.
fn parse_playback_job_id(
    payload: &PlaybackStatePayload,
    reason: &str,
) -> Result<PlaybackJobId, CloseInfo> {
    PlaybackJobId::parse(payload.job_id.as_str())
        .map_err(|_| CloseInfo::new(close_code::PROTOCOL_ERROR, reason))
}

/// Parse le `jobId` d'un `playback.ended` ; erreur protocolaire sinon.
fn parse_playback_ended_job_id(
    payload: &PlaybackEndedPayload,
    reason: &str,
) -> Result<PlaybackJobId, CloseInfo> {
    PlaybackJobId::parse(payload.job_id.as_str())
        .map_err(|_| CloseInfo::new(close_code::PROTOCOL_ERROR, reason))
}

/// Applique un throttle simple par emplacement ; `true` si le message passe.
fn throttle(slot: &mut Option<Instant>, min_interval: Duration) -> bool {
    let now = Instant::now();
    if let Some(previous) = *slot {
        if now.duration_since(previous) < min_interval {
            return false;
        }
    }
    *slot = Some(now);
    true
}

/// Valide l'`id` d'une trame client : non vide et ≤ 64 caractères (protocole §4).
fn validate_request_id(id: &str) -> Result<(), CloseInfo> {
    if id.trim().is_empty() || id.chars().count() > MAX_REQUEST_ID_CHARS {
        return Err(CloseInfo::new(
            close_code::PROTOCOL_ERROR,
            close_reason::PROTOCOL_ERROR,
        ));
    }
    Ok(())
}

/// Suivi de liveness : seule une trame `Pong` rafraîchit la fenêtre.
///
/// Le `heartbeat` applicatif (message JSON C→S) ne rafraîchit PAS cette
/// fenêtre : sinon un client qui n'enverrait que des heartbeats ne serait
/// jamais fermé après le timeout sans Pong (P2, AUD-COR-12).
#[derive(Debug)]
struct PongLiveness {
    last_pong: Instant,
    timeout: Duration,
}

impl PongLiveness {
    fn new(timeout: Duration) -> Self {
        Self {
            last_pong: Instant::now(),
            timeout,
        }
    }

    /// Un `Pong` reçu du pair rafraîchit la fenêtre.
    fn on_pong(&mut self) {
        self.last_pong = Instant::now();
    }

    /// La fenêtre est-elle expirée sans `Pong` ?
    fn is_expired(&self) -> bool {
        self.last_pong.elapsed() >= self.timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_validation_accepts_64_chars_and_rejects_others() {
        assert!(validate_request_id("cli-0001").is_ok());
        assert!(validate_request_id(&"a".repeat(MAX_REQUEST_ID_CHARS)).is_ok());
        assert!(validate_request_id("").is_err());
        assert!(validate_request_id("   ").is_err());
        assert!(validate_request_id(&"a".repeat(MAX_REQUEST_ID_CHARS + 1)).is_err());
    }

    #[test]
    fn pong_liveness_is_not_refreshed_by_application_heartbeats() {
        let mut liveness = PongLiveness::new(Duration::from_millis(40));
        std::thread::sleep(Duration::from_millis(80));
        assert!(
            liveness.is_expired(),
            "la fenêtre doit expirer sans trame Pong"
        );
        // Un heartbeat applicatif n'appelle pas `on_pong()` : la fenêtre reste
        // expirée (le code de session ne met à jour que sur `Message::Pong`).
        assert!(liveness.is_expired());
        liveness.on_pong();
        assert!(!liveness.is_expired(), "un Pong reçu rafraîchit la fenêtre");
    }
}
