//! Routes HTTP `/overlay/*` — parité REST avec l'ancien serveur Node (Phase 3b).
//!
//! ## Périmètre
//!
//! Ce module remplace `server/src/components/overlay/overlayRoutes.ts` et
//! `server/src/services/overlayAuth.ts` (voir `docs/rust/server-overlay.md`).
//! Il expose :
//!
//! | Méthode | Chemin | Authentification |
//! |---------|--------|------------------|
//! | `POST` | `/overlay/pair/consume` | publique (code d'appairage) |
//! | `GET` | `/overlay/media/{assetId}` | URL signée HMAC (`exp`, `sig`, `clientId`) |
//! | `GET` | `/overlay/media-board/items` | `Authorization: Bearer` (repli `?token=`) |
//! | `POST` | `/overlay/media-board/items` | `Authorization: Bearer` (repli `?token=`) — **écriture refusée en `invite_read_only`** |
//! | `POST` | `/overlay/media-board/upload?filename=&title=&message=` | `Authorization: Bearer` (query-token fallback) — **writes refused in `invite_read_only`** |
//! | `GET` | `/overlay/media-board/ingestions` | `Authorization: Bearer` (repli `?token=`) |
//! | `GET` | `/overlay/media-board/media/{assetId}` | `Authorization: Bearer` (repli `?token=`) |
//! | `PATCH` | `/overlay/media-board/items/{itemId}` | `Authorization: Bearer` (repli `?token=`) — **écriture refusée en `invite_read_only`** |
//! | `DELETE` | `/overlay/media-board/items/{itemId}` | `Authorization: Bearer` (repli `?token=`) — **écriture refusée en `invite_read_only`** |
//!
//! `GET /overlay/config` **n'existe pas** en protocole v2 (supprimé, cf.
//! `docs/rust/protocol-v2.md` §8) : la route tombe sur le `fallback` de
//! `crate::app`, qui répond `404 {"error":"request_error"}`.
//!
//! ## Sessions en lecture seule
//!
//! Un jeton dont `overlay_clients.session_mode = INVITE_READ_ONLY` peut lire la
//! board (`GET items`, `GET ingestions`, `GET media`) mais **pas** la modifier :
//! `POST`, `PATCH` et `DELETE` répondent `403 {"error":"read_only_session"}` (garde
//! `media_board::ensure_writable`, AUD-V-003). La garde s'applique après
//! l'authentification et donc identiquement pour `Authorization: Bearer` et
//! pour le repli `?token=` du board.
//!
//! ## Conventions
//!
//! - AUCUN handler ne lit l'horloge système : tout instant vient de
//!   `state.clock` (tests déterministes, cf. `docs/rust/server-core.md` §9).
//! - Les erreurs applicatives sont déjà au format JSON minimal du legacy
//!   (`{"error": "<code>"}`, éventuellement enrichi de `message`/`code`) : le
//!   middleware `normalize_errors` laisse intact un corps JSON.
//! - Les accès base passent par `state.db` (`Option<Db>`) ; une base absente
//!   répond `503 {"error":"database_unavailable"}` plutôt que de paniquer.
//! - Aucun `unwrap`/`expect`/`panic` (lints workspace).
//!
//! ## Handoff Phase 4b / Phase 8
//!
//! - Les URLs signées sont générées par le hub WebSocket (Phase 4b) via
//!   [`crate::media_url`] ; la signature et sa vérification sont partagées.
//! - Le renderer board (Phase 8) envoie un en-tête `Authorization` quand c'est
//!   possible ; pour les balises `src` (`<video>`, `<img>`) qui ne peuvent pas
//!   porter d'en-tête, l'extracteur accepte un repli `?token=` **limité aux
//!   routes du board média** `/overlay/media-board/*` (cf.
//!   `http/overlay/auth.rs::query_token`). Ce repli n'existe sur aucune autre
//!   route overlay.

