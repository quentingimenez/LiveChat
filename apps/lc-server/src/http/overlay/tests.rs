//! Tests in-process des routes `/overlay/*` (Phase 3b).
//!
//! Ces tests montent le routeur *complet* (`crate::app::router`) pour exercer
//! la chaîne de middlewares réelle (body limit, normalisation d'erreurs, CORS,
//! compression, rate limiting) en plus des handlers overlay. Aucun réseau :
//! l'ingestion média (POST items avec URL) n'est pas testée ici, elle dépend
//! de `lc-media`/ffmpeg (Phase 5b).
//!
//! Convention : chaque test ouvre sa propre base SQLite en mémoire et son
//! propre `FixedClock`, pour rester déterministe et parallélisable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Duration, TimeZone, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{
    FixedClock, GuildId, GuildInvite, GuildMember, GuildMemberId, GuildRole, JobStatus, MediaAsset,
    MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind, MediaStatus, OverlayClient,
    OverlayClientId, OverlaySessionMode, PairingCode, PairingCodeEntry, PairingMode, PlaybackJob,
    PlaybackJobId,
};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt as _;

use crate::app::AppState;
use crate::config::Config;
use crate::shutdown::Shutdown;

use super::auth;

/// Secret HMAC fixe des tests (jamais utilisé en production).
const MEDIA_SECRET: &str = "secret-de-test-overlay";
/// Jeton Bearer d'un client de test.
const BEARER_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// Deuxième jeton (client d'une autre guilde).
const OTHER_TOKEN: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

/// Instant figé des tests (2026-11-10T12:00:00Z).
fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .expect("instant de test valide")
}

/// Secondes UNIX de l'horloge de test.
fn fixed_now_secs() -> u64 {
    u64::try_from(fixed_now().timestamp()).expect("horodatage positif")
}

/// Guilde principale des tests.
fn guild() -> GuildId {
    GuildId::parse("123456789012345678").expect("guild id valide")
}

/// Autre guilde (isolation des contrôles d'appartenance).
fn other_guild() -> GuildId {
    GuildId::parse("987654321098765432").expect("guild id valide")
}

/// Configuration serveur minimale pointant sur un répertoire média temporaire.
fn test_config(media_dir: &Path) -> Config {
    let pairs: Vec<(String, String)> = vec![
        ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
        ("MEDIA_URL_SECRET".to_owned(), MEDIA_SECRET.to_owned()),
        (
            "MEDIA_STORAGE_DIR".to_owned(),
            media_dir.to_string_lossy().into_owned(),
        ),
        ("MEDIA_BOARD_MAX_TOTAL_MB".to_owned(), "64".to_owned()),
    ];
    Config::from_pairs(pairs).expect("configuration de test valide")
}

/// Base mémoire + routeur complet avec horloge figée.
async fn harness(media_dir: &Path) -> (Router, Db) {
    let db = Db::open_in_memory(&DbConfig::default())
        .await
        .expect("base en mémoire");
    let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
    let state = AppState::new(
        Arc::new(test_config(media_dir)),
        Some(db.clone()),
        Shutdown::detached(),
    )
    .with_clock(clock);
    (crate::app::router(state), db)
}

/// Exécute une requête et renvoie statut, en-têtes et corps brut.
async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.expect("réponse HTTP");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("corps lisible");
    (status, headers, bytes.to_vec())
}

/// Construit une requête JSON (ou vide) avec Bearer optionnel.
fn request(method: Method, uri: &str, token: Option<&str>, body: Option<&Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            builder
                .body(Body::from(serde_json::to_vec(body).expect("JSON")))
                .expect("requête valide")
        }
        None => builder.body(Body::empty()).expect("requête valide"),
    }
}

/// Corps JSON décodé.
fn body_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("corps JSON")
}

/// Construit une requete avec un corps brut (JSON malforme volontaire).
fn raw_request(method: Method, uri: &str, token: Option<&str>, raw: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder
        .body(Body::from(raw.to_owned()))
        .expect("requete valide")
}

/// Client overlay de test (jeton haché SHA-256).
fn make_client(
    guild_id: &GuildId,
    label: &str,
    token: &str,
    session_mode: OverlaySessionMode,
    author: Option<&str>,
) -> OverlayClient {
    let now = fixed_now();
    OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: label.to_owned(),
        token_hash: auth::hash_token(token),
        session_mode,
        default_author_name: author.map(str::to_owned),
        default_author_image: None,
        created_by_discord_user_id: Some("4242".to_owned()),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    }
}

