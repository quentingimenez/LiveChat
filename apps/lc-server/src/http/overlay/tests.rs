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
    FixedClock, GuildId, MediaAsset, MediaAssetId, MediaKind, MediaStatus, MemeBoardItem,
    MemeBoardItemId, OverlayClient, OverlayClientId, OverlaySessionMode, PairingCode,
    PairingCodeEntry, PairingMode,
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
    author: Option<&str>,
    expires_at: Option<DateTime<Utc>>,
) -> PairingCodeEntry {
    PairingCodeEntry {
        code: PairingCode::parse(code).expect("code valide"),
        guild_id: guild_id.clone(),
        mode,
        created_by_discord_user_id: "4242".to_owned(),
        author_name: author.map(str::to_owned),
        author_image: None,
        expires_at,
        used_at: None,
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

/// Insère un item de board.
async fn insert_item(
    db: &Db,
    guild_id: &GuildId,
    asset: &MediaAsset,
    title: Option<&str>,
    message: Option<&str>,
    created_at: DateTime<Utc>,
) -> MemeBoardItem {
    let item = MemeBoardItem {
        id: MemeBoardItemId::new_v4(),
        guild_id: guild_id.clone(),
        media_asset_id: asset.id,
        title: title.map(str::to_owned),
        message: message.map(str::to_owned),
        created_by_discord_user_id: Some("4242".to_owned()),
        created_by_name: Some("Alice".to_owned()),
        created_at,
    };
    db.meme_board().create(&item).await.expect("insertion item");
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
            Some(&json!({"code": "invite"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&bytes);
    assert_eq!(body["sessionMode"], "invite_read_only");
    assert_eq!(body["deviceName"], "Overlay-User");
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
    let uri = signed_uri(&escaped, &client, fixed_now_secs() + 300);
    let (status, _, bytes) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&bytes)["error"], "media_not_found_on_disk");
}

// --------------------------------------------------------------------------
// /overlay/meme-board/*
// --------------------------------------------------------------------------

#[tokio::test]
async fn meme_board_requires_bearer_and_rejects_query_token() {
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
        request(Method::GET, "/overlay/meme-board/items", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(&bytes)["error"], "unauthorized");

    // Fallback `?token=` supprimé en v2 : ignoré, donc 401.
    let (status, _, _) = send(
        &app,
        request(
            Method::GET,
            &format!("/overlay/meme-board/items?token={BEARER_TOKEN}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Mauvais jeton.
    let (status, _, _) = send(
        &app,
        request(
            Method::GET,
            "/overlay/meme-board/items",
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
            "/overlay/meme-board/items",
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
                "/overlay/meme-board/items?guildId={}",
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
async fn meme_board_lists_filters_and_paginates() {
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
            "/overlay/meme-board/items",
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

    // Filtre `q` (titre + URL source).
    let (status, _, bytes) = send(
        &app,
        request(
            Method::GET,
            "/overlay/meme-board/items?q=beta",
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
            "/overlay/meme-board/items?q=clip",
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
            "/overlay/meme-board/items?limit=2&offset=1",
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
            "/overlay/meme-board/items?limit=0",
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
            "/overlay/meme-board/items?limit=999",
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&bytes)["limit"], 200);
}

#[tokio::test]
async fn meme_board_patch_and_delete_semantics() {
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

    let uri = format!("/overlay/meme-board/items/{}", item.id.to_canonical());

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
        .meme_board()
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
                "/overlay/meme-board/items/{}",
                MemeBoardItemId::new_v4().to_canonical()
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
async fn meme_board_create_rejects_missing_url_without_network() {
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
            "/overlay/meme-board/items",
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
            "/overlay/meme-board/items",
            Some(BEARER_TOKEN),
            Some(&json!({"url": "   "})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn meme_board_media_requires_auth_and_binding() {
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
    let uri = format!("/overlay/meme-board/media/{}", asset.id.to_canonical());

    let (status, _, _) = send(&app, request(Method::GET, &uri, None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, bytes_out) =
        send(&app, request(Method::GET, &uri, Some(OTHER_TOKEN), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body_json(&bytes_out)["error"], "media_not_found");

    let (status, headers, bytes_out) =
        send(&app, request(Method::GET, &uri, Some(BEARER_TOKEN), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes_out, bytes);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");

    // Asset inconnu → 404 media_not_found.
    let (status, _, _) = send(
        &app,
        request(
            Method::GET,
            &format!(
                "/overlay/meme-board/media/{}",
                MediaAssetId::new_v4().to_canonical()
            ),
            Some(BEARER_TOKEN),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
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
