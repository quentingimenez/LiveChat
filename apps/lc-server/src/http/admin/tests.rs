//! Tests d'intégration du panneau d'administration.
//!
//! Tous les tests passent par `crate::app::router()` (pas seulement par
//! `admin::router()`) : ils vérifient donc aussi l'intégration avec les couches
//! globales (en-têtes de sécurité, limite de corps, rate limit, normalisation
//! des erreurs) et la cohabitation des CSP admin/globale.
//!
//! Chaque test crée sa propre base SQLite (en mémoire par défaut, sur fichier
//! pour les tests de concurrence qui ont besoin de plusieurs connexions) et son
//! propre dossier temporaire (URL `DATABASE_URL` pointant dans ce dossier) : les
//! réglages runtime (`admin_runtime.json`) sont ainsi isolés du dépôt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Method, Request as HttpRequest, StatusCode};
use axum::Router;
use lc_db::{Db, DbConfig};
use lc_domain::{
    Guild, GuildId, GuildInvite, GuildMember, GuildMemberId, GuildRole, IngestClient,
    IngestClientId, JobStatus, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId,
    MediaKind, MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode, PairingCode,
    PairingCodeEntry, PairingMode, PlaybackJob, PlaybackJobId,
};
use lc_protocol::{close_code, DeviceSettingsStatePayload, ServerMessage, SessionMode};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use time::{Duration, UtcDateTime};
use tower::ServiceExt;

use super::hub::{admin_hub, AdminHub, NoopAdminHub};
use super::oauth::{AdminOAuth, DiscordIdentity, OAuthError};
use super::runtime_settings::SETTINGS_FILE_NAME;
use super::session;
use super::ADMIN_CSP;
use crate::app::{router, AppState};
use crate::config::Config;
use crate::discord::guild_leave::{DiscordGuildLeaver, LeaveError, LeaveOutcome};
use crate::http::security::{CONTENT_SECURITY_POLICY, PERMISSIONS_POLICY};
use crate::shutdown::Shutdown;

pub(super) const TOKEN: &str = "jeton-admin-de-test-0123456789abcdef";
/// `Host` envoyé par les requêtes du harnais (origine du panneau en test).
pub(super) const TEST_HOST: &str = "127.0.0.1:40000";
/// Origine correspondant à `TEST_HOST`, utilisée pour les mutations (CSRF).
pub(super) const TEST_ORIGIN: &str = "http://127.0.0.1:40000";

/// Issue configurée d'un [`FakeLeaver`].
#[derive(Debug, Clone, Copy)]
pub(super) enum FakeLeaveResult {
    Left,
    NotMember,
    Api,
}

/// Leaver factice : enregistre les identifiants reçus, sans réseau.
#[derive(Debug)]
pub(super) struct FakeLeaver {
    calls: Mutex<Vec<String>>,
    result: FakeLeaveResult,
}

impl FakeLeaver {
    pub(super) fn new(result: FakeLeaveResult) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            result,
        })
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("verrou du leaver").clone()
    }
}

#[async_trait::async_trait]
impl DiscordGuildLeaver for FakeLeaver {
    async fn leave(&self, discord_guild_id: &str) -> Result<LeaveOutcome, LeaveError> {
        self.calls
            .lock()
            .expect("verrou du leaver")
            .push(discord_guild_id.to_owned());
        match self.result {
            FakeLeaveResult::Left => Ok(LeaveOutcome::Left),
            FakeLeaveResult::NotMember => Ok(LeaveOutcome::NotMember),
            FakeLeaveResult::Api => Err(LeaveError::Api("échec Discord simulé".to_owned())),
        }
    }
}

/// Fake Discord OAuth client: records exchanges and returns a fixed identity
/// (or a simulated failure when `identity` is `None`).
#[derive(Debug)]
struct FakeOAuth {
    calls: Mutex<Vec<(String, String)>>,
    identity: Option<DiscordIdentity>,
}

impl FakeOAuth {
    fn returning(identity: DiscordIdentity) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            identity: Some(identity),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            identity: None,
        })
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("verrou OAuth").clone()
    }
}

#[async_trait::async_trait]
impl AdminOAuth for FakeOAuth {
    async fn exchange(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<DiscordIdentity, OAuthError> {
        self.calls
            .lock()
            .expect("verrou OAuth")
            .push((code.to_owned(), redirect_uri.to_owned()));
        match &self.identity {
            Some(identity) => Ok(identity.clone()),
            None => Err(OAuthError::Exchange("échange Discord simulé".to_owned())),
        }
    }
}

/// Admin hub fake that records close calls, sent frames and device state.
#[derive(Debug, Default)]
pub(super) struct RecordingAdminHub {
    closes: Mutex<Vec<(uuid::Uuid, u16)>>,
    sent: Mutex<Vec<(uuid::Uuid, ServerMessage)>>,
    device_settings: Mutex<std::collections::HashMap<uuid::Uuid, DeviceSettingsStatePayload>>,
    session_modes: Mutex<std::collections::HashMap<uuid::Uuid, SessionMode>>,
    connected: AtomicBool,
    deliver: AtomicBool,
}

impl RecordingAdminHub {
    fn closes(&self) -> Vec<(uuid::Uuid, u16)> {
        self.closes.lock().expect("verrou du hub").clone()
    }

    /// Frames sent through [`AdminHub::send_to_client`], in order.
    pub(super) fn sent(&self) -> Vec<(uuid::Uuid, ServerMessage)> {
        self.sent.lock().expect("verrou du hub").clone()
    }

    /// Session modes applied by [`AdminHub::set_session_mode`].
    pub(super) fn session_modes(&self) -> std::collections::HashMap<uuid::Uuid, SessionMode> {
        self.session_modes.lock().expect("verrou du hub").clone()
    }

    /// Sets whether [`AdminHub::is_connected`] reports the client online.
    pub(super) fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::SeqCst);
    }

    /// Sets whether [`AdminHub::send_to_client`] reports a delivery.
    pub(super) fn set_deliver(&self, deliver: bool) {
        self.deliver.store(deliver, Ordering::SeqCst);
    }

    /// Publishes a `device.settings.state` for an overlay client.
    pub(super) fn set_device_settings(
        &self,
        client_id: uuid::Uuid,
        state: DeviceSettingsStatePayload,
    ) {
        self.device_settings
            .lock()
            .expect("verrou du hub")
            .insert(client_id, state);
    }
}

#[async_trait::async_trait]
impl AdminHub for RecordingAdminHub {
    fn connected_count(&self) -> usize {
        0
    }

    fn connected_for_guild(&self, _guild_id: &GuildId) -> usize {
        0
    }

    fn is_connected(&self, _client_id: &uuid::Uuid) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn close_client(&self, client_id: &uuid::Uuid, code: u16) -> bool {
        self.closes
            .lock()
            .expect("verrou du hub")
            .push((*client_id, code));
        true
    }

    fn close_client_with_reason(&self, client_id: &uuid::Uuid, code: u16, _reason: &str) -> bool {
        self.close_client(client_id, code)
    }

    fn send_to_client(&self, client_id: &uuid::Uuid, message: ServerMessage) -> bool {
        self.sent
            .lock()
            .expect("verrou du hub")
            .push((*client_id, message));
        self.deliver.load(Ordering::SeqCst)
    }

    fn device_settings(&self, client_id: &uuid::Uuid) -> Option<DeviceSettingsStatePayload> {
        self.device_settings
            .lock()
            .expect("verrou du hub")
            .get(client_id)
            .copied()
    }

    fn set_session_mode(&self, client_id: &uuid::Uuid, mode: SessionMode) -> bool {
        self.session_modes
            .lock()
            .expect("verrou du hub")
            .insert(*client_id, mode);
        true
    }

    async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
        0
    }

    async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
        0
    }
}

/// Environnement de test : dossier temporaire, base mémoire et routeur complet.
pub(super) struct Harness {
    pub(super) dir: TempDir,
    pub(super) state: AppState,
    pub(super) db: Db,
    pub(super) app: Router,
    /// Raw cookie token of the session created for the harness (if any login
    /// method is configured). Most tests use it implicitly.
    pub(super) session_token: Option<String>,
}

impl Harness {
    pub(super) async fn new() -> Self {
        Self::with_pairs(&[]).await
    }

    pub(super) async fn with_pairs(extra: &[(&str, &str)]) -> Self {
        Self::with_pairs_and_leaver(extra, None).await
    }

    pub(super) async fn with_leaver(leaver: Arc<dyn DiscordGuildLeaver>) -> Self {
        Self::with_pairs_and_leaver(&[], Some(leaver)).await
    }

    /// Harness with an injected admin hub (device-control tests).
    pub(super) async fn with_hub(hub: Arc<dyn AdminHub>) -> Self {
        Self::build_with_hub(&[], None, None, Some(hub)).await
    }

    /// Harness on a file-backed database with several real SQLite connections.
    ///
    /// Concurrency tests need writers that can hold their transactions at the
    /// same time: an in-memory database is forced to a single connection, so it
    /// can only prove pool serialization.
    pub(super) async fn with_file_db(max_connections: u32) -> Self {
        Self::build_with_hub_and_db(&[], None, None, None, Some(max_connections)).await
    }

    pub(super) async fn with_pairs_and_leaver(
        extra: &[(&str, &str)],
        leaver: Option<Arc<dyn DiscordGuildLeaver>>,
    ) -> Self {
        Self::build(extra, leaver, None).await
    }

    pub(super) async fn with_pairs_and_oauth(
        extra: &[(&str, &str)],
        oauth: Arc<dyn AdminOAuth>,
    ) -> Self {
        Self::build(extra, None, Some(oauth)).await
    }

    pub(super) async fn build(
        extra: &[(&str, &str)],
        leaver: Option<Arc<dyn DiscordGuildLeaver>>,
        oauth: Option<Arc<dyn AdminOAuth>>,
    ) -> Self {
        Self::build_with_hub(extra, leaver, oauth, None).await
    }

    async fn build_with_hub(
        extra: &[(&str, &str)],
        leaver: Option<Arc<dyn DiscordGuildLeaver>>,
        oauth: Option<Arc<dyn AdminOAuth>>,
        hub: Option<Arc<dyn AdminHub>>,
    ) -> Self {
        Self::build_with_hub_and_db(extra, leaver, oauth, hub, None).await
    }

    async fn build_with_hub_and_db(
        extra: &[(&str, &str)],
        leaver: Option<Arc<dyn DiscordGuildLeaver>>,
        oauth: Option<Arc<dyn AdminOAuth>>,
        hub: Option<Arc<dyn AdminHub>>,
        file_db_connections: Option<u32>,
    ) -> Self {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let database_path = dir.path().join("livechat.sqlite3");
        // Racine média confinée dans le dossier du harnais : la purge admin
        // n'est autorisée à supprimer que des fichiers sous cette racine (V2-S).
        let media_storage_dir = dir.path().join("media");
        std::fs::create_dir_all(&media_storage_dir).expect("racine média de test");
        let mut pairs: Vec<(String, String)> = vec![
            ("API_URL".to_owned(), "http://localhost:3000".to_owned()),
            ("ADMIN_UI_TOKEN".to_owned(), TOKEN.to_owned()),
            (
                "DATABASE_URL".to_owned(),
                format!("sqlite://{}", database_path.display()),
            ),
            (
                "MEDIA_STORAGE_DIR".to_owned(),
                media_storage_dir.display().to_string(),
            ),
        ];
        for (name, value) in extra {
            pairs.push(((*name).to_owned(), (*value).to_owned()));
        }

        let config = Config::from_pairs(pairs).expect("configuration de test valide");
        let db = match file_db_connections {
            Some(max_connections) => Db::open(
                &database_path,
                &DbConfig::default().with_max_connections(max_connections),
            )
            .await
            .expect("base de test fichier"),
            None => Db::open_in_memory(&DbConfig::default())
                .await
                .expect("base de test en mémoire"),
        };
        // Log in once like the browser would: the raw token is kept by the
        // harness and every request carries it as a cookie. Tests that verify
        // the 401 path build requests without it explicitly.
        let session_token = if config.admin_auth_enabled() {
            Some(
                session::create_secret(&db, lc_domain::datetime_to_ms(UtcDateTime::now()))
                    .await
                    .expect("session de test")
                    .0,
            )
        } else {
            None
        };
        let mut state = AppState::new(Arc::new(config), Some(db.clone()), Shutdown::detached());
        if let Some(leaver) = leaver {
            state = state.with_discord_leaver(leaver);
        }
        if let Some(oauth) = oauth {
            state = state.with_admin_oauth(oauth);
        }
        if let Some(hub) = hub {
            state = state.with_admin_hub(hub);
        }
        let app = router(state.clone());

        Self {
            dir,
            state,
            db,
            app,
            session_token,
        }
    }

    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub(super) fn session_cookie(&self) -> Option<String> {
        self.session_token
            .as_ref()
            .map(|token| format!("{}={token}", session::SESSION_COOKIE_NAME))
    }