/// Code d'appairage de test.
fn pairing_entry(
    code: &str,
    guild_id: &GuildId,
    mode: PairingMode,
    grants_admin: bool,
    author: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> PairingCodeEntry {
    PairingCodeEntry {
        code: PairingCode::parse(code).expect("code valide"),
        guild_id: guild_id.clone(),
        mode,
        grants_admin,
        created_by_discord_user_id: "4242".to_owned(),
        author_name: author.map(str::to_owned),
        author_image: None,
        expires_at,
        used_at: None,
        created_at: fixed_now() - Duration::minutes(1),
    }
}

/// Invitation de test (aucune identité Discord).
fn invite_entry(
    code: &str,
    guild_id: &GuildId,
    role: GuildRole,
    member_id: Option<GuildMemberId>,
    max_uses: Option<i64>,
) -> GuildInvite {
    GuildInvite {
        code: PairingCode::parse(code).expect("code valide"),
        guild_id: guild_id.clone(),
        role,
        member_id,
        max_uses,
        use_count: 0,
        created_by_member_id: None,
        expires_at: None,
        revoked_at: None,
        created_at: fixed_now() - Duration::minutes(1),
    }
}

/// Insère un média READY pointant sur un fichier réel du répertoire média.
async fn insert_asset(
    db: &Db,
    media_dir: &Path,
    file_name: &str,
    mime: &str,
    kind: MediaKind,
    bytes: &[u8],
    expires_at: DateTime<Utc>,
) -> MediaAsset {
    let path = media_dir.join(file_name);
    tokio::fs::write(&path, bytes)
        .await
        .expect("écriture média");
    let now = fixed_now();
    let asset = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: format!("hash-{file_name}"),
        source_url: format!("https://example.test/{file_name}"),
        kind,
        mime: mime.to_owned(),
        duration_sec: Some(30),
        width: None,
        height: None,
        is_vertical: false,
        storage_path: path.to_string_lossy().into_owned(),
        size_bytes: i64::try_from(bytes.len()).expect("taille"),
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: now,
        expires_at,
        created_at: now,
    };
    db.media_assets()
        .insert(&asset)
        .await
        .expect("insertion média");
    asset
}

/// Insère un job de lecture `PENDING` référençant l'asset (binding média → guilde).
async fn insert_playback_job(db: &Db, guild_id: &GuildId, asset_id: MediaAssetId) -> PlaybackJob {
    let now = fixed_now();
    let job = PlaybackJob {
        id: PlaybackJobId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: Some(asset_id),
        text: None,
        show_text: false,
        author_name: None,
        author_image: None,
        duration_sec: 30,
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
    db.playback_jobs()
        .create(&job)
        .await
        .expect("insertion job de lecture");
    job
}

/// Insère un item de board.
async fn insert_item(
    db: &Db,
    guild_id: &GuildId,
    asset: &MediaAsset,
    title: Option<&str>,
    message: Option<&str>,
    created_at: DateTime<Utc>,
) -> MediaBoardItem {
    let item = MediaBoardItem {
        id: MediaBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: asset.id,
        title: title.map(str::to_owned),
        message: message.map(str::to_owned),
        created_by_discord_user_id: Some("4242".to_owned()),
        created_by_name: Some("Alice".to_owned()),
        created_at,
    };
    db.media_board()
        .create(&item)
        .await
        .expect("insertion item");
    item
}

/// URL média signée (format protocole v2).
fn signed_uri(asset: &MediaAsset, client: &OverlayClient, exp: u64) -> String {
    let signature = crate::media_url::sign(
        MEDIA_SECRET.as_bytes(),
        &asset.id.to_canonical(),
        &client.id.to_canonical(),
        exp,
    )
    .expect("signature HMAC");
    format!(
        "/overlay/media/{}?clientId={}&exp={}&sig={}",
        asset.id.to_canonical(),
        client.id.to_canonical(),
        exp,
        signature
    )
}

/// URL de média du board signée (`board:` namespace).
fn signed_board_uri(asset: &MediaAsset, client: &OverlayClient, exp: u64) -> String {
    let asset_key = asset.id.to_canonical();
    let client_key = client.id.to_canonical();
    let signature = crate::media_url::sign(
        MEDIA_SECRET.as_bytes(),
        &format!("board:{asset_key}"),
        &client_key,
        exp,
    )
    .expect("signature HMAC board");
    format!(
        "/overlay/media-board/media/{asset_key}?clientId={client_key}&exp={exp}&sig={signature}"
    )
}

// --------------------------------------------------------------------------
// POST /overlay/pair/consume
// --------------------------------------------------------------------------

#[tokio::test]
async fn pair_consume_creates_client_and_returns_legacy_payload() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let code = pairing_entry(
        "ABC123",
        &guild(),
        PairingMode::Normal,
        false,
        Some("Alice"),
        Some(fixed_now() + Duration::minutes(5)),
    );
    db.pairing_codes().create(&code).await.expect("code créé");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "  abc123 ", "deviceName": " Poste salon "})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    let token = body["clientToken"].as_str().expect("token").to_owned();
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(body["deviceName"], "Poste salon");
    assert_eq!(body["authorName"], "Alice");
    assert_eq!(body["sessionMode"], "normal");
    assert_eq!(body["protocolVersion"], 2);
    assert_eq!(body["apiBaseUrl"], "http://127.0.0.1:8099");
    assert_eq!(body["guildId"], guild().as_str());
    let client_id = body["clientId"].as_str().expect("clientId");

    // Le token brut n'est pas stocké : seul son SHA-256 l'est.
    let stored = db
        .overlay_clients()
        .get(OverlayClientId::parse(client_id).expect("uuid client"))
        .await
        .expect("lecture client")
        .expect("client créé");
    assert_eq!(stored.token_hash, auth::hash_token(&token));
    assert_eq!(stored.label, "Poste salon");
    assert_eq!(stored.session_mode, OverlaySessionMode::Normal);

    // Un code ne se consomme qu'une fois.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "ABC123"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(&bytes)["error"],
        "pairing_code_invalid_or_expired"
    );
}