// Les helpers (`db`, `resolve_media_file`, ...) et certains handlers renvoient
// `Result<_, Response>` : `Response` est le type d'erreur natif d'axum et le
// boxing systématique dégraderait la lisibilité sans gain mesurable sur ces
// chemins I/O. Le lint `result_large_err` est donc neutralisé à l'échelle du
// module (cf. décision dans `docs/rust/server-overlay.md` §2).
#![allow(clippy::result_large_err)]

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::error;

use crate::app::AppState;

mod admin;
mod auth;
mod discord_link;
pub(crate) mod guild;
mod ingest_code;
mod media;
mod media_board;
mod media_ingestion;
mod media_upload;
mod pair;
mod queue;

pub use media::sign_media_url;

#[cfg(test)]
mod guild_tests;
#[cfg(test)]
mod ingestion_tests;
#[cfg(test)]
mod media_upload_tests;
#[cfg(test)]
mod tests;

/// Sous-routeur monté par `crate::http::mod` puis `crate::app::router`.
///
/// Sous-routeur `/overlay`, monté tel quel (fusion sans préfixe) par
/// [`crate::app::router`] : il porte donc lui-même le préfixe `/overlay`,
/// comme [`crate::http::ingest::router`].
///
/// Les routes du board média exigent l'extracteur [`auth::AuthedOverlayClient`]
/// (Bearer uniquement, exécuté avant le handler) ; l'appairage et le média
/// signé ont leurs propres contrôles (code d'appairage / signature HMAC).
pub fn router() -> Router<AppState> {
    let authed = Router::new()
        .route(
            "/overlay/media-board/items",
            get(media_board::list_items).post(media_board::create_item),
        )
        .route(
            "/overlay/media-board/upload",
            post(media_upload::upload_item),
        )
        .route(
            "/overlay/media-board/ingestions",
            get(media_board::list_ingestions),
        )
        .route(
            "/overlay/media-board/items/{item_id}",
            axum::routing::patch(media_board::update_item).delete(media_board::delete_item),
        )
        .route(
            "/overlay/media-board/media/{asset_id}",
            get(media_board::get_media),
        )
        .route("/overlay/guild", get(guild::get_guild))
        .route(
            "/overlay/profile",
            axum::routing::patch(guild::update_profile),
        )
        .route("/overlay/device-code", post(guild::create_device_code))
        .route(
            "/overlay/ingest-code",
            post(ingest_code::create_ingest_code),
        )
        .route("/overlay/leave", post(guild::leave))
        .route(
            "/overlay/queue",
            get(queue::get_queue).delete(queue::clear_queue),
        )
        .route(
            "/overlay/queue/{job_id}",
            axum::routing::delete(queue::remove_job),
        )
        .route("/overlay/admin/members", get(admin::list_members))
        .route(
            "/overlay/admin/members/{member_id}",
            axum::routing::patch(admin::update_member).delete(admin::remove_member_handler),
        )
        .route(
            "/overlay/admin/invites",
            get(admin::list_invites).post(admin::create_invite_handler),
        )
        .route(
            "/overlay/admin/invites/{code}/revoke",
            post(admin::revoke_invite),
        )
        .route(
            "/overlay/admin/settings",
            axum::routing::patch(admin::update_settings),
        )
        .route(
            "/overlay/admin/discord-link-code",
            post(discord_link::create_link_code),
        )
        .route(
            "/overlay/admin/discord-link",
            axum::routing::delete(discord_link::unlink),
        );

    Router::new()
        .route("/overlay/pair/consume", post(pair::consume_pairing))
        .route("/overlay/media/{asset_id}", get(media::get_media))
        .merge(authed)
}

/// Instant courant, toujours lu depuis l'horloge injectée.
fn now(state: &AppState) -> DateTime<Utc> {
    state.clock.now()
}

/// Accès à la base ou réponse `503` normalisée.
fn db(state: &AppState) -> Result<&lc_db::Db, Response> {
    state.db.as_ref().ok_or_else(database_unavailable)
}

/// Réponse JSON applicative.
fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Erreur JSON minimale du legacy (`{"error": code}`).
fn error(status: StatusCode, code: &'static str) -> Response {
    crate::http::error::json_error(status, code)
}