    pub(super) fn apply_session_cookie(&self, request: &mut HttpRequest<Body>) {
        if let Some(cookie) = self.session_cookie() {
            request.headers_mut().insert(
                header::COOKIE,
                header::HeaderValue::from_str(&cookie).expect("cookie valide"),
            );
        }
    }

    pub(super) fn request(&self, method: Method, path: &str) -> HttpRequest<Body> {
        self.request_from(method, path, IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    pub(super) fn request_from(&self, method: Method, path: &str, ip: IpAddr) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::ORIGIN, TEST_ORIGIN)
            .body(Body::empty())
            .expect("requête de test valide");
        self.apply_session_cookie(&mut request);
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(ip, 40_000)));
        request
    }

    /// Request with no session cookie (used to assert 401 responses).
    pub(super) fn request_without_session(&self, method: Method, path: &str) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::ORIGIN, TEST_ORIGIN)
            .body(Body::empty())
            .expect("requête de test valide");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
        request
    }

    /// Request with an arbitrary `Cookie` header.
    pub(super) fn request_with_cookie(
        &self,
        method: Method,
        path: &str,
        cookie: &str,
    ) -> HttpRequest<Body> {
        let mut request = self.request_without_session(method, path);
        request.headers_mut().insert(
            header::COOKIE,
            header::HeaderValue::from_str(cookie).expect("cookie valide"),
        );
        request
    }

    /// Requête de mutation avec origine explicite (`None` = origine absente).
    pub(super) fn request_with_origin(
        &self,
        method: Method,
        path: &str,
        origin: Option<&str>,
    ) -> HttpRequest<Body> {
        let mut builder = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST);
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        let mut request = builder.body(Body::empty()).expect("requête de test valide");
        self.apply_session_cookie(&mut request);
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
        request
    }

    /// Requête de mutation avec un `Referer` (et sans `Origin`).
    pub(super) fn request_with_referer(
        &self,
        method: Method,
        path: &str,
        referer: &str,
    ) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::REFERER, referer)
            .body(Body::empty())
            .expect("requête de test valide");
        self.apply_session_cookie(&mut request);
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
        request
    }

    pub(super) fn request_json(
        &self,
        method: Method,
        path: &str,
        body: &Value,
    ) -> HttpRequest<Body> {
        let mut request = self.request_json_without_session(method, path, body);
        self.apply_session_cookie(&mut request);
        request
    }

    pub(super) fn request_json_without_session(
        &self,
        method: Method,
        path: &str,
        body: &Value,
    ) -> HttpRequest<Body> {
        let payload = serde_json::to_vec(body).expect("corps JSON sérialisable");
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::ORIGIN, TEST_ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload))
            .expect("requête de test valide");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
        request
    }

    pub(super) async fn send(
        &self,
        request: HttpRequest<Body>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("réponse axum");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("corps lisible");
        (status, headers, bytes.to_vec())
    }

    pub(super) async fn call(&self, method: Method, path: &str) -> (StatusCode, Value) {
        let (status, _, bytes) = self.send(self.request(method, path)).await;
        (status, json_or_null(&bytes))
    }

    pub(super) async fn call_json(
        &self,
        method: Method,
        path: &str,
        body: &Value,
    ) -> (StatusCode, Value) {
        let (status, _, bytes) = self.send(self.request_json(method, path, body)).await;
        (status, json_or_null(&bytes))
    }

    pub(super) async fn call_with_headers(
        &self,
        method: Method,
        path: &str,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.send(self.request(method, path)).await
    }

    pub(super) fn db(&self) -> &Db {
        &self.db
    }

    pub(super) fn state(&self) -> &AppState {
        &self.state
    }
}

pub(super) fn json_or_null(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(super) fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// All `Set-Cookie` headers whose name matches `name` (attributes included).
fn set_cookie_headers(headers: &HeaderMap, name: &str) -> Vec<String> {
    let prefix = format!("{name}=");
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter(|value| value.starts_with(&prefix))
        .map(str::to_owned)
        .collect()
}

/// Value of the first matching `Set-Cookie` header.
fn set_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    set_cookie_headers(headers, name)
        .first()
        .and_then(|header| header.split(';').next())
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value.to_owned())
}

/// Query parameter of a `Location` header (absolute URLs only).
fn location_query_param(headers: &HeaderMap, name: &str) -> Option<String> {
    let location = header_value(headers, "location")?;
    let url = url::Url::parse(&location).ok()?;

    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn guild(id: &str) -> Guild {
    Guild {
        id: GuildId::parse(id).expect("identifiant de guilde valide"),
        name: Some(format!("Guilde {id}")),
        busy_until: None,
        default_media_time: Some(7),
        max_media_time: Some(120),
        display_media_full: true,
    }
}

/// Crée une guilde indépendante liée à `discord_guild_id`.
pub(super) async fn create_linked_standalone(db: &Db, guild_id: &GuildId, discord_guild_id: &str) {
    db.guilds()
        .create_standalone(guild_id, "Indépendante", UtcDateTime::now())
        .await
        .expect("guilde indépendante");
    let discord_guild_id = GuildId::parse(discord_guild_id).expect("serveur Discord valide");
    assert!(db
        .guilds()
        .link_discord(guild_id, Some(&discord_guild_id))
        .await
        .expect("liaison Discord"));
}

pub(super) fn media_asset(size_bytes: i64, storage_path: &Path) -> MediaAsset {
    let now = UtcDateTime::now();
    MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: format!("hash-{}", MediaAssetId::new_v4()),
        source_url: "https://example.invalid/media.png".to_owned(),
        source_title: None,
        kind: MediaKind::Image,
        mime: "image/png".to_owned(),
        duration_sec: None,
        width: Some(640),
        height: Some(360),
        is_vertical: false,
        storage_path: storage_path.to_string_lossy().into_owned(),
        size_bytes,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: now,
        expires_at: now + Duration::hours(24),
        created_at: now,
    }
}

pub(super) fn board_item(guild_id: &GuildId, media_asset_id: MediaAssetId) -> MediaBoardItem {
    MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id,
        title: Some("Titre".to_owned()),
        message: None,
        created_by_discord_user_id: Some("1001".to_owned()),
        created_by_name: Some("Alice".to_owned()),
        created_at: UtcDateTime::now(),
        artwork_asset_id: None,
        artwork_on_overlay: false,
        display_duration_sec: None,
        display_scale_percent: None,
    }
}

pub(super) fn playback_job(guild_id: &GuildId, status: JobStatus) -> PlaybackJob {
    let now = UtcDateTime::now();
    PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: None,
        text: None,
        show_text: false,
        author_name: Some("Alice".to_owned()),
        author_image: None,
        duration_sec: 12,
        source: lc_domain::enums::JobSource::Board,
        status,
        resume_offset_sec: 0,
        last_playback_state_at: None,
        submission_date: now,
        execution_date: now,
        scheduled_at: now,
        started_at: None,
        finished_at: None,
    }
}

pub(super) fn overlay_client(
    guild_id: &GuildId,
    session_mode: OverlaySessionMode,
) -> OverlayClient {
    OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: "Salon principal".to_owned(),
        token_hash: format!("hash-{}", OverlayClientId::new_v4()),
        session_mode,
        default_author_name: Some("Alice".to_owned()),
        default_author_image: None,
        created_by_discord_user_id: Some("1001".to_owned()),
        device_hash: None,
        last_seen_at: None,
        created_at: UtcDateTime::now(),
        revoked_at: None,
    }
}

fn ingest_client(guild_id: &GuildId, discord_user_id: &str, label: &str) -> IngestClient {
    IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: label.to_owned(),
        token_hash: format!("hash-{}", IngestClientId::new_v4()),
        default_author_name: "Alice".to_owned(),
        default_author_image: None,
        created_by_discord_user_id: discord_user_id.to_owned(),
        last_seen_at: None,
        created_at: UtcDateTime::now(),
        revoked_at: None,
    }
}

fn pairing_code(
    code: &str,
    guild_id: &GuildId,
    expires_at: Option<time::UtcDateTime>,
    used_at: Option<time::UtcDateTime>,
    created_by: &str,
) -> PairingCodeEntry {
    PairingCodeEntry {
        code: PairingCode::parse(code).expect("code de pairing valide"),
        guild_id: guild_id.clone(),
        mode: PairingMode::Normal,
        grants_admin: false,
        created_by_discord_user_id: created_by.to_owned(),
        author_name: Some("Alice".to_owned()),
        author_image: None,
        expires_at,
        used_at,
        created_at: UtcDateTime::now(),
    }
}

pub(super) fn guild_member(guild_id: &GuildId, role: GuildRole) -> GuildMember {
    GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild_id.clone(),
        display_name: "Alice".to_owned(),
        avatar: None,
        role,
        created_at: UtcDateTime::now(),
        removed_at: None,
        discord_user_id: None,
        profile_custom: true,
        discord_synced_at: None,
    }
}

pub(super) fn guild_invite(code: &str, guild_id: &GuildId) -> GuildInvite {
    GuildInvite {
        code: PairingCode::parse(code).expect("code d'invitation valide"),
        guild_id: guild_id.clone(),
        role: GuildRole::Member,
        member_id: None,
        max_uses: Some(1),
        use_count: 0,
        created_by_member_id: None,
        expires_at: None,
        revoked_at: None,
        created_at: UtcDateTime::now(),
    }
}

// ---------------------------------------------------------------------------
// Authentification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_503_when_no_method_is_configured() {
    let harness = Harness::with_pairs(&[("ADMIN_UI_TOKEN", "")]).await;

    for (method, path) in [
        (Method::GET, "/admin/api/overview"),
        (Method::POST, "/admin/api/guilds/42/stop"),
        (Method::POST, "/admin/auth/secret"),
        (Method::GET, "/admin/auth/discord"),
        (Method::POST, "/admin/auth/logout"),
    ] {
        let (status, body) = harness.call(method.clone(), path).await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "route {method} {path}"
        );
        assert_eq!(body["error"], "admin_ui_disabled");
    }

    // The session endpoint is the sole exception: it still reports which
    // methods are available (none here).
    let (status, _, bytes) = harness
        .send(harness.request_without_session(Method::GET, "/admin/auth/session"))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let body = json_or_null(&bytes);
    assert_eq!(body["error"], "unauthorized");
    assert_eq!(
        body["methods"],
        json!({ "discord": false, "secret": false })
    );
}

#[tokio::test]
async fn auth_403_pour_un_client_non_loopback() {
    let harness = Harness::new().await;
    let request = harness.request_from(
        Method::GET,
        "/admin/api/overview",
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
    );

    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json_or_null(&bytes)["error"], "admin_local_only");
}