#[tokio::test]
async fn pair_consume_rejects_missing_and_expired_codes() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"deviceName": "Poste"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body_json(&bytes)["error"], "invalid_payload");

    db.pairing_codes()
        .create(&pairing_entry(
            "EXPIRED",
            &guild(),
            PairingMode::Normal,
            false,
            Some("Alice"),
            Some(fixed_now() - Duration::minutes(1)),
        ))
        .await
        .expect("code expiré créé");
    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "EXPIRED"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(&bytes)["error"],
        "pairing_code_invalid_or_expired"
    );
}

#[tokio::test]
async fn pair_consume_honors_invite_read_only_and_default_label() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    db.pairing_codes()
        .create(&pairing_entry(
            "INVITE",
            &guild(),
            PairingMode::InviteReadOnly,
            false,
            None,
            None,
        ))
        .await
        .expect("code invité créé");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "invite", "displayName": "Invité"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["sessionMode"], "invite_read_only");
    assert_eq!(body["deviceName"], "Overlay-User");
    assert_eq!(body["displayName"], "Invité");
    assert!(body["authorName"].is_null());

    let client_id =
        OverlayClientId::parse(body["clientId"].as_str().expect("clientId")).expect("uuid");
    let stored = db
        .overlay_clients()
        .get(client_id)
        .await
        .expect("lecture")
        .expect("client");
    assert_eq!(stored.session_mode, OverlaySessionMode::InviteReadOnly);
}

#[tokio::test]
async fn pair_consume_revokes_previous_client_with_same_label() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let previous = make_client(
        &guild(),
        "Poste salon",
        "previous-token",
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&previous)
        .await
        .expect("client précédent");
    db.pairing_codes()
        .create(&pairing_entry(
            "REVOKE",
            &guild(),
            PairingMode::Normal,
            false,
            Some("Alice"),
            None,
        ))
        .await
        .expect("code créé");

    let (status, _, _) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "REVOKE", "deviceName": "Poste salon"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let previous = db
        .overlay_clients()
        .get(previous.id)
        .await
        .expect("lecture")
        .expect("client");
    assert!(previous.revoked_at.is_some());
}

#[tokio::test]
async fn pair_consume_invite_requires_a_display_name_without_consuming() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let invite = invite_entry("K7QXM2PD", &guild(), GuildRole::Member, None, Some(2));
    db.guild_invites()
        .create(&invite)
        .await
        .expect("invitation créée");

    // Un nom vide (ou uniquement des espaces) est refusé et ne consomme rien.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "k7qx-m2pd", "displayName": "   "})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body_json(&bytes)["error"], "display_name_required");
    let stored = db
        .guild_invites()
        .get(&invite.code)
        .await
        .expect("lecture invitation")
        .expect("invitation conservée");
    assert_eq!(stored.use_count, 0, "le refus ne consomme pas l'invitation");

    // Avec un pseudo, l'invitation est consommée et le membre le reçoit.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "k7qx-m2pd", "displayName": "  Léa   B. "})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["displayName"], "Léa B.");
    assert_eq!(body["role"], "member");
    let stored = db
        .guild_invites()
        .get(&invite.code)
        .await
        .expect("lecture invitation")
        .expect("invitation conservée");
    assert_eq!(stored.use_count, 1);

    let client_id =
        OverlayClientId::parse(body["clientId"].as_str().expect("clientId")).expect("uuid");
    let client = db
        .overlay_clients()
        .get(client_id)
        .await
        .expect("lecture client")
        .expect("client créé");
    assert_eq!(client.default_author_name.as_deref(), Some("Léa B."));
    assert!(client.default_author_image.is_none());
}

#[tokio::test]
async fn pair_consume_pair_code_falls_back_to_the_discord_author() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let mut code = pairing_entry(
        "DISC42",
        &guild(),
        PairingMode::Normal,
        false,
        Some("Discordeur"),
        Some(fixed_now() + Duration::minutes(5)),
    );
    code.author_image = Some("https://cdn.example/avatar.png".to_owned());
    db.pairing_codes().create(&code).await.expect("code créé");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "DISC42"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["displayName"], "Discordeur");
    assert_eq!(body["role"], "member");

    let member_id =
        GuildMemberId::parse(body["memberId"].as_str().expect("memberId")).expect("uuid");
    let member = db
        .guild_members()
        .get(member_id)
        .await
        .expect("lecture membre")
        .expect("membre créé");
    assert_eq!(member.display_name, "Discordeur");
    assert_eq!(
        member.avatar.as_deref(),
        Some("https://cdn.example/avatar.png")
    );

    let client_id =
        OverlayClientId::parse(body["clientId"].as_str().expect("clientId")).expect("uuid");
    let client = db
        .overlay_clients()
        .get(client_id)
        .await
        .expect("lecture client")
        .expect("client créé");
    assert_eq!(client.default_author_name.as_deref(), Some("Discordeur"));
    assert_eq!(
        client.default_author_image.as_deref(),
        Some("https://cdn.example/avatar.png")
    );
}

