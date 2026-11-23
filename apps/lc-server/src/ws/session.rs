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
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use lc_db::Db;
use lc_domain::{
    now_ms, GuildId, JobSource, JobStatus, MediaBoardItemId, MediaKind, OverlayClientId,
    PlaybackJob, PlaybackJobId,
};
use lc_playback::{PlaybackEndedEvent, PlaybackStateEvent, PlaybackStateOutcome};
use lc_protocol::{
    close_code, close_reason, is_version_below, outdated_protocol_reason, AckError, AckOutcome,
    AckPayload, ClientEnvelope, ClientMessage, HelloPayload, PlaybackEndedPayload,
    PlaybackEndedReason, PlaybackStatePayload, ServerEnvelope, ServerErrorPayload, ServerMessage,
    SessionMode, SettingsPayload, StopRequestSource, WelcomeConfig, WelcomePayload,
    HELLO_TIMEOUT_MS, MAX_CLIENT_MESSAGES_PER_SECOND, MAX_FRAME_BYTES, MAX_MEDIA_TEXT_CHARS,
    MAX_PLAYBACK_REMAINING_MS, PEERS_REQUEST_MIN_INTERVAL_MS, PING_INTERVAL_MS,
    PLAYBACK_STATE_MIN_INTERVAL_MS, PROTOCOL_VERSION,
};
use time::UtcDateTime;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep, timeout, MissedTickBehavior};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::hub::{self, AuthError, Hub, HubEngine, OutFrame, TOO_MANY_CONNECTIONS_REASON};
use crate::http::overlay::guild::guild_icon_url;
use crate::media::display_duration::card_duration_sec;

/// Borne dure de transport de l'upgrade : volontairement supérieure à la
/// limite applicative (64 Kio) afin de pouvoir répondre `4408` aux trames
/// intermédiaires au lieu de laisser axum couper en 1006/1009 (P2, AUD-COR-12).
const MAX_TRANSPORT_MESSAGE_BYTES: usize = 1024 * 1024;

/// Longueur maximale de l'`id` d'une trame client (protocole §4).
const MAX_REQUEST_ID_CHARS: usize = 64;

/// Fenêtre glissante du rate limiter client.
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);

/// Fenêtre de coalescence d'un `media.trigger` identique (par session).
///
/// Une rafale de raccourcis identiques (répétition de touche, double clic,
/// client qui rejoue sa file) déclenchait un job et une quinzaine de requêtes
/// SQL par trame. La garde est indexée sur l'item : un vrai changement de média
/// passe immédiatement, seule la répétition de la même demande est absorbée.
const MEDIA_TRIGGER_MIN_INTERVAL: Duration = Duration::from_millis(1_000);

/// Code close `1008` (policy violation) pour une origine refusée.
///
/// Le close est utilisé plutôt qu'une réponse HTTP 403 car la requête est déjà
/// un upgrade WebSocket ; la raison documentée est `origin_not_allowed`.
const POLICY_VIOLATION: u16 = 1008;

/// Raison de fermeture quand l'origine WebSocket n'est pas autorisée.
const ORIGIN_NOT_ALLOWED_REASON: &str = "origin_not_allowed";

/// Délai maximal accordé à une écriture sur le socket (B-01).
///
/// Un client qui cesse de lire peut bloquer `SinkExt::send` indéfiniment :
/// la session ne lirait plus ses messages, ne verrait plus la demande de
/// fermeture du hub et ne pourrait plus s'arrêter. Passé ce délai, la
/// connexion est fermée `4408 slow_client` et la tâche se termine.
const WRITER_SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// Résultat d'une écriture socket bornée par [`WRITER_SEND_TIMEOUT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundedSend {
    /// Trame écrite.
    Sent,
    /// Le socket a refusé l'écriture (fermé ou erreur de transport).
    Failed,
    /// Le délai est écoulé : le client ne draine plus.
    TimedOut,
}

