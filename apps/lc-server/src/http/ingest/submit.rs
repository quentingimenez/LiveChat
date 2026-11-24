//! `POST /ingest` : soumission d'une URL, d'un média ou d'un texte.
//!
//! Reproduit l'ordre exact du legacy (`ingestRoutes.ts`) pour que les statuts
//! et corps JSON restent identiques côté extension navigateur :
//!
//! 1. authentification (client d'ingestion uniquement) : 401 si l'ingestion est
//!    configurée mais le token absent/invalide, 503 si aucun client actif ;
//! 2. normalisation du corps ;
//! 3. `guildId` demandé différent de celui du client → 403 `guild_mismatch` ;
//! 4. `authorName`/`authorImage` → 400 (valeurs liées au token) ;
//! 5. corps vide / `saveToBoard` sans source → 400 `invalid_payload` ;
//! 6. ingestion média éventuelle (cache par `source_hash` sauf `forceRefresh`) ;
//! 7. `saveToBoard` → 201 avec l'élément de board média ;
//!    sinon création d'un `PlaybackJob` puis notification du hook temps réel
//!    (Phase 4b) → 201.
//!
//! La branche « rich tweet card » du legacy (résolution oEmbed + carte riche)
//! est hors périmètre de la Phase 3c : elle sera traitée avec Phase 6b et est
//! documentée dans `docs/rust/server-ingest.md`.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use time::UtcDateTime;

use lc_domain::entities::{GuildMember, IngestClient, MediaAsset};
use lc_domain::enums::JobSource;
use lc_domain::ids::GuildMemberId;
use lc_domain::rights::{Action, Surface};
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};

use crate::app::AppState;
use crate::media::board;
use crate::media::errors::MediaServiceError;
use crate::media::hook;
use crate::media::jobs::{self, JobSpec};
use crate::media::MediaService;

use super::auth::{self, IngestAuth};
use super::body::NormalizedIngest;
use super::{
    expired_token_error, invalid_payload, media_service_error, simple_error,
    DEFAULT_INGEST_AUTHOR_NAME,
};

/// Author prefix of an ingest client paired from the overlay (`member:<id>`).
///
/// Mirrors `overlay::ingest_code::INGEST_CODE_CREATOR_PREFIX`: the pairing code
/// that created the client records the member id instead of a Discord
/// snowflake.
const MEMBER_AUTHOR_PREFIX: &str = "member:";

/// Member that owns an ingest client.
///
/// An ingest client has no foreign key to `guild_members`: the author recorded
/// at pairing time is the link. Overlay-issued codes store `member:<id>`; codes
/// issued from Discord store the Discord snowflake, resolved against
/// `guild_members.discord_user_id` (the same rule as `admin/ingest_authors.rs`
/// uses to list known authors). A removed member resolves to `None`.
pub(super) async fn owning_member(
    db: &lc_db::Db,
    client: &IngestClient,
) -> Result<Option<GuildMember>, lc_db::DbError> {
    let author = client.created_by_discord_user_id.trim();
    if let Some(raw_member_id) = author.strip_prefix(MEMBER_AUTHOR_PREFIX) {
        let Ok(member_id) = GuildMemberId::parse(raw_member_id) else {
            return Ok(None);
        };
        return Ok(db
            .guild_members()
            .get(member_id)
            .await?
            .filter(|member| member.is_active() && member.guild_id == client.guild_id));
    }
    if author.is_empty() {
        return Ok(None);
    }
    db.guild_members()
        .find_active_by_discord_user(&client.guild_id, author)
        .await
}