#[tokio::test]
async fn pair_consume_pair_code_typed_name_and_avatar_win() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let mut code = pairing_entry(
        "TYPED42",
        &guild(),
        PairingMode::Normal,
        false,
        Some("Discordeur"),
        Some(fixed_now() + Duration::minutes(5)),
    );
    code.author_image = Some("https://cdn.example/discord.png".to_owned());
    db.pairing_codes().create(&code).await.expect("code créé");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({
                "code": "TYPED42",
                "displayName": "  Léa  ",
                "avatar": " https://typed.example/lea.png ",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["displayName"], "Léa");

    let member_id =
        GuildMemberId::parse(body["memberId"].as_str().expect("memberId")).expect("uuid");
    let member = db
        .guild_members()
        .get(member_id)
        .await
        .expect("lecture membre")
        .expect("membre créé");
    assert_eq!(member.display_name, "Léa");
    assert_eq!(
        member.avatar.as_deref(),
        Some("https://typed.example/lea.png")
    );

    let client_id =
        OverlayClientId::parse(body["clientId"].as_str().expect("clientId")).expect("uuid");
    let client = db
        .overlay_clients()
        .get(client_id)
        .await
        .expect("lecture client")
        .expect("client créé");
    assert_eq!(client.default_author_name.as_deref(), Some("Léa"));
    assert_eq!(
        client.default_author_image.as_deref(),
        Some("https://typed.example/lea.png")
    );
}

#[tokio::test]
async fn pair_consume_pair_code_without_any_name_is_refused_without_consuming() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let code = pairing_entry(
        "NONAME",
        &guild(),
        PairingMode::Normal,
        false,
        None,
        Some(fixed_now() + Duration::minutes(5)),
    );
    db.pairing_codes().create(&code).await.expect("code créé");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "NONAME"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body_json(&bytes)["error"], "display_name_required");
    let stored = db
        .pairing_codes()
        .get(&code.code)
        .await
        .expect("lecture code")
        .expect("code conservé");
    assert!(stored.used_at.is_none(), "le refus ne consomme pas le code");
}

#[tokio::test]
async fn pair_consume_device_code_needs_no_display_name() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let member = GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild(),
        display_name: "Porteur".to_owned(),
        avatar: Some("https://cdn.example/porteur.png".to_owned()),
        role: GuildRole::Member,
        created_at: fixed_now() - Duration::minutes(5),
        removed_at: None,
    };
    db.guild_members()
        .create(&member)
        .await
        .expect("membre créé");
    let invite = invite_entry(
        "DEVICE22",
        &guild(),
        GuildRole::Member,
        Some(member.id),
        Some(1),
    );
    db.guild_invites()
        .create(&invite)
        .await
        .expect("code appareil créé");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "DEVICE22"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["displayName"], "Porteur");
    assert_eq!(body["memberId"], member.id.to_canonical());
    assert_eq!(body["role"], "member");

    let client_id =
        OverlayClientId::parse(body["clientId"].as_str().expect("clientId")).expect("uuid");
    let client = db
        .overlay_clients()
        .get(client_id)
        .await
        .expect("lecture client")
        .expect("client créé");
    assert_eq!(client.default_author_name.as_deref(), Some("Porteur"));
    assert_eq!(
        client.default_author_image.as_deref(),
        Some("https://cdn.example/porteur.png")
    );
}

#[tokio::test]
async fn pair_consume_admin_pairing_code_grants_admin() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let code = pairing_entry(
        "ADMIN42",
        &guild(),
        PairingMode::Normal,
        true,
        Some("Chef"),
        Some(fixed_now() + Duration::minutes(5)),
    );
    db.pairing_codes()
        .create(&code)
        .await
        .expect("code admin créé");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(&json!({"code": "ADMIN42"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["role"], "admin");
    assert_eq!(body["sessionMode"], "normal");

    let member_id =
        GuildMemberId::parse(body["memberId"].as_str().expect("memberId")).expect("uuid");
    let member = db
        .guild_members()
        .get(member_id)
        .await
        .expect("lecture membre")
        .expect("membre créé");
    assert_eq!(member.role, GuildRole::Admin);
}

// --------------------------------------------------------------------------
// GET /overlay/media/{assetId}
// --------------------------------------------------------------------------

#[tokio::test]
async fn signed_media_serves_file_with_legacy_headers_and_range() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let bytes = b"0123456789abcdef";
    let asset = insert_asset(
        &db,
        dir.path(),
        "clip.mp4",
        "video/mp4",
        MediaKind::Video,
        bytes,
        fixed_now() + Duration::hours(12),
    )
    .await;
    let client = make_client(
        &guild(),
        "Poste",
        "media-token",
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client média");
    insert_playback_job(&db, &guild(), asset.id).await;
    let uri = signed_uri(&asset, &client, fixed_now_secs() + 300);

    let (status, headers, body) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, bytes);
    assert_eq!(headers[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(headers[header::ACCEPT_RANGES], "bytes");

    // `HEAD` (géré par la route `get` d'axum) : aucun corps.
    let (status, headers, body) = send(&app, request(Method::HEAD, &uri, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    assert_eq!(headers[header::CONTENT_LENGTH], bytes.len().to_string());

    // Range → 206 + Content-Range, corps partiel. `tower-http` ne compresse
    // jamais une réponse `Content-Range` : on vérifie qu'aucun encodage n'est
    // appliqué même si le client annonce gzip.
    let mut range_request = request(Method::GET, &uri, None, None);
    range_request.headers_mut().insert(
        header::RANGE,
        axum::http::HeaderValue::from_static("bytes=2-5"),
    );
    range_request.headers_mut().insert(
        header::ACCEPT_ENCODING,
        axum::http::HeaderValue::from_static("gzip"),
    );
    let (status, headers, body) = send(&app, range_request).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"2345");
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes 2-5/{}", bytes.len())
    );
    assert!(!headers.contains_key(header::CONTENT_ENCODING));

    // Range invalide → 416 (`Content-Range: bytes */total` conservé).
    let mut invalid_range = request(Method::GET, &uri, None, None);
    invalid_range.headers_mut().insert(
        header::RANGE,
        axum::http::HeaderValue::from_static("bytes=99-"),
    );
    let (status, headers, _) = send(&app, invalid_range).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes */{}", bytes.len())
    );
}

#[tokio::test]
async fn signed_media_requires_a_playback_job_in_the_client_guild() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let asset = insert_asset(
        &db,
        dir.path(),
        "binding.mp4",
        "video/mp4",
        MediaKind::Video,
        b"binding",
        fixed_now() + Duration::hours(12),
    )
    .await;
    let client = make_client(
        &guild(),
        "Poste",
        "binding-token",
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client média");
    let uri = signed_uri(&asset, &client, fixed_now_secs() + 300);

    // Sans job de la guilde, le média n'est pas accessible (parité legacy 403).
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "forbidden");

    // Un job d'une autre guilde ne suffit pas.
    insert_playback_job(&db, &other_guild(), asset.id).await;
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "forbidden");

    // Un job de la guilde du client ouvre l'accès.
    insert_playback_job(&db, &guild(), asset.id).await;
    let (status, _, body) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, b"binding");
}