/// `401 {"error":"unauthorized"}`.
fn unauthorized() -> Response {
    error(StatusCode::UNAUTHORIZED, "unauthorized")
}

/// `500 {"error":"internal_error"}`.
fn internal_error() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}

/// `503 {"error":"database_unavailable"}` (serveur démarré sans `DATABASE_URL`).
fn database_unavailable() -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
}

/// Journalise une erreur `lc-db` et renvoie `500 internal_error`.
fn db_failure(operation: &'static str, db_error: &lc_db::DbError) -> Response {
    error!(operation, error = %db_error, "échec d'accès base sur une route overlay");
    internal_error()
}

/// Conflit d'unicité quel que soit le chemin de conversion de l'erreur.
///
/// `lc-db` convertit parfois `SQLITE_CONSTRAINT` en `DomainError::Conflict`
/// (`map_sqlx_error`), parfois laisse le `DbError::Sqlx` brut : les deux formes
/// doivent être reconnues, sinon les branches de récupération sont mortes
/// (AUD-A2-010).
fn is_conflict(db_error: &lc_db::DbError) -> bool {
    db_error.is_unique_violation()
        || matches!(
            db_error,
            lc_db::DbError::Domain(lc_domain::DomainError::Conflict(_))
        )
}

/// Tests ciblés AUD-V-003 : garde `read_only_session` sur les écritures board.
///
/// Ces tests montent le routeur complet (`crate::app::router`) comme
/// `tests.rs`, mais vivent ici pour rester dans le périmètre de la correction ;
/// ils couvrent les deux voies d'authentification (Bearer et `?token=`) et les
/// deux modes de session (`normal` / `invite_read_only`).
#[cfg(test)]
mod session_mode_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::Path;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{header, Method, Request, StatusCode};
    use axum::Router;
    use chrono::{DateTime, TimeZone, Utc};
    use lc_db::{Db, DbConfig};
    use lc_domain::{
        FixedClock, GuildId, MediaAsset, MediaAssetId, MediaBoardItem, MediaBoardItemId, MediaKind,
        MediaStatus, OverlayClient, OverlayClientId, OverlaySessionMode,
    };
    use serde_json::{json, Value};
    use tempfile::TempDir;
    use tower::ServiceExt as _;

    use crate::app::AppState;
    use crate::config::Config;
    use crate::shutdown::Shutdown;

    use super::auth;

    /// Jeton (64 hex) d'un client en session normale.
    const NORMAL_TOKEN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    /// Jeton (64 hex) d'un client en session `invite_read_only`.
    const READ_ONLY_TOKEN: &str =
        "2222222222222222222222222222222222222222222222222222222222222222";
    /// Guilde unique des tests de session.
    const GUILD: &str = "123456789012345678";

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
            .single()
            .unwrap()
    }

    fn guild() -> GuildId {
        GuildId::parse(GUILD).unwrap()
    }

    fn test_config(media_dir: &Path) -> Config {
        let pairs = vec![
            ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
            (
                "MEDIA_URL_SECRET".to_owned(),
                "secret-test-session-mode".to_owned(),
            ),
            (
                "MEDIA_STORAGE_DIR".to_owned(),
                media_dir.to_string_lossy().into_owned(),
            ),
            ("MEDIA_BOARD_MAX_TOTAL_MB".to_owned(), "64".to_owned()),
        ];
        Config::from_pairs(pairs).unwrap()
    }

    async fn harness(media_dir: &Path) -> (Router, Db) {
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
        let state = AppState::new(
            Arc::new(test_config(media_dir)),
            Some(db.clone()),
            Shutdown::detached(),
        )
        .with_clock(clock);
        (crate::app::router(state), db)
    }

    async fn insert_client(db: &Db, mode: OverlaySessionMode, token: &str) -> OverlayClient {
        let now = fixed_now();
        let client = OverlayClient {
            id: OverlayClientId::new_v4(),
            guild_id: guild(),
            label: "poste-test".to_owned(),
            token_hash: auth::hash_token(token),
            session_mode: mode,
            default_author_name: None,
            default_author_image: None,
            created_by_discord_user_id: None,
            last_seen_at: None,
            created_at: now,
            revoked_at: None,
        };
        db.overlay_clients().create(&client).await.unwrap();
        client
    }

    async fn insert_asset(db: &Db, media_dir: &Path) -> MediaAsset {
        let file_path = media_dir.join("media-test.bin");
        tokio::fs::write(&file_path, b"media-test-body")
            .await
            .unwrap();
        let now = fixed_now();
        let asset = MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "hash-test-session-mode".to_owned(),
            source_url: "https://example.com/media.mp4".to_owned(),
            kind: MediaKind::Video,
            mime: "video/mp4".to_owned(),
            duration_sec: Some(30),
            width: Some(1280),
            height: Some(720),
            is_vertical: false,
            storage_path: file_path.to_string_lossy().into_owned(),
            size_bytes: 15,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + chrono::Duration::days(1),
            created_at: now,
        };
        db.media_assets().insert(&asset).await.unwrap();
        asset
    }

    async fn insert_item(db: &Db, asset: &MediaAsset, title: &str) -> MediaBoardItem {
        let item = MediaBoardItem {
            id: MediaBoardItemId::new_v4(),
            guild_id: guild(),
            media_asset_id: asset.id,
            title: Some(title.to_owned()),
            message: None,
            created_by_discord_user_id: None,
            created_by_name: Some("test".to_owned()),
            created_at: fixed_now(),
        };
        db.media_board().create(&item).await.unwrap();
        item
    }

    fn request(
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        match body {
            Some(body) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                builder
                    .body(Body::from(serde_json::to_vec(body).unwrap()))
                    .unwrap()
            }
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    async fn send_status(app: &Router, request: Request<Body>) -> StatusCode {
        app.clone().oneshot(request).await.unwrap().status()
    }

    async fn send_json(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }

    /// Session invitée : lecture autorisée, toutes les écritures refusées.
    #[tokio::test]
    async fn read_only_allows_reads_and_blocks_mutations() {
        let dir = TempDir::new().unwrap();
        let (app, db) = harness(dir.path()).await;
        insert_client(&db, OverlaySessionMode::InviteReadOnly, READ_ONLY_TOKEN).await;
        let asset = insert_asset(&db, dir.path()).await;
        let item = insert_item(&db, &asset, "titre initial").await;

        // GET items et GET media restent autorisés.
        let (status, body) = send_json(
            &app,
            request(
                Method::GET,
                "/overlay/media-board/items",
                Some(READ_ONLY_TOKEN),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["items"].as_array().map(Vec::len), Some(1));
        let media_uri = format!("/overlay/media-board/media/{}", asset.id.to_canonical());
        let status = send_status(
            &app,
            request(Method::GET, &media_uri, Some(READ_ONLY_TOKEN), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // POST : refusé avant même la validation de la charge utile.
        let (status, body) = send_json(
            &app,
            request(
                Method::POST,
                "/overlay/media-board/items",
                Some(READ_ONLY_TOKEN),
                Some(&json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // PATCH : refusé.
        let item_uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());
        let (status, body) = send_json(
            &app,
            request(
                Method::PATCH,
                &item_uri,
                Some(READ_ONLY_TOKEN),
                Some(&json!({"title": "hack"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // DELETE : refusé.
        let (status, body) = send_json(
            &app,
            request(Method::DELETE, &item_uri, Some(READ_ONLY_TOKEN), None),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // L'item est intact en base (aucune mutation partielle).
        let stored = db
            .media_board()
            .get(item.id)
            .await
            .unwrap()
            .expect("item conservé");
        assert_eq!(stored.title.as_deref(), Some("titre initial"));
    }

    /// Session normale : comportement legacy inchangé.
    #[tokio::test]
    async fn normal_session_mutations_stay_allowed() {
        let dir = TempDir::new().unwrap();
        let (app, db) = harness(dir.path()).await;
        insert_client(&db, OverlaySessionMode::Normal, NORMAL_TOKEN).await;
        let asset = insert_asset(&db, dir.path()).await;
        let item = insert_item(&db, &asset, "titre initial").await;

        // POST sans url : on atteint bien la validation legacy `400`, pas la garde.
        let (status, body) = send_json(
            &app,
            request(
                Method::POST,
                "/overlay/media-board/items",
                Some(NORMAL_TOKEN),
                Some(&json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_payload");

        // PATCH autorisé.
        let item_uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());
        let (status, body) = send_json(
            &app,
            request(
                Method::PATCH,
                &item_uri,
                Some(NORMAL_TOKEN),
                Some(&json!({"title": "titre modifié"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["item"]["title"], "titre modifié");

        // DELETE autorisé.
        let (status, body) = send_json(
            &app,
            request(Method::DELETE, &item_uri, Some(NORMAL_TOKEN), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["deleted"], true);
    }

    /// Le repli `?token=` du board est soumis à la même garde que le Bearer.
    #[tokio::test]
    async fn query_token_cannot_bypass_read_only_gate() {
        let dir = TempDir::new().unwrap();
        let (app, db) = harness(dir.path()).await;
        insert_client(&db, OverlaySessionMode::InviteReadOnly, READ_ONLY_TOKEN).await;
        insert_client(&db, OverlaySessionMode::Normal, NORMAL_TOKEN).await;
        let asset = insert_asset(&db, dir.path()).await;
        let item = insert_item(&db, &asset, "titre initial").await;
        let item_uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());

        // PATCH via ?token= invitée (sans Bearer) : 403.
        let uri = format!("{item_uri}?token={READ_ONLY_TOKEN}");
        let (status, body) = send_json(
            &app,
            request(Method::PATCH, &uri, None, Some(&json!({"title": "hack"}))),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // DELETE via ?token= invitée : 403.
        let (status, body) = send_json(&app, request(Method::DELETE, &uri, None, None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // POST via ?token= invitée : 403 (avant validation `url`).
        let uri = format!("/overlay/media-board/items?token={READ_ONLY_TOKEN}");
        let (status, body) =
            send_json(&app, request(Method::POST, &uri, None, Some(&json!({})))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");

        // PATCH via ?token= normale : autorisé (le mode suit le jeton, pas la voie).
        let uri = format!("{item_uri}?token={NORMAL_TOKEN}");
        let (status, body) = send_json(
            &app,
            request(
                Method::PATCH,
                &uri,
                None,
                Some(&json!({"message": "via query"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["updated"], true);
    }

    /// Priorité du Bearer : le mode retenu est toujours celui du jeton serveur.
    #[tokio::test]
    async fn bearer_token_takes_precedence_over_query_token() {
        let dir = TempDir::new().unwrap();
        let (app, db) = harness(dir.path()).await;
        insert_client(&db, OverlaySessionMode::InviteReadOnly, READ_ONLY_TOKEN).await;
        insert_client(&db, OverlaySessionMode::Normal, NORMAL_TOKEN).await;
        let asset = insert_asset(&db, dir.path()).await;
        let item = insert_item(&db, &asset, "titre initial").await;
        let item_uri = format!("/overlay/media-board/items/{}", item.id.to_canonical());

        // Bearer normal + ?token= invité → le Bearer prime, écriture autorisée.
        let uri = format!("{item_uri}?token={READ_ONLY_TOKEN}");
        let (status, body) = send_json(
            &app,
            request(
                Method::PATCH,
                &uri,
                Some(NORMAL_TOKEN),
                Some(&json!({"title": "via bearer"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["updated"], true);

        // Bearer invité + ?token= normal → le Bearer prime, écriture refusée.
        let uri = format!("{item_uri}?token={NORMAL_TOKEN}");
        let (status, body) = send_json(
            &app,
            request(
                Method::PATCH,
                &uri,
                Some(READ_ONLY_TOKEN),
                Some(&json!({"title": "hack"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "read_only_session");
    }
}