/// Handler de `POST /ingest`.
pub async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    // 1. Authentification — toujours avant la validation du corps.
    let client = match auth::resolve(&state, &headers).await {
        Ok(IngestAuth::Client(client)) => client,
        Ok(IngestAuth::Expired) => return expired_token_error(),
        Ok(IngestAuth::Missing { enabled: true }) => {
            return simple_error(StatusCode::UNAUTHORIZED, "unauthorized");
        }
        Ok(IngestAuth::Missing { enabled: false }) => {
            return simple_error(StatusCode::SERVICE_UNAVAILABLE, "ingest_api_disabled");
        }
        Err(error) => return media_service_error(error),
    };

    // 1b. The extension surface re-reads the member owning the ingest client at
    //     every submission: a removed member or a denied `extension.send` is
    //     `403 forbidden_role`.
    let Some(db) = state.db.as_ref() else {
        tracing::error!("client d'ingestion authentifié sans base de données");
        return super::internal_error();
    };
    let role = match owning_member(db, &client).await {
        Ok(Some(member)) => member.role,
        Ok(None) => return simple_error(StatusCode::FORBIDDEN, "forbidden_role"),
        Err(error) => {
            tracing::error!(error = %error, "reading the member of an ingest client failed");
            return super::internal_error();
        }
    };
    if !state
        .rights
        .allowed(Surface::Extension, Action::Send, role.into())
    {
        return simple_error(StatusCode::FORBIDDEN, "forbidden_role");
    }

    // 2. Corps JSON : un JSON malformé est une erreur de payload dédiée, pas
    // un `request_error` générique (AUD-A2-009).
    let value = match body {
        Ok(Json(value)) => value,
        Err(rejection) => {
            tracing::debug!(error = ?rejection, "corps JSON invalide (ingest submit)");
            return invalid_payload(None);
        }
    };

    // 3. Normalisation du corps.
    let request = NormalizedIngest::from_value(&value);
    let now = state.clock.now();

    // 3. Cohérence de guilde.
    if let Some(requested) = request.requested_guild_id.as_deref() {
        if requested != client.guild_id.as_str() {
            return simple_error(StatusCode::FORBIDDEN, "guild_mismatch");
        }
    }

    // 4. Auteur lié au token : le client ne peut pas le surcharger.
    if request.author_name.is_some() || request.author_image.is_some() {
        return invalid_payload(Some(
            "authorName/authorImage are token-bound for ingest clients",
        ));
    }

    // 5. Contenu minimal.
    let has_source = request.url.is_some() || request.media.is_some();
    if !has_source && request.text.is_none() {
        return invalid_payload(None);
    }
    if request.save_to_board && !has_source {
        return invalid_payload(Some("saveToBoard requires a resolvable media source"));
    }
    if request.duration_sec_unusable
        || request
            .duration_sec
            .is_some_and(|seconds| !lc_domain::entities::is_valid_display_duration(seconds))
    {
        return invalid_payload(Some("durationSec must be between 1 and 600"));
    }

    // 5b. Politique de source (garde SSRF/LFI) : une source refusée est
    //     rejetée AVANT toute création d'asset, avec le même format d'erreur
    //     que le pipeline (`media_ingestion_failed`).
    if has_source {
        if let Some(raw_source) = request.media.as_deref().or(request.url.as_deref()) {
            if let Err(error) = lc_media::source::validate_ingest_source(raw_source).await {
                return media_service_error(MediaServiceError::Media(error));
            }
        }
    }

    // 6. Texte enrichi (offset YouTube) et métadonnées du job.
    let offset = lc_discord::text::youtube_start_offset_sec(
        request.url.as_deref(),
        request.media.as_deref(),
    );
    let job_text =
        lc_discord::rich_payload::build_media_text_payload(request.text.as_deref(), offset);
    let job_show_text = request.show_text.unwrap_or(request.text.is_some());
    let job_author_name = client.default_author_name.trim().to_owned();
    let job_author_name = if job_author_name.is_empty() {
        DEFAULT_INGEST_AUTHOR_NAME.to_owned()
    } else {
        job_author_name
    };
    let job_author_image = client
        .default_author_image
        .clone()
        .filter(|value| !value.trim().is_empty());

    // 7. Ingestion média (si une source est fournie).
    let mut service: Option<MediaService> = None;
    let mut asset: Option<MediaAsset> = None;
    if has_source {
        let media_service = match MediaService::for_state(&state) {
            Ok(service) => service,
            Err(error) => return media_service_error(error),
        };
        match media_service
            .ingest(
                request.url.as_deref(),
                request.media.as_deref(),
                request.force_refresh,
            )
            .await
        {
            Ok(ingested) => asset = Some(ingested),
            Err(error) => return media_service_error(error),
        }
        service = Some(media_service);
    }

    if asset.is_none() && job_text.is_none() {
        return invalid_payload(None);
    }

    let outcome = IngestOutcome {
        asset: asset.as_ref(),
        service: service.as_ref(),
        job_text: job_text.as_deref(),
        title: None,
        show_text: job_show_text,
        author_name: job_author_name.as_str(),
        author_image: job_author_image.as_deref(),
        duration_sec: request.duration_sec,
        save_to_board: request.save_to_board,
    };
    finish_ingest(&state, db, &client, outcome, now).await
}