#[tokio::test]
async fn auth_401_without_session_and_bearer_ignored() {
    let harness = Harness::new().await;
    let (status, _, bytes) = harness
        .send(harness.request_without_session(Method::GET, "/admin/api/overview"))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json_or_null(&bytes)["error"], "unauthorized");

    // The legacy Bearer token is gone entirely.
    let mut request = harness.request_without_session(Method::GET, "/admin/api/overview");
    request.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&format!("Bearer {TOKEN}")).expect("en-tête valide"),
    );
    let (status, _, bytes) = harness.send(request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json_or_null(&bytes)["error"], "unauthorized");

    // The session endpoint reports the configured methods.
    let (status, _, bytes) = harness
        .send(harness.request_without_session(Method::GET, "/admin/auth/session"))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let body = json_or_null(&bytes);
    assert_eq!(body["error"], "unauthorized");
    assert_eq!(body["methods"], json!({ "discord": false, "secret": true }));
}

#[tokio::test]
async fn auth_secret_login_sets_cookie_and_returns_session() {
    let harness = Harness::new().await;
    let (status, headers, bytes) = harness
        .send(harness.request_json_without_session(
            Method::POST,
            "/admin/auth/secret",
            &json!({ "secret": TOKEN }),
        ))
        .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(bytes.is_empty());
    let headers_for_cookie = set_cookie_headers(&headers, session::SESSION_COOKIE_NAME);
    assert_eq!(headers_for_cookie.len(), 1, "un seul cookie de session");
    let cookie = headers_for_cookie[0].as_str();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Path=/admin"));
    assert!(cookie.contains("Max-Age=604800"));
    assert!(!cookie.contains("Secure"), "API_URL est en http");

    let token = set_cookie_value(&headers, session::SESSION_COOKIE_NAME).expect("token de session");
    assert!(!token.is_empty());
    let stored: String =
        sqlx::query_scalar("SELECT token_hash FROM admin_sessions WHERE token_hash = ?1")
            .bind(sha256_hex(&token))
            .fetch_one(harness.db().pool())
            .await
            .expect("hash stocké");
    assert_eq!(stored, sha256_hex(&token), "seul le SHA-256 est stocké");
    assert_ne!(stored, token);

    let session_request = harness.request_with_cookie(
        Method::GET,
        "/admin/auth/session",
        &format!("{}={token}", session::SESSION_COOKIE_NAME),
    );
    let (status, _, bytes) = harness.send(session_request).await;
    assert_eq!(status, StatusCode::OK);
    let body = json_or_null(&bytes);
    assert_eq!(body["method"], "secret");
    assert!(body["user"].is_null());
    assert!(body["expiresAt"]
        .as_str()
        .is_some_and(|date| date.ends_with('Z')));
}

#[tokio::test]
async fn auth_secret_login_rejects_a_wrong_secret() {
    let harness = Harness::new().await;
    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/auth/secret",
            &json!({ "secret": "mauvais-secret" }),
        )
        .await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "invalid_secret");
}

#[tokio::test]
async fn auth_secret_login_is_404_when_only_disabled() {
    let harness = Harness::with_pairs(&[
        ("ADMIN_UI_TOKEN", ""),
        ("DISCORD_CLIENT_ID", "123456789012345678"),
        ("DISCORD_CLIENT_SECRET", "oauth-secret"),
        ("ADMIN_DISCORD_USER_IDS", "1001"),
    ])
    .await;

    let (status, _, body) = harness
        .send(harness.request_json_without_session(
            Method::POST,
            "/admin/auth/secret",
            &json!({ "secret": TOKEN }),
        ))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_or_null(&body)["error"], "method_disabled");
}

#[tokio::test]
async fn auth_logout_deletes_the_session_and_clears_the_cookie() {
    let harness = Harness::new().await;
    let token = harness.session_token.clone().expect("session du harnais");
    let (status, headers, _) = harness
        .send(harness.request_with_origin(Method::POST, "/admin/auth/logout", Some(TEST_ORIGIN)))
        .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    let cleared = set_cookie_headers(&headers, session::SESSION_COOKIE_NAME);
    assert_eq!(cleared.len(), 1);
    assert!(cleared[0].contains("Max-Age=0"));

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_sessions WHERE token_hash = ?1")
            .bind(sha256_hex(&token))
            .fetch_one(harness.db().pool())
            .await
            .expect("comptage des sessions");
    assert_eq!(count, 0, "la session est supprimée");

    let (status, _, _) = harness
        .send(harness.request_with_cookie(
            Method::GET,
            "/admin/auth/session",
            &format!("{}={token}", session::SESSION_COOKIE_NAME),
        ))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_expired_session_is_refused_and_deleted() {
    let harness = Harness::new().await;
    let token = harness.session_token.clone().expect("session du harnais");
    sqlx::query("UPDATE admin_sessions SET expires_at = 0 WHERE token_hash = ?1")
        .bind(sha256_hex(&token))
        .execute(harness.db().pool())
        .await
        .expect("expiration forcée");

    let (status, _, _) = harness
        .send(harness.request_with_cookie(
            Method::GET,
            "/admin/auth/session",
            &format!("{}={token}", session::SESSION_COOKIE_NAME),
        ))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_sessions WHERE token_hash = ?1")
            .bind(sha256_hex(&token))
            .fetch_one(harness.db().pool())
            .await
            .expect("comptage des sessions");
    assert_eq!(count, 0, "la session expirée est supprimée à la lecture");
}

#[tokio::test]
async fn auth_expired_sessions_are_purged_at_startup() {
    let harness = Harness::new().await;
    let token = harness.session_token.clone().expect("session du harnais");
    sqlx::query("UPDATE admin_sessions SET expires_at = 0 WHERE token_hash = ?1")
        .bind(sha256_hex(&token))
        .execute(harness.db().pool())
        .await
        .expect("expiration forcée");

    let removed = super::purge_expired_sessions(harness.db())
        .await
        .expect("nettoyage de démarrage");
    assert_eq!(removed, 1);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admin_sessions")
        .fetch_one(harness.db().pool())
        .await
        .expect("comptage des sessions");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn auth_session_cookie_has_the_contract_attributes() {
    let harness = Harness::with_pairs(&[("API_URL", "https://panel.example")]).await;
    let (status, headers, _) = harness
        .send(harness.request_json_without_session(
            Method::POST,
            "/admin/auth/secret",
            &json!({ "secret": TOKEN }),
        ))
        .await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    let cookie = set_cookie_headers(&headers, session::SESSION_COOKIE_NAME);
    assert_eq!(cookie.len(), 1);
    for attribute in [
        "HttpOnly",
        "SameSite=Strict",
        "Path=/admin",
        "Max-Age=604800",
        "Secure",
    ] {
        assert!(
            cookie[0].contains(attribute),
            "attribut {attribute} absent de {}",
            cookie[0]
        );
    }
}

#[tokio::test]
async fn auth_secret_and_logout_require_an_origin() {
    let harness = Harness::new().await;

    let secret_request = harness.request_json_without_session(
        Method::POST,
        "/admin/auth/secret",
        &json!({ "secret": TOKEN }),
    );
    let mut secret_request = secret_request;
    secret_request.headers_mut().remove(header::ORIGIN);
    let (status, _, bytes) = harness.send(secret_request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json_or_null(&bytes)["error"], "invalid_origin");

    let logout_request = harness.request_with_origin(Method::POST, "/admin/auth/logout", None);
    let (status, _, bytes) = harness.send(logout_request).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json_or_null(&bytes)["error"], "invalid_origin");
}

fn discord_identity(id: &str, name: &str) -> DiscordIdentity {
    DiscordIdentity {
        id: id.to_owned(),
        name: name.to_owned(),
        avatar_url: Some(format!(
            "https://cdn.discordapp.com/avatars/{id}/avatar-hash.png"
        )),
    }
}

async fn discord_harness(oauth: Arc<dyn AdminOAuth>, allowed_ids: &str) -> Harness {
    Harness::with_pairs_and_oauth(
        &[
            ("DISCORD_CLIENT_ID", "123456789012345678"),
            ("DISCORD_CLIENT_SECRET", "oauth-secret"),
            ("ADMIN_DISCORD_USER_IDS", allowed_ids),
        ],
        oauth,
    )
    .await
}

#[tokio::test]
async fn auth_discord_login_redirects_with_state_and_scope() {
    let oauth = FakeOAuth::returning(discord_identity("1001", "Alice"));
    let harness = discord_harness(oauth.clone(), "1001").await;

    let (status, headers, _) = harness
        .send(harness.request_without_session(Method::GET, "/admin/auth/discord"))
        .await;

    assert_eq!(status, StatusCode::FOUND);
    let location = header_value(&headers, "location").expect("Location");
    let url = url::Url::parse(&location).expect("URL Discord");
    assert_eq!(url.host_str(), Some("discord.com"));
    assert_eq!(url.path(), "/oauth2/authorize");
    assert_eq!(
        location_query_param(&headers, "client_id").as_deref(),
        Some("123456789012345678")
    );
    assert_eq!(
        location_query_param(&headers, "scope").as_deref(),
        Some("identify")
    );
    assert_eq!(
        location_query_param(&headers, "redirect_uri").as_deref(),
        Some("http://localhost:3000/admin/auth/discord/callback")
    );
    let state = location_query_param(&headers, "state").expect("state");
    assert_eq!(state.len(), 43);

    let state_cookies = set_cookie_headers(&headers, session::OAUTH_STATE_COOKIE_NAME);
    assert_eq!(state_cookies.len(), 1);
    for attribute in [
        "HttpOnly",
        "SameSite=Lax",
        "Path=/admin/auth",
        "Max-Age=600",
    ] {
        assert!(
            state_cookies[0].contains(attribute),
            "attribut {attribute} absent de {}",
            state_cookies[0]
        );
    }
    assert_eq!(
        set_cookie_value(&headers, session::OAUTH_STATE_COOKIE_NAME).as_deref(),
        Some(state.as_str())
    );
    assert!(oauth.calls().is_empty(), "aucun échange avant le callback");
}

#[tokio::test]
async fn auth_discord_callback_bad_state_redirects_to_login() {
    let oauth = FakeOAuth::returning(discord_identity("1001", "Alice"));
    let harness = discord_harness(oauth.clone(), "1001").await;

    for cookie in [None, Some("lc_admin_oauth_state=attendu")] {
        let request = match cookie {
            Some(cookie) => harness.request_with_cookie(
                Method::GET,
                "/admin/auth/discord/callback?code=code-1&state=fourni",
                cookie,
            ),
            None => harness.request_without_session(
                Method::GET,
                "/admin/auth/discord/callback?code=code-1&state=fourni",
            ),
        };
        let (status, headers, _) = harness.send(request).await;
        assert_eq!(status, StatusCode::FOUND);
        assert_eq!(
            header_value(&headers, "location").as_deref(),
            Some("/admin/#/login?error=oauth_state")
        );
    }
    assert!(oauth.calls().is_empty(), "le code n'est jamais échangé");
}

#[tokio::test]
async fn auth_discord_callback_oauth_failed_redirects_to_login() {
    let oauth = FakeOAuth::failing();
    let harness = discord_harness(oauth.clone(), "1001").await;
    let request = harness.request_with_cookie(
        Method::GET,
        "/admin/auth/discord/callback?code=code-1&state=state-1",
        "lc_admin_oauth_state=state-1",
    );

    let (status, headers, _) = harness.send(request).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        header_value(&headers, "location").as_deref(),
        Some("/admin/#/login?error=oauth_failed")
    );
    assert_eq!(
        oauth.calls(),
        vec![(
            "code-1".to_owned(),
            "http://localhost:3000/admin/auth/discord/callback".to_owned()
        )]
    );
    assert!(
        set_cookie_headers(&headers, session::SESSION_COOKIE_NAME).is_empty(),
        "aucune session en cas d'échec"
    );
}

#[tokio::test]
async fn auth_discord_callback_rejects_an_id_outside_the_list() {
    let oauth = FakeOAuth::returning(discord_identity("9999", "Mallory"));
    let harness = discord_harness(oauth, "1001").await;
    let request = harness.request_with_cookie(
        Method::GET,
        "/admin/auth/discord/callback?code=code-1&state=state-1",
        "lc_admin_oauth_state=state-1",
    );

    let (status, headers, _) = harness.send(request).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        header_value(&headers, "location").as_deref(),
        Some("/admin/#/login?error=not_allowed")
    );
    assert!(
        set_cookie_headers(&headers, session::SESSION_COOKIE_NAME).is_empty(),
        "aucune session pour un id refusé"
    );
}

