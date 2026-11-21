//! Tests bout en bout des guildes indépendantes : invitations, rôles,
//! administration de guilde et file d'attente (routeur complet, base mémoire).
//!
//! [`Harness`] et ses helpers sont `pub(super)` : les tests des autres modules
//! `overlay` (liaison Discord, code d'extension) les réutilisent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use lc_db::{Db, DbConfig};
use lc_domain::{FixedClock, GuildId, GuildRole, PairingCode, PairingCodeEntry, PairingMode};
use lc_protocol::{close_code, close_reason};
use serde_json::{json, Value};
use tempfile::TempDir;
use time::UtcDateTime;
use tower::ServiceExt as _;

use crate::app::AppState;
use crate::config::Config;
use crate::discord::guild_leave::DiscordGuildLeaver;
use crate::http::admin::hub::AdminHub;
use crate::shutdown::Shutdown;

use super::guild::{create_invite, NewInvite};

pub(super) fn fixed_now() -> UtcDateTime {
    time::macros::utc_datetime!(2026-11-10 12:00:00)
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
        ("ADMIN_UI_TOKEN".to_owned(), "admin-secret".to_owned()),
        ("ADMIN_UI_LOCAL_ONLY".to_owned(), "false".to_owned()),
    ])
    .unwrap()
}

/// Admin hub fake recording close codes, reasons and sent protocol messages.
#[derive(Debug, Default)]
pub(super) struct RecordingAdminHub {
    closes: Mutex<Vec<(uuid::Uuid, u16, String)>>,
    sent: Mutex<Vec<(uuid::Uuid, lc_protocol::ServerMessage)>>,
    broadcasts: Mutex<Vec<(GuildId, lc_protocol::ServerMessage)>>,
    connected: Mutex<Vec<uuid::Uuid>>,
    device_states:
        Mutex<std::collections::HashMap<uuid::Uuid, lc_protocol::DeviceSettingsStatePayload>>,
    deliver: Mutex<bool>,
}

impl RecordingAdminHub {
    pub(super) fn closes(&self) -> Vec<(uuid::Uuid, u16, String)> {
        self.closes.lock().expect("verrou du hub").clone()
    }

    pub(super) fn sent(&self) -> Vec<(uuid::Uuid, lc_protocol::ServerMessage)> {
        self.sent.lock().expect("verrou du hub").clone()
    }

    /// Messages broadcast to a guild room, in order.
    pub(super) fn broadcasts(&self) -> Vec<(GuildId, lc_protocol::ServerMessage)> {
        self.broadcasts.lock().expect("verrou du hub").clone()
    }

    pub(super) fn set_connected(&self, client_id: uuid::Uuid, connected: bool) {
        let mut ids = self.connected.lock().expect("verrou du hub");
        ids.retain(|id| *id != client_id);
        if connected {
            ids.push(client_id);
        }
    }

    pub(super) fn set_device_settings(
        &self,
        client_id: uuid::Uuid,
        settings: lc_protocol::DeviceSettingsStatePayload,
    ) {
        self.device_states
            .lock()
            .expect("verrou du hub")
            .insert(client_id, settings);
    }

    pub(super) fn set_deliver(&self, deliver: bool) {
        *self.deliver.lock().expect("verrou du hub") = deliver;
    }
}

#[async_trait::async_trait]
impl AdminHub for RecordingAdminHub {
    fn connected_count(&self) -> usize {
        self.connected.lock().expect("verrou du hub").len()
    }

    fn connected_for_guild(&self, _guild_id: &GuildId) -> usize {
        0
    }

    fn is_connected(&self, client_id: &uuid::Uuid) -> bool {
        self.connected
            .lock()
            .expect("verrou du hub")
            .contains(client_id)
    }

    fn close_client(&self, client_id: &uuid::Uuid, code: u16) -> bool {
        self.closes
            .lock()
            .expect("verrou du hub")
            .push((*client_id, code, String::new()));
        true
    }

    fn close_client_with_reason(&self, client_id: &uuid::Uuid, code: u16, reason: &str) -> bool {
        self.closes
            .lock()
            .expect("verrou du hub")
            .push((*client_id, code, reason.to_owned()));
        true
    }

    fn send_to_client(&self, client_id: &uuid::Uuid, message: lc_protocol::ServerMessage) -> bool {
        self.sent
            .lock()
            .expect("verrou du hub")
            .push((*client_id, message));
        *self.deliver.lock().expect("verrou du hub")
    }

