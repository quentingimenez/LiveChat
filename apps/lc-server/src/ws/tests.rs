//! Tests du hub WebSocket v2.
//!
//! Les tests d'intégration bout-en-bout utilisent un vrai serveur axum sur un
//! port éphémère, une base SQLite temporaire et un client
//! `tokio-tungstenite` : ils vérifient le handshake, les codes de fermeture,
//! les limites, la signature des URLs média et la préemption côté session.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use lc_db::Db;
use lc_domain::{
    Guild, GuildId, JobStatus, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId,
    MediaKind as DomainMediaKind, MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode,
    PlaybackJob, PlaybackJobId,
};
use lc_playback::PlaybackStateEvent;
use lc_protocol::{
    close_code, close_reason, media_signature_input, MediaKind as ProtocolMediaKind, PlaybackState,
    PlaybackStatePayload, StopReason, StopRequestPayload, StopRequestSource,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::app::AppState;
use crate::config::Config;
use crate::shutdown::Shutdown;

use super::hub::{self, Hub, MAX_PEERS, MAX_SOCKETS_PER_CLIENT, MAX_SOCKETS_PER_GUILD};
use super::router_with_hub;

/// Client WebSocket de test.
type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// En-tête de la fixture `hello` (identifiants stables des tests).
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const TOKEN_B: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const APP_VERSION: &str = "3.0.0";
const GUILD_ID: &str = "123456789012345678";
const TS: i64 = 1_700_000_000_000;
const MEDIA_SECRET: &str = "test-secret";

/// Serveur de test : socket d'écoute, base SQLite temporaire et tâche axum.
#[allow(dead_code)]
struct TestServer {
    addr: SocketAddr,
    db: Db,
    config: Arc<Config>,
    state: AppState,
    hub: Arc<Hub>,
    shutdown: Shutdown,
    serve: tokio::task::JoinHandle<()>,
    _dir: TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.trigger();
        self.serve.abort();
    }
}

/// Configuration d'un serveur de test (avec ou sans secret média).
fn test_config(media_secret: bool) -> Arc<Config> {
    let mut pairs = vec![
        ("API_URL".to_owned(), "http://127.0.0.1:3000".to_owned()),
        ("MIN_OVERLAY_VERSION".to_owned(), APP_VERSION.to_owned()),
        ("NODE_ENV".to_owned(), "test".to_owned()),
    ];
    if media_secret {
        pairs.push(("MEDIA_URL_SECRET".to_owned(), MEDIA_SECRET.to_owned()));
    }
    Arc::new(Config::from_pairs(pairs).expect("configuration de test"))
}

/// Démarre un serveur de test complet sur un port éphémère.
async fn start_server() -> TestServer {
    start_server_with(test_config(true)).await
}

/// Démarre un serveur sans secret média : les URLs média ne sont pas signables.
async fn start_server_without_media_secret() -> TestServer {
    start_server_with(test_config(false)).await
}