#[tokio::test]
async fn signed_media_rejects_bad_signature_expiry_and_missing_params() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let asset = insert_asset(
        &db,
        dir.path(),
        "clip.mp4",
        "video/mp4",
        MediaKind::Video,
        b"payload",
        fixed_now() + Duration::hours(12),
    )
    .await;
    let client = make_client(
        &guild(),
        "Poste",
        "media-token",
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client média");

    // Paramètres absents.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            &format!("/overlay/media/{}", asset.id.to_canonical()),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");

    // Signature falsifiée.
    let uri = format!(
        "/overlay/media/{}?clientId={}&exp={}&sig={}",
        asset.id.to_canonical(),
        client.id.to_canonical(),
        fixed_now_secs() + 300,
        "0".repeat(64)
    );
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");

    // `exp` mal formé.
    let uri = format!(
        "/overlay/media/{}?clientId={}&exp=demain&sig={}",
        asset.id.to_canonical(),
        client.id.to_canonical(),
        "0".repeat(64)
    );
    let (status, _, _) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Signature valide mais exp dépassée.
    let uri = signed_uri(&asset, &client, fixed_now_secs() - 10);
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_expired");

    // Asset inconnu (UUID valide) → 404 media_not_found.
    let unknown = MediaAssetId::new_v4();
    let signature = crate::media_url::sign(
        MEDIA_SECRET.as_bytes(),
        &unknown.to_canonical(),
        &client.id.to_canonical(),
        fixed_now_secs() + 300,
    )
    .expect("signature");
    let uri = format!(
        "/overlay/media/{}?clientId={}&exp={}&sig={}",
        unknown.to_canonical(),
        client.id.to_canonical(),
        fixed_now_secs() + 300,
        signature
    );
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&bytes)["error"], "media_not_found");
}

#[tokio::test]
async fn signed_media_rejects_revoked_client_and_expired_asset() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let asset = insert_asset(
        &db,
        dir.path(),
        "clip.mp4",
        "video/mp4",
        MediaKind::Video,
        b"payload",
        fixed_now() + Duration::hours(12),
    )
    .await;
    let client = make_client(
        &guild(),
        "Poste",
        "media-token",
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client média");
    db.overlay_clients()
        .revoke(client.id, fixed_now())
        .await
        .expect("révocation");

    let uri = signed_uri(&asset, &client, fixed_now_secs() + 300);
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");

    // Asset expiré → 404 media_not_found (après réactivation du client).
    let fresh_client = make_client(
        &guild(),
        "Poste 2",
        "media-token-2",
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&fresh_client)
        .await
        .expect("client frais");
    let expired_asset = insert_asset(
        &db,
        dir.path(),
        "expired.mp4",
        "video/mp4",
        MediaKind::Video,
        b"payload",
        fixed_now() - Duration::minutes(1),
    )
    .await;
    let uri = signed_uri(&expired_asset, &fresh_client, fixed_now_secs() + 300);
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&bytes)["error"], "media_not_found");
}

#[tokio::test]
async fn signed_media_missing_or_escaped_file_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let outside = TempDir::new().expect("tempdir hors zone");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        "media-token",
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client média");

    // Fichier absent.
    let now = fixed_now();
    let missing = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: "hash-missing".to_owned(),
        source_url: "https://example.test/missing.mp4".to_owned(),
        kind: MediaKind::Video,
        mime: "video/mp4".to_owned(),
        duration_sec: Some(1),
        width: None,
        height: None,
        is_vertical: false,
        storage_path: dir.path().join("absent.mp4").to_string_lossy().into_owned(),
        size_bytes: 1,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: now,
        expires_at: now + Duration::hours(12),
        created_at: now,
    };
    db.media_assets()
        .insert(&missing)
        .await
        .expect("asset manquant");
    insert_playback_job(&db, &guild(), missing.id).await;
    let uri = signed_uri(&missing, &client, fixed_now_secs() + 300);
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&bytes)["error"], "media_not_found_on_disk");

    // Fichier hors de MEDIA_STORAGE_DIR (traversée refusée).
    let escaped_path = outside.path().join("escape.mp4");
    tokio::fs::write(&escaped_path, b"evil")
        .await
        .expect("écriture hors zone");
    let escaped = MediaAsset {
        id: MediaAssetId::new_v4(),
        source_hash: "hash-escaped".to_owned(),
        source_url: "https://example.test/escape.mp4".to_owned(),
        kind: MediaKind::Video,
        mime: "video/mp4".to_owned(),
        duration_sec: Some(1),
        width: None,
        height: None,
        is_vertical: false,
        storage_path: escaped_path.to_string_lossy().into_owned(),
        size_bytes: 4,
        status: MediaStatus::Ready,
        error: None,
        last_accessed_at: now,
        expires_at: now + Duration::hours(12),
        created_at: now,
    };
    db.media_assets()
        .insert(&escaped)
        .await
        .expect("asset hors zone");
    insert_playback_job(&db, &guild(), escaped.id).await;
    let uri = signed_uri(&escaped, &client, fixed_now_secs() + 300);
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&bytes)["error"], "media_not_found_on_disk");
}

