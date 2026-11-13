//! `POST /ingest/pair/consume` : appairage d'un client d'ingestion par code.
//!
//! Reproduit le legacy à l'ordre près :
//! 1. code absent → 400 `invalid_payload` ;
//! 2. base absente → 503 `ingest_api_disabled` ;
//! 3. purge des codes expirés, lookup du code utilisable ;
//! 4. code inconnu/consommé/expiré → 404 `pairing_code_invalid_or_expired` ;
//! 5. code réservé à l'invitation overlay (`INVITE_READ_ONLY`) → 403
//!    `pairing_code_scope_mismatch` ;
//! 6. consommation atomique du code (usage unique) ;
//! 7. révocation des anciens clients de la guilde portant le même libellé ;
//! 8. création du client et retour du token en clair (seul hash persisté).
//!
//! La réponse renvoie la configuration nécessaire à l'extension navigateur
//! (`apiBaseUrl`, `ingestApiToken`, identifiants et valeurs par défaut).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lc_domain::enums::PairingMode;
use lc_domain::ids::{IngestClientId, PairingCode};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::media::tokens::{generate_ingest_token, hash_ingest_token};

use super::body;
use super::{
    invalid_payload, is_conflict, media_service_error, simple_error, DEFAULT_INGEST_AUTHOR_NAME,
    DEFAULT_INGEST_DEVICE_PREFIX,
};

/// Longueur maximale d'un libellé de client (deviceName), alignée sur
/// `/overlay/pair/consume` (AUD-A2-012).
const MAX_DEVICE_NAME_CHARS: usize = 200;

/// Tronque un libellé de client à [`MAX_DEVICE_NAME_CHARS`] caractères, sur
/// une frontière UTF-8, après trim.
fn truncate_device_name(raw: &str) -> String {
    raw.trim().chars().take(MAX_DEVICE_NAME_CHARS).collect()
}