    fn device_settings(
        &self,
        client_id: &uuid::Uuid,
    ) -> Option<lc_protocol::DeviceSettingsStatePayload> {
        self.device_states
            .lock()
            .expect("verrou du hub")
            .get(client_id)
            .copied()
    }

    fn set_session_mode(&self, client_id: &uuid::Uuid, _mode: lc_protocol::SessionMode) -> bool {
        self.is_connected(client_id)
    }

    fn broadcast_to_guild(&self, guild_id: &GuildId, message: lc_protocol::ServerMessage) -> usize {
        self.broadcasts
            .lock()
            .expect("verrou du hub")
            .push((guild_id.clone(), message));
        1
    }

    async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
        0
    }

    async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
        0
    }
}

pub(super) struct Harness {
    pub(super) app: Router,
    pub(super) state: AppState,
    pub(super) db: Db,
    pub(super) guild_id: GuildId,
    _media: TempDir,
}

impl Harness {
    /// Routeur complet + une guilde indépendante « Les Sardines ».
    pub(super) async fn new() -> Self {
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
        let app = crate::app::router(state.clone());
        Self {
            app,
            state,
            db,
            guild_id,
            _media: media,
        }
    }

    /// Remplace le client de départ Discord après la construction du routeur.
    pub(super) async fn with_leaver(leaver: Arc<dyn DiscordGuildLeaver>) -> Self {
        let mut harness = Self::new().await;
        harness.state = harness.state.clone().with_discord_leaver(leaver);
        harness.app = crate::app::router(harness.state.clone());
        harness
    }

    pub(super) async fn call(
        &self,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        call_app(&self.app, method, uri, token, &[], body).await
    }

    /// Code émis comme le ferait le panel serveur (aucun membre créateur).
    pub(super) async fn server_invite(&self, role: GuildRole) -> String {
        self.invite_for(&self.guild_id, role).await
    }

    /// Path of a file inside the harness media directory.
    pub(super) fn media_path(&self, name: &str) -> std::path::PathBuf {
        self._media.path().join(name)
    }

    /// Creates another standalone guild in the same database.
    pub(super) async fn second_guild(&self, name: &str) -> GuildId {
        let guild_id = GuildId::generate();
        self.db
            .guilds()
            .create_standalone(&guild_id, name, fixed_now())
            .await
            .unwrap();
        guild_id
    }

