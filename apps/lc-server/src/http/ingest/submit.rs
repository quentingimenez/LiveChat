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
//! 7. `saveToBoard` → 201 avec l'élément de meme board ;
//!    sinon création d'un `PlaybackJob` puis notification du hook temps réel
//!    (Phase 4b) → 201.
//!
//! La branche « rich tweet card » du legacy (résolution oEmbed + carte riche)
//! est hors périmètre de la Phase 3c : elle sera traitée avec Phase 6b et est
//! documentée dans `docs/rust/server-ingest.md`.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use lc_domain::entities::MediaAsset;

use crate::app::AppState;
use crate::media::board;
use crate::media::errors::MediaServiceError;
use crate::media::hook;
use crate::media::jobs::{self, JobSpec};
use crate::media::MediaService;

use super::auth::{self, IngestAuth};
use super::body::NormalizedIngest;
use super::{invalid_payload, media_service_error, simple_error, DEFAULT_INGEST_AUTHOR_NAME};

/// Handler de `POST /ingest`.
pub async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let value = body.map(|Json(value)| value).unwrap_or(Value::Null);

    // 1. Authentification — toujours avant la validation du corps.
    let client = match auth::resolve(&state, &headers).await {
        Ok(IngestAuth::Client(client)) => client,
        Ok(IngestAuth::Missing { enabled: true }) => {
            return simple_error(StatusCode::UNAUTHORIZED, "unauthorized");
        }
        Ok(IngestAuth::Missing { enabled: false }) => {
            return simple_error(StatusCode::SERVICE_UNAVAILABLE, "ingest_api_disabled");
        }
        Err(error) => return media_service_error(error),
    };

    // 2. Normalisation du corps.
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

    let Some(db) = state.db.as_ref() else {
        // Un client authentifié implique une base ; garde-fou défensif.
        tracing::error!("client d'ingestion authentifié sans base de données");
        return super::internal_error();
    };

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

    let guild_id = client.guild_id.clone();

    // 8. Enregistrement sur le meme board (pas de job, parité legacy).
    if request.save_to_board {
        let Some(asset) = asset.as_ref() else {
            return invalid_payload(Some("saveToBoard requires media"));
        };
        let Some(media_service) = service.as_ref() else {
            return media_service_error(MediaServiceError::unavailable(
                "service média indisponible pour saveToBoard",
            ));
        };

        let added = board::add_to_board(
            db,
            media_service.config(),
            &guild_id,
            asset,
            job_text.as_deref(),
            None,
            Some(client.created_by_discord_user_id.as_str()),
            Some(job_author_name.as_str()),
            now,
        )
        .await;

        return match added {
            Ok(result) => {
                let payload = json!({
                    "accepted": true,
                    "savedToBoard": true,
                    "created": result.created,
                    "guildId": guild_id.as_str(),
                    "memeBoardItemId": result.item.id.to_canonical(),
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
        media: asset.as_ref(),
        text: job_text.as_deref(),
        show_text: job_show_text,
        author_name: Some(job_author_name.as_str()),
        author_image: job_author_image.as_deref(),
        duration_sec: request.duration_sec,
        priority: 0,
    };

    match jobs::create_for_ingest(db, state.config.default_duration, spec, now).await {
        Ok(job) => {
            hook::notify_ingested(&job).await;
            let payload = json!({
                "accepted": true,
                "jobId": job.id.to_canonical(),
                "guildId": guild_id.as_str(),
                "hasMedia": asset.is_some(),
                "hasText": job_text.is_some(),
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
    use chrono::{Duration, Utc};
    use lc_db::{Db, DbConfig};
    use lc_domain::entities::IngestClient;
    use lc_domain::ids::{GuildId, IngestClientId, PlaybackJobId};
    use serde_json::{json, Value};
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
        let now = Utc::now();
        let client = IngestClient {
            id: IngestClientId::new_v4(),
            guild_id: GuildId::parse(GUILD).expect("guilde"),
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
            .revoke(client.id, Utc::now() + Duration::seconds(1))
            .await
            .expect("révocation");

        // Plus aucun client actif : l'ingestion est désactivée (503) et non
        // simplement non autorisée (401), comme `isIngestApiEnabled` legacy.
        let (status, body) = post(fixture.app.clone(), Some(TOKEN), json!({ "text": "x" })).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(body["error"], "ingest_api_disabled");
    }
}
