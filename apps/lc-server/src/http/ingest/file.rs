//! `POST /ingest/file` : envoi direct d'un fichier par un client d'ingestion.
//!
//! Cette route accepte un corps binaire brut, sans multipart : les Raccourcis
//! iOS et les clients légers peuvent envoyer le fichier tel quel. La source est
//! le nom de fichier fourni en query (`filename`), avec repli sur
//! `Content-Disposition`.
//!
//! Contrat :
//!
//! - `Authorization: Bearer <token d'ingestion>` ;
//! - query `filename`, `text`, `title`, `showText`, `saveToBoard`,
//!   `durationSec` ;
//! - corps brut plafonné par `MEDIA_MAX_SIZE_MB` ;
//! - réponse `202 {accepted, ingestionId, guildId}` dès le fichier reçu, comme
//!   un média de [`super::submit`] ; le résultat se lit sur
//!   `GET /ingest/ingestions/{id}`.
//!
//! Le fichier est écrit dans un dossier temporaire, haché en SHA-256, puis
//! ingéré en tâche de fond ([`super::task`]) par [`MediaService::ingest_file`] :
//! le cache `upload:sha256:<hex>` déduplique les envois identiques. Le dossier
//! temporaire appartient à la tâche et disparaît avec elle.
//!
//! L'auteur du job est le profil **live** du membre propriétaire du client,
//! relu à chaque envoi (cf. [`super::submit::resolved_author`]) : le client ne
//! choisit ni son pseudo ni son avatar.

use std::io;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use lc_domain::entities::{GuildMember, IngestClient};
use lc_domain::rights::{Action, Surface};

use crate::app::AppState;
use crate::http::error::ApiError;
use crate::media::MediaService;

use super::auth;
use super::submit::{owning_member, resolved_author};
use super::task::{self, IngestSource, IngestTask};
use super::{internal_error, invalid_payload, media_service_error, simple_error};

/// Nom de repli quand ni la query ni `Content-Disposition` ne fournissent de nom.
const DEFAULT_UPLOAD_FILENAME: &str = "upload";

/// Préfixe des dossiers temporaires d'upload.
const TEMP_PREFIX: &str = "lc-ingest-file-";

/// Nombre d'essais de création d'un dossier temporaire unique.
const TEMP_CREATE_ATTEMPTS: usize = 8;

/// Query de `POST /ingest/file`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileQuery {
    /// Nom d'origine du fichier (sert de source au média et de titre au board).
    #[serde(default)]
    filename: Option<String>,
    /// Texte à afficher dans l'overlay.
    #[serde(default)]
    text: Option<String>,
    /// Titre de l'élément de board (sinon le nom du fichier).
    #[serde(default)]
    title: Option<String>,
    /// `showText` explicite ; défaut : vrai si `text` est fourni.
    #[serde(default)]
    show_text: Option<bool>,
    /// Enregistre sur le board au lieu de créer un job de lecture.
    #[serde(default)]
    save_to_board: Option<bool>,
    /// Durée d'affichage demandée, en secondes (1 à 600, arrondie au supérieur).
    #[serde(default)]
    duration_sec: Option<f64>,
}