/// Handler de `POST /ingest/pair/consume`.
pub async fn consume(
    State(state): State<AppState>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    // Un JSON malformé est une erreur de payload dédiée, pas un
    // `request_error` générique (AUD-A2-009).
    let value = match body {
        Ok(Json(value)) => value,
        Err(rejection) => {
            tracing::debug!(error = ?rejection, "corps JSON invalide (ingest pair/consume)");
            return invalid_payload(None);
        }
    };

    let Some(raw_code) = body::non_empty_string(value.get("code")) else {
        return invalid_payload(None);
    };
    let Ok(code) = PairingCode::parse(&raw_code) else {
        // Le legacy normalisait puis cherchait le code : un code mal formé est
        // un code inconnu, pas une erreur de payload.
        return simple_error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired");
    };

    let Some(db) = state.db.as_ref() else {
        tracing::warn!("appairage refusé : aucune base de données configurée");
        return simple_error(StatusCode::SERVICE_UNAVAILABLE, "ingest_api_disabled");
    };

    let now = state.clock.now();
    let codes = db.pairing_codes();

    if let Err(error) = codes.delete_expired(now).await {
        return media_service_error(error.into());
    }

    let entry = match codes.get(&code).await {
        Ok(Some(entry)) => entry,
        Ok(None) => return pairing_code_invalid(),
        Err(error) => return media_service_error(error.into()),
    };

    if !entry.is_usable_at(now) {
        return pairing_code_invalid();
    }

    // Ce code a été créé pour un overlay invité : l'ingestion ne doit pas
    // pouvoir le consommer.
    if entry.mode == PairingMode::InviteReadOnly {
        return simple_error(StatusCode::FORBIDDEN, "pairing_code_scope_mismatch");
    }

    let entry = match codes.consume(&code, now).await {
        Ok(Some(entry)) => entry,
        Ok(None) => return pairing_code_invalid(),
        Err(error) => return media_service_error(error.into()),
    };

    let author_name = entry
        .author_name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_INGEST_AUTHOR_NAME.to_owned());
    let author_image = entry
        .author_image
        .clone()
        .filter(|value| !value.trim().is_empty());

    let device_name = body::device_name(&value)
        .map(|name| truncate_device_name(&name))
        .unwrap_or_else(|| {
            truncate_device_name(&format!("{DEFAULT_INGEST_DEVICE_PREFIX}-{author_name}"))
        });

    let clients = db.ingest_clients();
    let guild_id = entry.guild_id.clone();

    // Legacy : révoque les clients de même (guilde, libellé) avant de créer.
    match clients.list_by_guild(&guild_id).await {
        Ok(existing) => {
            for client in existing
                .iter()
                .filter(|client| client.is_active() && client.label == device_name)
            {
                if let Err(error) = clients.revoke(client.id, now).await {
                    tracing::warn!(
                        error = %error,
                        client_id = %client.id.to_canonical(),
                        "impossible de révoquer un ancien client d'ingestion"
                    );
                }
            }
        }
        Err(error) => return media_service_error(error.into()),
    }

    let mut raw_token = generate_ingest_token();
    let token_hash = hash_ingest_token(&raw_token);

    let mut client = lc_domain::entities::IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: device_name.clone(),
        token_hash,
        default_author_name: author_name.clone(),
        default_author_image: author_image.clone(),
        created_by_discord_user_id: entry.created_by_discord_user_id.clone(),
        last_seen_at: None,
        created_at: now,
        revoked_at: None,
    };

    // AUD-A2-010 : une collision de `token_hash` (unique en base) ne doit pas
    // être avalée en 500 générique — on régénère un jeton une fois, comme le
    // comportement attendu d'une création de token. Un vrai conflit persistant
    // (improbable) reste une erreur serveur.
    let mut create_error: Option<lc_db::DbError> = None;
    for attempt in 0..2_u8 {
        match clients.create(&client).await {
            Ok(()) => {
                create_error = None;
                break;
            }
            Err(error) => {
                let retryable = attempt == 0 && is_conflict(&error);
                create_error = Some(error);
                if !retryable {
                    break;
                }
                tracing::warn!(
                    attempt,
                    "collision de hash de token d'ingestion : nouvelle génération"
                );
                raw_token = generate_ingest_token();
                client.token_hash = hash_ingest_token(&raw_token);
            }
        }
    }
    if let Some(error) = create_error {
        return media_service_error(error.into());
    }

    tracing::info!(
        client_id = %client.id.to_canonical(),
        guild_id = %guild_id.as_str(),
        device = %device_name,
        "client d'ingestion appairé"
    );

    let api_base_url = state.config.api_url.as_str().trim_end_matches('/');
    let payload = json!({
        "apiBaseUrl": api_base_url,
        "ingestApiToken": raw_token,
        "ingestClientId": client.id.to_canonical(),
        "guildId": guild_id.as_str(),
        "deviceName": device_name,
        "authorName": author_name,
        "authorImage": author_image,
    });

    (StatusCode::OK, Json(payload)).into_response()
}