#[tokio::test]
async fn auth_discord_callback_allowed_creates_a_session() {
    let identity = discord_identity("1001", "Alice");
    let oauth = FakeOAuth::returning(identity.clone());
    let harness = discord_harness(oauth.clone(), "1001").await;
    let request = harness.request_with_cookie(
        Method::GET,
        "/admin/auth/discord/callback?code=code-1&state=state-1",
        "lc_admin_oauth_state=state-1",
    );

    let (status, headers, _) = harness.send(request).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        header_value(&headers, "location").as_deref(),
        Some("/admin/#/dashboard")
    );
    let cookies = set_cookie_headers(&headers, session::SESSION_COOKIE_NAME);
    assert_eq!(cookies.len(), 1);
    for attribute in [
        "HttpOnly",
        "SameSite=Strict",
        "Path=/admin",
        "Max-Age=604800",
    ] {
        assert!(cookies[0].contains(attribute), "attribut {attribute}");
    }
    let token = set_cookie_value(&headers, session::SESSION_COOKIE_NAME).expect("token");
    assert_eq!(
        oauth.calls(),
        vec![(
            "code-1".to_owned(),
            "http://localhost:3000/admin/auth/discord/callback".to_owned()
        )]
    );

    let session_request = harness.request_with_cookie(
        Method::GET,
        "/admin/auth/session",
        &format!("{}={token}", session::SESSION_COOKIE_NAME),
    );
    let (status, _, bytes) = harness.send(session_request).await;
    assert_eq!(status, StatusCode::OK);
    let body = json_or_null(&bytes);
    assert_eq!(body["method"], "discord");
    assert_eq!(body["user"]["id"], identity.id);
    assert_eq!(body["user"]["name"], identity.name);
    assert_eq!(
        body["user"]["avatarUrl"].as_str(),
        identity.avatar_url.as_deref()
    );
}

/// `(actor_method, actor_name, action, status)` of every audit row, oldest first.
async fn login_audit_rows(harness: &Harness) -> Vec<(Option<String>, Option<String>, String, i64)> {
    sqlx::query_as("SELECT actor_method, actor_name, action, status FROM admin_audit ORDER BY id")
        .fetch_all(harness.db().pool())
        .await
        .expect("audit rows")
}

#[tokio::test]
async fn a_secret_login_is_journaled_with_its_actor() {
    let harness = Harness::new().await;
    let (status, _, _) = harness
        .send(harness.request_json_without_session(
            Method::POST,
            "/admin/auth/secret",
            &json!({ "secret": TOKEN }),
        ))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(
        login_audit_rows(&harness).await,
        vec![(
            Some("secret".to_owned()),
            Some("secret".to_owned()),
            "auth.login_secret".to_owned(),
            204
        )]
    );
}

#[tokio::test]
async fn a_discord_login_is_journaled_with_the_discord_name() {
    let oauth = FakeOAuth::returning(discord_identity("1001", "Alice"));
    let harness = discord_harness(oauth, "1001").await;
    let (status, _, _) = harness
        .send(harness.request_with_cookie(
            Method::GET,
            "/admin/auth/discord/callback?code=code-1&state=state-1",
            "lc_admin_oauth_state=state-1",
        ))
        .await;
    assert_eq!(status, StatusCode::FOUND);

    assert_eq!(
        login_audit_rows(&harness).await,
        vec![(
            Some("discord".to_owned()),
            Some("Alice".to_owned()),
            "auth.login_discord".to_owned(),
            200
        )]
    );
}

#[tokio::test]
async fn a_refused_discord_account_is_journaled_by_name() {
    let oauth = FakeOAuth::returning(discord_identity("9999", "Mallory"));
    let harness = discord_harness(oauth, "1001").await;
    harness
        .send(harness.request_with_cookie(
            Method::GET,
            "/admin/auth/discord/callback?code=code-1&state=state-1",
            "lc_admin_oauth_state=state-1",
        ))
        .await;

    assert_eq!(
        login_audit_rows(&harness).await,
        vec![(
            Some("discord".to_owned()),
            Some("Mallory".to_owned()),
            "auth.login_discord".to_owned(),
            403
        )]
    );
}

#[tokio::test]
async fn failed_discord_callbacks_are_journaled_without_an_actor() {
    let harness = discord_harness(FakeOAuth::failing(), "1001").await;
    // Bad `state`, then a code Discord refuses to exchange.
    harness
        .send(harness.request_without_session(
            Method::GET,
            "/admin/auth/discord/callback?code=code-1&state=state-1",
        ))
        .await;
    harness
        .send(harness.request_with_cookie(
            Method::GET,
            "/admin/auth/discord/callback?code=code-1&state=state-1",
            "lc_admin_oauth_state=state-1",
        ))
        .await;
    // Starting the login is not journaled: nothing happened yet.
    harness
        .send(harness.request_without_session(Method::GET, "/admin/auth/discord"))
        .await;

    assert_eq!(
        login_audit_rows(&harness).await,
        vec![
            (None, None, "auth.login_discord".to_owned(), 400),
            (None, None, "auth.login_discord".to_owned(), 502),
        ]
    );
}

#[tokio::test]
async fn trusted_proxy_ignores_a_forged_first_forwarded_hop() {
    let harness = Harness::with_pairs(&[
        ("ADMIN_TRUST_FORWARDED_FOR", "true"),
        ("TRUSTED_PROXIES", "127.0.0.1"),
    ])
    .await;
    let proxy = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

    // The client forges `127.0.0.1`; the proxy appends the real address.
    let mut forged = harness.request_from(Method::GET, "/admin/api/overview", proxy);
    forged.headers_mut().insert(
        "x-forwarded-for",
        header::HeaderValue::from_static("127.0.0.1, 203.0.113.9"),
    );
    let (status, _, bytes) = harness.send(forged).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json_or_null(&bytes)["error"], "admin_local_only");

    // `X-Real-IP` set by the proxy decides when present.
    let mut local = harness.request_from(Method::GET, "/admin/api/overview", proxy);
    local.headers_mut().insert(
        "x-forwarded-for",
        header::HeaderValue::from_static("203.0.113.9, 127.0.0.1"),
    );
    local
        .headers_mut()
        .insert("x-real-ip", header::HeaderValue::from_static("127.0.0.1"));
    let (status, _, _) = harness.send(local).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn forged_real_ip_from_an_undeclared_proxy_is_not_local() {
    let harness = Harness::with_pairs(&[("ADMIN_TRUST_FORWARDED_FOR", "true")]).await;
    let remote = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9));

    // `X-Real-IP` alone, then the last `X-Forwarded-For` hop: neither header
    // is readable from a peer that is not declared in `TRUSTED_PROXIES`.
    for (name, value) in [
        ("x-real-ip", "127.0.0.1"),
        ("x-forwarded-for", "203.0.113.9, 127.0.0.1"),
    ] {
        let mut request = harness.request_from(Method::GET, "/admin/api/overview", remote);
        request
            .headers_mut()
            .insert(name, header::HeaderValue::from_static(value));

        let (status, _, bytes) = harness.send(request).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "en-tête {name}");
        assert_eq!(json_or_null(&bytes)["error"], "admin_local_only");
    }
}

#[tokio::test]
async fn declared_proxy_without_a_forwarded_address_fails_closed() {
    // The direct peer is declared in `TRUSTED_PROXIES` and
    // `ADMIN_TRUST_FORWARDED_FOR` is on, but the request announces no usable
    // client address. The proxy speaks for nobody: the check must fail closed
    // instead of judging the proxy's own (loopback) address.
    let harness = Harness::with_pairs(&[
        ("ADMIN_TRUST_FORWARDED_FOR", "true"),
        ("TRUSTED_PROXIES", "127.0.0.1"),
    ])
    .await;
    let proxy = IpAddr::V4(Ipv4Addr::LOCALHOST);

    // Both code paths apply the check: the `/admin/api/**` extractor and the
    // standalone auth routes.
    for path in ["/admin/api/overview", "/admin/auth/session"] {
        // No `X-Real-IP` and no `X-Forwarded-For`.
        let request = harness.request_from(Method::GET, path, proxy);
        let (status, _, bytes) = harness.send(request).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} depuis le proxy déclaré sans en-tête"
        );
        assert_eq!(json_or_null(&bytes)["error"], "admin_local_only");

        // An unparseable forwarded address must not fall back either.
        let mut request = harness.request_from(Method::GET, path, proxy);
        request
            .headers_mut()
            .insert("x-real-ip", header::HeaderValue::from_static("not-an-ip"));
        let (status, _, bytes) = harness.send(request).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} depuis le proxy déclaré avec un en-tête inexploitable"
        );
        assert_eq!(json_or_null(&bytes)["error"], "admin_local_only");
    }
}

#[tokio::test]
async fn auth_local_only_desactive_accepte_un_client_externe() {
    let harness = Harness::with_pairs(&[("ADMIN_UI_LOCAL_ONLY", "false")]).await;
    let request = harness.request_from(
        Method::GET,
        "/admin/api/ingest-authors",
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
    );

    let (status, _, _) = harness.send(request).await;

    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn csrf_mutation_sans_origine_refusee() {
    let harness = Harness::new().await;
    let request = harness.request_with_origin(Method::POST, "/admin/api/guilds/42/stop", None);

    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json_or_null(&bytes)["error"], "invalid_origin");
}

#[tokio::test]
async fn csrf_mutation_origine_hostile_refusee() {
    let harness = Harness::new().await;

    // Origine étrangère, opaque (`null`) ou préfixe trompeur : toutes refusées.
    for origin in [
        "https://evil.example",
        "null",
        "http://127.0.0.1:40000.evil.example",
    ] {
        let request =
            harness.request_with_origin(Method::POST, "/admin/api/guilds/42/stop", Some(origin));

        let (status, _, bytes) = harness.send(request).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "origine refusée : {origin}");
        assert_eq!(json_or_null(&bytes)["error"], "invalid_origin");
    }
}

#[tokio::test]
async fn csrf_referer_origine_identique_accepte() {
    let harness = Harness::new().await;
    let request = harness.request_with_referer(
        Method::POST,
        "/admin/api/guilds/42/stop",
        "http://127.0.0.1:40000/admin/",
    );

    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_or_null(&bytes)["stopped"], true);
}

#[tokio::test]
async fn csrf_origine_api_url_acceptee_derriere_proxy() {
    // Derrière un reverse proxy, `Host` peut être réécrit : l'origine publique
    // `API_URL` reste alors la référence acceptée.
    let harness = Harness::with_pairs(&[("API_URL", "http://proxy.example")]).await;
    let request = harness.request_with_origin(
        Method::POST,
        "/admin/api/guilds/42/stop",
        Some("http://proxy.example"),
    );

    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_or_null(&bytes)["stopped"], true);
}

#[tokio::test]
async fn csrf_get_avec_origine_hostile_reste_accepte() {
    // Le contrôle d'origine ne s'applique qu'aux mutations (GET non protégé par
    // CSRF par construction).
    let harness = Harness::new().await;
    let request = harness.request_with_origin(
        Method::GET,
        "/admin/api/overview",
        Some("https://evil.example"),
    );

    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert!(json_or_null(&bytes)["generatedAt"].is_string());
}

#[tokio::test]
async fn page_admin_publique_mais_api_protegee() {
    let harness = Harness::with_pairs(&[("ADMIN_UI_TOKEN", "")]).await;

    let request = HttpRequest::builder()
        .method(Method::GET)
        .uri("/admin/")
        .body(Body::empty())
        .expect("requête valide");
    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::OK);
    assert!(text(&bytes).contains("LiveChat"));
}

// ---------------------------------------------------------------------------
// Assets statiques et en-têtes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn statique_redirige_vers_le_slash_final() {
    let harness = Harness::new().await;

    let (status, headers, _) = harness.call_with_headers(Method::GET, "/admin").await;

    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        header_value(&headers, "location").as_deref(),
        Some("/admin/")
    );
}