/// Validated ingest result, shared by the JSON and raw-file routes.
pub(super) struct IngestOutcome<'a> {
    /// Ingested media, if the request carried one.
    pub(super) asset: Option<&'a MediaAsset>,
    /// Media service that produced `asset` (required by the board branch).
    pub(super) service: Option<&'a MediaService>,
    /// Text already built for playback (rich payload included).
    pub(super) job_text: Option<&'a str>,
    /// Board title override (raw-file uploads only).
    pub(super) title: Option<&'a str>,
    /// Whether the overlay displays `job_text`.
    pub(super) show_text: bool,
    /// Token-bound author display name.
    pub(super) author_name: &'a str,
    /// Token-bound author avatar.
    pub(super) author_image: Option<&'a str>,
    /// Requested display duration in seconds.
    pub(super) duration_sec: Option<i64>,
    /// Store the media on the board instead of creating a playback job.
    pub(super) save_to_board: bool,
}

/// Persists the board item or the playback job and builds the 201 response.
pub(super) async fn finish_ingest(
    state: &AppState,
    db: &lc_db::Db,
    client: &IngestClient,
    outcome: IngestOutcome<'_>,
    now: UtcDateTime,
) -> Response {
    let guild_id = client.guild_id.clone();

    // 8. Enregistrement sur le board média (pas de job, parité legacy).
    if outcome.save_to_board {
        let Some(asset) = outcome.asset else {
            return invalid_payload(Some("saveToBoard requires media"));
        };
        let Some(media_service) = outcome.service else {
            return media_service_error(MediaServiceError::unavailable(
                "service média indisponible pour saveToBoard",
            ));
        };

        // The item keeps the media's own title (falling back to the asset's
        // source title): the text sent with the media belongs to that play
        // only and is never stored on the board.
        let added = board::add_to_board(
            db,
            media_service.config(),
            &guild_id,
            asset,
            outcome.title,
            Some(client.created_by_discord_user_id.as_str()),
            Some(outcome.author_name),
            outcome.duration_sec,
            now,
        )
        .await;

        return match added {
            Ok(result) => {
                if result.created {
                    state.admin_hub.broadcast_to_guild(
                        &guild_id,
                        ServerMessage::BoardChanged(BoardChangedPayload {
                            change: BoardChange::Added,
                            item_id: result.item.id.to_canonical(),
                        }),
                    );
                }
                let payload = json!({
                    "accepted": true,
                    "savedToBoard": true,
                    "created": result.created,
                    "guildId": guild_id.as_str(),
                    "mediaBoardItemId": result.item.id.to_canonical(),
                    "mediaAssetId": asset.id.to_canonical(),
                });
                (StatusCode::CREATED, Json(payload)).into_response()
            }
            Err(error) => media_service_error(error),
        };
    }

    // 9. Création du job de lecture puis notification du hook temps réel.
    let spec = JobSpec {
        guild_id: guild_id.clone(),
        media: outcome.asset,
        text: outcome.job_text,
        show_text: outcome.show_text,
        author_name: Some(outcome.author_name),
        author_image: outcome.author_image,
        duration_sec: outcome.duration_sec,
        source: JobSource::Ingest,
        source_label: Some(client.label.as_str()),
    };

    match jobs::create_for_ingest(db, state.runtime().default_duration(), spec, now).await {
        Ok(job) => {
            hook::notify_ingested(&job).await;
            let payload = json!({
                "accepted": true,
                "jobId": job.id.to_canonical(),
                "guildId": guild_id.as_str(),
                "hasMedia": outcome.asset.is_some(),
                "hasText": outcome.job_text.is_some(),
            });
            (StatusCode::CREATED, Json(payload)).into_response()
        }
        Err(error) => media_service_error(error),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use axum::http::StatusCode;
    use axum::Router;
    use lc_db::{Db, DbConfig};
    use lc_domain::entities::{GuildMember, IngestClient};
    use lc_domain::enums::JobSource;
    use lc_domain::ids::{GuildId, GuildMemberId, IngestClientId, PlaybackJobId};
    use lc_domain::GuildRole;
    use serde_json::{json, Value};
    use time::{Duration, UtcDateTime};
    use tower::ServiceExt;

    use crate::app::AppState;
    use crate::config::Config;
    use crate::media::tokens::hash_ingest_token;
    use crate::shutdown::Shutdown;

    use super::super::router;

    const GUILD: &str = "123456789012345678";
    const TOKEN: &str = "token-de-test-ingestion";

    struct Fixture {
        _dir: tempfile::TempDir,
        app: Router,
        db: Db,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        let config =
            Config::from_pairs([("API_URL".to_owned(), "http://localhost:3000".to_owned())])
                .expect("configuration de test valide");
        let state = AppState::new(Arc::new(config), Some(db.clone()), Shutdown::detached());
        let app = router().with_state(state);
        Fixture { _dir: dir, app, db }
    }

    async fn insert_client(db: &Db) {
        let now = UtcDateTime::now();
        let guild_id = GuildId::parse(GUILD).expect("guilde");
        // The ingest client is linked to its author, whose guild member owns
        // the `extension.send` right (the client has no member foreign key).
        let member = GuildMember {
            id: GuildMemberId::new_v4(),
            guild_id: guild_id.clone(),
            display_name: "Alice".into(),
            avatar: None,
            role: GuildRole::Member,
            created_at: now,
            removed_at: None,
            discord_user_id: Some("42".to_owned()),
            profile_custom: false,
            discord_synced_at: None,
        };
        db.guild_members().create(&member).await.expect("membre");
        let client = IngestClient {
            id: IngestClientId::new_v4(),
            guild_id,
            label: "Extension-Test".into(),
            token_hash: hash_ingest_token(TOKEN),
            default_author_name: "LiveChat Extension".into(),
            default_author_image: None,
            created_by_discord_user_id: "42".into(),
            last_seen_at: None,
            created_at: now,
            revoked_at: None,
        };
        db.ingest_clients().create(&client).await.expect("client");
    }

    async fn post(app: Router, token: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut builder = HttpRequest::builder()
            .method("POST")
            .uri("/ingest")
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let request = builder.body(Body::from(body.to_string())).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn ingestion_disabled_returns_503_without_active_client() {
        let fixture = fixture().await;
        let (status, body) = post(fixture.app.clone(), None, json!({ "text": "bonjour" })).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(body["error"], "ingest_api_disabled");
    }

    #[tokio::test]
    async fn invalid_token_returns_401_when_ingestion_is_enabled() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some("mauvais-token"),
            json!({ "text": "bonjour" }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "unauthorized");
    }

    #[tokio::test]
    async fn text_only_submission_creates_a_pending_job() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;

        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "guildId": GUILD, "text": "bonjour" }),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["accepted"], true);
        assert_eq!(body["hasMedia"], false);
        assert_eq!(body["hasText"], true);
        assert_eq!(body["guildId"], GUILD);

        let job_id = body["jobId"].as_str().expect("jobId");
        let job = fixture
            .db
            .playback_jobs()
            .get(PlaybackJobId::parse(job_id).expect("uuid"))
            .await
            .expect("lecture")
            .expect("job persisté");
        assert_eq!(job.guild_id.as_str(), GUILD);
        assert_eq!(job.source, JobSource::Ingest);
    }

    #[tokio::test]
    async fn mismatched_guild_is_rejected() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "guildId": "999999999999999999", "text": "bonjour" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "guild_mismatch");
    }

    #[tokio::test]
    async fn token_bound_author_fields_are_rejected() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "text": "bonjour", "authorName": "Jack" }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload");
        assert!(body["message"]
            .as_str()
            .expect("message")
            .contains("token-bound"));
    }

    #[tokio::test]
    async fn empty_body_is_rejected() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(fixture.app.clone(), Some(TOKEN), json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload");
    }

    #[tokio::test]
    async fn ingest_rejects_duration_above_max() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "text": "bonjour", "durationSec": 601 }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload");
        assert!(body["message"]
            .as_str()
            .expect("message")
            .contains("durationSec"));
    }

    #[tokio::test]
    async fn ingest_rejects_unusable_duration_values() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        for duration in [json!(0), json!(-3), json!("8"), json!(true)] {
            let (status, body) = post(
                fixture.app.clone(),
                Some(TOKEN),
                json!({ "text": "bonjour", "durationSec": duration }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{duration}: {body}");
            assert_eq!(body["error"], "invalid_payload", "{duration}");
        }
    }

    #[tokio::test]
    async fn ingest_accepts_null_duration() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "text": "bonjour", "durationSec": null }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    #[tokio::test]
    async fn ingest_accepts_duration_at_max() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "text": "bonjour", "durationSec": 600 }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    /// Puts a ready image in the media cache for `url`, so `/ingest` reuses it
    /// without any download (an IP literal also skips the DNS lookup).
    async fn insert_cached_image(fixture: &Fixture, url: &str) {
        use lc_domain::{MediaAsset, MediaKind, MediaStatus};
        let path = fixture._dir.path().join("cached.png");
        tokio::fs::write(&path, b"png-body").await.unwrap();
        let now = UtcDateTime::now();
        let canonical = lc_media::source::canonicalize_source_url(url);
        let asset = MediaAsset {
            id: lc_domain::ids::MediaAssetId::new_v4(),
            source_hash: lc_media::source::source_hash(&canonical),
            source_url: url.to_owned(),
            source_title: None,
            kind: MediaKind::Image,
            mime: "image/png".to_owned(),
            duration_sec: None,
            width: Some(10),
            height: Some(10),
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: 8,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + Duration::days(1),
            created_at: now,
        };
        fixture.db.media_assets().insert(&asset).await.unwrap();
    }

    #[tokio::test]
    async fn save_to_board_keeps_the_requested_image_duration() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let url = "https://93.184.215.14/cached.png";
        insert_cached_image(&fixture, url).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "url": url, "saveToBoard": true, "durationSec": 12 }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let guild = GuildId::parse(GUILD).unwrap();
        let items = fixture
            .db
            .media_board()
            .list_by_guild(&guild)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "{body}");
        assert_eq!(items[0].display_duration_sec, Some(12));
    }

    #[tokio::test]
    async fn save_to_board_with_a_text_stores_no_message() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let url = "https://93.184.215.14/cached.png";
        insert_cached_image(&fixture, url).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "url": url, "text": "bonjour", "saveToBoard": true }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["savedToBoard"], true);

        let guild = GuildId::parse(GUILD).unwrap();
        let items = fixture
            .db
            .media_board()
            .list_by_guild(&guild)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "{body}");
        // The text sent with a media belongs to that play only: it is neither
        // the item's message nor its title.
        assert_eq!(items[0].message, None);
        assert_eq!(items[0].title, None);
    }

    #[tokio::test]
    async fn sending_a_media_with_a_text_keeps_the_text_on_the_job() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let url = "https://93.184.215.14/cached.png";
        insert_cached_image(&fixture, url).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "url": url, "text": "bonjour", "showText": true }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["hasMedia"], true);
        assert_eq!(body["hasText"], true);

        let job_id = body["jobId"].as_str().expect("jobId");
        let job = fixture
            .db
            .playback_jobs()
            .get(PlaybackJobId::parse(job_id).expect("uuid"))
            .await
            .expect("lecture")
            .expect("job persisté");
        assert_eq!(job.text.as_deref(), Some("bonjour"));
        assert!(job.show_text);

        // A play never writes the text on the board.
        let guild = GuildId::parse(GUILD).unwrap();
        assert!(fixture
            .db
            .media_board()
            .list_by_guild(&guild)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn save_to_board_requires_a_media_source() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "text": "bonjour", "saveToBoard": true }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["message"]
            .as_str()
            .expect("message")
            .contains("resolvable media source"));
    }

    async fn post_raw(app: Router, token: Option<&str>, raw: &str) -> (StatusCode, Value) {
        let mut builder = HttpRequest::builder()
            .method("POST")
            .uri("/ingest")
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let request = builder.body(Body::from(raw.to_owned())).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn malformed_json_is_a_payload_error() {
        // AUD-A2-009 : l'authentification passe d'abord, puis le corps JSON
        // illisible produit `invalid_payload` (pas `request_error`).
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let (status, body) = post_raw(fixture.app.clone(), Some(TOKEN), "{\"text\": ").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload", "{body}");
    }

    #[tokio::test]
    async fn idle_expired_client_returns_401_ingest_token_expired() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let client = fixture
            .db
            .ingest_clients()
            .list()
            .await
            .expect("clients")
            .into_iter()
            .next()
            .expect("client");
        // Force l'inactivité : last_seen_at et created_at trop anciens.
        let stale = UtcDateTime::now() - Duration::days(181);
        sqlx::query("UPDATE ingest_clients SET created_at = ?, last_seen_at = ? WHERE id = ?")
            .bind(lc_domain::datetime_to_ms(stale))
            .bind(lc_domain::datetime_to_ms(stale))
            .bind(client.id.to_canonical())
            .execute(fixture.db.pool())
            .await
            .expect("inactivité forcée");

        let (status, body) = post(
            fixture.app.clone(),
            Some(TOKEN),
            json!({ "text": "bonjour" }),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "ingest_token_expired");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("board"),
            "un message de ré-appairage est attendu"
        );

        // Aucune écriture de heartbeat sur un client expiré.
        let stored = fixture
            .db
            .ingest_clients()
            .get(client.id)
            .await
            .expect("lecture")
            .expect("client");
        // La precision SQLite est la milliseconde : comparer en ms tronqués.
        assert_eq!(
            stored.last_seen_at.map(lc_domain::datetime_to_ms),
            Some(lc_domain::datetime_to_ms(stale))
        );
    }

    #[tokio::test]
    async fn revoked_token_leaves_ingestion_disabled() {
        let fixture = fixture().await;
        insert_client(&fixture.db).await;
        let client = fixture
            .db
            .ingest_clients()
            .list()
            .await
            .expect("clients")
            .into_iter()
            .next()
            .expect("client");
        fixture
            .db
            .ingest_clients()
            .revoke(client.id, UtcDateTime::now() + Duration::seconds(1))
            .await
            .expect("révocation");

        // Plus aucun client actif : l'ingestion est désactivée (503) et non
        // simplement non autorisée (401), comme `isIngestApiEnabled` legacy.
        let (status, body) = post(fixture.app.clone(), Some(TOKEN), json!({ "text": "x" })).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(body["error"], "ingest_api_disabled");
    }
}