/// Handler de `POST /ingest/file`.
pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    body: Body,
) -> Response {
    // 1. Authentification avant toute lecture du corps ou de la query.
    let Some(db) = state.db.as_ref() else {
        tracing::error!("client d'ingestion authentifié sans base de données");
        return internal_error();
    };
    let (client, member) = match authorize(&state, db, &headers).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };

    // 2. Query : une query malformée est un payload invalide, pas une erreur
    //    interne.
    let query = match Query::<FileQuery>::try_from_uri(&uri) {
        Ok(Query(query)) => query,
        Err(rejection) => {
            tracing::debug!(error = %rejection, "query invalide (ingest file)");
            return invalid_payload(Some("Les paramètres de la requête sont invalides."));
        }
    };

    let max_bytes = state
        .runtime()
        .media_max_size_mb()
        .saturating_mul(1024 * 1024);

    // 3. Refus précoce d'un corps annoncé trop gros, avant de lire le premier
    //    octet. Un corps sans `Content-Length` (chunked) reste borné pendant la
    //    lecture.
    if let Some(declared) = content_length(&headers) {
        if declared > max_bytes {
            return payload_too_large(max_bytes);
        }
    }

    let filename = match upload_filename(query.filename.as_deref(), &headers) {
        Ok(filename) => filename,
        Err(response) => return response,
    };
    let text = non_empty_trimmed(query.text.as_deref());
    let title = non_empty_trimmed(query.title.as_deref());
    let duration_sec = match display_duration(query.duration_sec) {
        Ok(duration) => duration,
        Err(response) => return response,
    };
    let show_text = query.show_text.unwrap_or(text.is_some());
    let save_to_board = query.save_to_board.unwrap_or(false);

    // 4. Réception du corps dans un dossier temporaire, avec hash SHA-256.
    let temp = match TempUploadDir::create().await {
        Ok(temp) => temp,
        Err(error) => {
            tracing::error!(error = %error, "création du dossier temporaire d'upload impossible");
            return internal_error();
        }
    };
    let path = temp.file_path();
    let content_hash = match receive_file(body, &path, max_bytes).await {
        Ok(hash) => hash,
        Err(response) => return response,
    };

    // 5. Le fichier est reçu : son ingestion (cache `upload:sha256:<hex>`
    //    inclus) se fait en tâche de fond, après la réponse `202`.
    let media_service = match MediaService::for_state(&state) {
        Ok(service) => service,
        Err(error) => return media_service_error(error),
    };
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    // 6. Auteur live du membre propriétaire, comme sur `/ingest` : le client
    //    ne choisit ni son pseudo ni son avatar.
    let (author_name, author_image) = resolved_author(&client, &member);

    let task = IngestTask {
        client,
        source: IngestSource::File {
            temp,
            filename,
            content_type,
            content_hash,
        },
        job_text: text,
        title,
        show_text,
        author_name,
        author_image,
        duration_sec,
        save_to_board,
    };
    task::accept(&state, media_service, task)
}

/// Resolves the ingest client, the member owning it and the extension send
/// right. The member is returned because it also carries the live author of
/// the job.
async fn authorize(
    state: &AppState,
    db: &lc_db::Db,
    headers: &HeaderMap,
) -> Result<(IngestClient, GuildMember), Response> {
    let client = auth::require_client(state, headers).await?;

    let member = match owning_member(db, &client).await {
        Ok(Some(member)) => member,
        Ok(None) => return Err(simple_error(StatusCode::FORBIDDEN, "forbidden_role")),
        Err(error) => {
            tracing::error!(error = %error, "reading the member of an ingest client failed");
            return Err(internal_error());
        }
    };
    if !state
        .rights
        .allowed(Surface::Extension, Action::Send, member.role.into())
    {
        return Err(simple_error(StatusCode::FORBIDDEN, "forbidden_role"));
    }
    Ok((client, member))
}

/// Reads the raw body into `path` and returns the lowercase SHA-256 hex digest.
async fn receive_file(body: Body, path: &PathBuf, max_bytes: u64) -> Result<String, Response> {
    let mut stream = body.into_data_stream();
    let mut file = match tokio::fs::File::create(path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::error!(error = %error, "création du fichier temporaire d'upload impossible");
            return Err(internal_error());
        }
    };
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                tracing::warn!(error = %error, "lecture du corps d'upload interrompue");
                return Err(invalid_payload(Some(
                    "Impossible de lire le corps de la requête.",
                )));
            }
        };
        total = total.saturating_add(chunk.len() as u64);
        if total > max_bytes {
            return Err(payload_too_large(max_bytes));
        }
        hasher.update(&chunk);
        if let Err(error) = file.write_all(&chunk).await {
            tracing::error!(error = %error, "écriture du fichier temporaire d'upload impossible");
            return Err(internal_error());
        }
    }

    if total == 0 {
        return Err(invalid_payload(Some("Le corps de la requête est vide.")));
    }
    if let Err(error) = file.flush().await {
        tracing::error!(error = %error, "flush du fichier temporaire d'upload impossible");
        return Err(internal_error());
    }
    drop(file);
    Ok(hex::encode(hasher.finalize()))
}

