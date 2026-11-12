//! Tests bout en bout des guildes indépendantes : invitations, rôles,
//! administration de guilde et file d'attente (routeur complet, base mémoire).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, TimeZone, Utc};
use lc_db::{Db, DbConfig};
use lc_domain::{FixedClock, GuildId, GuildRole};
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt as _;

use crate::app::AppState;
use crate::config::Config;
use crate::shutdown::Shutdown;

use super::guild::{create_invite, NewInvite};

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
}

fn test_config(media_dir: &Path) -> Config {
    Config::from_pairs(vec![
        ("API_URL".to_owned(), "http://127.0.0.1:8099".to_owned()),
        (
            "MEDIA_URL_SECRET".to_owned(),
            "secret-test-guild".to_owned(),
        ),
        (
            "MEDIA_STORAGE_DIR".to_owned(),
            media_dir.to_string_lossy().into_owned(),
        ),
        ("MEDIA_BOARD_MAX_TOTAL_MB".to_owned(), "64".to_owned()),
    ])
    .unwrap()
}

struct Harness {
    app: Router,
    db: Db,
    guild_id: GuildId,
    _media: TempDir,
}

impl Harness {
    /// Routeur complet + une guilde indépendante « Les Sardines ».
    async fn new() -> Self {
        let media = TempDir::new().unwrap();
        let db = Db::open_in_memory(&DbConfig::default()).await.unwrap();
        let clock: Arc<dyn lc_domain::Clock> = Arc::new(FixedClock::new(fixed_now()));
        let state = AppState::new(
            Arc::new(test_config(media.path())),
            Some(db.clone()),
            Shutdown::detached(),
        )
        .with_clock(clock);
        let guild_id = GuildId::generate();
        db.guilds()
            .create_standalone(&guild_id, "Les Sardines", fixed_now())
            .await
            .unwrap();
        Self {
            app: crate::app::router(state),
            db,
            guild_id,
            _media: media,
        }
    }