/// Écrit une trame en bornant l'attente du socket.
///
/// `limit` est un paramètre pour que le test du timeout n'attende pas les 5 s
/// de production. La session mappe `TimedOut` sur une fermeture
/// `4408 slow_client` ; `Failed` garde le comportement existant (sortie sans
/// close explicite).
async fn send_bounded<S>(sink: &mut S, message: Message, limit: Duration) -> BoundedSend
where
    S: futures_util::Sink<Message> + Unpin,
{
    match timeout(limit, sink.send(message)).await {
        Ok(Ok(())) => BoundedSend::Sent,
        Ok(Err(_)) => BoundedSend::Failed,
        Err(_) => BoundedSend::TimedOut,
    }
}

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
///
/// Aucun profil d'auteur n'est figé ici : `media.trigger` relit le membre en
/// base à chaque déclenchement, pour qu'une modification de profil soit prise
/// en compte sans reconnexion.
struct SessionContext {
    guild_id: GuildId,
    client_id: OverlayClientId,
    client_id_str: String,
    session_mode: SessionMode,
}

/// Horodatages et gardes des derniers messages soumis à throttle (par connexion).
#[derive(Default)]
struct SessionTimers {
    last_playback_state: Option<Instant>,
    last_playback_ended: Option<Instant>,
    last_stop_request: Option<Instant>,
    last_peers_request: Option<Instant>,
    /// Dernier `media.trigger` accepté, pour l'idempotence des répétitions.
    last_media_trigger: Option<MediaTriggerGuard>,
}

/// Garde d'idempotence du dernier `media.trigger` accepté sur la session.
///
/// Une session traite ses trames séquentiellement : au plus une exécution de
/// `media.trigger` est en vol. Cette garde couvre la répétition une fois la
/// première terminée — une demande identique reçue dans la fenêtre ré-acquitte
/// le job déjà créé, sans relire la base ni créer un job. Les erreurs ne sont
/// pas mémorisées : un item devenu prêt entre deux trames repasse donc.
struct MediaTriggerGuard {
    item_id: MediaBoardItemId,
    job_id: Option<String>,
    outcome: Option<AckOutcome>,
    position: Option<u32>,
    /// Instant du dernier trigger accepté ou coalescé (fenêtre glissante).
    accepted_at: Instant,
}

impl MediaTriggerGuard {
    /// La demande est-elle une répétition identique dans la fenêtre ?
    ///
    /// Un item différent passe toujours. Une répétition dans la fenêtre la
    /// prolonge, pour absorber la rafale entière et pas seulement sa première
    /// trame.
    fn coalesce(&mut self, item_id: MediaBoardItemId, now: Instant) -> bool {
        if self.item_id != item_id
            || now.duration_since(self.accepted_at) >= MEDIA_TRIGGER_MIN_INTERVAL
        {
            return false;
        }
        self.accepted_at = now;
        true
    }
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