/// Démarre un serveur de test avec la configuration fournie.
async fn start_server_with(config: Arc<Config>) -> TestServer {
    let dir = tempfile::tempdir().expect("répertoire temporaire");
    let path = dir.path().join("livechat-test.sqlite3");
    let db = Db::open(path.as_path(), &lc_db::DbConfig::default())
        .await
        .expect("base de test");
    let shutdown = Shutdown::new().0;
    let state = AppState::new(Arc::clone(&config), Some(db.clone()), shutdown.clone());
    let hub = Arc::new(Hub::new());
    let app = router_with_hub(Arc::clone(&hub)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind du serveur de test");
    let addr = listener.local_addr().expect("adresse locale");
    let serve = tokio::spawn(async move {
        drop(axum::serve(listener, app).await);
    });
    TestServer {
        addr,
        db,
        config,
        state,
        hub,
        shutdown,
        serve,
        _dir: dir,
    }
}

/// Ouvre une connexion WebSocket cliente vers le serveur de test.
async fn connect(addr: SocketAddr) -> WsClient {
    let url = format!("ws://{addr}/ws");
    let (socket, _response) = connect_async(url.as_str())
        .await
        .expect("connexion WebSocket");
    socket
}

/// Construit une enveloppe v2 JSON.
fn envelope(kind: &str, id: &str, payload: Value) -> Value {
    json!({ "v": 2, "type": kind, "id": id, "ts": TS, "payload": payload })
}

/// Envoie une trame JSON au serveur.
async fn send_value(socket: &mut WsClient, value: &Value) {
    let text = serde_json::to_string(value).expect("sérialisation JSON");
    socket
        .send(Message::Text(text.into()))
        .await
        .expect("envoi de trame");
}

/// Envoie `hello` avec le token et la version fournis.
async fn send_hello(socket: &mut WsClient, token: &str, app_version: &str) {
    send_value(
        socket,
        &envelope(
            "hello",
            "cli-hello-0001",
            json!({ "token": token, "appVersion": app_version }),
        ),
    )
    .await;
}

/// Lit la prochaine trame JSON sous 5 s.
async fn recv_value(socket: &mut WsClient) -> Value {
    let message = timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("timeout d'attente d'une trame")
        .expect("flux WebSocket terminé")
        .expect("trame WebSocket invalide");
    match message {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("JSON serveur"),
        other => panic!("trame inattendue: {other:?}"),
    }
}

/// Lit les trames jusqu'à en trouver une du type demandé.
async fn recv_until_type(socket: &mut WsClient, kind: &str) -> Value {
    for _ in 0..20 {
        let value = recv_value(socket).await;
        if value.get("type").and_then(Value::as_str) == Some(kind) {
            return value;
        }
    }
    panic!("aucune trame de type {kind} reçue");
}

/// Résultat du handshake de test.
struct Handshake {
    welcome: Value,
    play: Option<Value>,
}

/// Lit `welcome`, `settings` puis `peers` après un `hello` réussi.
///
/// Le moteur de lecture peut diffuser un `play` en parallèle du handshake :
/// la trame est capturée si elle arrive avant le `peers` final.
async fn handshake_full(socket: &mut WsClient, token: &str) -> Handshake {
    send_hello(socket, token, APP_VERSION).await;
    let mut welcome: Option<Value> = None;
    let mut play: Option<Value> = None;
    for _ in 0..50 {
        let value = recv_value(socket).await;
        match value.get("type").and_then(Value::as_str) {
            Some("welcome") => welcome = Some(value),
            Some("play") => play = Some(value),
            Some("peers") => {
                assert_eq!(value["payload"]["guildId"], GUILD_ID);
                assert!(!value["payload"]["peers"]
                    .as_array()
                    .expect("peers array")
                    .is_empty());
                return Handshake {
                    welcome: welcome.expect("welcome avant peers"),
                    play,
                };
            }
            _ => {}
        }
    }
    panic!("handshake incomplet : pas de peers reçus");
}

/// Variante ne retournant que le `welcome`.
async fn handshake(socket: &mut WsClient, token: &str) -> Value {
    handshake_full(socket, token).await.welcome
}

/// Ouvre une connexion WebSocket cliente avec un en-tête `Origin` explicite.
async fn connect_with_origin(addr: SocketAddr, origin: &str) -> WsClient {
    let url = format!("ws://{addr}/ws");
    let mut request = url
        .as_str()
        .into_client_request()
        .expect("requête WebSocket");
    request.headers_mut().insert(
        axum::http::HeaderName::from_static("origin"),
        origin.parse().expect("valeur d'en-tête Origin"),
    );
    let (socket, _response) = connect_async(request)
        .await
        .expect("connexion WebSocket avec Origin");
    socket
}

/// Attend la fermeture et vérifie le code **et** la raison.
async fn expect_close_with_reason(socket: &mut WsClient, expected: u16, expected_reason: &str) {
    loop {
        let message = timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("timeout d'attente de la fermeture")
            .expect("flux terminé sans trame de fermeture")
            .expect("trame WebSocket invalide");
        match message {
            Message::Close(Some(frame)) => {
                let code: u16 = frame.code.into();
                assert_eq!(
                    code, expected,
                    "code de fermeture inattendu (raison: {})",
                    frame.reason
                );
                assert_eq!(frame.reason.to_string(), expected_reason);
                return;
            }
            Message::Close(None) => panic!("fermeture sans code"),
            _ => continue,
        }
    }
}

/// Attend une trame de fermeture et vérifie son code.
async fn expect_close(socket: &mut WsClient, expected: u16) {
    loop {
        let message = timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("timeout d'attente de la fermeture")
            .expect("flux terminé sans trame de fermeture")
            .expect("trame WebSocket invalide");
        match message {
            Message::Close(Some(frame)) => {
                let code: u16 = frame.code.into();
                assert_eq!(
                    code, expected,
                    "code de fermeture inattendu (raison: {})",
                    frame.reason
                );
                return;
            }
            Message::Close(None) => panic!("fermeture sans code"),
            _ => continue,
        }
    }
}

/// Renvoie l'identifiant de guilde canonique des tests.
fn guild_id() -> GuildId {
    GuildId::parse(GUILD_ID).expect("GuildId de test")
}

/// Renvoie la date fixe des tests.
fn now() -> chrono::DateTime<Utc> {
    lc_domain::ms_to_datetime(TS).expect("date de test")
}

/// Crée une guilde de test.
async fn seed_guild(db: &Db) -> GuildId {
    let guild = Guild {
        id: guild_id(),
        name: Some("Guilde de test".to_owned()),
        busy_until: None,
        default_media_time: None,
        max_media_time: None,
        display_media_full: false,
    };
    db.guilds().upsert(&guild).await.expect("guilde de test");
    guild.id
}

/// Crée un client overlay de test et retourne son identifiant.
async fn seed_overlay_client(
    db: &Db,
    guild_id: &GuildId,
    token: &str,
    mode: OverlaySessionMode,
) -> OverlayClientId {
    let client = OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: "board-de-test".to_owned(),
        token_hash: hub::hash_token(token),
        session_mode: mode,
        default_author_name: Some("Auteur de test".to_owned()),
        default_author_image: None,
        created_by_discord_user_id: None,
        last_seen_at: None,
        created_at: now(),
        revoked_at: None,
    };
    let id = client.id;
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client overlay de test");
    id
}