#[tokio::test]
async fn statique_sert_index_avec_no_store_et_csp_admin() {
    let harness = Harness::new().await;

    let (status, headers, bytes) = harness.call_with_headers(Method::GET, "/admin/").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_value(&headers, "content-type").as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        header_value(&headers, "cache-control").as_deref(),
        Some("no-store")
    );
    assert_eq!(
        header_value(&headers, "content-security-policy").as_deref(),
        Some(ADMIN_CSP)
    );
    // Les en-têtes autres que la CSP sont posés par la couche globale
    // (`http::security`), exécutée après : la Permissions-Policy effective est
    // donc celle du serveur, un sur-ensemble de celle du panneau.
    assert_eq!(
        header_value(&headers, "permissions-policy").as_deref(),
        Some(PERMISSIONS_POLICY)
    );
    assert_eq!(
        header_value(&headers, "x-frame-options").as_deref(),
        Some("DENY")
    );
    assert!(text(&bytes).contains("/admin/assets/"));
}

#[tokio::test]
async fn statique_sert_les_assets_hashes_avec_mime_et_cache_immutable() {
    let harness = Harness::new().await;
    let assets = super::static_files::embedded_files();
    let javascript = assets
        .iter()
        .find(|path| path.starts_with("assets/") && path.ends_with(".js"))
        .expect("the built console ships a hashed JavaScript asset");
    let stylesheet = assets
        .iter()
        .find(|path| path.starts_with("assets/") && path.ends_with(".css"))
        .expect("the built console ships a hashed CSS asset");

    for (asset, content_type) in [
        (javascript, "text/javascript; charset=utf-8"),
        (stylesheet, "text/css; charset=utf-8"),
    ] {
        let (status, headers, bytes) = harness
            .call_with_headers(Method::GET, &format!("/admin/{asset}"))
            .await;
        assert_eq!(status, StatusCode::OK, "asset {asset}");
        assert_eq!(
            header_value(&headers, "content-type").as_deref(),
            Some(content_type),
            "MIME type of {asset}"
        );
        assert_eq!(
            header_value(&headers, "cache-control").as_deref(),
            Some("public, max-age=31536000, immutable"),
            "cache policy of {asset}"
        );
        assert!(!bytes.is_empty(), "asset {asset} is not empty");
        // Security headers are not lost on immutable assets.
        assert_eq!(
            header_value(&headers, "content-security-policy").as_deref(),
            Some(ADMIN_CSP)
        );
        assert_eq!(
            header_value(&headers, "x-content-type-options").as_deref(),
            Some("nosniff")
        );
        assert_eq!(
            header_value(&headers, "x-frame-options").as_deref(),
            Some("DENY")
        );
    }
}

#[tokio::test]
async fn statique_chemin_inconnu_renvoie_404_sans_index() {
    let harness = Harness::new().await;

    let (status, headers, bytes) = harness
        .call_with_headers(Method::GET, "/admin/assets/missing-file.js")
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        header_value(&headers, "cache-control").as_deref(),
        Some("no-store")
    );
    assert!(
        !text(&bytes).contains("<!doctype html>"),
        "an unknown asset must never fall back to index.html"
    );
}

#[tokio::test]
async fn csp_admin_et_csp_globale_ne_se_recouvrent_pas() {
    let harness = Harness::new().await;

    let (_, admin_headers, _) = harness.call_with_headers(Method::GET, "/admin/").await;
    let (_, health_headers, _) = harness.call_with_headers(Method::GET, "/health").await;

    assert_eq!(
        header_value(&admin_headers, "content-security-policy").as_deref(),
        Some(ADMIN_CSP)
    );
    assert_eq!(
        header_value(&health_headers, "content-security-policy").as_deref(),
        Some(CONTENT_SECURITY_POLICY)
    );
    assert_ne!(ADMIN_CSP, CONTENT_SECURITY_POLICY);
    assert_eq!(
        header_value(&health_headers, "permissions-policy").as_deref(),
        Some(PERMISSIONS_POLICY)
    );
    assert_eq!(
        header_value(&health_headers, "cache-control"),
        None,
        "la CSP globale ne doit pas hériter du no-store admin"
    );
}

// ---------------------------------------------------------------------------
// Réglages runtime (TIKTOK_COOKIE)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn runtime_settings_valeurs_par_defaut_puis_persistance() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], false);

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "tiktokCookie": "  cookie: abc123  " }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updated"], true);
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], true);

    // Read back: only the presence flag is exposed.
    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], true);

    let settings_file = harness.path(SETTINGS_FILE_NAME);
    assert!(settings_file.is_file(), "admin_runtime.json doit être créé");
    let contents = std::fs::read_to_string(&settings_file).expect("fichier lisible");
    assert!(contents.contains("abc123"));
    assert!(
        !harness.path(&format!("{SETTINGS_FILE_NAME}.tmp")).exists(),
        "aucun fichier temporaire ne doit rester"
    );
}

#[tokio::test]
async fn runtime_settings_never_expose_the_tiktok_cookie() {
    let harness = Harness::with_pairs(&[("TIKTOK_COOKIE", "sessionid=environment-secret")]).await;

    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], true);
    assert!(
        !body.to_string().contains("environment-secret"),
        "the environment cookie leaked in the runtime settings"
    );

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "tiktokCookie": "sessionid=runtime-secret" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], true);
    assert!(
        !body.to_string().contains("runtime-secret"),
        "the patched cookie leaked in the PATCH response"
    );

    // Clearing the override must also hide the environment cookie: the empty
    // runtime value is authoritative, not a signal to fall back to TIKTOK_COOKIE.
    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "tiktokCookie": "" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], false);

    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("tiktokCookie").is_none());
    assert_eq!(body["hasTikTokCookie"], false);
    assert!(
        !body.to_string().contains("environment-secret"),
        "the environment cookie leaked after the runtime cookie was cleared"
    );
}

#[tokio::test]
async fn runtime_settings_payload_invalide() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call_json(Method::PATCH, "/admin/api/runtime-settings", &json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/runtime-settings",
            &json!({ "tiktokCookie": 42 }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
}

// ---------------------------------------------------------------------------
// Aperçu (overview)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overview_vide_renvoie_les_bornes_de_configuration() {
    let harness = Harness::new().await;

    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["guilds"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(body["totals"]["guildCount"], 0);
    assert_eq!(body["totals"]["overlayClientCount"], 0);
    assert_eq!(body["totals"]["overlayConnectedCount"], 0);
    assert_eq!(body["totals"]["ingestClientCount"], 0);
    assert_eq!(body["totals"]["pairingCodeCount"], 0);
    assert_eq!(body["pairingCodes"]["active"], 0);
    assert_eq!(body["pairingCodes"]["expired"], 0);
    assert_eq!(body["cache"]["usedBytes"], 0);
    assert_eq!(body["cache"]["maxBytes"], 5120_i64 * 1024 * 1024);
    assert_eq!(body["cache"]["remainingBytes"], 5120_i64 * 1024 * 1024);
    assert_eq!(body["cache"]["usedPercent"], 0.0);
    let generated_at = body["generatedAt"].as_str().expect("date ISO");
    assert!(
        generated_at.ends_with('Z'),
        "date UTC ISO 8601 : {generated_at}"
    );
}

#[tokio::test]
async fn overview_agrege_guildes_medias_et_playback() {
    let harness = Harness::new().await;
    let db = harness.db();
    let now = UtcDateTime::now();

    let guild_id = GuildId::parse("42").expect("guilde valide");
    db.guilds().upsert(&guild("42")).await.expect("guilde");

    // Média présent au board (1 Mio) + média en cache seul (2 Mio).
    let board_path = harness.path("board.png");
    std::fs::write(&board_path, b"board").expect("fichier board");
    let board_asset = media_asset(1_048_576, &board_path);
    let cache_path = harness.path("cache.png");
    std::fs::write(&cache_path, b"cache").expect("fichier cache");
    let cache_asset = media_asset(2_097_152, &cache_path);

    db.media_assets()
        .insert(&board_asset)
        .await
        .expect("média board");
    db.media_assets()
        .insert(&cache_asset)
        .await
        .expect("média cache");
    db.media_board()
        .create(&board_item(&guild_id, board_asset.id))
        .await
        .expect("élément de board");

    for status in [
        JobStatus::Pending,
        JobStatus::Playing,
        JobStatus::Failed,
        JobStatus::Done,
    ] {
        db.playback_jobs()
            .create(&playback_job(&guild_id, status))
            .await
            .expect("job de playback");
    }

    db.pairing_codes()
        .create(&pairing_code(
            "ACTIF001",
            &guild_id,
            Some(now + Duration::hours(1)),
            None,
            "1001",
        ))
        .await
        .expect("code actif");
    db.pairing_codes()
        .create(&pairing_code(
            "EXPIRE01",
            &guild_id,
            Some(now - Duration::hours(1)),
            None,
            "1001",
        ))
        .await
        .expect("code expiré");

    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::OK);

    let guilds = body["guilds"].as_array().expect("tableau des guildes");
    assert_eq!(guilds.len(), 1);
    let payload = &guilds[0];
    assert_eq!(payload["id"], "42");
    assert_eq!(payload["name"], "Guilde 42");
    assert_eq!(payload["editableName"], "Guilde 42");
    assert_eq!(payload["settings"]["defaultMediaTime"], 7);
    assert_eq!(payload["settings"]["maxMediaTime"], 120);
    assert_eq!(payload["settings"]["displayMediaFull"], true);
    assert_eq!(payload["settings"]["overrides"]["defaultMediaTime"], 7);
    assert_eq!(payload["settings"]["overrides"]["displayMediaFull"], true);
    assert_eq!(payload["mediaBoard"]["itemsCount"], 1);
    assert_eq!(payload["mediaBoard"]["usedBytes"], 1_048_576_i64);
    assert_eq!(payload["mediaBoard"]["maxBytes"], 15360_i64 * 1024 * 1024);
    assert_eq!(payload["playback"]["pending"], 1);
    assert_eq!(payload["playback"]["playing"], 1);
    assert_eq!(payload["playback"]["failed"], 1);
    assert_eq!(payload["playback"]["done"], 1);
    assert_eq!(payload["overlays"]["total"], 0);
    assert_eq!(payload["overlays"]["connectedCount"], 0);
    assert_eq!(payload["ingest"]["total"], 0);

    assert_eq!(body["totals"]["guildCount"], 1);
    assert_eq!(body["totals"]["pairingCodeCount"], 2);
    assert_eq!(body["pairingCodes"]["total"], 2);
    assert_eq!(body["pairingCodes"]["active"], 1);
    assert_eq!(body["pairingCodes"]["expired"], 1);
    // Cache : seul le média READY absent du board compte.
    assert_eq!(body["cache"]["usedBytes"], 2_097_152_i64);
}

#[tokio::test]
async fn overview_liste_les_clients_actifs_uniquement() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde valide");
    db.guilds().upsert(&guild("42")).await.expect("guilde");

    db.overlay_clients()
        .create(&overlay_client(
            &guild_id,
            OverlaySessionMode::InviteReadOnly,
        ))
        .await
        .expect("client overlay");
    let mut revoked_overlay = overlay_client(&guild_id, OverlaySessionMode::Normal);
    revoked_overlay.revoked_at = Some(UtcDateTime::now());
    db.overlay_clients()
        .create(&revoked_overlay)
        .await
        .expect("client overlay révoqué");

    db.ingest_clients()
        .create(&ingest_client(&guild_id, "1001", "Chrome"))
        .await
        .expect("client ingest");
    let mut revoked_ingest = ingest_client(&guild_id, "1002", "Firefox");
    revoked_ingest.revoked_at = Some(UtcDateTime::now());
    db.ingest_clients()
        .create(&revoked_ingest)
        .await
        .expect("client ingest révoqué");

    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::OK);

    let guilds = body["guilds"].as_array().expect("tableau des guildes");
    let payload = &guilds[0];
    assert_eq!(payload["overlays"]["total"], 1);
    assert_eq!(payload["overlays"]["connectedCount"], 0);
    assert_eq!(payload["overlays"]["clients"][0]["connected"], false);
    assert_eq!(
        payload["overlays"]["clients"][0]["sessionMode"],
        "invite_read_only"
    );
    assert_eq!(payload["ingest"]["total"], 1);
    assert_eq!(payload["ingest"]["clients"][0]["label"], "Chrome");
}