/// Filename from the query, else `Content-Disposition`, else the default.
fn upload_filename(query_filename: Option<&str>, headers: &HeaderMap) -> Result<String, Response> {
    if let Some(raw) = query_filename {
        return validate_filename(raw)
            .ok_or_else(|| invalid_payload(Some("Le nom du fichier est invalide.")));
    }
    if let Some(raw) = headers
        .get(CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .and_then(content_disposition_filename)
    {
        if let Some(name) = validate_filename(&raw) {
            return Ok(name);
        }
    }
    Ok(DEFAULT_UPLOAD_FILENAME.to_owned())
}

/// Keeps the final path component and rejects control characters and `..`.
fn validate_filename(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 255
        || trimmed.contains("..")
        || trimmed.chars().any(char::is_control)
        || trimmed
            .chars()
            .any(|character| matches!(character, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return None;
    }
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_owned())
}

/// Best-effort extraction of the `filename=` parameter.
fn content_disposition_filename(value: &str) -> Option<String> {
    value.split(';').find_map(|part| {
        let (name, raw) = part.trim().split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("filename") {
            return None;
        }
        let raw = raw.trim();
        let raw = raw
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(raw);
        (!raw.is_empty()).then(|| raw.to_owned())
    })
}

/// Parses `Content-Length`, ignoring an absent header.
fn content_length(headers: &HeaderMap) -> Option<u64> {
    let raw = headers.get(CONTENT_LENGTH)?.to_str().ok()?;
    raw.trim().parse::<u64>().ok()
}

/// Trims and drops an empty query string.
fn non_empty_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Validates and ceil-rounds `durationSec` (1 to 600 seconds).
fn display_duration(raw: Option<f64>) -> Result<Option<i64>, Response> {
    let Some(number) = raw else {
        return Ok(None);
    };
    if !number.is_finite() || number <= 0.0 {
        return Err(invalid_payload(Some(
            "La durée doit être comprise entre 1 et 600 secondes.",
        )));
    }
    let ceil = number.ceil();
    if ceil > i64::MAX as f64 || !lc_domain::entities::is_valid_display_duration(ceil as i64) {
        return Err(invalid_payload(Some(
            "La durée doit être comprise entre 1 et 600 secondes.",
        )));
    }
    Ok(Some(ceil as i64))
}

/// `413` with the same `maxBytes` shape as the board upload routes.
fn payload_too_large(max_bytes: u64) -> Response {
    ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large")
        .detail("maxBytes", max_bytes)
        .into_response()
}

/// Temporary directory holding one upload, removed on drop.
pub(super) struct TempUploadDir {
    path: PathBuf,
}

impl TempUploadDir {
    async fn create() -> io::Result<Self> {
        let base = std::env::temp_dir();
        for _ in 0..TEMP_CREATE_ATTEMPTS {
            let path = base.join(format!("{TEMP_PREFIX}{}", Uuid::new_v4()));
            match tokio::fs::create_dir(&path).await {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "temporary upload directory collision",
        ))
    }

    pub(super) fn file_path(&self) -> PathBuf {
        self.path.join("upload.bin")
    }
}

impl Drop for TempUploadDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    error = %error,
                    path = %self.path.display(),
                    "nettoyage du dossier temporaire d'upload impossible"
                );
            }
        }
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
    use lc_domain::entities::{GuildMember, IngestClient, PlaybackJob};
    use lc_domain::ids::{GuildId, GuildMemberId, IngestClientId, PlaybackJobId};
    use lc_domain::GuildRole;
    use serde_json::Value;
    use sha2::{Digest, Sha256};
    use time::{Duration, UtcDateTime};
    use tower::ServiceExt;

    use crate::app::AppState;
    use crate::config::Config;
    use crate::http::ingest::router;
    use crate::http::ingest::test_support::{get_ingestion, wait_ingestion};
    use crate::media::tokens::hash_ingest_token;
    use crate::shutdown::Shutdown;

    const GUILD: &str = "123456789012345678";
    const TOKEN: &str = "token-de-test-ingestion";

    struct Fixture {
        _dir: tempfile::TempDir,
        app: Router,
        db: Db,
    }

    async fn fixture(max_size_mb: u64) -> Fixture {
        fixture_with(max_size_mb, &[]).await
    }

    async fn fixture_with(max_size_mb: u64, extra: &[(&str, &str)]) -> Fixture {
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let db = Db::open(dir.path().join("test.db"), &DbConfig::default())
            .await
            .expect("base de test");
        let mut pairs = vec![
            ("API_URL".to_owned(), "http://localhost:3000".to_owned()),
            ("MEDIA_MAX_SIZE_MB".to_owned(), max_size_mb.to_string()),
            (
                "MEDIA_STORAGE_DIR".to_owned(),
                dir.path().join("media").to_string_lossy().into_owned(),
            ),
        ];
        pairs.extend(
            extra
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
        );
        let config = Config::from_pairs(pairs).expect("configuration de test valide");
        let state = AppState::new(Arc::new(config), Some(db.clone()), Shutdown::detached());
        let app = router().with_state(state);
        Fixture { _dir: dir, app, db }
    }

    async fn insert_client(db: &Db) {
        let now = UtcDateTime::now();
        let guild_id = GuildId::parse(GUILD).expect("guilde");
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

    /// Puts the content in the upload cache so `ingest_file` reuses it without
    /// running the media pipeline (no ffmpeg needed).
    async fn insert_cached_upload(fixture: &Fixture, filename: &str, body: &[u8]) {
        use lc_domain::{MediaAsset, MediaKind, MediaStatus};

        let path = fixture._dir.path().join("cached-upload.png");
        tokio::fs::write(&path, body).await.unwrap();
        let hash = hex::encode(Sha256::digest(body));
        let source = lc_media::source::MediaSource::from_upload(filename, &hash);
        let now = UtcDateTime::now();
        let asset = MediaAsset {
            id: lc_domain::ids::MediaAssetId::new_v4(),
            source_hash: source.source_hash,
            source_url: source.canonical_url,
            source_title: None,
            kind: MediaKind::Image,
            mime: "image/png".to_owned(),
            duration_sec: None,
            width: Some(10),
            height: Some(10),
            is_vertical: false,
            storage_path: path.to_string_lossy().into_owned(),
            size_bytes: body.len() as i64,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + Duration::days(1),
            created_at: now,
        };
        fixture.db.media_assets().insert(&asset).await.unwrap();
    }

    async fn post_file(
        app: Router,
        token: Option<&str>,
        uri: &str,
        body: Vec<u8>,
        extra_headers: &[(&str, &str)],
    ) -> (StatusCode, Value) {
        let mut builder = HttpRequest::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "image/png");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(Body::from(body)).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Reads back the playback job created by a 201 response.
    async fn read_job(fixture: &Fixture, body: &Value) -> PlaybackJob {
        let job_id = body["jobId"].as_str().expect("jobId");
        fixture
            .db
            .playback_jobs()
            .get(PlaybackJobId::parse(job_id).expect("uuid"))
            .await
            .expect("lecture")
            .expect("job persisté")
    }

    #[tokio::test]
    async fn ingestion_disabled_returns_503() {
        let fixture = fixture(2048).await;
        let (status, body) = post_file(
            fixture.app.clone(),
            None,
            "/ingest/file?filename=photo.png",
            b"x".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(body["error"], "ingest_api_disabled");
    }

    #[tokio::test]
    async fn invalid_token_returns_401() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        let (status, body) = post_file(
            fixture.app.clone(),
            Some("mauvais-token"),
            "/ingest/file?filename=photo.png",
            b"x".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "unauthorized");
    }

    #[tokio::test]
    async fn empty_body_returns_400() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png",
            Vec::new(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload");
    }

    #[tokio::test]
    async fn idle_expired_client_returns_401_on_file_upload() {
        let fixture = fixture(2048).await;
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
        let stale = UtcDateTime::now() - Duration::days(181);
        sqlx::query("UPDATE ingest_clients SET created_at = ?, last_seen_at = ? WHERE id = ?")
            .bind(lc_domain::datetime_to_ms(stale))
            .bind(lc_domain::datetime_to_ms(stale))
            .bind(client.id.to_canonical())
            .execute(fixture.db.pool())
            .await
            .expect("inactivité forcée");

        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png",
            b"png-body".to_vec(),
            &[],
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
    }

    #[tokio::test]
    async fn invalid_filename_returns_400() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=..%2Fsecret",
            b"png-body".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload");
    }

    #[tokio::test]
    async fn duration_above_600_returns_400() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png&durationSec=601",
            b"png-body".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload");
        assert!(body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("durée doit être comprise entre 1 et 600 secondes"));
    }

    #[tokio::test]
    async fn oversized_body_returns_413_with_the_limit() {
        let fixture = fixture(1).await;
        insert_client(&fixture.db).await;
        let body = vec![0_u8; 1024 * 1024 + 1];
        let (status, response) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png",
            body,
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{response}");
        assert_eq!(response["error"], "payload_too_large");
        assert_eq!(response["maxBytes"], 1024 * 1024);
    }

    #[tokio::test]
    async fn declared_oversized_body_returns_413_before_reading() {
        let fixture = fixture(1).await;
        insert_client(&fixture.db).await;
        let (status, response) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png",
            b"png-body".to_vec(),
            &[("content-length", "2097152")],
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{response}");
        assert_eq!(response["error"], "payload_too_large");
    }

    #[tokio::test]
    async fn upload_creates_a_job_with_media_and_text() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        insert_cached_upload(&fixture, "photo.png", b"png-body").await;

        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png&text=bonjour&showText=true",
            b"png-body".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(body["accepted"], true);
        let done = wait_ingestion(&fixture.app, TOKEN, &body).await;
        assert_eq!(done["status"], "ready", "{done}");

        let job_id = done["jobId"].as_str().expect("jobId");
        let job = fixture
            .db
            .playback_jobs()
            .get(PlaybackJobId::parse(job_id).expect("uuid"))
            .await
            .expect("lecture")
            .expect("job persisté");
        assert_eq!(job.text.as_deref(), Some("bonjour"));
        assert!(job.show_text);
        assert!(job.media_asset_id.is_some());
    }

    /// An uploaded media shows the live profile of the member owning the
    /// client, including a `data:` avatar uploaded from their own computer.
    #[tokio::test]
    async fn upload_uses_the_paired_members_live_profile() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        insert_cached_upload(&fixture, "photo.png", b"png-body").await;
        let guild = GuildId::parse(GUILD).unwrap();
        let member = fixture
            .db
            .guild_members()
            .list_by_guild(&guild)
            .await
            .expect("membres")
            .into_iter()
            .next()
            .expect("membre");
        let avatar = "data:image/png;base64,\
                      iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8\
                      z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        fixture
            .db
            .guild_members()
            .update_profile(member.id, "Nhics_", Some(avatar))
            .await
            .expect("profil du membre");

        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png",
            b"png-body".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let done = wait_ingestion(&fixture.app, TOKEN, &body).await;
        let job = read_job(&fixture, &done).await;
        assert_eq!(job.author_name.as_deref(), Some("Nhics_"));
        assert_eq!(job.author_image.as_deref(), Some(avatar));
    }

    #[tokio::test]
    async fn upload_to_board_uses_title_and_duration() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        insert_cached_upload(&fixture, "photo.png", b"png-body").await;

        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=photo.png&saveToBoard=true&title=Ma%20photo&durationSec=12",
            b"png-body".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let done = wait_ingestion(&fixture.app, TOKEN, &body).await;
        assert_eq!(done["status"], "ready", "{done}");
        assert!(done["itemId"].is_string(), "{done}");

        let guild = GuildId::parse(GUILD).unwrap();
        let items = fixture
            .db
            .media_board()
            .list_by_guild(&guild)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "{body}");
        assert_eq!(items[0].title.as_deref(), Some("Ma photo"));
        assert_eq!(items[0].display_duration_sec, Some(12));
        assert_eq!(items[0].message, None);
    }

    #[tokio::test]
    async fn content_disposition_filename_is_used() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        insert_cached_upload(&fixture, "photo.png", b"png-body").await;

        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file",
            b"png-body".to_vec(),
            &[("content-disposition", "attachment; filename=\"photo.png\"")],
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let done = wait_ingestion(&fixture.app, TOKEN, &body).await;
        assert_eq!(done["status"], "ready", "{done}");
    }

    #[tokio::test]
    async fn both_slash_variants_are_mounted() {
        let fixture = fixture(2048).await;
        insert_client(&fixture.db).await;
        insert_cached_upload(&fixture, "photo.png", b"png-body").await;

        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file/?filename=photo.png",
            b"png-body".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    }

    /// A file the pipeline cannot read is still answered `202`: the failure
    /// is reported on the ingestion, with the stable media code the clients
    /// translate, and no job is created.
    #[tokio::test]
    async fn upload_failure_is_reported_on_the_ingestion() {
        let fixture = fixture_with(2048, &[("FFPROBE_BINARY", "lc-missing-ffprobe")]).await;
        insert_client(&fixture.db).await;

        let (status, body) = post_file(
            fixture.app.clone(),
            Some(TOKEN),
            "/ingest/file?filename=video.mp4",
            b"not-a-media".to_vec(),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");

        let done = wait_ingestion(&fixture.app, TOKEN, &body).await;
        assert_eq!(done["status"], "failed", "{done}");
        assert_eq!(done["error"]["code"], "media_ingestion_failed", "{done}");
        assert!(done["error"]["mediaCode"].is_string(), "{done}");
        assert!(done.get("jobId").is_none(), "{done}");
        let guild = GuildId::parse(GUILD).unwrap();
        let jobs = fixture
            .db
            .playback_jobs()
            .list_pending_by_guild(&guild)
            .await
            .unwrap();
        assert!(jobs.is_empty(), "aucun job ne doit être créé");
    }

    /// The answer never waits for the media: while the pipeline is blocked
    /// (here on the lock of the upload's source hash), the client already has
    /// its `202` and the ingestion reads `processing`. The work then finishes
    /// on its own, with no connection left.
    #[tokio::test]
    async fn upload_is_answered_while_the_pipeline_is_still_busy() {
        let fixture = fixture_with(2048, &[("FFPROBE_BINARY", "lc-missing-ffprobe")]).await;
        insert_client(&fixture.db).await;
        let body = b"busy-pipeline-body".to_vec();
        let hash = hex::encode(Sha256::digest(&body));
        let source = lc_media::source::MediaSource::from_upload("video.mp4", &hash);
        let busy = lc_media::ingest::lock_source_hash(&source.source_hash).await;

        let (status, accepted) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            post_file(
                fixture.app.clone(),
                Some(TOKEN),
                "/ingest/file?filename=video.mp4",
                body,
                &[],
            ),
        )
        .await
        .expect("la réponse ne doit pas attendre le pipeline");
        assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
        let id = accepted["ingestionId"].as_str().unwrap();
        let (status, state) = get_ingestion(&fixture.app, TOKEN, id).await;
        assert_eq!(status, StatusCode::OK, "{state}");
        assert_eq!(state["status"], "processing", "{state}");

        drop(busy);
        let done = wait_ingestion(&fixture.app, TOKEN, &accepted).await;
        assert_eq!(done["status"], "failed", "{done}");
    }
}