/// Crée un média vidéo prêt de test.
async fn seed_media_asset(db: &Db, duration_sec: i64) -> MediaAssetId {
    let asset = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: format!("hash-{}", MediaAssetId::new_v4()),
        source_url: "https://example.invalid/video.mp4".to_owned(),
        kind: DomainMediaKind::Video,
        mime: "video/mp4".to_owned(),
        duration_sec: Some(duration_sec),
        width: Some(1920),
        height: Some(1080),
        is_vertical: false,
        storage_path: "media/test.mp4".to_owned(),
        size_bytes: 1024,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: now(),
        expires_at: now() + chrono::Duration::hours(24),
        created_at: now(),
    };
    let id = asset.id;
    db.media_assets()
        .insert(&asset)
        .await
        .expect("média de test");
    id
}

/// Crée un job de lecture de test.
#[allow(clippy::too_many_arguments)]
async fn seed_job(
    db: &Db,
    guild_id: &GuildId,
    media_asset_id: MediaAssetId,
    duration_sec: i64,
    status: JobStatus,
    started_at: Option<chrono::DateTime<Utc>>,
) -> PlaybackJobId {
    let job = PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: Some(media_asset_id),
        text: Some("média de test".to_owned()),
        show_text: true,
        author_name: Some("Auteur de test".to_owned()),
        author_image: None,
        duration_sec,
        priority: 0,
        status,
        resumes_after_job_id: None,
        resume_offset_sec: 0,
        remaining_ms_snapshot: None,
        last_playback_state_at: None,
        submission_date: now(),
        execution_date: now(),
        scheduled_at: now(),
        started_at,
        finished_at: None,
    };
    let id = job.id;
    db.playback_jobs().create(&job).await.expect("job de test");
    id
}

// ---------------------------------------------------------------------------
// Handshake et authentification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hello_returns_welcome_then_settings_then_peers() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    let client_id =
        seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    let welcome = handshake(&mut socket, TOKEN).await;

    assert_eq!(welcome["v"], 2);
    assert_eq!(welcome["type"], "welcome");
    let payload = &welcome["payload"];
    assert_eq!(payload["protocolVersion"], 2);
    assert_eq!(payload["guildId"], GUILD_ID);
    assert_eq!(payload["clientId"], client_id.to_canonical());
    assert_eq!(payload["sessionMode"], "normal");
    assert_eq!(payload["serverVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(payload["config"]["showTextDefault"], true);
    assert_eq!(payload["config"]["defaultMediaTime"], 5);
    assert!(payload.get("currentJob").is_none());
}

#[tokio::test]
async fn empty_token_closes_with_4401() {
    let server = start_server().await;
    let mut socket = connect(server.addr).await;
    send_hello(&mut socket, "   ", APP_VERSION).await;
    expect_close(&mut socket, close_code::UNAUTHORIZED).await;
}

#[tokio::test]
async fn unknown_token_closes_with_4401() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    send_hello(&mut socket, &"a".repeat(64), APP_VERSION).await;
    expect_close(&mut socket, close_code::UNAUTHORIZED).await;
}

#[tokio::test]
async fn revoked_client_closes_with_4402() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    let client_id =
        seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    server
        .db
        .overlay_clients()
        .revoke(client_id, Utc::now())
        .await
        .expect("révocation de test");
    let mut socket = connect(server.addr).await;
    send_hello(&mut socket, TOKEN, APP_VERSION).await;
    expect_close(&mut socket, close_code::REVOKED).await;
}

#[tokio::test]
async fn outdated_app_version_closes_with_4403_and_min_version() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    send_hello(&mut socket, TOKEN, "2.9.0").await;

    loop {
        let message = timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("timeout 4403")
            .expect("flux terminé")
            .expect("trame");
        match message {
            Message::Close(Some(frame)) => {
                let code: u16 = frame.code.into();
                assert_eq!(code, close_code::OUTDATED_PROTOCOL);
                let reason = frame.reason.to_string();
                let parsed: Value = serde_json::from_str(reason.as_str()).expect("reason JSON");
                assert_eq!(parsed["minVersion"], APP_VERSION);
                return;
            }
            Message::Close(None) => panic!("fermeture sans code"),
            _ => continue,
        }
    }
}

#[tokio::test]
async fn non_hello_first_frame_closes_with_1002() {
    let server = start_server().await;
    let mut socket = connect(server.addr).await;
    send_value(
        &mut socket,
        &envelope(
            "heartbeat",
            "cli-heartbeat-0001",
            json!({
                "clientId": "client",
                "guildId": GUILD_ID,
                "appVersion": APP_VERSION
            }),
        ),
    )
    .await;
    expect_close(&mut socket, close_code::PROTOCOL_ERROR).await;
}

// ---------------------------------------------------------------------------
// Limites de trames
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flooding_after_welcome_closes_with_4409() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    handshake(&mut socket, TOKEN).await;

    for index in 0..21 {
        send_value(
            &mut socket,
            &envelope(
                "heartbeat",
                &format!("cli-heartbeat-{index:04}"),
                json!({
                    "clientId": "client",
                    "guildId": GUILD_ID,
                    "appVersion": APP_VERSION
                }),
            ),
        )
        .await;
    }
    expect_close(&mut socket, close_code::RATE_LIMITED).await;
}

