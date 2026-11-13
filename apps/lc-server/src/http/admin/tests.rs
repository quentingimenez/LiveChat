//! Tests d'intégration du panneau d'administration.
//!
//! Tous les tests passent par `crate::app::router()` (pas seulement par
//! `admin::router()`) : ils vérifient donc aussi l'intégration avec les couches
//! globales (en-têtes de sécurité, limite de corps, rate limit, normalisation
//! des erreurs) et la cohabitation des CSP admin/globale.
//!
//! Chaque test crée sa propre base SQLite en mémoire et son propre dossier
//! temporaire (URL `DATABASE_URL` pointant dans ce dossier) : les réglages
//! runtime (`admin_runtime.json`) sont ainsi isolés du dépôt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Method, Request as HttpRequest, StatusCode};
use axum::Router;
use chrono::{Duration, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{
    Guild, GuildId, GuildInvite, GuildMember, GuildMemberId, GuildRole, IngestClient,
    IngestClientId, JobStatus, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId,
    MediaKind, MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode, PairingCode,
    PairingCodeEntry, PairingMode, PlaybackJob, PlaybackJobId,
};
use lc_protocol::close_code;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use tower::ServiceExt;

use super::hub::{admin_hub, AdminHub, NoopAdminHub};
use super::runtime_settings::SETTINGS_FILE_NAME;
use super::ADMIN_CSP;
use crate::app::{router, AppState};
use crate::config::Config;
use crate::discord::guild_leave::{DiscordGuildLeaver, LeaveError, LeaveOutcome};
use crate::http::security::{CONTENT_SECURITY_POLICY, PERMISSIONS_POLICY};
use crate::shutdown::Shutdown;

const TOKEN: &str = "jeton-admin-de-test";
/// `Host` envoyé par les requêtes du harnais (origine du panneau en test).
const TEST_HOST: &str = "127.0.0.1:40000";
/// Origine correspondant à `TEST_HOST`, utilisée pour les mutations (CSRF).
const TEST_ORIGIN: &str = "http://127.0.0.1:40000";

/// Issue configurée d'un [`FakeLeaver`].
#[derive(Debug, Clone, Copy)]
enum FakeLeaveResult {
    Left,
    NotMember,
    Api,
}

/// Leaver factice : enregistre les identifiants reçus, sans réseau.
#[derive(Debug)]
struct FakeLeaver {
    calls: Mutex<Vec<String>>,
    result: FakeLeaveResult,
}

impl FakeLeaver {
    fn new(result: FakeLeaveResult) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            result,
        })
    }

    fn calls(&self) -> Vec<String> {
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

/// Admin hub fake that records close calls.
#[derive(Debug, Default)]
struct RecordingAdminHub {
    closes: Mutex<Vec<(uuid::Uuid, u16)>>,
}

impl RecordingAdminHub {
    fn closes(&self) -> Vec<(uuid::Uuid, u16)> {
        self.closes.lock().expect("verrou du hub").clone()
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
        false
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

    async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
        0
    }

    async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
        0
    }
}

/// Environnement de test : dossier temporaire, base mémoire et routeur complet.
struct Harness {
    dir: TempDir,
    state: AppState,
    db: Db,
    app: Router,
}

impl Harness {
    async fn new() -> Self {
        Self::with_pairs(&[]).await
    }

    async fn with_pairs(extra: &[(&str, &str)]) -> Self {
        Self::with_pairs_and_leaver(extra, None).await
    }

    async fn with_leaver(leaver: Arc<dyn DiscordGuildLeaver>) -> Self {
        Self::with_pairs_and_leaver(&[], Some(leaver)).await
    }

    async fn with_pairs_and_leaver(
        extra: &[(&str, &str)],
        leaver: Option<Arc<dyn DiscordGuildLeaver>>,
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
        let db = Db::open_in_memory(&DbConfig::default())
            .await
            .expect("base de test en mémoire");
        let mut state = AppState::new(Arc::new(config), Some(db.clone()), Shutdown::detached());
        if let Some(leaver) = leaver {
            state = state.with_discord_leaver(leaver);
        }
        let app = router(state.clone());

        Self {
            dir,
            state,
            db,
            app,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn request(&self, method: Method, path: &str) -> HttpRequest<Body> {
        self.request_from(method, path, IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    fn request_from(&self, method: Method, path: &str, ip: IpAddr) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::ORIGIN, TEST_ORIGIN)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .expect("requête de test valide");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(ip, 40_000)));
        request
    }