// ---------------------------------------------------------------------------
// Réglages de guilde
// ---------------------------------------------------------------------------

#[tokio::test]
async fn settings_patch_cree_puis_fusionne_les_champs() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/guilds/42/settings",
            &json!({ "defaultMediaTime": 9, "displayMediaFull": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updated"], true);
    assert_eq!(body["guild"]["id"], "42");
    assert_eq!(body["guild"]["defaultMediaTime"], 9);
    assert_eq!(body["guild"]["displayMediaFull"], true);
    assert_eq!(body["guild"]["maxMediaTime"], Value::Null);

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/guilds/42/settings",
            &json!({ "maxMediaTime": 300 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["guild"]["defaultMediaTime"], 9);
    assert_eq!(body["guild"]["maxMediaTime"], 300);

    let stored = harness
        .db()
        .guilds()
        .get(&GuildId::parse("42").expect("guilde"))
        .await
        .expect("lecture guilde")
        .expect("guilde présente");
    assert_eq!(stored.default_media_time, Some(9));
    assert_eq!(stored.max_media_time, Some(300));
    assert!(stored.display_media_full);
}

#[tokio::test]
async fn settings_patch_payload_et_identifiants_invalides() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call_json(Method::PATCH, "/admin/api/guilds/abc/settings", &json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_guild_id");

    let (status, body) = harness
        .call_json(Method::PATCH, "/admin/api/guilds/42/settings", &json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/guilds/42/settings",
            &json!({ "defaultMediaTime": "neuf" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            "/admin/api/guilds/42/settings",
            &json!({ "displayMediaFull": "oui" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
}

// ---------------------------------------------------------------------------
// Stop et purge d'une guilde
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_libere_les_jobs_playing() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    db.guilds().upsert(&guild("42")).await.expect("guilde");
    db.playback_jobs()
        .create(&playback_job(&guild_id, JobStatus::Playing))
        .await
        .expect("job playing");
    db.playback_jobs()
        .create(&playback_job(&guild_id, JobStatus::Pending))
        .await
        .expect("job pending");

    let (status, body) = harness
        .call(Method::POST, "/admin/api/guilds/42/stop")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["stopped"], true);
    assert_eq!(body["guildId"], "42");
    assert_eq!(body["releasedCount"], 1);

    let stored = db
        .guilds()
        .get(&guild_id)
        .await
        .expect("lecture guilde")
        .expect("guilde présente");
    assert_eq!(stored.busy_until, None);
}

#[tokio::test]
async fn purge_supprime_les_donnees_de_la_guilde_et_les_medias_orphelins() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    db.guilds().upsert(&guild("42")).await.expect("guilde");

    db.ingest_clients()
        .create(&ingest_client(&guild_id, "1001", "Chrome"))
        .await
        .expect("client ingest");
    db.overlay_clients()
        .create(&overlay_client(&guild_id, OverlaySessionMode::Normal))
        .await
        .expect("client overlay");
    db.pairing_codes()
        .create(&pairing_code("PURGE001", &guild_id, None, None, "1001"))
        .await
        .expect("code de pairing");
    db.playback_jobs()
        .create(&playback_job(&guild_id, JobStatus::Pending))
        .await
        .expect("job");

    // Média référencé par le board de la guilde purgée : après suppression des
    // éléments du board, il devient orphelin et son fichier est effacé (il est
    // sous MEDIA_STORAGE_DIR, seul endroit où la purge est autorisée à supprimer).
    let orphan_path = harness.path("media").join("orphelin.png");
    std::fs::write(&orphan_path, b"orphelin").expect("fichier orphelin");
    let orphan_asset = media_asset(1024, &orphan_path);
    db.media_assets()
        .insert(&orphan_asset)
        .await
        .expect("média orphelin");
    db.media_board()
        .create(&board_item(&guild_id, orphan_asset.id))
        .await
        .expect("élément de board orphelin");

    // Média référencé par le board d'une AUTRE guilde : il n'est pas candidat.
    let other_guild_id = GuildId::parse("43").expect("guilde");
    let kept_path = harness.path("conserve.png");
    std::fs::write(&kept_path, b"conserve").expect("fichier conservé");
    let kept_asset = media_asset(2048, &kept_path);
    db.media_assets()
        .insert(&kept_asset)
        .await
        .expect("média conservé");
    db.media_board()
        .create(&board_item(&other_guild_id, kept_asset.id))
        .await
        .expect("élément de board d'une autre guilde");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds/42/purge",
            &json!({ "confirmGuildId": "42" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["purged"], true);
    assert_eq!(body["deleted"]["guildSettings"], 1);
    assert_eq!(body["deleted"]["ingestClients"], 1);
    assert_eq!(body["deleted"]["overlayClients"], 1);
    assert_eq!(body["deleted"]["pairingCodes"], 1);
    assert_eq!(body["deleted"]["playbackJobs"], 1);
    assert_eq!(body["deleted"]["mediaBoardItems"], 1);
    assert_eq!(body["deleted"]["orphanMediaAssets"], 1);
    assert_eq!(body["deleted"]["orphanMediaFileDeleteErrors"], 0);

    assert!(!orphan_path.exists(), "le fichier orphelin est supprimé");
    assert!(
        kept_path.exists(),
        "le média référencé par le board d'une autre guilde est conservé"
    );
    assert!(
        db.media_assets()
            .get(kept_asset.id)
            .await
            .expect("lecture")
            .is_some(),
        "la ligne du média conservé reste en base"
    );
    assert!(db.guilds().get(&guild_id).await.expect("lecture").is_none());
}

#[tokio::test]
async fn purge_closes_overlay_sockets_with_guild_deleted() {
    let harness = Harness::new().await;
    let guild_id = GuildId::parse("42").expect("guilde");
    harness
        .db
        .guilds()
        .upsert(&guild("42"))
        .await
        .expect("guilde");
    let overlay = overlay_client(&guild_id, OverlaySessionMode::Normal);
    let overlay_id = overlay.id;
    harness
        .db
        .overlay_clients()
        .create(&overlay)
        .await
        .expect("client overlay");

    let hub = Arc::new(RecordingAdminHub::default());
    let state = harness
        .state
        .clone()
        .with_admin_hub(hub.clone() as Arc<dyn AdminHub>);
    let app = router(state);
    let response = app
        .oneshot(harness.request_json(
            Method::POST,
            "/admin/api/guilds/42/purge",
            &json!({ "confirmGuildId": "42" }),
        ))
        .await
        .expect("réponse de purge");
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        hub.closes(),
        vec![(overlay_id.as_uuid(), close_code::GUILD_DELETED)]
    );
}

#[tokio::test]
async fn purge_ne_supprime_pas_un_fichier_hors_de_la_racine_media() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    db.guilds().upsert(&guild("42")).await.expect("guilde");

    // Média orphelin dont le fichier est hors de MEDIA_STORAGE_DIR : la purge
    // supprime la ligne mais refuse la suppression du fichier (confinement V2-S).
    let outside_path = harness.path("hors-racine.png");
    std::fs::write(&outside_path, b"hors racine").expect("fichier hors racine");
    let outside_asset = media_asset(1024, &outside_path);
    db.media_assets()
        .insert(&outside_asset)
        .await
        .expect("média hors racine");
    db.media_board()
        .create(&board_item(&guild_id, outside_asset.id))
        .await
        .expect("élément de board hors racine");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds/42/purge",
            &json!({ "confirmGuildId": "42" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["purged"], true);
    assert_eq!(body["deleted"]["orphanMediaAssets"], 1);
    assert_eq!(body["deleted"]["orphanMediaFileDeleteErrors"], 0);

    assert!(
        outside_path.exists(),
        "un fichier hors de la racine média n'est jamais supprimé par la purge"
    );
    assert!(
        db.media_assets()
            .get(outside_asset.id)
            .await
            .expect("lecture")
            .is_none(),
        "la ligne du média orphelin est quand même supprimée"
    );
}

#[tokio::test]
async fn purge_refuse_une_confirmation_differente() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds/42/purge",
            &json!({ "confirmGuildId": "43" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "confirm_guild_id_mismatch");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds/42/purge",
            &json!({ "confirmGuildId": "42", "removeOrphanMedia": "oui" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
}

#[tokio::test]
async fn revoke_clients_revokes_guild_clients_and_keeps_guild_data() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    let other_guild_id = GuildId::parse("43").expect("autre guilde");
    create_linked_standalone(db, &guild_id, "123456789012345678").await;
    db.guilds()
        .upsert(&guild("43"))
        .await
        .expect("autre guilde");
    let now = UtcDateTime::now();
    let pre_revoked_at_ms = lc_domain::datetime_to_ms(now - Duration::minutes(5));
    db.guilds()
        .set_busy_until(&guild_id, Some(now + Duration::hours(1)))
        .await
        .expect("verrou de lecture");

    let playing_job = playback_job(&guild_id, JobStatus::Playing);
    db.playback_jobs()
        .create(&playing_job)
        .await
        .expect("job playing");

    let overlay_one = overlay_client(&guild_id, OverlaySessionMode::Normal);
    let overlay_two = overlay_client(&guild_id, OverlaySessionMode::Normal);
    let mut already_revoked_overlay = overlay_client(&guild_id, OverlaySessionMode::Normal);
    already_revoked_overlay.revoked_at = Some(now - Duration::minutes(5));
    let other_overlay = overlay_client(&other_guild_id, OverlaySessionMode::Normal);
    for client in [
        &overlay_one,
        &overlay_two,
        &already_revoked_overlay,
        &other_overlay,
    ] {
        db.overlay_clients()
            .create(client)
            .await
            .expect("client overlay");
    }

    let ingest_one = ingest_client(&guild_id, "1001", "Chrome");
    let ingest_two = ingest_client(&guild_id, "1002", "Firefox");
    let mut already_revoked_ingest = ingest_client(&guild_id, "1003", "Edge");
    already_revoked_ingest.revoked_at = Some(now - Duration::minutes(5));
    let other_ingest = ingest_client(&other_guild_id, "2001", "Safari");
    for client in [
        &ingest_one,
        &ingest_two,
        &already_revoked_ingest,
        &other_ingest,
    ] {
        db.ingest_clients()
            .create(client)
            .await
            .expect("client ingest");
    }

    let admin = guild_member(&guild_id, GuildRole::Admin);
    let member = guild_member(&guild_id, GuildRole::Member);
    let other_member = guild_member(&other_guild_id, GuildRole::Member);
    for guild_member in [&admin, &member, &other_member] {
        db.guild_members()
            .create(guild_member)
            .await
            .expect("membre");
    }

    for (code, target) in [
        ("PAIR0001", &guild_id),
        ("PAIR0002", &guild_id),
        ("PAIR0003", &other_guild_id),
    ] {
        db.pairing_codes()
            .create(&pairing_code(code, target, None, None, "1001"))
            .await
            .expect("code de pairing");
    }
    for (code, target) in [
        ("INVITE01", &guild_id),
        ("INVITE02", &guild_id),
        ("INVITE03", &other_guild_id),
    ] {
        db.guild_invites()
            .create(&guild_invite(code, target))
            .await
            .expect("invitation");
    }

    let board_path = harness.path("media").join("board-kept.png");
    std::fs::write(&board_path, b"board").expect("fichier board");
    let board_asset = media_asset(1024, &board_path);
    db.media_assets()
        .insert(&board_asset)
        .await
        .expect("média board");
    db.media_board()
        .create(&board_item(&guild_id, board_asset.id))
        .await
        .expect("élément de board");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds/42/revoke-clients",
            &json!({ "confirmGuildId": "42" }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["guildId"], "42");
    assert_eq!(
        body["stop"],
        json!({ "releasedPlayingJobs": 1, "disconnectedOverlaySockets": 0 })
    );
    assert_eq!(
        body["revoked"],
        json!({
            "overlayClients": 2,
            "ingestClients": 2,
            "members": 2,
            "pairingCodes": 2,
            "invites": 2,
        })
    );

    for client in [&overlay_one, &overlay_two] {
        let stored = db
            .overlay_clients()
            .get(client.id)
            .await
            .expect("lecture overlay")
            .expect("overlay présent");
        assert!(stored.revoked_at.is_some(), "overlay actif révoqué");
    }
    let stored = db
        .overlay_clients()
        .get(already_revoked_overlay.id)
        .await
        .expect("lecture overlay")
        .expect("overlay présent");
    assert_eq!(
        stored.revoked_at.map(lc_domain::datetime_to_ms),
        Some(pre_revoked_at_ms)
    );
    let stored = db
        .overlay_clients()
        .get(other_overlay.id)
        .await
        .expect("lecture overlay")
        .expect("overlay présent");
    assert!(stored.revoked_at.is_none(), "autre guilde intacte");

    for client in [&ingest_one, &ingest_two] {
        let stored = db
            .ingest_clients()
            .get(client.id)
            .await
            .expect("lecture ingest")
            .expect("ingest présent");
        assert!(stored.revoked_at.is_some(), "client ingest révoqué");
    }
    let stored = db
        .ingest_clients()
        .get(already_revoked_ingest.id)
        .await
        .expect("lecture ingest")
        .expect("ingest présent");
    assert_eq!(
        stored.revoked_at.map(lc_domain::datetime_to_ms),
        Some(pre_revoked_at_ms)
    );
    let stored = db
        .ingest_clients()
        .get(other_ingest.id)
        .await
        .expect("lecture ingest")
        .expect("ingest présent");
    assert!(stored.revoked_at.is_none(), "autre guilde intacte");

    for guild_member in [&admin, &member] {
        let stored = db
            .guild_members()
            .get(guild_member.id)
            .await
            .expect("lecture membre")
            .expect("membre présent");
        assert!(stored.removed_at.is_some(), "membre marqué retiré");
    }
    let stored = db
        .guild_members()
        .get(other_member.id)
        .await
        .expect("lecture membre")
        .expect("membre présent");
    assert!(stored.removed_at.is_none(), "autre guilde intacte");

    for code in ["PAIR0001", "PAIR0002"] {
        assert!(
            db.pairing_codes()
                .get(&PairingCode::parse(code).expect("code"))
                .await
                .expect("lecture pairing")
                .is_none(),
            "code de pairing supprimé"
        );
    }
    assert!(
        db.pairing_codes()
            .get(&PairingCode::parse("PAIR0003").expect("code"))
            .await
            .expect("lecture pairing")
            .is_some(),
        "code de l'autre guilde conservé"
    );
    assert!(db
        .guild_invites()
        .list_by_guild(&guild_id)
        .await
        .expect("invitations")
        .is_empty());
    assert_eq!(
        db.guild_invites()
            .list_by_guild(&other_guild_id)
            .await
            .expect("invitations de l'autre guilde")
            .len(),
        1
    );

    let stored_guild = db
        .guilds()
        .get(&guild_id)
        .await
        .expect("lecture guilde")
        .expect("guilde conservée");
    assert_eq!(stored_guild.name.as_deref(), Some("Indépendante"));
    assert_eq!(stored_guild.busy_until, None);
    let access = db
        .guilds()
        .access(&guild_id)
        .await
        .expect("lecture liaison")
        .expect("guilde conservée");
    assert_eq!(
        access.discord_guild_id.as_deref(),
        Some("123456789012345678")
    );
    assert_eq!(
        db.media_board()
            .list_by_guild(&guild_id)
            .await
            .expect("board")
            .len(),
        1,
        "board média conservé"
    );
    assert!(
        db.media_assets()
            .get(board_asset.id)
            .await
            .expect("lecture média")
            .is_some(),
        "média du board conservé"
    );
    assert!(board_path.exists(), "fichier du board conservé");

    let stored_job = db
        .playback_jobs()
        .get(playing_job.id)
        .await
        .expect("lecture job")
        .expect("job conservé");
    assert_eq!(stored_job.status, JobStatus::Done);
}

#[tokio::test]
async fn revoke_clients_unknown_guild_returns_zero_counts() {
    let harness = Harness::new().await;
    let unknown = GuildId::parse("987654321098765432").expect("unknown guild");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{unknown}/revoke-clients"),
            &json!({ "confirmGuildId": unknown.as_str() }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "guildId": unknown.as_str(),
            "stop": {
                "releasedPlayingJobs": 0,
                "disconnectedOverlaySockets": 0,
            },
            "revoked": {
                "overlayClients": 0,
                "ingestClients": 0,
                "members": 0,
                "pairingCodes": 0,
                "invites": 0,
            },
        })
    );
    assert!(
        harness
            .db()
            .guilds()
            .get(&unknown)
            .await
            .expect("reading guild")
            .is_none(),
        "an unknown guild must not be created"
    );
}

#[tokio::test]
async fn revoke_clients_rejects_wrong_confirmation() {
    let harness = Harness::new().await;

    for body in [json!({ "confirmGuildId": "43" }), json!({})] {
        let (status, response) = harness
            .call_json(Method::POST, "/admin/api/guilds/42/revoke-clients", &body)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"], "confirm_guild_id_mismatch");
    }
}

#[tokio::test]
async fn clear_media_board_deletes_items_and_only_orphan_media() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    db.guilds().upsert(&guild("42")).await.expect("guilde");
    let overlay = overlay_client(&guild_id, OverlaySessionMode::Normal);
    db.overlay_clients()
        .create(&overlay)
        .await
        .expect("client overlay");

    let orphan_path = harness.path("media").join("media-orphan.png");
    std::fs::write(&orphan_path, b"orphan").expect("fichier orphelin");
    let orphan_asset = media_asset(1024, &orphan_path);
    db.media_assets()
        .insert(&orphan_asset)
        .await
        .expect("média orphelin");
    db.media_board()
        .create(&board_item(&guild_id, orphan_asset.id))
        .await
        .expect("élément orphelin");

    let used_path = harness.path("media").join("media-used.png");
    std::fs::write(&used_path, b"used").expect("fichier utilisé");
    let used_asset = media_asset(2048, &used_path);
    db.media_assets()
        .insert(&used_asset)
        .await
        .expect("média utilisé");
    db.media_board()
        .create(&board_item(&guild_id, used_asset.id))
        .await
        .expect("élément utilisé");
    let mut job = playback_job(&guild_id, JobStatus::Pending);
    job.media_asset_id = Some(used_asset.id);
    db.playback_jobs()
        .create(&job)
        .await
        .expect("job référençant le média");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds/42/media-board/clear",
            &json!({ "confirmGuildId": "42" }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["guildId"], "42");
    assert_eq!(
        body["deleted"],
        json!({
            "mediaBoardItems": 2,
            "orphanMediaAssets": 1,
            "orphanMediaFileDeleteErrors": 0,
        })
    );

    assert!(db
        .media_board()
        .list_by_guild(&guild_id)
        .await
        .expect("board")
        .is_empty());
    assert!(!orphan_path.exists(), "fichier orphelin supprimé");
    assert!(
        db.media_assets()
            .get(orphan_asset.id)
            .await
            .expect("lecture média")
            .is_none(),
        "ligne du média orphelin supprimée"
    );
    assert!(used_path.exists(), "fichier encore référencé conservé");
    assert!(
        db.media_assets()
            .get(used_asset.id)
            .await
            .expect("lecture média")
            .is_some(),
        "média encore référencé conservé"
    );
    assert!(
        db.playback_jobs()
            .get(job.id)
            .await
            .expect("lecture job")
            .is_some(),
        "job de playback conservé"
    );
    assert!(
        db.guilds()
            .get(&guild_id)
            .await
            .expect("lecture guilde")
            .is_some(),
        "guilde conservée"
    );
    let stored_overlay = db
        .overlay_clients()
        .get(overlay.id)
        .await
        .expect("lecture overlay")
        .expect("overlay conservé");
    assert!(stored_overlay.revoked_at.is_none());
}

#[tokio::test]
async fn clear_media_board_rejects_wrong_confirmation() {
    let harness = Harness::new().await;

    for body in [json!({ "confirmGuildId": "43" }), json!({})] {
        let (status, response) = harness
            .call_json(
                Method::POST,
                "/admin/api/guilds/42/media-board/clear",
                &body,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"], "confirm_guild_id_mismatch");
    }
}

// ---------------------------------------------------------------------------
// Discord : départ du bot et détachement d'une guilde
// ---------------------------------------------------------------------------

#[tokio::test]
async fn purge_avec_leave_discord_quitte_le_serveur_natif() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Left);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let native_id = GuildId::parse("123456789012345678").expect("guilde native");
    harness
        .db()
        .guilds()
        .upsert(&guild(native_id.as_str()))
        .await
        .expect("guilde native");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{native_id}/purge"),
            &json!({ "confirmGuildId": native_id.as_str(), "leaveDiscord": true }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["purged"], true);
    assert_eq!(
        body["discord"],
        json!({
            "serverId": native_id.as_str(),
            "requested": true,
            "outcome": "left",
            "error": null,
        })
    );
    assert_eq!(leaver.calls(), vec![native_id.to_string()]);
    assert!(
        harness
            .db()
            .guilds()
            .get(&native_id)
            .await
            .expect("lecture")
            .is_none(),
        "la ligne de la guilde native est supprimée"
    );
}

#[tokio::test]
async fn purge_sans_leave_discord_ne_quitte_pas_le_serveur() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Left);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let native_id = GuildId::parse("123456789012345678").expect("guilde native");
    harness
        .db()
        .guilds()
        .upsert(&guild(native_id.as_str()))
        .await
        .expect("guilde native");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{native_id}/purge"),
            &json!({ "confirmGuildId": native_id.as_str() }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["discord"]["requested"], false);
    assert_eq!(body["discord"]["outcome"], "skipped");
    assert_eq!(body["discord"]["serverId"], native_id.as_str());
    assert!(body["discord"]["error"].is_null());
    assert!(leaver.calls().is_empty());
}