#[tokio::test]
async fn oversized_frame_closes_with_4408() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    handshake(&mut socket, TOKEN).await;

    let oversized = "x".repeat(lc_protocol::MAX_FRAME_BYTES + 1);
    socket
        .send(Message::Text(oversized.into()))
        .await
        .expect("envoi de la trame géante");
    expect_close(&mut socket, close_code::FRAME_TOO_LARGE).await;
}

// ---------------------------------------------------------------------------
// Playback : welcome.currentJob, play signé, stop.request, lecture seule
// ---------------------------------------------------------------------------

#[tokio::test]
async fn welcome_exposes_current_job_with_signed_url() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let asset_id = seed_media_asset(&server.db, 20).await;
    seed_job(
        &server.db,
        &guild,
        asset_id,
        20,
        JobStatus::Playing,
        Some(Utc::now()),
    )
    .await;

    let mut socket = connect(server.addr).await;
    let welcome = handshake(&mut socket, TOKEN).await;
    let current_job = &welcome["payload"]["currentJob"];
    assert!(!current_job.is_null(), "currentJob attendu au welcome");
    assert!(current_job["jobId"].as_str().is_some());
    assert_eq!(current_job["durationSec"], 20);
    assert!(current_job["startedAt"].as_i64().is_some());
    assert!(current_job["remainingMs"].as_u64().is_some());
    let url = current_job["media"]["url"]
        .as_str()
        .expect("URL média signée");
    assert!(url.contains("clientId="));
    assert!(url.contains("exp="));
    assert!(url.contains("sig="));
    // Aucun offset de reprise : le champ reste null et l'URL n'ajoute pas de
    // paramètre `startOffsetSec` (spécification §6.2/§7.6).
    assert!(current_job["media"]["startOffsetSec"].is_null());
    assert!(!url.contains("startOffsetSec"));
}

#[tokio::test]
async fn dispatched_play_carries_a_verifiable_signature() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    let client_id =
        seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let asset_id = seed_media_asset(&server.db, 12).await;
    let mut socket = connect(server.addr).await;
    let handshake = handshake_full(&mut socket, TOKEN).await;
    assert!(
        handshake.play.is_none(),
        "aucun play ne doit précéder le dispatch du job"
    );

    // Le job est enregistré après le handshake pour que le socket soit déjà
    // dans la room au moment du dispatch (sinon le sinks renverrait 0 client et
    // le moteur marquerait le job FAILED, comportement legacy de room vide).
    let now = now();
    let job = PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild.clone(),
        media_asset_id: Some(asset_id),
        text: Some("média de test".to_owned()),
        show_text: true,
        author_name: Some("Auteur".to_owned()),
        author_image: None,
        duration_sec: 12,
        priority: 0,
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
    let engine = server.hub.engine().expect("moteur initialisé au hello");
    engine.enqueue(&job).await.expect("enqueue du job");

    let play = recv_until_type(&mut socket, "play").await;
    let payload = &play["payload"];
    assert_eq!(payload["jobId"], job.id.to_canonical());
    assert_eq!(payload["durationSec"], 12);
    assert_eq!(payload["media"]["assetId"], asset_id.to_canonical());
    let url = payload["media"]["url"].as_str().expect("URL média");
    let query = url
        .split_once('?')
        .map(|(_, query)| query)
        .expect("query string");
    let mut signature: Option<&str> = None;
    let mut expiry: Option<u64> = None;
    let mut client: Option<&str> = None;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "sig" => signature = Some(value),
            "exp" => expiry = Some(value.parse().expect("exp numérique")),
            "clientId" => client = Some(value),
            _ => {}
        }
    }
    let signature = signature.expect("sig");
    let expiry = expiry.expect("exp");
    let client = client.expect("clientId");
    assert_eq!(client, client_id.to_canonical());
    let expected_input = media_signature_input(asset_id.to_canonical().as_str(), client, expiry);
    let expected_signature = crate::media_url::sign(
        MEDIA_SECRET.as_bytes(),
        asset_id.to_canonical().as_str(),
        client,
        expiry,
    )
    .expect("signature HMAC");
    assert_eq!(
        expected_input,
        format!("{}|{}|{}", asset_id.to_canonical(), client, expiry)
    );
    assert!(crate::media_url::verify(
        MEDIA_SECRET.as_bytes(),
        asset_id.to_canonical().as_str(),
        client,
        expiry,
        signature,
    ));
    assert_eq!(signature, expected_signature);
}

#[tokio::test]
async fn stop_request_stops_the_job_and_broadcasts_stop() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let asset_id = seed_media_asset(&server.db, 12).await;
    let job_id = seed_job(
        &server.db,
        &guild,
        asset_id,
        12,
        JobStatus::Playing,
        Some(Utc::now()),
    )
    .await;
    let mut socket = connect(server.addr).await;
    handshake(&mut socket, TOKEN).await;

    send_value(
        &mut socket,
        &envelope(
            "stop.request",
            "cli-stop-request-0001",
            json!({ "jobId": job_id.to_canonical(), "source": "hotkey" }),
        ),
    )
    .await;
    let ack = recv_until_type(&mut socket, "ack").await;
    assert_eq!(ack["payload"]["ref"], "cli-stop-request-0001");
    assert_eq!(ack["payload"]["ok"], true);

    let stop = recv_until_type(&mut socket, "stop").await;
    assert_eq!(stop["payload"]["reason"], "manual");
    assert_eq!(stop["payload"]["jobId"], job_id.to_canonical());

    let stored = server
        .db
        .playback_jobs()
        .get(job_id)
        .await
        .expect("lecture du job")
        .expect("job présent");
    assert_eq!(stored.status, JobStatus::Done);
}