// --------------------------------------------------------------------------
// /overlay/media-board/*
// --------------------------------------------------------------------------

#[tokio::test]
async fn media_board_requires_bearer_token_only() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");

    let (status, _, bytes) = send(
        &app,
        request(Method::GET, "/overlay/media-board/items", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(&bytes)["error"], "unauthorized");

    // Le repli `?token=` a été supprimé : un jeton valide en query n'est plus
    // accepté sur aucune route board.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            &format!("/overlay/media-board/items?token={BEARER_TOKEN}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(&bytes)["error"], "unauthorized");

    // Mauvais jeton.
    let (status, _, _) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items",
            Some("mauvais-jeton"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Bon jeton.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["total"], 0);
    assert_eq!(body["limit"], 50);
    assert_eq!(body["offset"], 0);

    // `guildId` d'une autre guilde → 403.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            &format!(
                "/overlay/media-board/items?guildId={}",
                other_guild().as_str()
            ),
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "forbidden");
}

#[tokio::test]
async fn media_board_lists_filters_and_paginates() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");

    let asset_a = insert_asset(
        &db,
        dir.path(),
        "alpha.mp4",
        "video/mp4",
        MediaKind::Video,
        b"aaa",
        fixed_now() + Duration::hours(12),
    )
    .await;
    let asset_b = insert_asset(
        &db,
        dir.path(),
        "beta.mp4",
        "video/mp4",
        MediaKind::Video,
        b"bbb",
        fixed_now() + Duration::hours(12),
    )
    .await;
    let asset_c = insert_asset(
        &db,
        dir.path(),
        "clip-gamma.mp4",
        "video/mp4",
        MediaKind::Video,
        b"ccc",
        fixed_now() + Duration::hours(12),
    )
    .await;
    insert_item(
        &db,
        &guild(),
        &asset_a,
        Some("Alpha"),
        None,
        fixed_now() - Duration::minutes(3),
    )
    .await;
    insert_item(
        &db,
        &guild(),
        &asset_b,
        Some("Beta"),
        Some("deux"),
        fixed_now() - Duration::minutes(2),
    )
    .await;
    insert_item(
        &db,
        &guild(),
        &asset_c,
        Some("Gamma"),
        None,
        fixed_now() - Duration::minutes(1),
    )
    .await;

    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["total"], 3);
    assert_eq!(body["items"].as_array().expect("items").len(), 3);
    assert_eq!(body["items"][0]["title"], "Gamma");
    assert_eq!(body["items"][0]["media"]["mime"], "video/mp4");
    assert_eq!(body["items"][0]["media"]["kind"], "video");
    assert_eq!(body["items"][0]["createdByName"], "Alice");
    assert!(body["items"][0]["createdAt"]
        .as_str()
        .expect("createdAt")
        .ends_with('Z'));

    // Chaque item porte une URL de média signée pour le client demandeur.
    let expected_exp = fixed_now_secs() + 3600;
    let media_url = body["items"][0]["mediaUrl"]
        .as_str()
        .expect("mediaUrl signée");
    assert!(
        media_url.starts_with("http://127.0.0.1:8099/overlay/media-board/media/"),
        "{media_url}"
    );
    assert!(
        media_url.contains(&format!("clientId={}", client.id.to_canonical())),
        "{media_url}"
    );
    assert_eq!(
        body["items"][0]["mediaUrlExpiresAt"],
        json!(expected_exp * 1000)
    );
    let path = media_url
        .strip_prefix("http://127.0.0.1:8099")
        .expect("origine API");
    let (status, headers, fetched) = send(&app, request(Method::GET, path, None, None)).await;
    assert_eq!(status, StatusCode::OK, "{fetched:?}");
    assert_eq!(fetched, b"ccc");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let mut ranged = request(Method::GET, path, None, None);
    ranged.headers_mut().insert(
        header::RANGE,
        axum::http::HeaderValue::from_static("bytes=1-2"),
    );
    let (status, _, ranged_body) = send(&app, ranged).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(ranged_body, b"cc");

    // Filtre `q` (titre + URL source).
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items?q=beta",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&bytes)["total"], 1);
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items?q=clip",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&bytes)["total"], 1);

    // Pagination.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items?limit=2&offset=1",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["total"], 3);
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 1);
    assert_eq!(body["items"].as_array().expect("items").len(), 2);

    // Clamp legacy : limit=0 → 1, limit=999 → 200.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items?limit=0",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&bytes)["limit"], 1);
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/media-board/items?limit=999",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&bytes)["limit"], 200);
}