    /// Requête de mutation avec origine explicite (`None` = origine absente).
    fn request_with_origin(
        &self,
        method: Method,
        path: &str,
        origin: Option<&str>,
    ) -> HttpRequest<Body> {
        let mut builder = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        let mut request = builder.body(Body::empty()).expect("requête de test valide");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
        request
    }

    /// Requête de mutation avec un `Referer` (et sans `Origin`).
    fn request_with_referer(&self, method: Method, path: &str, referer: &str) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::REFERER, referer)
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .expect("requête de test valide");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
        request
    }

    fn request_json(&self, method: Method, path: &str, body: &Value) -> HttpRequest<Body> {
        let payload = serde_json::to_vec(body).expect("corps JSON sérialisable");
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, TEST_HOST)
            .header(header::ORIGIN, TEST_ORIGIN)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::from(payload))
            .expect("requête de test valide");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
        request
    }

    async fn send(&self, request: HttpRequest<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
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

    async fn call(&self, method: Method, path: &str) -> (StatusCode, Value) {
        let (status, _, bytes) = self.send(self.request(method, path)).await;
        (status, json_or_null(&bytes))
    }

    async fn call_json(&self, method: Method, path: &str, body: &Value) -> (StatusCode, Value) {
        let (status, _, bytes) = self.send(self.request_json(method, path, body)).await;
        (status, json_or_null(&bytes))
    }

    async fn call_with_headers(
        &self,
        method: Method,
        path: &str,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        self.send(self.request(method, path)).await
    }

    fn db(&self) -> &Db {
        &self.db
    }
}

fn json_or_null(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
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

fn guild(id: &str) -> Guild {
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
async fn create_linked_standalone(db: &Db, guild_id: &GuildId, discord_guild_id: &str) {
    db.guilds()
        .create_standalone(guild_id, "Indépendante", Utc::now())
        .await
        .expect("guilde indépendante");
    let discord_guild_id = GuildId::parse(discord_guild_id).expect("serveur Discord valide");
    assert!(db
        .guilds()
        .link_discord(guild_id, Some(&discord_guild_id))
        .await
        .expect("liaison Discord"));
}

fn media_asset(size_bytes: i64, storage_path: &Path) -> MediaAsset {
    let now = Utc::now();
    MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: format!("hash-{}", MediaAssetId::new_v4()),
        source_url: "https://example.invalid/media.png".to_owned(),
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

fn board_item(guild_id: &GuildId, media_asset_id: MediaAssetId) -> MediaBoardItem {
    MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id,
        title: Some("Titre".to_owned()),
        message: None,
        created_by_discord_user_id: Some("1001".to_owned()),
        created_by_name: Some("Alice".to_owned()),
        created_at: Utc::now(),
    }
}

fn playback_job(guild_id: &GuildId, status: JobStatus) -> PlaybackJob {
    let now = Utc::now();
    PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: None,
        text: None,
        show_text: false,
        author_name: Some("Alice".to_owned()),
        author_image: None,
        duration_sec: 12,
        priority: 3,
        status,
        resumes_after_job_id: None,
        resume_offset_sec: 0,
        remaining_ms_snapshot: None,
        last_playback_state_at: None,
        submission_date: now,
        execution_date: now,
        scheduled_at: now,
        started_at: None,
        finished_at: None,
    }
}

fn overlay_client(guild_id: &GuildId, session_mode: OverlaySessionMode) -> OverlayClient {
    OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: "Salon principal".to_owned(),
        token_hash: format!("hash-{}", OverlayClientId::new_v4()),
        session_mode,
        default_author_name: Some("Alice".to_owned()),
        default_author_image: None,
        created_by_discord_user_id: Some("1001".to_owned()),
        last_seen_at: None,
        created_at: Utc::now(),
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
        created_at: Utc::now(),
        revoked_at: None,
    }
}

fn pairing_code(
    code: &str,
    guild_id: &GuildId,
    expires_at: Option<chrono::DateTime<Utc>>,
    used_at: Option<chrono::DateTime<Utc>>,
    created_by: &str,
) -> PairingCodeEntry {
    PairingCodeEntry {
        code: PairingCode::parse(code).expect("code de pairing valide"),
        guild_id: guild_id.clone(),
        mode: PairingMode::Normal,
        created_by_discord_user_id: created_by.to_owned(),
        author_name: Some("Alice".to_owned()),
        author_image: None,
        expires_at,
        used_at,
        created_at: Utc::now(),
    }
}

fn guild_member(guild_id: &GuildId, role: GuildRole) -> GuildMember {
    GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild_id.clone(),
        display_name: "Alice".to_owned(),
        avatar: None,
        role,
        created_at: Utc::now(),
        removed_at: None,
    }
}