    async fn call(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    /// Code émis comme le ferait le panel serveur (aucun membre créateur).
    async fn server_invite(&self, role: GuildRole) -> String {
        let invite = create_invite(
            &self.db,
            NewInvite {
                guild_id: self.guild_id.clone(),
                role,
                member_id: None,
                max_uses: Some(1),
                created_by_member_id: None,
                expires_at: None,
            },
            fixed_now(),
        )
        .await
        .unwrap()
        .unwrap();
        invite.code.as_str().to_owned()
    }

    /// Consomme un code ; retourne le corps de la réponse d'appairage.
    async fn join(&self, code: &str, display_name: &str) -> Value {
        let (status, body) = self
            .call(
                Method::POST,
                "/overlay/pair/consume",
                None,
                Some(json!({"code": code, "displayName": display_name})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// Crée une invitation en tant qu'admin et la fait consommer.
    async fn invite_and_join(&self, admin_token: &str, role: &str, name: &str) -> Value {
        let (status, body) = self
            .call(
                Method::POST,
                "/overlay/admin/invites",
                Some(admin_token),
                Some(json!({"role": role, "maxUses": 1, "expiresInHours": 24})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        self.join(body["invite"]["code"].as_str().unwrap(), name)
            .await
    }
}

fn token(body: &Value) -> String {
    body["clientToken"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn code_admin_puis_invitation_a_usage_unique() {
    let harness = Harness::new().await;
    let code = harness.server_invite(GuildRole::Admin).await;

    let admin = harness.join(&code, "  Quentin ").await;
    assert_eq!(admin["role"], "admin");
    assert_eq!(admin["sessionMode"], "normal");
    assert_eq!(admin["displayName"], "Quentin");
    assert_eq!(admin["guildName"], "Les Sardines");
    assert_eq!(admin["guildId"], harness.guild_id.as_str());

    // Le code admin est à usage unique.
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(json!({"code": code})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "pairing_code_invalid_or_expired");

    let admin_token = token(&admin);
    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["guild"]["name"], "Les Sardines");
    assert_eq!(body["guild"]["standalone"], true);
    assert_eq!(body["member"]["role"], "admin");
    assert_eq!(body["permissions"]["manageMembers"], true);

    // Un code saisi avec tiret et en minuscules est accepté.
    let (_, created) = harness
        .call(
            Method::POST,
            "/overlay/admin/invites",
            Some(&admin_token),
            Some(json!({"role": "guest"})),
        )
        .await;
    let raw = created["invite"]["code"].as_str().unwrap();
    assert_eq!(created["invite"]["maxUses"], Value::Null);
    let typed = format!("{}-{}", &raw[..4], &raw[4..]).to_lowercase();
    let guest = harness.join(&typed, "Invité-42").await;
    assert_eq!(guest["role"], "guest");
    assert_eq!(guest["sessionMode"], "invite_read_only");

    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&token(&guest)), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["permissions"]["play"], false);
    assert_eq!(body["permissions"]["manageMembers"], false);
    assert_eq!(body["permissions"]["viewQueue"], true);
}

#[tokio::test]
async fn un_invite_ou_un_membre_n_administre_pas() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let member = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let guest = harness.invite_and_join(&admin_token, "guest", "Max").await;

    for body in [&member, &guest] {
        let other = token(body);
        for (method, uri, payload) in [
            (Method::GET, "/overlay/admin/members", None),
            (Method::GET, "/overlay/admin/invites", None),
            (
                Method::POST,
                "/overlay/admin/invites",
                Some(json!({"role": "admin"})),
            ),
            (
                Method::PATCH,
                "/overlay/admin/settings",
                Some(json!({"name": "Piraté"})),
            ),
            (Method::DELETE, "/overlay/queue", None),
            (
                Method::DELETE,
                &*format!(
                    "/overlay/admin/members/{}",
                    admin["memberId"].as_str().unwrap()
                ),
                None,
            ),
        ] {
            let (status, response) = harness.call(method, uri, Some(&other), payload).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri} : {response}");
            assert_eq!(response["error"], "forbidden");
        }
        // La file reste lisible par tous.
        let (status, _) = harness
            .call(Method::GET, "/overlay/queue", Some(&other), None)
            .await;
        assert_eq!(status, StatusCode::OK);
    }

    // Sans jeton : 401.
    let (status, _) = harness
        .call(Method::GET, "/overlay/admin/members", None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn roles_exclusion_et_dernier_admin() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let admin_id = admin["memberId"].as_str().unwrap().to_owned();
    let guest = harness.invite_and_join(&admin_token, "guest", "Max").await;
    let guest_token = token(&guest);
    let guest_id = guest["memberId"].as_str().unwrap().to_owned();

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/members",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 2);
    let items = body["items"].as_array().unwrap();
    let me = items
        .iter()
        .find(|item| item["displayName"] == "Quentin")
        .unwrap();
    assert_eq!(me["isSelf"], true);
    assert_eq!(me["role"], "admin");
    let max = items
        .iter()
        .find(|item| item["displayName"] == "Max")
        .unwrap();
    assert_eq!(max["role"], "guest");
    assert_eq!(max["isSelf"], false);

    // Promotion invité → membre : effective à la requête suivante.
    let (status, _) = harness
        .call(
            Method::PATCH,
            &format!("/overlay/admin/members/{guest_id}"),
            Some(&admin_token),
            Some(json!({"role": "member"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&guest_token), None)
        .await;
    assert_eq!(body["member"]["role"], "member");
    assert_eq!(body["permissions"]["play"], true);

    // Le dernier admin ne peut ni se rétrograder ni partir.
    let (status, body) = harness
        .call(
            Method::PATCH,
            &format!("/overlay/admin/members/{admin_id}"),
            Some(&admin_token),
            Some(json!({"role": "member"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "last_admin");
    let (status, body) = harness
        .call(Method::POST, "/overlay/leave", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "last_admin");

    let (status, body) = harness
        .call(
            Method::PATCH,
            &format!("/overlay/admin/members/{guest_id}"),
            Some(&admin_token),
            Some(json!({"role": "roi"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_role");

    // Exclusion : le jeton de l'exclu ne vaut plus rien.
    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/members/{guest_id}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removed"], true);
    let (status, _) = harness
        .call(Method::GET, "/overlay/guild", Some(&guest_token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Un membre d'une autre guilde est introuvable.
    let (status, _) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/members/{}", uuid::Uuid::new_v4()),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn code_appareil_rattache_au_meme_membre() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/device-code",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code = body["invite"]["code"].as_str().unwrap().to_owned();

    let second = harness.join(&code, "Nom ignoré").await;
    assert_eq!(second["memberId"], admin["memberId"]);
    assert_eq!(second["role"], "admin");
    assert_eq!(second["displayName"], "Quentin");
    assert_ne!(second["clientId"], admin["clientId"]);

    // Les deux overlays restent actifs ; le code appareil n'est pas listé.
    let (_, members) = harness
        .call(
            Method::GET,
            "/overlay/admin/members",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(members["count"], 1);
    assert_eq!(members["items"][0]["deviceCount"], 2);
    let (_, invites) = harness
        .call(
            Method::GET,
            "/overlay/admin/invites",
            Some(&admin_token),
            None,
        )
        .await;
    // Seul le code admin d'origine apparaît.
    assert_eq!(invites["count"], 1);
    assert_eq!(invites["items"][0]["role"], "admin");
    assert_eq!(invites["items"][0]["memberId"], Value::Null);
    assert_ne!(invites["items"][0]["code"], code.as_str());

    // Profil : le pseudo se met à jour.
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/profile",
            Some(&token(&second)),
            Some(json!({"displayName": "Quentin B."})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, guild) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(guild["member"]["displayName"], "Quentin B.");
}

#[tokio::test]
async fn reglages_revocation_et_board_des_invites() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let guest = harness.invite_and_join(&admin_token, "guest", "Max").await;
    let guest_token = token(&guest);

    // Par défaut un invité voit le board.
    let (status, _) = harness
        .call(
            Method::GET,
            "/overlay/meme-board/items",
            Some(&guest_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({
                "name": "Sardines",
                "defaultMediaTime": 30,
                "maxMediaTime": 0,
                "guestsCanViewBoard": false,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["guild"]["name"], "Sardines");
    assert_eq!(body["guild"]["settings"]["defaultMediaTime"], 30);
    assert_eq!(body["guild"]["settings"]["maxMediaTime"], Value::Null);
    assert_eq!(body["guild"]["guestsCanViewBoard"], false);

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/meme-board/items",
            Some(&guest_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "forbidden");
    let (status, _) = harness
        .call(
            Method::GET,
            "/overlay/meme-board/items",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"defaultMediaTime": "trente"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Une invitation révoquée ne se consomme plus.
    let (_, created) = harness
        .call(
            Method::POST,
            "/overlay/admin/invites",
            Some(&admin_token),
            Some(json!({"role": "member", "maxUses": 10})),
        )
        .await;
    let code = created["invite"]["code"].as_str().unwrap().to_owned();
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/invites/{code}/revoke"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["revoked"], true);
    let (status, _) = harness
        .call(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(json!({"code": code})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, listed) = harness
        .call(
            Method::GET,
            "/overlay/admin/invites",
            Some(&admin_token),
            None,
        )
        .await;
    let revoked = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|invite| invite["code"] == code.as_str())
        .unwrap();
    assert_eq!(revoked["isActive"], false);
}

#[tokio::test]
async fn file_d_attente_lue_et_moderee() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);

    let now_ms = fixed_now().timestamp_millis();
    for (index, (id, status)) in [
        (uuid::Uuid::new_v4(), "PLAYING"),
        (uuid::Uuid::new_v4(), "PENDING"),
        (uuid::Uuid::new_v4(), "PENDING"),
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO playback_jobs ( \
                id, guild_id, text, author_name, duration_sec, status, \
                submission_date, execution_date, scheduled_at, started_at \
             ) VALUES (?, ?, ?, 'Léa', 10, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(harness.guild_id.as_str())
        .bind(format!("mème {index}"))
        .bind(status)
        .bind(now_ms + index as i64)
        .bind(now_ms)
        .bind(now_ms)
        .bind((status == "PLAYING").then_some(now_ms))
        .execute(harness.db.pool())
        .await
        .unwrap();
    }

    let (status, body) = harness
        .call(Method::GET, "/overlay/queue", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["playing"]["text"], "mème 0");
    assert_eq!(body["playing"]["status"], "playing");
    let pending = body["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0]["text"], "mème 1");
    assert_eq!(pending[0]["authorName"], "Léa");

    // Le job en cours ne se retire pas par cette route.
    let playing_id = body["playing"]["id"].as_str().unwrap().to_owned();
    let (_, removed) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/queue/{playing_id}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(removed["removed"], false);

    let first_pending = pending[0]["id"].as_str().unwrap().to_owned();
    let (_, removed) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/queue/{first_pending}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(removed["removed"], true);

    let (_, cleared) = harness
        .call(Method::DELETE, "/overlay/queue", Some(&admin_token), None)
        .await;
    assert_eq!(cleared["removedCount"], 1);

    let (_, body) = harness
        .call(Method::GET, "/overlay/queue", Some(&admin_token), None)
        .await;
    assert_eq!(body["pending"].as_array().unwrap().len(), 0);
    assert_eq!(body["playing"]["id"], playing_id.as_str());
}