#[tokio::test]
async fn invite_read_only_refuses_media_trigger() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(
        &server.db,
        &guild,
        TOKEN,
        OverlaySessionMode::InviteReadOnly,
    )
    .await;
    let mut socket = connect(server.addr).await;
    let welcome = handshake(&mut socket, TOKEN).await;
    assert_eq!(welcome["payload"]["sessionMode"], "invite_read_only");

    send_value(
        &mut socket,
        &envelope(
            "media.trigger",
            "cli-media-trigger-0001",
            json!({ "itemId": "clx7q2x0b0004abcd1234efgh", "trigger": "shortcut" }),
        ),
    )
    .await;
    let ack = recv_until_type(&mut socket, "ack").await;
    assert_eq!(ack["payload"]["ok"], false);
    assert_eq!(ack["payload"]["error"]["code"], "read_only");
}

/// Legacy overlay 3.0.0 compatibility: remove in 3.1.
#[tokio::test]
async fn legacy_meme_trigger_type_still_plays_a_media_item() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let asset_id = seed_media_asset(&server.db, 12).await;
    // Le serveur utilise l'horloge système : l'asset de test doit expirer dans
    // le futur réel pour passer le contrôle `media_not_ready`.
    sqlx::query("UPDATE media_assets SET expires_at = ? WHERE id = ?")
        .bind((Utc::now() + chrono::Duration::hours(24)).timestamp_millis())
        .bind(asset_id.to_canonical())
        .execute(server.db.pool())
        .await
        .expect("expiration du média de test");
    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild.clone(),
        media_asset_id: asset_id,
        title: Some("Legacy".to_owned()),
        message: None,
        created_by_discord_user_id: None,
        created_by_name: Some("Auteur".to_owned()),
        created_at: now(),
    };
    server
        .db
        .media_board()
        .create(&item)
        .await
        .expect("item de board");
    let mut socket = connect(server.addr).await;
    handshake(&mut socket, TOKEN).await;

    send_value(
        &mut socket,
        &envelope(
            "meme.trigger",
            "cli-legacy-meme-trigger-0001",
            json!({ "itemId": item.id.to_canonical(), "trigger": "shortcut" }),
        ),
    )
    .await;
    let ack = recv_until_type(&mut socket, "ack").await;
    assert_eq!(ack["payload"]["ok"], true);
    assert_eq!(ack["payload"]["ref"], "cli-legacy-meme-trigger-0001");
    let play = recv_until_type(&mut socket, "play").await;
    assert!(play["payload"]["jobId"].as_str().is_some());
}

#[tokio::test]
async fn heartbeat_is_accepted_and_updates_last_seen() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    let client_id =
        seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    handshake(&mut socket, TOKEN).await;

    send_value(
        &mut socket,
        &envelope(
            "heartbeat",
            "cli-heartbeat-0001",
            json!({
                "clientId": "client",
                "guildId": GUILD_ID,
                "appVersion": APP_VERSION
            }),
        ),
    )
    .await;
    let mut seen = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let stored = server
            .db
            .overlay_clients()
            .get(client_id)
            .await
            .expect("lecture client")
            .expect("client présent");
        if stored.last_seen_at.is_some() {
            seen = true;
            break;
        }
    }
    assert!(seen, "last_seen_at doit être mis à jour par le heartbeat");
}

#[tokio::test]
async fn peers_request_returns_ack_then_peers() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    handshake(&mut socket, TOKEN).await;

    send_value(
        &mut socket,
        &envelope("peers.request", "cli-peers-request-0001", json!({})),
    )
    .await;
    let ack = recv_until_type(&mut socket, "ack").await;
    assert_eq!(ack["payload"]["ok"], true);
    let peers = recv_until_type(&mut socket, "peers").await;
    assert!(peers["payload"]["peers"]
        .as_array()
        .expect("peers array")
        .iter()
        .any(|peer| peer["label"] == "Auteur de test"));
}

#[tokio::test]
async fn playback_state_is_persisted_for_a_corresponding_job() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let asset_id = seed_media_asset(&server.db, 12).await;
    let job_id = seed_job(
        &server.db,
        &guild,
        asset_id,
        12,
        JobStatus::Playing,
        Some(Utc::now()),
    )
    .await;
    let mut socket = connect(server.addr).await;
    handshake(&mut socket, TOKEN).await;

    let event = json!({ "jobId": job_id.to_canonical(), "state": "playing", "remainingMs": 9_500 });
    send_value(
        &mut socket,
        &envelope("playback.state", "cli-state-0001", event),
    )
    .await;
    let mut persisted = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let stored = server
            .db
            .playback_jobs()
            .get(job_id)
            .await
            .expect("lecture job")
            .expect("job présent");
        if stored.remaining_ms_snapshot.is_some() {
            persisted = true;
            break;
        }
    }
    assert!(persisted, "playback.state doit mettre à jour le snapshot");
}

// ---------------------------------------------------------------------------
// Tests unitaires du hub (registre et helpers)
// ---------------------------------------------------------------------------