fn guild_invite(code: &str, guild_id: &GuildId) -> GuildInvite {
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
        created_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Authentification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_503_quand_le_token_n_est_pas_configure() {
    let harness = Harness::with_pairs(&[("ADMIN_UI_TOKEN", "")]).await;

    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "admin_ui_disabled");
    assert_eq!(body["message"], "ADMIN_UI_TOKEN is not configured");
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
async fn auth_401_sans_token() {
    let harness = Harness::new().await;
    let request = HttpRequest::builder()
        .method(Method::GET)
        .uri("/admin/api/overview")
        .body(Body::empty())
        .expect("requête valide");
    let mut request = request;
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));

    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json_or_null(&bytes)["error"], "unauthorized");
}

#[tokio::test]
async fn auth_refuse_le_token_en_query_string() {
    let harness = Harness::new().await;

    let (status, _) = harness.call(Method::GET, "/admin/api/ingest-authors").await;
    assert_eq!(status, StatusCode::OK);

    // AUD-COR-05 : le jeton n'est plus accepté en query string, même si app.js
    // le lisait autrefois dans `?token=` pour en faire un Bearer.
    let request = HttpRequest::builder()
        .method(Method::GET)
        .uri(format!("/admin/api/ingest-authors?token={TOKEN}"))
        .body(Body::empty())
        .expect("requête valide");
    let mut request = request;
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));

    let (status, _, bytes) = harness.send(request).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json_or_null(&bytes)["error"], "unauthorized");
}

#[tokio::test]
async fn trusted_proxy_ignores_a_forged_first_forwarded_hop() {
    let harness = Harness::with_pairs(&[("ADMIN_TRUST_FORWARDED_FOR", "true")]).await;
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
    assert!(text(&bytes).contains("styles.css"));
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
    assert_eq!(body["tiktokCookie"], "");
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
    assert_eq!(body["tiktokCookie"], "abc123");
    assert_eq!(body["hasTikTokCookie"], true);

    // Relecture : le fichier est relu à chaque GET.
    let (status, body) = harness
        .call(Method::GET, "/admin/api/runtime-settings")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tiktokCookie"], "abc123");

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
    let now = Utc::now();

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
    revoked_overlay.revoked_at = Some(Utc::now());
    db.overlay_clients()
        .create(&revoked_overlay)
        .await
        .expect("client overlay révoqué");

    db.ingest_clients()
        .create(&ingest_client(&guild_id, "1001", "Chrome"))
        .await
        .expect("client ingest");
    let mut revoked_ingest = ingest_client(&guild_id, "1002", "Firefox");
    revoked_ingest.revoked_at = Some(Utc::now());
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
    let now = Utc::now();
    let pre_revoked_at_ms = (now - Duration::minutes(5)).timestamp_millis();
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
        stored.revoked_at.map(|date| date.timestamp_millis()),
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
        stored.revoked_at.map(|date| date.timestamp_millis()),
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
async fn revoke_clients_requires_admin_token() {
    let harness = Harness::new().await;
    let request = HttpRequest::builder()
        .method(Method::POST)
        .uri("/admin/api/guilds/42/revoke-clients")
        .header(header::HOST, TEST_HOST)
        .header(header::ORIGIN, TEST_ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"confirmGuildId":"42"}"#))
        .expect("requête valide");
    let mut request = request;
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));

    let (status, _, bytes) = harness.send(request).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json_or_null(&bytes)["error"], "unauthorized");
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
        .create_standalone(&guild_id, "Indépendante", Utc::now())
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
        .create_standalone(&unlinked_id, "Non liée", Utc::now())
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
    let now = Utc::now();

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
    let now = Utc::now();

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

/// AUD-COR-05 : le JavaScript du panneau ne doit plus lire ni propager le jeton
/// via l'URL (le jeton reste dans le champ et/ou `localStorage`, puis part en
/// `Authorization: Bearer`).
#[test]
fn app_js_ne_propage_pas_le_jeton_dans_l_url() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../static/admin/app.js");
    let source = std::fs::read_to_string(&path).expect("static/admin/app.js lisible");

    for interdit in [
        "syncTokenInUrl",
        "history.replaceState",
        "searchParams.set('token'",
        "searchParams.set(\"token\"",
        "searchParams.delete('token'",
        "location.search",
    ] {
        assert!(
            !source.contains(interdit),
            "app.js ne doit plus contenir {interdit:?} (AUD-COR-05)"
        );
    }

    assert!(
        source.contains("localStorage.getItem('livechat_admin_token')"),
        "le jeton doit être lu depuis localStorage"
    );
}