/// Réponse 404 du legacy pour un code inutilisable.
fn pairing_code_invalid() -> Response {
    simple_error(StatusCode::NOT_FOUND, "pairing_code_invalid_or_expired")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use axum::Router;
    use chrono::{Duration, Utc};
    use lc_db::{Db, DbConfig};
    use lc_domain::entities::PairingCodeEntry;
    use lc_domain::ids::GuildId;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::config::Config;
    use crate::shutdown::Shutdown;

    use super::super::router;
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        app: Router,
        db: Db,
        guild_id: GuildId,
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
        Fixture {
            _dir: dir,
            app: router().with_state(state),
            db,
            guild_id: GuildId::parse("123456789012345678").expect("guilde"),
        }
    }

    fn pairing_entry(
        guild_id: &GuildId,
        code: &str,
        mode: PairingMode,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> PairingCodeEntry {
        let now = Utc::now();
        PairingCodeEntry {
            code: PairingCode::parse(code).expect("code"),
            guild_id: guild_id.clone(),
            mode,
            grants_admin: false,
            created_by_discord_user_id: "42".into(),
            author_name: Some("Alice".into()),
            author_image: Some("https://example.com/a.png".into()),
            expires_at,
            used_at: None,
            created_at: now,
        }
    }

    async fn post(app: Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let request = HttpRequest::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn consumes_a_code_and_creates_a_client() {
        let fixture = fixture().await;
        fixture
            .db
            .pairing_codes()
            .create(&pairing_entry(
                &fixture.guild_id,
                "AB12CD",
                PairingMode::Normal,
                None,
            ))
            .await
            .expect("code inséré");

        let (status, body) = post(
            fixture.app.clone(),
            "/ingest/pair/consume",
            json!({ "code": " ab12cd ", "deviceName": "Mon PC" }),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["apiBaseUrl"], "http://localhost:3000");
        assert_eq!(body["deviceName"], "Mon PC");
        assert_eq!(body["authorName"], "Alice");
        assert_eq!(body["authorImage"], "https://example.com/a.png");
        assert_eq!(body["guildId"], fixture.guild_id.as_str());
        let token = body["ingestApiToken"].as_str().expect("token");

        // Le token en clair n'est jamais persisté : seul son hash l'est.
        let client = fixture
            .db
            .ingest_clients()
            .get_by_token_hash(&hash_ingest_token(token))
            .await
            .expect("lecture")
            .expect("client créé");
        assert_eq!(client.label, "Mon PC");
        assert!(client.is_active());

        // Le code est consommé (usage unique).
        let entry = fixture
            .db
            .pairing_codes()
            .get(&PairingCode::parse("AB12CD").expect("code"))
            .await
            .expect("lecture")
            .expect("code existant");
        assert!(entry.used_at.is_some());
    }

    #[tokio::test]
    async fn expired_or_unknown_code_is_rejected() {
        let fixture = fixture().await;
        let expired = Utc::now() - Duration::minutes(1);
        fixture
            .db
            .pairing_codes()
            .create(&pairing_entry(
                &fixture.guild_id,
                "ZZ99ZZ",
                PairingMode::Normal,
                Some(expired),
            ))
            .await
            .expect("code inséré");

        for code in ["ZZ99ZZ", "INCONNU"] {
            let (status, body) = post(
                fixture.app.clone(),
                "/ingest/pair/consume",
                json!({ "code": code }),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{code}: {body}");
            assert_eq!(body["error"], "pairing_code_invalid_or_expired");
        }
    }

    #[tokio::test]
    async fn invite_only_code_cannot_pair_an_ingest_client() {
        let fixture = fixture().await;
        fixture
            .db
            .pairing_codes()
            .create(&pairing_entry(
                &fixture.guild_id,
                "INV123",
                PairingMode::InviteReadOnly,
                None,
            ))
            .await
            .expect("code inséré");

        let (status, body) = post(
            fixture.app.clone(),
            "/ingest/pair/consume",
            json!({ "code": "INV123" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "pairing_code_scope_mismatch");
    }

    async fn post_raw(app: Router, uri: &str, raw: &str) -> (StatusCode, Value) {
        let request = HttpRequest::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(raw.to_owned()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn missing_code_is_a_payload_error() {
        let fixture = fixture().await;
        let (status, body) = post(fixture.app.clone(), "/ingest/pair/consume", json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_payload");
    }

    #[tokio::test]
    async fn malformed_json_is_a_payload_error() {
        // AUD-A2-009 : un corps JSON illisible doit produire `invalid_payload`,
        // pas le `request_error` générique de la normalisation d'erreurs.
        let fixture = fixture().await;
        let (status, body) =
            post_raw(fixture.app.clone(), "/ingest/pair/consume", "{\"code\": ").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload", "{body}");
    }

    #[tokio::test]
    async fn long_device_name_is_truncated() {
        // AUD-A2-012 : le libellé est borné à 200 caractères, comme côté overlay.
        let fixture = fixture().await;
        fixture
            .db
            .pairing_codes()
            .create(&pairing_entry(
                &fixture.guild_id,
                "TRUNC1",
                PairingMode::Normal,
                None,
            ))
            .await
            .expect("code inséré");

        let long_name = "a".repeat(300);
        let (status, body) = post(
            fixture.app.clone(),
            "/ingest/pair/consume",
            json!({ "code": "TRUNC1", "deviceName": long_name }),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["deviceName"].as_str().map(|name| name.chars().count()),
            Some(200)
        );

        let token = body["ingestApiToken"].as_str().expect("token");
        let client = fixture
            .db
            .ingest_clients()
            .get_by_token_hash(&hash_ingest_token(token))
            .await
            .expect("lecture")
            .expect("client créé");
        assert_eq!(client.label.chars().count(), 200);
    }
}