#[test]
fn max_peers_is_frozen() {
    assert_eq!(MAX_PEERS, 8);
}

#[test]
fn sha256_token_hash_matches_the_reference_vector() {
    assert_eq!(
        hub::hash_token("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn close_reason_mapping_is_stable() {
    assert_eq!(
        hub::close_reason_for_code(close_code::REVOKED),
        lc_protocol::close_reason::REVOKED
    );
    assert_eq!(
        hub::close_reason_for_code(close_code::RATE_LIMITED),
        lc_protocol::close_reason::RATE_LIMITED
    );
    assert_eq!(
        hub::close_reason_for_code(close_code::NORMAL),
        lc_protocol::close_reason::CLIENT_SHUTDOWN
    );
}

#[test]
fn signed_play_uses_the_adaptive_media_ttl() {
    let config = Config::from_pairs(vec![
        ("API_URL".to_owned(), "http://127.0.0.1:3000".to_owned()),
        ("MEDIA_URL_SECRET".to_owned(), MEDIA_SECRET.to_owned()),
    ])
    .expect("configuration de test");
    let payload = lc_protocol::PlayPayload {
        job_id: "job-1".to_owned(),
        media: Some(lc_protocol::MediaPayload {
            asset_id: "asset-1".to_owned(),
            url: String::new(),
            mime: "video/mp4".to_owned(),
            kind: ProtocolMediaKind::Video,
            duration_sec: Some(600),
            is_vertical: false,
            start_offset_sec: Some(12),
        }),
        text: lc_protocol::TextPayload {
            value: "texte".to_owned(),
            enabled: true,
        },
        author: lc_protocol::AuthorPayload {
            name: "Auteur".to_owned(),
            image: None,
            enabled: true,
        },
        tweet_card: None,
        duration_sec: 588,
        ..Default::default()
    };
    let now_sec = 1_700_000_000;
    let signed =
        hub::signed_play_payload(&payload, "client-1", &config, now_sec).expect("payload signé");
    let media = signed.media.expect("média signé");
    assert_eq!(media.start_offset_sec, Some(12));
    let expected_expiry = now_sec + 720;
    assert!(media.url.contains(&format!("exp={expected_expiry}")));
    assert!(media.url.contains("&startOffsetSec=12#t=12"));
    assert!(media.url.contains("sig="));
    assert!(media.url.contains("clientId=client-1"));
}

#[test]
fn playback_state_event_and_stop_payload_shapes_match_the_protocol() {
    let event = PlaybackStateEvent::new(guild_id(), None, PlaybackState::Playing, Some(1_000));
    assert_eq!(event.remaining_ms, Some(1_000));
    assert!(matches!(event.state, PlaybackState::Playing));
    let stop = lc_protocol::StopPayload {
        reason: StopReason::Preempt,
        job_id: None,
    };
    let json = serde_json::to_value(&stop).expect("sérialisation");
    assert_eq!(json["reason"], "preempt");
    assert!(json["jobId"].is_null());
    let state_payload = PlaybackStatePayload {
        job_id: "job".to_owned(),
        state: PlaybackState::Paused,
        remaining_ms: None,
    };
    let json = serde_json::to_value(&state_payload).expect("sérialisation");
    assert_eq!(json["state"], "paused");
    assert!(json["remainingMs"].is_null());
    let request = StopRequestPayload {
        job_id: Some("job".to_owned()),
        source: Some(StopRequestSource::Admin),
    };
    let json = serde_json::to_value(&request).expect("sérialisation");
    assert_eq!(json["source"], "admin");
    assert_eq!(json["jobId"], "job");
}

// ---------------------------------------------------------------------------
// COR-06 : Origin allowlist et plafonds de sockets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn origin_tauri_localhost_is_allowed() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect_with_origin(server.addr, "http://tauri.localhost").await;
    let welcome = handshake(&mut socket, TOKEN).await;
    assert_eq!(welcome["payload"]["protocolVersion"], 2);
}

#[tokio::test]
async fn unknown_origin_is_closed_with_1008() {
    let server = start_server().await;
    let mut socket = connect_with_origin(server.addr, "https://evil.example").await;
    expect_close_with_reason(&mut socket, 1008, "origin_not_allowed").await;
}

#[tokio::test]
async fn too_many_sockets_per_client_are_rejected_with_4409() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    let client = seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut sockets = Vec::new();
    for _ in 0..MAX_SOCKETS_PER_CLIENT {
        let mut socket = connect(server.addr).await;
        let welcome = handshake(&mut socket, TOKEN).await;
        assert_eq!(welcome["payload"]["clientId"], client.to_canonical());
        sockets.push(socket);
    }
    let mut fourth = connect(server.addr).await;
    send_hello(&mut fourth, TOKEN, APP_VERSION).await;
    expect_close_with_reason(
        &mut fourth,
        close_code::RATE_LIMITED,
        "too_many_connections",
    )
    .await;
    drop(sockets.pop());
    tokio::time::sleep(Duration::from_millis(250)).await;
    let mut reconnected = connect(server.addr).await;
    let _welcome = handshake(&mut reconnected, TOKEN).await;
}