#[tokio::test]
async fn purge_leave_discord_not_member_reste_200() {
    let leaver = FakeLeaver::new(FakeLeaveResult::NotMember);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let native_id = GuildId::parse("123456789012345678").expect("guilde native");
    harness
        .db()
        .guilds()
        .upsert(&guild(native_id.as_str()))
        .await
        .expect("guilde native");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{native_id}/purge"),
            &json!({ "confirmGuildId": native_id.as_str(), "leaveDiscord": true }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["purged"], true);
    assert_eq!(body["discord"]["outcome"], "not_member");
    assert!(body["discord"]["error"].is_null());
    assert_eq!(leaver.calls(), vec![native_id.to_string()]);
}

#[tokio::test]
async fn purge_leave_discord_en_erreur_reste_200() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Api);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let native_id = GuildId::parse("123456789012345678").expect("guilde native");
    harness
        .db()
        .guilds()
        .upsert(&guild(native_id.as_str()))
        .await
        .expect("guilde native");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{native_id}/purge"),
            &json!({ "confirmGuildId": native_id.as_str(), "leaveDiscord": true }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["purged"], true);
    assert_eq!(body["discord"]["outcome"], "error");
    assert_eq!(body["discord"]["error"], "Discord : échec Discord simulé");
    assert_eq!(leaver.calls(), vec![native_id.to_string()]);
}

#[tokio::test]
async fn purge_leave_discord_au_mauvais_type_est_refuse() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Left);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let native_id = GuildId::parse("123456789012345678").expect("guilde native");
    harness
        .db()
        .guilds()
        .upsert(&guild(native_id.as_str()))
        .await
        .expect("guilde native");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{native_id}/purge"),
            &json!({ "confirmGuildId": native_id.as_str(), "leaveDiscord": "true" }),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
    assert!(leaver.calls().is_empty());
}