    let min_overlay_version = state.runtime().min_overlay_version();
    let hello = match wait_for_hello(&mut stream, min_overlay_version.as_str()).await {
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

    if is_version_below(hello.app_version.as_str(), min_overlay_version.as_str()) {
        let reason = outdated_protocol_reason(min_overlay_version.as_str());
        let info = CloseInfo::new(close_code::OUTDATED_PROTOCOL, reason.as_str());
        close_sink(&mut sink, &info).await;
        return;
    }

    // `appVersion` and `platform` are accepted with this `hello`: store them
    // for the admin console (`overlayVersion` / `platform`).
    if let Err(error) = db
        .overlay_clients()
        .update_client_info(
            client.id,
            Some(hello.app_version.as_str()),
            hello.platform.map(lc_protocol::Platform::as_str),
        )
        .await
    {
        warn!(error = %error, client = %client_id_str, "enregistrement des informations du client impossible");
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
    };

    let (sender, mut receiver) = mpsc::channel::<OutFrame>(hub::OUTBOUND_QUEUE_CAPACITY);
    let (connection_id, mut close_rx) = match hub.register(
        guild_id.clone(),
        client.id,
        client_id_str,
        client.label,
        session_mode,
        // Peers keep the client's author defaults at connect time; the play
        // payload is the part that must follow the live member profile.
        client.default_author_name.clone(),
        client.default_author_image.clone(),
        sender,
    ) {
        Ok(registered) => registered,
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

    if !send_welcome(&mut sink, &hub, &engine, &db, &context).await {
        hub.unregister(connection_id);
        return;
    }

    hub.broadcast_peers(&guild_id);
    info!(client = %context.client_id_str, guild = %guild_id, "overlay connecté au hub WebSocket");
    // A member linked to Discord takes their current name and avatar.
    crate::discord::profiles::spawn_refresh_on_connect(&state, context.client_id);

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
                            let min_overlay_version = state.runtime().min_overlay_version();
                            let reason = outdated_protocol_reason(min_overlay_version.as_str());
                            close_info = Some(CloseInfo::new(
                                close_code::OUTDATED_PROTOCOL,
                                reason.as_str(),
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
                        match send_bounded(
                            &mut sink,
                            Message::Text(text.into()),
                            WRITER_SEND_TIMEOUT,
                        )
                        .await
                        {
                            BoundedSend::Sent => {}
                            BoundedSend::Failed => break,
                            BoundedSend::TimedOut => {
                                warn!(
                                    client = %context.client_id_str,
                                    "écriture WebSocket bloquée : client lent, fermeture 4408"
                                );
                                close_info = Some(CloseInfo::new(
                                    hub::SLOW_CLIENT_CLOSE_CODE,
                                    hub::SLOW_CLIENT_REASON,
                                ));
                                break;
                            }
                        }
                    }
                    Some(OutFrame::Close(code, reason)) => {
                        close_info = Some(CloseInfo { code, reason });
                        break;
                    }
                    None => break,
                }
            }
            changed = close_rx.changed() => {
                if changed.is_ok() {
                    if let Some(close) = close_rx.borrow_and_update().clone() {
                        close_info = Some(CloseInfo::new(close.code, close.reason.as_str()));
                    }
                }
                break;
            }
            _ = ping_interval.tick() => {
                if liveness.is_expired() {
                    close_info = Some(CloseInfo::new(
                        close_code::GOING_AWAY,
                        close_reason::HEARTBEAT_TIMEOUT,
                    ));
                    break;
                }
                match send_bounded(
                    &mut sink,
                    Message::Ping(Default::default()),
                    WRITER_SEND_TIMEOUT,
                )
                .await
                {
                    BoundedSend::Sent => {}
                    BoundedSend::Failed => break,
                    BoundedSend::TimedOut => {
                        warn!(
                            client = %context.client_id_str,
                            "ping WebSocket bloqué : client lent, fermeture 4408"
                        );
                        close_info = Some(CloseInfo::new(
                            hub::SLOW_CLIENT_CLOSE_CODE,
                            hub::SLOW_CLIENT_REASON,
                        ));
                        break;
                    }
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
    db: &Db,
    context: &SessionContext,
) -> bool {
    let (show_text_default, default_media_time, max_media_time) =
        hub.guild_defaults(&context.guild_id).await;
    let access = match db.guilds().access(&context.guild_id).await {
        Ok(access) => access,
        Err(error) => {
            warn!(error = %error, "lecture du rattachement de guilde impossible au welcome");
            None
        }
    };
    let current_job = match engine.current_job(&context.guild_id).await {
        Ok(Some(current_job)) => {
            let now_sec = u64::try_from(UtcDateTime::now().unix_timestamp()).unwrap_or(0);
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
        guild_name: access.as_ref().and_then(|access| access.name.clone()),
        guild_icon_url: access.as_ref().and_then(guild_icon_url),
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
    match send_bounded(sink, Message::Text(text.into()), WRITER_SEND_TIMEOUT).await {
        BoundedSend::Sent => true,
        BoundedSend::Failed | BoundedSend::TimedOut => false,
    }
}

/// Envoie une trame de fermeture en ignorant les erreurs d'écriture.
///
/// L'écriture est bornée : même une fermeture vers un client qui ne lit plus
/// ne peut pas retenir la tâche indéfiniment.
async fn close_sink(sink: &mut SplitSink<WebSocket, Message>, info: &CloseInfo) {
    let frame = CloseFrame {
        code: info.code,
        reason: info.reason.clone().into(),
    };
    let _ = send_bounded(sink, Message::Close(Some(frame)), WRITER_SEND_TIMEOUT).await;
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
    // The admin may change the mode of this live connection: always read the
    // hub's current value instead of the mode captured at `hello`.
    let session_mode = hub
        .connection_session_mode(connection_id)
        .unwrap_or(context.session_mode);
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
                if mode != session_mode {
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
            if context.session_mode == SessionMode::InviteReadOnly {
                debug!(
                    client = %context.client_id_str,
                    "playback.ended ignoré en lecture seule"
                );
                return Ok(());
            }
            let job_id = parse_playback_ended_job_id(&payload, close_reason::BAD_PLAYBACK_STATE)?;
            if !throttle(
                &mut timers.last_playback_ended,
                Duration::from_millis(PLAYBACK_STATE_MIN_INTERVAL_MS),
            ) {
                debug!(
                    client = %context.client_id_str,
                    "playback.ended ignoré : throttle"
                );
                return Ok(());
            }
            let connected = hub.connected_for_guild(&context.guild_id);
            if !playback_ended_releases_job(payload.reason, connected) {
                debug!(
                    client = %context.client_id_str,
                    connected,
                    "playback.ended local ignoré : d'autres overlays lisent encore le job"
                );
                return Ok(());
            }
            match engine
                .handle_playback_ended(PlaybackEndedEvent::new(
                    context.guild_id.clone(),
                    job_id,
                    payload.reason,
                ))
                .await
            {
                Ok(outcome) => {
                    debug!(
                        released = outcome.released,
                        next_started = outcome.next_started,
                        "playback.ended traité"
                    );
                    // The job is finished and the queue may have moved on.
                    state.events.overview_changed();
                }
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
            if session_mode == SessionMode::InviteReadOnly {
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
                Ok(outcome) => {
                    debug!(stopped = outcome.stopped.len(), "stop.request appliqué");
                    // A stop without a job id stops the guild's playback for
                    // everyone: journal it like the console's stop route.
                    if job_id.is_none() {
                        crate::http::overlay::record_playback_stop(
                            state,
                            db,
                            &context.guild_id,
                            context.client_id,
                            outcome.stopped.len(),
                        )
                        .await;
                    }
                }
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
            if session_mode == SessionMode::InviteReadOnly {
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
            // Idempotence : une session traite ses trames séquentiellement, donc
            // au plus une exécution de trigger est en vol. Une répétition de la
            // même demande dans la fenêtre ré-acquitte le job déjà créé sans
            // relire la base ni créer un second job ; un autre item passe.
            if let Some(guard) = timers.last_media_trigger.as_mut() {
                if guard.coalesce(item_id, Instant::now()) {
                    debug!(
                        item = %payload.item_id,
                        job = guard.job_id.as_deref().unwrap_or_default(),
                        "media.trigger identique coalescé",
                    );
                    send_trigger_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        None,
                        guard.outcome,
                        guard.position,
                        guard.job_id.clone(),
                    );
                    return Ok(());
                }
            }
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
            // The member profile is read at trigger time: a `PATCH
            // /overlay/profile` must be visible immediately, without waiting
            // for a reconnection.
            let (author_name, author_image) = trigger_author(db, context.client_id).await;
            let author_name = author_name.or_else(|| item.created_by_name.clone());
            // An image has no length of its own: the time chosen for the item,
            // else the guild default (a text card scales with its text). The
            // guild cap applies at dispatch.
            let duration_sec = if media.kind == MediaKind::Image {
                match item.display_duration_sec {
                    Some(seconds) => seconds,
                    None => {
                        let (_, default_media_time, _) =
                            hub.guild_defaults(&context.guild_id).await;
                        let guild_default = i64::try_from(default_media_time).unwrap_or(i64::MAX);
                        card_duration_sec(&media, guild_default).unwrap_or(guild_default)
                    }
                }
            } else {
                media
                    .duration_sec
                    .filter(|duration| *duration > 0)
                    .unwrap_or(0)
            };
            let job = PlaybackJob {
                id: PlaybackJobId::new_v4(),
                guild_id: context.guild_id.clone(),
                media_asset_id: Some(media.id),
                text: text.clone(),
                show_text: text.is_some(),
                author_name,
                author_image,
                duration_sec,
                source: JobSource::Board,
                status: JobStatus::Pending,
                resume_offset_sec: 0,
                last_playback_state_at: None,
                submission_date: now,
                execution_date: now,
                scheduled_at: now,
                started_at: None,
                finished_at: None,
            };
            info!(
                guild = %context.guild_id,
                job = %job.id,
                source = job.source.as_db_str(),
                media_asset_id = %media.id,
                duration_sec = job.duration_sec,
                "job de lecture créé"
            );
            match engine.enqueue(&job).await {
                Ok(()) => {
                    let mut outcome = None;
                    let mut position = None;
                    match db.playback_jobs().get(job.id).await {
                        Ok(Some(stored)) => match stored.status {
                            JobStatus::Playing => outcome = Some(AckOutcome::Playing),
                            JobStatus::Pending => {
                                outcome = Some(AckOutcome::Queued);
                                match db
                                    .playback_jobs()
                                    .list_pending_by_guild(&context.guild_id)
                                    .await
                                {
                                    Ok(pending) => {
                                        position = pending
                                            .iter()
                                            .position(|pending_job| pending_job.id == job.id)
                                            .and_then(|index| u32::try_from(index + 1).ok());
                                    }
                                    Err(error) => warn!(
                                        error = %error,
                                        "lecture de la file d'attente impossible après enfilement"
                                    ),
                                }
                            }
                            _ => {}
                        },
                        Ok(None) => warn!(
                            job_id = %job.id,
                            "job introuvable après enfilement"
                        ),
                        Err(error) => warn!(
                            error = %error,
                            job_id = %job.id,
                            "relecture du job impossible après enfilement"
                        ),
                    }
                    let job_id = job.id.to_canonical();
                    timers.last_media_trigger = Some(MediaTriggerGuard {
                        item_id,
                        job_id: Some(job_id.clone()),
                        outcome,
                        position,
                        accepted_at: Instant::now(),
                    });
                    send_trigger_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        None,
                        outcome,
                        position,
                        Some(job_id),
                    );
                    // The playback queue changed: refresh the admin overview.
                    state.events.overview_changed();
                    debug!(
                        item = %payload.item_id,
                        "média déclenché",
                    );
                }
                Err(error) => {
                    warn!(error = %error, "échec de l'enfilement d'un média");
                    send_trigger_ack(
                        hub,
                        connection_id,
                        request_id.as_str(),
                        Some(ack_error(
                            "internal_error",
                            "échec de l'enfilement du média",
                        )),
                        None,
                        None,
                        None,
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
        ClientMessage::DeviceSettingsState(payload) => {
            if !payload.is_valid() {
                debug!(
                    client = %context.client_id_str,
                    volume = payload.volume,
                    "device.settings.state ignoré : volume hors de 0..1"
                );
                return Ok(());
            }
            hub.set_device_settings(&context.client_id.as_uuid(), payload);
            debug!(
                client = %context.client_id_str,
                volume = payload.volume,
                media_muted = payload.media_muted,
                paused = payload.paused,
                "device.settings.state mémorisé"
            );
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

/// Current stream author of an overlay socket, read from the database.
///
/// The owning member's live profile (`display_name`, `avatar`) wins. The
/// client's `default_author_*` values are only a fallback for an overlay with
/// no attached member (`legacy` pairings). A database error is logged and
/// treated as "no profile" so it never blocks playback.
async fn trigger_author(db: &Db, client_id: OverlayClientId) -> (Option<String>, Option<String>) {
    match db.guild_members().get_for_overlay_client(client_id).await {
        Ok(Some(member)) => (Some(member.display_name), member.avatar),
        Ok(None) => match db.overlay_clients().get(client_id).await {
            Ok(Some(client)) => (client.default_author_name, client.default_author_image),
            Ok(None) => (None, None),
            Err(error) => {
                warn!(error = %error, client = %client_id, "lecture du profil d'auteur impossible");
                (None, None)
            }
        },
        Err(error) => {
            warn!(error = %error, client = %client_id, "lecture du membre d'auteur impossible");
            (None, None)
        }
    }
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
        outcome: None,
        position: None,
        job_id: None,
    };
    send_ack_payload(hub, connection_id, payload);
}

/// Sends a `media.trigger` ack carrying its dispatch result.
fn send_trigger_ack(
    hub: &Arc<Hub>,
    connection_id: Uuid,
    request_id: &str,
    error: Option<AckError>,
    outcome: Option<AckOutcome>,
    position: Option<u32>,
    job_id: Option<String>,
) {
    let payload = AckPayload {
        ref_id: request_id.to_owned(),
        ok: error.is_none(),
        error,
        outcome,
        position,
        job_id,
    };
    send_ack_payload(hub, connection_id, payload);
}

/// Serializes and sends an `ack` payload on the connection channel.
fn send_ack_payload(hub: &Arc<Hub>, connection_id: Uuid, payload: AckPayload) {
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
/// Whether a `playback.ended` releases the job for the whole guild.
///
/// Any overlay finishing a job releases it, except one that stopped it on
/// its own device only (`reason: "local"`): the other connected overlays are
/// still playing it and one of them reports the real end. Alone in the
/// guild, a local stop is the end of the job, so the queue moves on at once.
fn playback_ended_releases_job(reason: PlaybackEndedReason, connected_overlays: usize) -> bool {
    reason != PlaybackEndedReason::Local || connected_overlays <= 1
}

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

    /// La garde coalesce les répétitions identiques dans la fenêtre, la
    /// prolonge à chaque répétition, et laisse passer un autre item ou une
    /// demande plus tardive.
    #[test]
    fn media_trigger_guard_coalesces_identical_repeats_inside_the_window() {
        let item = MediaBoardItemId::new_v4();
        let other = MediaBoardItemId::new_v4();
        let accepted = Instant::now();
        let mut guard = MediaTriggerGuard {
            item_id: item,
            job_id: Some("job".to_owned()),
            outcome: Some(AckOutcome::Playing),
            position: None,
            accepted_at: accepted,
        };

        assert!(guard.coalesce(item, accepted + Duration::from_millis(250)));
        assert_eq!(guard.job_id.as_deref(), Some("job"));
        assert_eq!(guard.outcome, Some(AckOutcome::Playing));
        // Fenêtre glissante : 900 ms après la répétition précédente, encore
        // dans la fenêtre.
        assert!(guard.coalesce(item, accepted + Duration::from_millis(1_150)));
        assert!(
            !guard.coalesce(other, accepted + Duration::from_millis(1_200)),
            "un autre item doit toujours passer"
        );
        assert!(
            !guard.coalesce(item, accepted + Duration::from_millis(2_151)),
            "une demande après la fenêtre doit repasser"
        );
    }

    /// Sink qui ne devient jamais prêt : simule un pair qui ne lit plus, sans
    /// attendre les 5 s de production.
    struct StalledSink;

    impl futures_util::Sink<Message> for StalledSink {
        type Error = ();

        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn start_send(self: std::pin::Pin<&mut Self>, _item: Message) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }

        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), Self::Error>> {
            std::task::Poll::Pending
        }
    }

    /// B-01 : l'écriture socket est bornée par un timeout, même si le sink ne
    /// devient jamais prêt. La session mappe ce résultat sur `4408 slow_client`.
    #[tokio::test]
    async fn writer_send_is_bounded_even_when_the_socket_stalls() {
        let mut sink = StalledSink;
        let started = Instant::now();
        let outcome = send_bounded(
            &mut sink,
            Message::Text("trame".to_owned().into()),
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(outcome, BoundedSend::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "le writer doit rendre la main au timeout, pas rester bloqué"
        );
    }
}