#[tokio::test]
async fn socket_caps_are_enforced_unit() {
    let hub = Hub::new();
    let guild = guild_id();
    let client = OverlayClientId::new_v4();
    for _ in 0..MAX_SOCKETS_PER_CLIENT {
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        hub.register(
            guild.clone(),
            client,
            client.to_canonical(),
            "board".to_owned(),
            lc_protocol::SessionMode::Normal,
            None,
            None,
            sender,
        )
        .expect("sous le plafond client");
    }
    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    assert_eq!(
        hub.register(
            guild,
            client,
            client.to_canonical(),
            "board".to_owned(),
            lc_protocol::SessionMode::Normal,
            None,
            None,
            sender,
        ),
        Err(hub::RegisterError::ClientLimit)
    );

    let other_guild = GuildId::parse("123456789012345679").expect("guilde de test");
    for _ in 0..MAX_SOCKETS_PER_GUILD {
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        hub.register(
            other_guild.clone(),
            OverlayClientId::new_v4(),
            "client".to_owned(),
            "board".to_owned(),
            lc_protocol::SessionMode::Normal,
            None,
            None,
            sender,
        )
        .expect("sous le plafond guilde");
    }
    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    assert_eq!(
        hub.register(
            other_guild,
            OverlayClientId::new_v4(),
            "client".to_owned(),
            "board".to_owned(),
            lc_protocol::SessionMode::Normal,
            None,
            None,
            sender,
        ),
        Err(hub::RegisterError::GuildLimit)
    );
}

// ---------------------------------------------------------------------------
// COR-07 : jobs texte-seul, révocation et purge
// ---------------------------------------------------------------------------

/// Construit un job sans média (texte/tweet-card uniquement).
fn text_only_job(guild_id: &GuildId, status: JobStatus) -> PlaybackJob {
    PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: None,
        text: Some("texte sans média".to_owned()),
        show_text: true,
        author_name: Some("Auteur".to_owned()),
        author_image: None,
        duration_sec: 5,
        priority: 0,
        status,
        resumes_after_job_id: None,
        resume_offset_sec: 0,
        remaining_ms_snapshot: None,
        last_playback_state_at: None,
        submission_date: now(),
        execution_date: now(),
        scheduled_at: now(),
        started_at: if status == JobStatus::Playing {
            // Horloge réelle : un `started_at` figé dans le passé (TS) serait
            // libéré par le watchdog « stale » du moteur au bootstrap.
            Some(Utc::now())
        } else {
            None
        },
        finished_at: None,
    }
}

#[tokio::test]
async fn text_only_job_is_played_and_marked_playing() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    let _welcome = handshake(&mut socket, TOKEN).await;

    let job = text_only_job(&guild, JobStatus::Pending);
    let job_id = job.id;
    let engine = server.hub.engine().expect("moteur initialisé au hello");
    engine
        .enqueue(&job)
        .await
        .expect("enqueue du job texte-seul");

    let play = recv_until_type(&mut socket, "play").await;
    assert_eq!(play["payload"]["jobId"], job_id.to_canonical());
    assert!(
        play["payload"]["media"].is_null(),
        "un job texte-seul doit être émis sans média : {play}"
    );
    assert_eq!(play["payload"]["text"]["value"], "texte sans média");

    let mut status = None;
    for _ in 0..40 {
        status = server
            .db
            .playback_jobs()
            .get(job_id)
            .await
            .expect("lecture du job")
            .map(|job| job.status);
        if status == Some(JobStatus::Playing) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(status, Some(JobStatus::Playing));
}

#[tokio::test]
async fn welcome_exposes_text_only_current_job() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let job = text_only_job(&guild, JobStatus::Playing);
    server
        .db
        .playback_jobs()
        .create(&job)
        .await
        .expect("job en cours");

    let mut socket = connect(server.addr).await;
    let welcome = handshake(&mut socket, TOKEN).await;
    let current = &welcome["payload"]["currentJob"];
    assert!(current.is_object(), "currentJob attendu : {welcome}");
    assert_eq!(current["jobId"], job.id.to_canonical());
    assert!(current["media"].is_null());
    assert!(current["remainingMs"].as_u64().is_some());
}

#[tokio::test]
async fn revocation_emits_stop_revoked_before_close_4402() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    let client_a = seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let _client_b =
        seed_overlay_client(&server.db, &guild, TOKEN_B, OverlaySessionMode::Normal).await;
    let asset = seed_media_asset(&server.db, 20).await;
    let job_id = seed_job(
        &server.db,
        &guild,
        asset,
        20,
        JobStatus::Playing,
        Some(Utc::now()),
    )
    .await;
    let mut socket_a = connect(server.addr).await;
    let _welcome = handshake(&mut socket_a, TOKEN).await;
    let mut socket_b = connect(server.addr).await;
    let _welcome = handshake(&mut socket_b, TOKEN_B).await;

    let closed = server
        .hub
        .close_client(&client_a.as_uuid(), close_code::REVOKED);
    assert!(closed, "le client révoqué doit avoir des sockets ouverts");

    let stop_a = recv_until_type(&mut socket_a, "stop").await;
    assert_eq!(stop_a["payload"]["reason"], "revoked");
    assert!(stop_a["payload"]["jobId"].is_null());
    expect_close(&mut socket_a, close_code::REVOKED).await;

    let stop_b = recv_until_type(&mut socket_b, "stop").await;
    assert_eq!(stop_b["payload"]["reason"], "revoked");
    assert!(stop_b["payload"]["jobId"].is_null());

    let mut status = None;
    for _ in 0..60 {
        status = server
            .db
            .playback_jobs()
            .get(job_id)
            .await
            .expect("lecture du job")
            .map(|job| job.status);
        if status == Some(JobStatus::Done) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(status, Some(JobStatus::Done));
}