#[tokio::test]
async fn discord_unlink_delie_la_guilde_et_quitte_le_serveur() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Left);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let guild_id = GuildId::parse("900000000000000001").expect("guilde indépendante");
    let discord_id = "123456789012345678";
    create_linked_standalone(harness.db(), &guild_id, discord_id).await;

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{guild_id}/discord-unlink"),
            &json!({ "leaveDiscord": true }),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["unlinked"], true);
    assert_eq!(body["guildId"], guild_id.as_str());
    assert_eq!(
        body["discord"],
        json!({
            "serverId": discord_id,
            "requested": true,
            "outcome": "left",
            "error": null,
        })
    );
    assert_eq!(leaver.calls(), vec![discord_id.to_owned()]);

    let access = harness
        .db()
        .guilds()
        .access(&guild_id)
        .await
        .expect("lecture")
        .expect("guilde présente");
    assert!(access.standalone);
    assert_eq!(access.discord_guild_id, None);
}

#[tokio::test]
async fn discord_unlink_refuse_une_guilde_native() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Left);
    let harness = Harness::with_leaver(leaver.clone()).await;
    let native_id = GuildId::parse("123456789012345678").expect("guilde native");
    harness
        .db()
        .guilds()
        .upsert(&guild(native_id.as_str()))
        .await
        .expect("guilde native");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{native_id}/discord-unlink"),
            &json!({}),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "native_guild_unlink");
    assert!(leaver.calls().is_empty());
}

#[tokio::test]
async fn discord_unlink_refuse_une_guilde_non_liee() {
    let harness = Harness::with_leaver(FakeLeaver::new(FakeLeaveResult::Left)).await;
    let guild_id = GuildId::parse("900000000000000001").expect("guilde indépendante");
    harness
        .db()
        .guilds()
        .create_standalone(&guild_id, "Indépendante", UtcDateTime::now())
        .await
        .expect("guilde indépendante");

    let (status, body) = harness
        .call_json(
            Method::POST,
            &format!("/admin/api/guilds/{guild_id}/discord-unlink"),
            &json!({}),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "not_linked");
}

#[tokio::test]
async fn discord_unlink_guilde_inconnue_renvoie_404() {
    let leaver = FakeLeaver::new(FakeLeaveResult::Left);
    let harness = Harness::with_leaver(leaver.clone()).await;

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/guilds/987654321098765432/discord-unlink",
            &json!({ "leaveDiscord": true }),
        )
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "guild_not_found");
    assert!(leaver.calls().is_empty());
}

#[tokio::test]
async fn overview_expose_le_rattachement_discord() {
    let harness = Harness::new().await;
    let db = harness.db();

    let native_id = GuildId::parse("123456789012345678").expect("guilde native");
    db.guilds()
        .upsert(&guild(native_id.as_str()))
        .await
        .expect("guilde native");

    let linked_id = GuildId::parse("900000000000000001").expect("guilde liée");
    create_linked_standalone(db, &linked_id, "223456789012345678").await;

    let unlinked_id = GuildId::parse("900000000000000002").expect("guilde non liée");
    db.guilds()
        .create_standalone(&unlinked_id, "Non liée", UtcDateTime::now())
        .await
        .expect("guilde non liée");

    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::OK);

    let guilds = body["guilds"].as_array().expect("tableau des guildes");
    let find = |id: &str| {
        guilds
            .iter()
            .find(|entry| entry["id"] == id)
            .expect("guilde présente dans l'aperçu")
    };

    assert_eq!(
        find(native_id.as_str())["discord"],
        json!({ "kind": "native", "serverId": native_id.as_str() })
    );
    assert_eq!(
        find(linked_id.as_str())["discord"],
        json!({ "kind": "linked", "serverId": "223456789012345678" })
    );
    assert_eq!(
        find(unlinked_id.as_str())["discord"],
        json!({ "kind": "none", "serverId": null })
    );
}

// ---------------------------------------------------------------------------
// Révocation des clients
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_overlay_marque_le_client_et_repond_false_au_second_appel() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    let client = overlay_client(&guild_id, OverlaySessionMode::Normal);
    let client_id = client.id;
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client overlay");

    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/overlay-clients/{client_id}/revoke"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found"], true);
    assert_eq!(body["revoked"], true);
    assert_eq!(body["guildId"], "42");

    let stored = db
        .overlay_clients()
        .get(client_id)
        .await
        .expect("lecture client")
        .expect("client présent");
    assert!(stored.revoked_at.is_some());

    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/overlay-clients/{client_id}/revoke"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["revoked"], false);
}

#[tokio::test]
async fn revoke_ingest_inconnu_renvoie_404() {
    let harness = Harness::new().await;
    let unknown = IngestClientId::new_v4();

    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/admin/api/ingest-clients/{unknown}/revoke"),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "ingest_client_not_found");

    let (status, body) = harness
        .call(
            Method::POST,
            "/admin/api/overlay-clients/pas-un-uuid/revoke",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "overlay_client_not_found");
}

// ---------------------------------------------------------------------------
// Codes de pairing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pairing_liste_filtre_et_compte() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    let now = UtcDateTime::now();

    db.pairing_codes()
        .create(&pairing_code(
            "ACTIF001",
            &guild_id,
            Some(now + Duration::hours(1)),
            None,
            "1001",
        ))
        .await
        .expect("actif");
    db.pairing_codes()
        .create(&pairing_code(
            "EXPIRE01",
            &guild_id,
            Some(now - Duration::hours(1)),
            None,
            "1001",
        ))
        .await
        .expect("expiré");
    db.pairing_codes()
        .create(&pairing_code(
            "USE00001",
            &guild_id,
            None,
            Some(now),
            "1001",
        ))
        .await
        .expect("utilisé");

    let (status, body) = harness.call(Method::GET, "/admin/api/pairing-codes").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["counts"]["total"], 3);
    assert_eq!(body["counts"]["active"], 1);
    assert_eq!(body["counts"]["expired"], 2);

    let (status, body) = harness
        .call(Method::GET, "/admin/api/pairing-codes?status=active")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["counts"]["total"], 1);
    assert_eq!(body["items"][0]["code"], "ACTIF001");
    assert_eq!(body["items"][0]["isActive"], true);

    let (status, body) = harness
        .call(Method::GET, "/admin/api/pairing-codes?guildId=99")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["counts"]["total"], 0);
}

#[tokio::test]
async fn pairing_supprime_les_expires_et_revoque_un_code() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    let now = UtcDateTime::now();

    db.pairing_codes()
        .create(&pairing_code(
            "ACTIF001",
            &guild_id,
            Some(now + Duration::hours(1)),
            None,
            "1001",
        ))
        .await
        .expect("actif");
    db.pairing_codes()
        .create(&pairing_code(
            "EXPIRE01",
            &guild_id,
            Some(now - Duration::hours(1)),
            None,
            "1001",
        ))
        .await
        .expect("expiré");

    let (status, body) = harness
        .call(Method::DELETE, "/admin/api/pairing-codes/expired")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deletedCount"], 1);
    assert_eq!(body["guildId"], Value::Null);

    let (status, body) = harness
        .call(Method::POST, "/admin/api/pairing-codes/actif001/revoke")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found"], true);
    assert_eq!(body["revoked"], true);
    assert_eq!(body["guildId"], "42");

    let stored = db
        .pairing_codes()
        .get(&PairingCode::parse("ACTIF001").expect("code"))
        .await
        .expect("lecture")
        .expect("code présent");
    assert!(stored.used_at.is_some());
    assert!(stored.expires_at.is_some());

    let (status, body) = harness
        .call(Method::POST, "/admin/api/pairing-codes/REVOQUE1/revoke")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "pairing_code_not_found");
}

// ---------------------------------------------------------------------------
// Auteurs et clients d'ingestion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_authors_fusionne_pairing_et_ingest() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");

    db.pairing_codes()
        .create(&pairing_code("AUTHOR01", &guild_id, None, None, "1001"))
        .await
        .expect("code auteur");
    db.ingest_clients()
        .create(&ingest_client(&guild_id, "1002", "Chrome"))
        .await
        .expect("client ingest");

    let (status, body) = harness.call(Method::GET, "/admin/api/ingest-authors").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 2);
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["discordUserId"], "1001");
    assert_eq!(items[0]["source"], "pairing");
    assert_eq!(items[1]["discordUserId"], "1002");
    assert_eq!(items[1]["source"], "ingest");
}

#[tokio::test]
async fn ingest_client_create_genere_un_token_et_hash_en_base() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    db.pairing_codes()
        .create(&pairing_code("AUTHOR01", &guild_id, None, None, "1001"))
        .await
        .expect("code auteur");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/ingest-clients",
            &json!({ "guildId": "42", "authorDiscordUserId": "1001", "label": "Chrome" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["created"], true);
    let token = body["ingestApiToken"]
        .as_str()
        .expect("token en clair une seule fois");
    assert_eq!(token.len(), 64, "32 octets en hexadécimal");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(body["client"]["label"], "Chrome-Alice");
    assert_eq!(body["client"]["createdByDiscordUserId"], "1001");
    assert_eq!(body["client"]["guildId"], "42");

    // Le token n'est jamais stocké en clair : hash SHA-256 uniquement.
    let client_id = body["client"]["id"].as_str().expect("identifiant client");
    let client = db
        .ingest_clients()
        .get(IngestClientId::parse(client_id).expect("identifiant valide"))
        .await
        .expect("lecture client")
        .expect("client présent");
    assert_eq!(client.token_hash, sha256_hex(token));
    assert_ne!(client.token_hash, token);
}

#[tokio::test]
async fn ingest_client_create_erreurs_de_validation() {
    let harness = Harness::new().await;

    let (status, body) = harness
        .call_json(Method::POST, "/admin/api/ingest-clients", &json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_guild_id");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/ingest-clients",
            &json!({ "guildId": "abc", "authorDiscordUserId": "1001", "label": "Chrome" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_guild_id");

    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/ingest-clients",
            &json!({ "guildId": "42", "authorDiscordUserId": "1001", "label": "Chrome" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "author_not_found");
}

#[tokio::test]
async fn ingest_client_create_label_borne() {
    let harness = Harness::new().await;
    let db = harness.db();
    let guild_id = GuildId::parse("42").expect("guilde");
    db.pairing_codes()
        .create(&pairing_code("AUTHOR01", &guild_id, None, None, "1001"))
        .await
        .expect("code auteur");

    // > 120 caractères : refusé avant toute création (AUD-COR-14/A3-008).
    let too_long = "x".repeat(121);
    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/ingest-clients",
            &json!({ "guildId": "42", "authorDiscordUserId": "1001", "label": too_long }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_label");

    // Caractère de contrôle : refusé (échappement d'UI/terminal).
    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/ingest-clients",
            &json!({ "guildId": "42", "authorDiscordUserId": "1001", "label": "Chrome\nEvil" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_label");

    // 120 caractères exactement : accepté, label final `{label}-{auteur}`.
    let max_label = "y".repeat(120);
    let (status, body) = harness
        .call_json(
            Method::POST,
            "/admin/api/ingest-clients",
            &json!({ "guildId": "42", "authorDiscordUserId": "1001", "label": max_label }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["client"]["label"], format!("{max_label}-Alice"));

    // Le client de trop grande longueur n'a pas été persisté.
    let clients = db
        .ingest_clients()
        .list()
        .await
        .expect("lecture des clients d'ingestion");
    assert_eq!(clients.len(), 1);
}

#[tokio::test]
async fn hub_noop_par_defaut_et_injectable() {
    let harness = Harness::new().await;
    let hub = admin_hub(&harness.state);
    assert_eq!(hub.connected_count(), 0);

    // AUD-COR-14 : plus de `OnceLock` ; le hub est porté par l'état et
    // remplaçable (le défaut Noop reste utilisé tant que rien n'est injecté).
    let state = harness
        .state
        .clone()
        .with_admin_hub(Arc::new(NoopAdminHub::new()));
    assert!(!admin_hub(&state).is_connected(&uuid::Uuid::nil()));
}
