//! `GET /ingest/whoami` : vérifie un jeton d'ingestion sans rien ingérer.
//!
//! Le Raccourci iOS ne regarde pas la réponse de `POST /ingest` : un jeton
//! invalide y déclenchait quand même la notification « Lien envoyé à
//! LiveChat », alors que le serveur avait répondu `401` en une milliseconde.
//! Cette route donne un moyen direct de tester le jeton seul :
//!
//! - aucune écriture (ni job, ni élément de board, ni téléchargement) ;
//! - les mêmes statuts que `POST /ingest` pour un jeton inconnu, expiré ou une
//!   ingestion désactivée (mapping commun [`auth::require_client`]) ;
//! - `200` avec le libellé et la guilde du client quand le jeton est valide,
//!   ce qui confirme *quel* appairage le raccourci utilise.
//!
//! Le contrôle rafraîchit le heartbeat du client comme une ingestion : vérifier
//! son jeton maintient l'appairage en vie, ce qui est le comportement attendu
//! d'un client qu'on utilise.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::AppState;

use super::auth;

/// Handler de `GET /ingest/whoami`.
pub async fn whoami(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let client = match auth::require_client(&state, &headers).await {
        Ok(client) => client,
        Err(response) => return response,
    };

    Json(json!({
        "ok": true,
        "clientId": client.id.to_canonical(),
        "label": client.label,
        "guildId": client.guild_id.as_str(),
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::Router;
    use lc_db::{Db, DbConfig};
    use lc_domain::entities::IngestClient;
    use lc_domain::ids::{GuildId, IngestClientId};
    use serde_json::Value;
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

    /// Creates an ingest client whose last activity is `active_since`. A member
    /// is not needed: `/ingest/whoami` vérifie le jeton, pas les droits.
    async fn insert_client_since(db: &Db, active_since: UtcDateTime) -> IngestClientId {
        let client = IngestClient {
            id: IngestClientId::new_v4(),
            guild_id: GuildId::parse(GUILD).expect("guilde"),
            label: "Extension-Test".into(),
            token_hash: hash_ingest_token(TOKEN),
            default_author_name: "LiveChat Extension".into(),
            default_author_image: None,
            created_by_discord_user_id: "42".into(),
            last_seen_at: Some(active_since),
            created_at: active_since,
            revoked_at: None,
        };
        let id = client.id;
        db.ingest_clients().create(&client).await.expect("client");
        id
    }

    async fn whoami(app: Router, token: Option<&str>) -> (StatusCode, Value) {
        let mut builder = HttpRequest::builder().method("GET").uri("/ingest/whoami");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn whoami_names_the_client_of_a_valid_token() {
        let fixture = fixture().await;
        let client_id = insert_client_since(&fixture.db, UtcDateTime::now()).await;

        let (status, body) = whoami(fixture.app.clone(), Some(TOKEN)).await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["ok"], true);
        assert_eq!(body["label"], "Extension-Test");
        assert_eq!(body["guildId"], GUILD);
        assert_eq!(
            body["clientId"].as_str(),
            Some(client_id.to_canonical().as_str())
        );
    }

    #[tokio::test]
    async fn whoami_rejects_an_unknown_token_with_the_ingest_401() {
        let fixture = fixture().await;
        insert_client_since(&fixture.db, UtcDateTime::now()).await;

        let (status, body) = whoami(fixture.app.clone(), Some("mauvais-token")).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "unauthorized");
    }

    #[tokio::test]
    async fn whoami_rejects_a_missing_token_when_ingestion_is_enabled() {
        let fixture = fixture().await;
        insert_client_since(&fixture.db, UtcDateTime::now()).await;

        let (status, body) = whoami(fixture.app.clone(), None).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "unauthorized");
    }

    #[tokio::test]
    async fn whoami_reports_a_disabled_ingestion_with_503() {
        let fixture = fixture().await;

        let (status, body) = whoami(fixture.app.clone(), Some(TOKEN)).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(body["error"], "ingest_api_disabled");
    }

    /// Un jeton au repos depuis plus de 180 jours est expiré : le code
    /// `ingest_token_expired` distingue « à ré-appairer » de « jeton inconnu ».
    #[tokio::test]
    async fn whoami_reports_an_idle_expired_token() {
        let fixture = fixture().await;
        let now = UtcDateTime::now();
        let idle = now - Duration::days(181);
        insert_client_since(&fixture.db, idle).await;

        let (status, body) = whoami(fixture.app.clone(), Some(TOKEN)).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"], "ingest_token_expired");
    }

    #[tokio::test]
    async fn whoami_refreshes_the_client_heartbeat() {
        let fixture = fixture().await;
        let now = UtcDateTime::now();
        let client_id = insert_client_since(&fixture.db, now - Duration::days(10)).await;

        let (status, _) = whoami(fixture.app.clone(), Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);

        let client = fixture
            .db
            .ingest_clients()
            .get(client_id)
            .await
            .expect("lecture")
            .expect("client");
        let last_seen = client.last_seen_at.expect("heartbeat");
        assert!(
            last_seen > now - Duration::days(10),
            "le heartbeat doit avoir été rafraîchi : {last_seen}"
        );
    }
}