#[tokio::test]
async fn media_board_patch_and_delete_semantics() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");
    let other = make_client(
        &other_guild(),
        "Autre poste",
        OTHER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Bob"),
    );
    db.overlay_clients()
        .create(&other)
        .await
        .expect("client autre guilde");

    let asset = insert_asset(
        &db,
        dir.path(),
        "alpha.mp4",
        "video/mp4",
        MediaKind::Video,
        b"aaa",
        fixed_now() + Duration::hours(12),
    )
    .await;
    let item = insert_item(
        &db,
        &guild(),
        &asset,
        Some("Ancien titre"),
        Some("Ancien message"),
        fixed_now(),
    )
    .await;

    let uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());

    // Aucun champ → 400 invalid_payload.
    let (status, _, bytes) = send(
        &app,
        request(Method::PATCH, &uri, Some(BEARER_TOKEN), Some(&json!({}))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body_json(&bytes)["error"], "invalid_payload");

    // Titre seul : le message est conservé.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::PATCH,
            &uri,
            Some(BEARER_TOKEN),
            Some(&json!({"title": "  Nouveau titre  "})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["updated"], true);
    assert_eq!(body["item"]["title"], "Nouveau titre");
    let stored = db
        .media_board()
        .get(item.id)
        .await
        .expect("lecture item")
        .expect("item");
    assert_eq!(stored.title.as_deref(), Some("Nouveau titre"));
    assert_eq!(stored.message.as_deref(), Some("Ancien message"));

    // `null` efface le champ.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::PATCH,
            &uri,
            Some(BEARER_TOKEN),
            Some(&json!({"message": null})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body_json(&bytes)["item"]["message"].is_null());

    // Item d'une autre guilde → 404 item_not_found.
    let (status, _, _) = send(
        &app,
        request(
            Method::PATCH,
            &uri,
            Some(OTHER_TOKEN),
            Some(&json!({"title": "x"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Item inconnu → 404 item_not_found.
    let (status, _, _) = send(
        &app,
        request(
            Method::PATCH,
            &format!(
                "/overlay/media-board/items/{}",
                MediaBoardItemId::new_v4().to_canonical()
            ),
            Some(BEARER_TOKEN),
            Some(&json!({"title": "x"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // DELETE : première fois true, seconde false.
    let (status, _, bytes) = send(
        &app,
        request(Method::DELETE, &uri, Some(BEARER_TOKEN), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&bytes)["deleted"], true);
    let (status, _, bytes) = send(
        &app,
        request(Method::DELETE, &uri, Some(BEARER_TOKEN), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&bytes)["deleted"], false);
}

#[tokio::test]
async fn media_board_create_rejects_missing_url_without_network() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");

    let (status, _, bytes) = send(
        &app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(BEARER_TOKEN),
            Some(&json!({"title": "sans URL"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let body = body_json(&bytes);
    assert_eq!(body["error"], "invalid_payload");
    assert_eq!(body["message"], "url is required");

    // URL uniquement blanche → même erreur, aucun appel réseau.
    let (status, _, _) = send(
        &app,
        request(
            Method::POST,
            "/overlay/media-board/items",
            Some(BEARER_TOKEN),
            Some(&json!({"url": "   "})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn media_board_media_requires_a_valid_signed_url() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");
    let asset = insert_asset(
        &db,
        dir.path(),
        "board.mp4",
        "video/mp4",
        MediaKind::Video,
        b"board-media",
        fixed_now() + Duration::hours(12),
    )
    .await;
    insert_item(&db, &guild(), &asset, Some("Board"), None, fixed_now()).await;
    let asset_key = asset.id.to_canonical();
    let client_key = client.id.to_canonical();
    let path = format!("/overlay/media-board/media/{asset_key}");

    // Sans paramètres signés : refus, même avec un Bearer valide.
    let (status, _, bytes) = send(&app, request(Method::GET, &path, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");
    let (status, _, bytes) =
        send(&app, request(Method::GET, &path, Some(BEARER_TOKEN), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");

    // `?token=` n'est plus accepté sur cette route.
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            &format!("{path}?token={BEARER_TOKEN}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");

    // Signature falsifiée.
    let exp = fixed_now_secs() + 300;
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            &format!(
                "{path}?clientId={client_key}&exp={exp}&sig={}",
                "0".repeat(64)
            ),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");

    // Signature de lecture (sans le préfixe `board:`) : refusée sur le board.
    let playback_signature =
        crate::media_url::sign(MEDIA_SECRET.as_bytes(), &asset_key, &client_key, exp)
            .expect("signature lecture");
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            &format!("{path}?clientId={client_key}&exp={exp}&sig={playback_signature}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");

    // Expiration dépassée.
    let expired_uri = signed_board_uri(&asset, &client, fixed_now_secs() - 10);
    let (status, _, bytes) = send(&app, request(Method::GET, &expired_uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_expired");

    // Client révoqué : l'URL signée ne sert plus.
    db.overlay_clients()
        .revoke(client.id, fixed_now())
        .await
        .expect("révocation");
    let valid_uri = signed_board_uri(&asset, &client, exp);
    let (status, _, bytes) = send(&app, request(Method::GET, &valid_uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&bytes)["error"], "media_url_invalid");
}

#[tokio::test]
async fn media_board_media_is_bound_to_the_client_guild_and_role() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");
    let other = make_client(
        &other_guild(),
        "Autre poste",
        OTHER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Bob"),
    );
    db.overlay_clients()
        .create(&other)
        .await
        .expect("client autre guilde");

    let bytes = b"board-media";
    let asset = insert_asset(
        &db,
        dir.path(),
        "board.mp4",
        "video/mp4",
        MediaKind::Video,
        bytes,
        fixed_now() + Duration::hours(12),
    )
    .await;
    insert_item(&db, &guild(), &asset, Some("Board"), None, fixed_now()).await;
    let exp = fixed_now_secs() + 300;

    // URL valide : 200 puis HEAD (aucun corps).
    let uri = signed_board_uri(&asset, &client, exp);
    let (status, headers, body) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, bytes);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let (status, headers, head_body) = send(&app, request(Method::HEAD, &uri, None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(head_body.is_empty());
    assert_eq!(headers[header::CONTENT_LENGTH], bytes.len().to_string());

    // Asset du board d'une autre guilde : refus.
    let other_uri = signed_board_uri(&asset, &other, exp);
    let (status, _, body) = send(&app, request(Method::GET, &other_uri, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&body)["error"], "media_not_found");

    // Asset inconnu (UUID valide) : refus.
    let unknown = MediaAsset {
        id: MediaAssetId::new_v4(),
        ..asset.clone()
    };
    let unknown_uri = signed_board_uri(&unknown, &client, exp);
    let (status, _, _) = send(&app, request(Method::GET, &unknown_uri, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Un invité dont la guilde interdit le board reçoit 403 forbidden.
    let guest = make_client(
        &guild(),
        "Invité",
        "guest-token",
        OverlaySessionMode::InviteReadOnly,
        None,
    );
    db.overlay_clients()
        .create(&guest)
        .await
        .expect("client invité");
    db.guilds()
        .set_guests_can_view_board(&guild(), false)
        .await
        .expect("réglage de guilde");
    let guest_uri = signed_board_uri(&asset, &guest, exp);
    let (status, _, body) = send(&app, request(Method::GET, &guest_uri, None, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body_json(&body)["error"], "forbidden");
}

// --------------------------------------------------------------------------
// Suppression de la route `/overlay/config`
// --------------------------------------------------------------------------

#[tokio::test]
async fn overlay_config_route_is_absent_in_v2() {
    let dir = TempDir::new().expect("tempdir");
    let (app, _db) = harness(dir.path()).await;
    let (status, _, bytes) = send(&app, request(Method::GET, "/overlay/config", None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&bytes)["error"], "request_error");
}

// --------------------------------------------------------------------------
// Helper de signature réutilisable par le hub (Phase 4b)
// --------------------------------------------------------------------------

#[test]
fn sign_media_url_matches_protocol_v2_format_and_verifies() {
    let asset = MediaAssetId::new_v4();
    let client = OverlayClientId::new_v4();
    let exp = fixed_now_secs() + 300;

    let uri = super::sign_media_url(
        MEDIA_SECRET.as_bytes(),
        "http://127.0.0.1:8099/",
        &asset,
        &client,
        exp,
    )
    .expect("URL signée");
    let expected_prefix = format!(
        "http://127.0.0.1:8099/overlay/media/{}?clientId={}&exp={exp}&sig=",
        asset.to_canonical(),
        client.to_canonical()
    );
    assert!(uri.starts_with(&expected_prefix), "URL inattendue : {uri}");

    let signature = uri.rsplit("sig=").next().expect("signature");
    assert_eq!(signature.len(), 64);
    assert!(signature.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(crate::media_url::verify(
        MEDIA_SECRET.as_bytes(),
        &asset.to_canonical(),
        &client.to_canonical(),
        exp,
        signature,
    ));

    // Un autre secret ne vérifie pas.
    assert!(!crate::media_url::verify(
        b"mauvais-secret",
        &asset.to_canonical(),
        &client.to_canonical(),
        exp,
        signature,
    ));

    // Le TTL du protocole est bien celui attendu pour une vidéo de 30 s.
    assert_eq!(
        lc_protocol::media_url_ttl_sec(lc_protocol::MediaKind::Video, Some(30)),
        300
    );
}

#[tokio::test]
async fn media_board_create_rejects_malformed_json() {
    // AUD-A2-009 : JSON illisible -> 400 invalid_payload (pas request_error),
    // apres le controle de session en ecriture.
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");

    let (status, _, bytes) = send(
        &app,
        raw_request(
            Method::POST,
            "/overlay/media-board/items",
            Some(BEARER_TOKEN),
            "{\"url\":",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body_json(&bytes)["error"], "invalid_payload");
}

#[tokio::test]
async fn media_board_patch_rejects_malformed_json() {
    let dir = TempDir::new().expect("tempdir");
    let (app, db) = harness(dir.path()).await;
    let client = make_client(
        &guild(),
        "Poste",
        BEARER_TOKEN,
        OverlaySessionMode::Normal,
        Some("Alice"),
    );
    db.overlay_clients()
        .create(&client)
        .await
        .expect("client board");
    let asset = insert_asset(
        &db,
        dir.path(),
        "malformed.mp4",
        "video/mp4",
        MediaKind::Video,
        b"malformed",
        fixed_now() + Duration::hours(12),
    )
    .await;
    let item = insert_item(&db, &guild(), &asset, Some("Titre"), None, fixed_now()).await;

    let uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());
    let (status, _, bytes) = send(
        &app,
        raw_request(Method::PATCH, &uri, Some(BEARER_TOKEN), "{"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body_json(&bytes)["error"], "invalid_payload");
}