#[tokio::test]
async fn purge_stop_has_a_dedicated_reason() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let asset = seed_media_asset(&server.db, 20).await;
    let job_id = seed_job(
        &server.db,
        &guild,
        asset,
        20,
        JobStatus::Playing,
        Some(Utc::now()),
    )
    .await;
    let mut socket = connect(server.addr).await;
    let _welcome = handshake(&mut socket, TOKEN).await;

    let stopped = server.hub.broadcast_purge(&guild).await;
    assert_eq!(stopped, 1, "un job PLAYING doit être arrêté par la purge");
    let stop = recv_until_type(&mut socket, "stop").await;
    assert_eq!(stop["payload"]["reason"], "purge");
    assert!(stop["payload"]["jobId"].is_null());

    let mut status = None;
    for _ in 0..40 {
        status = server
            .db
            .playback_jobs()
            .get(job_id)
            .await
            .expect("lecture du job")
            .map(|job| job.status);
        if status == Some(JobStatus::Done) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(status, Some(JobStatus::Done));
}

// ---------------------------------------------------------------------------
// P2 — AUD-COR-12 : robustesse des trames et erreurs non fatales
// ---------------------------------------------------------------------------

/// Une trame de 300 Kio (au-delà des 64 Kio applicatifs, sous la borne de
/// transport de 1 Mio) doit recevoir un close `4408`, pas un `1006` transport.
#[tokio::test]
async fn oversized_frame_is_closed_with_4408_instead_of_1006() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    let _welcome = handshake(&mut socket, TOKEN).await;

    socket
        .send(Message::Text("x".repeat(300 * 1024).into()))
        .await
        .expect("envoi d'une trame surdimensionnée");
    expect_close_with_reason(
        &mut socket,
        close_code::FRAME_TOO_LARGE,
        close_reason::FRAME_TOO_LARGE,
    )
    .await;
}

/// Un `id` vide est une violation de protocole (`1002`), avant tout traitement.
#[tokio::test]
async fn empty_request_id_is_closed_with_1002() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    let _welcome = handshake(&mut socket, TOKEN).await;

    send_value(
        &mut socket,
        &envelope(
            "heartbeat",
            "",
            json!({
                "clientId": "test",
                "guildId": GUILD_ID,
                "appVersion": APP_VERSION,
            }),
        ),
    )
    .await;
    expect_close_with_reason(
        &mut socket,
        close_code::PROTOCOL_ERROR,
        close_reason::PROTOCOL_ERROR,
    )
    .await;
}

/// Un `id` de plus de 64 caractères est refusé de la même manière.
#[tokio::test]
async fn oversized_request_id_is_closed_with_1002() {
    let server = start_server().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let mut socket = connect(server.addr).await;
    let _welcome = handshake(&mut socket, TOKEN).await;

    send_value(
        &mut socket,
        &envelope(
            "heartbeat",
            &"a".repeat(65),
            json!({
                "clientId": "test",
                "guildId": GUILD_ID,
                "appVersion": APP_VERSION,
            }),
        ),
    )
    .await;
    expect_close_with_reason(
        &mut socket,
        close_code::PROTOCOL_ERROR,
        close_reason::PROTOCOL_ERROR,
    )
    .await;
}

/// Un échec de dispatch non fatal (ici : URL média non signable) émet une trame
/// `error` et laisse la connexion ouverte.
#[tokio::test]
async fn dispatch_failure_emits_non_fatal_error_frame() {
    let server = start_server_without_media_secret().await;
    let guild = seed_guild(&server.db).await;
    seed_overlay_client(&server.db, &guild, TOKEN, OverlaySessionMode::Normal).await;
    let asset_id = seed_media_asset(&server.db, 12).await;
    let mut socket = connect(server.addr).await;
    let _welcome = handshake(&mut socket, TOKEN).await;

    let now = now();
    let job = PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild.clone(),
        media_asset_id: Some(asset_id),
        text: Some("média de test".to_owned()),
        show_text: true,
        author_name: Some("Auteur".to_owned()),
        author_image: None,
        duration_sec: 12,
        priority: 0,
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
    let engine = server.hub.engine().expect("moteur initialisé au hello");
    engine.enqueue(&job).await.expect("enqueue du job");

    let error = recv_until_type(&mut socket, "error").await;
    assert_eq!(error["payload"]["code"], "dispatch_failed");
    assert_eq!(error["payload"]["jobId"], job.id.to_canonical());

    // La connexion survit à l'erreur : une requête suivante est servie.
    send_value(
        &mut socket,
        &envelope("peers.request", "cli-peers-after-error", json!({})),
    )
    .await;
    let ack = recv_until_type(&mut socket, "ack").await;
    assert_eq!(ack["payload"]["ref"], "cli-peers-after-error");
    assert_eq!(ack["payload"]["ok"], true);
}