    /// Single-use invite code for `guild_id`, as issued by the server panel.
    pub(super) async fn invite_for(&self, guild_id: &GuildId, role: GuildRole) -> String {
        let invite = create_invite(
            &self.db,
            NewInvite {
                guild_id: guild_id.clone(),
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
    pub(super) async fn join(&self, code: &str, display_name: &str) -> Value {
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
    pub(super) async fn invite_and_join(&self, admin_token: &str, role: &str, name: &str) -> Value {
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

/// Calls a router with the same request shape as [`Harness::call`].
pub(super) async fn call_app(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

pub(super) fn token(body: &Value) -> String {
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
async fn guild_response_exposes_the_linked_discord_icon() {
    let harness = Harness::new().await;
    let discord_id = GuildId::parse("987654321098765432").unwrap();
    harness
        .db
        .guilds()
        .link_discord(&harness.guild_id, Some(&discord_id))
        .await
        .unwrap();
    harness
        .db
        .guilds()
        .set_discord_icon(discord_id.as_str(), Some("abc_DEF_123"))
        .await
        .unwrap();
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);

    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["guild"]["iconUrl"],
        "https://cdn.discordapp.com/icons/987654321098765432/abc_DEF_123.png?size=128"
    );

    harness
        .db
        .guilds()
        .set_discord_icon(discord_id.as_str(), None)
        .await
        .unwrap();
    let (status, body) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["guild"]["iconUrl"].is_null());
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
async fn member_removal_and_leave_use_distinct_close_reasons() {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingAdminHub::default());
    let state = harness
        .state
        .clone()
        .with_admin_hub(hub.clone() as Arc<dyn AdminHub>);
    let app = crate::app::router(state);

    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let guest = harness.invite_and_join(&admin_token, "guest", "Max").await;
    let guest_client_id = uuid::Uuid::parse_str(guest["clientId"].as_str().unwrap()).unwrap();
    let guest_id = guest["memberId"].as_str().unwrap().to_owned();

    let (status, body) = call_app(
        &app,
        Method::DELETE,
        &format!("/overlay/admin/members/{guest_id}"),
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        hub.closes(),
        vec![(
            guest_client_id,
            close_code::REVOKED,
            "member_removed".to_owned()
        )]
    );

    let leaver = harness.invite_and_join(&admin_token, "guest", "Léa").await;
    let leaver_token = token(&leaver);
    let leaver_client_id = uuid::Uuid::parse_str(leaver["clientId"].as_str().unwrap()).unwrap();
    let (status, body) = call_app(
        &app,
        Method::POST,
        "/overlay/leave",
        Some(&leaver_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let calls = hub.closes();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1],
        (
            leaver_client_id,
            close_code::REVOKED,
            close_reason::REVOKED.to_owned()
        )
    );
}

#[tokio::test]
async fn code_appareil_rattache_membre_existant() {
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
async fn permissions_emit_media_keys() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let guest = harness.invite_and_join(&admin_token, "guest", "Max").await;

    for body in [&admin, &guest] {
        let (status, guild) = harness
            .call(Method::GET, "/overlay/guild", Some(&token(body)), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{guild}");
        let permissions = &guild["permissions"];
        assert!(permissions.get("addMedia").is_some());
        assert!(permissions.get("editAnyMedia").is_some());
    }
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
            "/overlay/media-board/items",
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
            "/overlay/media-board/items",
            Some(&guest_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "forbidden");
    let (status, _) = harness
        .call(
            Method::GET,
            "/overlay/media-board/items",
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

    let now_ms = lc_domain::datetime_to_ms(fixed_now());
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
        .bind(format!("média {index}"))
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
    assert_eq!(body["playing"]["text"], "média 0");
    assert_eq!(body["playing"]["status"], "playing");
    let pending = body["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0]["text"], "média 1");
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

/// Inserts a `pairing_codes` row as the bot or `/overlay/ingest-code` would.
async fn insert_pairing_code(
    harness: &Harness,
    guild_id: &GuildId,
    code: &str,
    author: &str,
    author_name: Option<&str>,
    expires_at: Option<UtcDateTime>,
    used_at: Option<UtcDateTime>,
) {
    harness
        .db
        .pairing_codes()
        .create(&PairingCodeEntry {
            code: PairingCode::parse(code).unwrap(),
            guild_id: guild_id.clone(),
            mode: PairingMode::Normal,
            grants_admin: false,
            created_by_discord_user_id: author.to_owned(),
            author_name: author_name.map(str::to_owned),
            author_image: None,
            expires_at,
            used_at,
            created_at: fixed_now(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn codes_d_appairage_listes_avec_nature_et_auteur() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let member_id = admin["memberId"].as_str().unwrap();
    let expires_at = fixed_now() + time::Duration::minutes(10);
    insert_pairing_code(
        &harness,
        &harness.guild_id,
        "DISC42",
        "123456789012345678",
        Some("Discordeur"),
        Some(expires_at),
        None,
    )
    .await;
    insert_pairing_code(
        &harness,
        &harness.guild_id,
        "EXT123",
        &format!("member:{member_id}"),
        Some("Ancien nom"),
        Some(expires_at),
        None,
    )
    .await;

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/pairing-codes",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 2);
    let items = body["items"].as_array().unwrap();

    let discord = items
        .iter()
        .find(|item| item["code"] == "DISC42")
        .expect("code Discord listé");
    assert_eq!(discord["kind"], "discord");
    assert_eq!(discord["authorName"], "Discordeur");
    assert_eq!(discord["isActive"], true);
    assert_eq!(
        discord["expiresAt"],
        lc_domain::time::to_rfc3339_millis(expires_at)
    );
    assert_eq!(
        discord["createdAt"],
        lc_domain::time::to_rfc3339_millis(fixed_now())
    );

    let extension = items
        .iter()
        .find(|item| item["code"] == "EXT123")
        .expect("code d'extension listé");
    assert_eq!(extension["kind"], "extension");
    // Le nom courant du membre prime sur celui enregistré à l'émission.
    assert_eq!(extension["authorName"], "Alice");
    assert_eq!(extension["isActive"], true);
}

#[tokio::test]
async fn code_d_une_autre_guilde_ni_liste_ni_revocable() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let other_guild = harness.second_guild("Autre guilde").await;
    insert_pairing_code(
        &harness,
        &other_guild,
        "OTHER1",
        "123456789012345678",
        Some("Ailleurs"),
        Some(fixed_now() + time::Duration::minutes(10)),
        None,
    )
    .await;

    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/pairing-codes",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 0);

    // Même réponse qu'un code inconnu : aucune fuite d'existence.
    for code in ["OTHER1", "ZZZZ99"] {
        let (status, body) = harness
            .call(
                Method::POST,
                &format!("/overlay/admin/pairing-codes/{code}/revoke"),
                Some(&admin_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{code} : {body}");
        assert_eq!(body["error"], "pairing_code_not_found");
    }

    let stored = harness
        .db
        .pairing_codes()
        .get(&PairingCode::parse("OTHER1").unwrap())
        .await
        .unwrap()
        .expect("code de l'autre guilde conservé");
    assert_eq!(stored.used_at, None);
}

#[tokio::test]
async fn codes_d_appairage_reserves_au_droit_manage_invites() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let member = harness
        .invite_and_join(&token(&admin), "member", "Léa")
        .await;
    let member_token = token(&member);

    for (method, uri) in [
        (Method::GET, "/overlay/admin/pairing-codes"),
        (Method::POST, "/overlay/admin/pairing-codes/ABCDEF/revoke"),
        (Method::DELETE, "/overlay/admin/pairing-codes/expired"),
    ] {
        let (status, body) = harness.call(method, uri, Some(&member_token), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri} : {body}");
        assert_eq!(body["error"], "forbidden");
    }
}

#[tokio::test]
async fn revocation_et_purge_des_codes_d_appairage() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let other_guild = harness.second_guild("Autre guilde").await;

    insert_pairing_code(
        &harness,
        &harness.guild_id,
        "OLD001",
        "123456789012345678",
        Some("Discordeur"),
        Some(fixed_now() - time::Duration::minutes(1)),
        None,
    )
    .await;
    insert_pairing_code(
        &harness,
        &harness.guild_id,
        "USED01",
        "123456789012345678",
        Some("Discordeur"),
        None,
        Some(fixed_now()),
    )
    .await;
    insert_pairing_code(
        &harness,
        &harness.guild_id,
        "LIVE01",
        "123456789012345678",
        Some("Discordeur"),
        None,
        None,
    )
    .await;
    insert_pairing_code(
        &harness,
        &other_guild,
        "FOREIG",
        "123456789012345678",
        Some("Ailleurs"),
        Some(fixed_now() - time::Duration::minutes(1)),
        None,
    )
    .await;

    // Purge : expirés et utilisés de la guilde seulement.
    let (status, body) = harness
        .call(
            Method::DELETE,
            "/overlay/admin/pairing-codes/expired",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"], 2);

    let (_, listed) = harness
        .call(
            Method::GET,
            "/overlay/admin/pairing-codes",
            Some(&admin_token),
            None,
        )
        .await;
    let remaining: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect();
    assert_eq!(remaining, vec!["LIVE01"]);

    let foreign = harness
        .db
        .pairing_codes()
        .get(&PairingCode::parse("FOREIG").unwrap())
        .await
        .unwrap()
        .expect("code de l'autre guilde intact");
    assert_eq!(foreign.guild_id, other_guild);

    // Révocation d'un code actif : inutilisable ensuite.
    insert_pairing_code(
        &harness,
        &harness.guild_id,
        "REVOK1",
        "123456789012345678",
        Some("Discordeur"),
        Some(fixed_now() + time::Duration::minutes(10)),
        None,
    )
    .await;
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/pairing-codes/REVOK1/revoke",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revoked"], true);

    let (_, listed) = harness
        .call(
            Method::GET,
            "/overlay/admin/pairing-codes",
            Some(&admin_token),
            None,
        )
        .await;
    let revoked = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "REVOK1")
        .unwrap();
    assert_eq!(revoked["isActive"], false);
    assert!(!revoked["usedAt"].is_null());

    // Le code révoqué ne s'appaire plus (`POST /overlay/pair/consume` purge au
    // passage les codes inutilisables, d'où la vérification du code étranger
    // plus haut).
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(json!({"code": "REVOK1", "displayName": "Intrus"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn member_payload_expose_le_compte_discord() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let member_id = admin["memberId"]
        .as_str()
        .unwrap()
        .parse::<lc_domain::GuildMemberId>()
        .unwrap();
    assert!(harness
        .db
        .guild_members()
        .link_discord(member_id, "987654321098765432")
        .await
        .unwrap());

    let (status, members) = harness
        .call(
            Method::GET,
            "/overlay/admin/members",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{members}");
    let me = members["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["displayName"] == "Alice")
        .unwrap();
    assert_eq!(me["discordLinked"], true);
    assert_eq!(me["discordUserId"], "987654321098765432");
    assert_eq!(me["deviceCount"], 1);
    assert!(!me["createdAt"].is_null());

    let (status, guild) = harness
        .call(Method::GET, "/overlay/guild", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{guild}");
    assert_eq!(guild["member"]["discordUserId"], "987654321098765432");
}
