//! Tests des routes d'administration des appareils depuis l'overlay
//! (`GET/PATCH/DELETE /overlay/admin/devices*`, `POST /overlay/admin/device-codes`).
//!
//! Ils réutilisent le harnais complet de [`super::guild_tests`] et son hub
//! factice, qui enregistre les fermetures (`closes()`) et les messages envoyés
//! (`sent()`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use axum::Router;
use lc_domain::{
    GuildId, GuildMemberId, GuildRole, OverlayClient, OverlayClientId, OverlaySessionMode,
};
use lc_protocol::{close_code, DeviceSettingsStatePayload, ServerMessage, SessionMode};
use serde_json::{json, Value};

use crate::http::admin::hub::AdminHub;

use super::auth::hash_token;
use super::guild_tests::{call_app, fixed_now, token, Harness, RecordingAdminHub};

/// Routeur du harnais avec un hub instrumenté.
fn router_with_hub(harness: &Harness, hub: Arc<RecordingAdminHub>) -> Router {
    crate::app::router(
        harness
            .state
            .clone()
            .with_admin_hub(hub as Arc<dyn AdminHub>),
    )
}

/// Insère un appareil actif, éventuellement rattaché à un membre.
async fn seed_device(
    harness: &Harness,
    guild_id: &GuildId,
    label: &str,
    secret: &str,
    member_id: Option<GuildMemberId>,
) -> OverlayClient {
    let client = OverlayClient {
        id: OverlayClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: label.to_owned(),
        token_hash: hash_token(secret),
        session_mode: OverlaySessionMode::Normal,
        default_author_name: None,
        default_author_image: None,
        created_by_discord_user_id: None,
        last_seen_at: Some(fixed_now()),
        created_at: fixed_now(),
        revoked_at: None,
    };
    harness.db.overlay_clients().create(&client).await.unwrap();
    if let Some(member_id) = member_id {
        harness
            .db
            .overlay_clients()
            .set_member(client.id, member_id)
            .await
            .unwrap();
    }
    client
}

fn member_id_of(body: &Value) -> GuildMemberId {
    body["memberId"].as_str().unwrap().parse().unwrap()
}

fn client_id_of(body: &Value) -> OverlayClientId {
    body["clientId"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn devices_are_listed_for_the_token_guild_only() {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingAdminHub::default());
    let app = router_with_hub(&harness, hub.clone());

    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);

    // L'appareil appelant est marqué `isSelf`.
    let (status, body) = call_app(
        &app,
        Method::GET,
        "/overlay/admin/devices",
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 1);
    assert_eq!(body["items"][0]["id"], admin["clientId"]);
    assert_eq!(body["items"][0]["memberId"], admin["memberId"]);
    assert_eq!(body["items"][0]["memberName"], "Alice");
    assert_eq!(body["items"][0]["role"], "admin");
    assert_eq!(body["items"][0]["isSelf"], true);
    assert_eq!(body["items"][0]["connected"], false);

    // Un second appareil de la guilde, rattaché au même membre.
    let other = seed_device(
        &harness,
        &harness.guild_id,
        "Studio",
        "secret-other-device",
        Some(member_id_of(&admin)),
    )
    .await;
    harness
        .db
        .overlay_clients()
        .update_client_info(other.id, Some("3.7.0"), Some("windows"))
        .await
        .unwrap();
    hub.set_connected(other.id.as_uuid(), true);
    hub.set_device_settings(
        other.id.as_uuid(),
        DeviceSettingsStatePayload {
            volume: 0.4,
            media_muted: true,
            paused: false,
        },
    );

    // Un appareil d'une autre guilde ne doit jamais apparaître.
    let elsewhere = harness.second_guild("Autre guilde").await;
    seed_device(&harness, &elsewhere, "Ailleurs", "secret-elsewhere", None).await;

    let (status, body) = call_app(
        &app,
        Method::GET,
        "/overlay/admin/devices",
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 2);
    let other_id = other.id.to_canonical();
    let items = body["items"].as_array().unwrap();
    let device = items
        .iter()
        .find(|item| item["id"] == other_id.as_str())
        .expect("appareil de la guilde listé");
    assert_eq!(device["label"], "Studio");
    assert_eq!(device["memberId"], admin["memberId"]);
    assert_eq!(device["memberName"], "Alice");
    assert_eq!(device["role"], "admin");
    assert_eq!(device["sessionMode"], "normal");
    assert_eq!(device["connected"], true);
    assert_eq!(device["overlayVersion"], "3.7.0");
    assert_eq!(device["platform"], "windows");
    assert_eq!(
        device["lastSeenAt"],
        lc_domain::time::to_rfc3339_millis(fixed_now())
    );
    assert_eq!(device["guildSettings"]["volume"], 0.4);
    assert_eq!(device["guildSettings"]["mediaMuted"], true);
    assert_eq!(device["guildSettings"]["paused"], false);
    assert_eq!(device["isSelf"], false);
}

#[tokio::test]
async fn device_routes_are_reserved_to_manage_members() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let member = harness
        .invite_and_join(&token(&admin), "member", "Léa")
        .await;
    let member_token = token(&member);
    let client_id = admin["clientId"].as_str().unwrap().to_owned();
    let member_id = member["memberId"].as_str().unwrap().to_owned();
    let device_path = format!("/overlay/admin/devices/{client_id}");

    for (method, uri, payload) in [
        (Method::GET, "/overlay/admin/devices".to_owned(), None),
        (
            Method::PATCH,
            device_path.clone(),
            Some(json!({"label": "Pirate"})),
        ),
        (Method::DELETE, device_path.clone(), None),
        (
            Method::PATCH,
            format!("{device_path}/guild-settings"),
            Some(json!({"paused": true})),
        ),
        (
            Method::POST,
            "/overlay/admin/device-codes".to_owned(),
            Some(json!({"memberId": member_id})),
        ),
    ] {
        let (status, body) = harness
            .call(method, &uri, Some(&member_token), payload)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri} : {body}");
        assert_eq!(body["error"], "forbidden");
    }

    // Sans jeton : 401.
    let (status, _) = harness
        .call(Method::GET, "/overlay/admin/devices", None, None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn revoking_a_device_closes_its_socket_and_invalidates_its_token() {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingAdminHub::default());
    hub.set_deliver(true);
    let app = router_with_hub(&harness, hub.clone());

    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let lea = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let lea_id = member_id_of(&lea).to_canonical();

    // Un code émis par l'admin rattache un second appareil à Léa.
    let (status, body) = call_app(
        &app,
        Method::POST,
        "/overlay/admin/device-codes",
        Some(&admin_token),
        &[],
        Some(json!({"memberId": lea_id})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code = body["invite"]["code"].as_str().unwrap().to_owned();
    let device = harness.join(&code, "Nom ignoré").await;
    assert_eq!(device["memberId"], lea["memberId"]);
    let device_token = token(&device);
    let device_client = client_id_of(&device);

    let (status, body) = call_app(
        &app,
        Method::GET,
        "/overlay/admin/devices",
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 3);

    let (status, body) = call_app(
        &app,
        Method::DELETE,
        &format!("/overlay/admin/devices/{}", device_client.to_canonical()),
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["found"], true);
    assert_eq!(body["revoked"], true);
    assert_eq!(
        hub.closes(),
        vec![(device_client.as_uuid(), close_code::REVOKED, String::new())]
    );

    // Le jeton révoqué n'authentifie plus rien.
    let (status, _) = call_app(
        &app,
        Method::GET,
        "/overlay/guild",
        Some(&device_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // L'appareil appelant ne peut pas se révoquer lui-même.
    let (status, body) = call_app(
        &app,
        Method::DELETE,
        &format!(
            "/overlay/admin/devices/{}",
            client_id_of(&admin).to_canonical()
        ),
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "cannot_revoke_self");
    let (status, _) = call_app(
        &app,
        Method::GET,
        "/overlay/guild",
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn devices_and_members_of_another_guild_are_not_found() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let other_guild = harness.second_guild("Autre guilde").await;
    let other_device =
        seed_device(&harness, &other_guild, "Ailleurs", "secret-elsewhere", None).await;
    let other_member = harness
        .join(
            &harness.invite_for(&other_guild, GuildRole::Member).await,
            "Ailleurs",
        )
        .await;

    let path = format!("/overlay/admin/devices/{}", other_device.id.to_canonical());
    for (method, uri, payload) in [
        (
            Method::PATCH,
            path.clone(),
            Some(json!({"label": "Pirate"})),
        ),
        (Method::DELETE, path.clone(), None),
        (
            Method::PATCH,
            format!("{path}/guild-settings"),
            Some(json!({"paused": true})),
        ),
    ] {
        let (status, body) = harness
            .call(method, &uri, Some(&admin_token), payload)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} : {body}");
        assert_eq!(body["error"], "device_not_found");
    }

    // Un identifiant inconnu répond exactement comme un appareil d'une autre
    // guilde.
    for raw_id in [uuid::Uuid::new_v4().to_string(), "not-a-uuid".to_owned()] {
        let (status, body) = harness
            .call(
                Method::DELETE,
                &format!("/overlay/admin/devices/{raw_id}"),
                Some(&admin_token),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "device_not_found");
    }

    // Code d'appareil pour un membre d'une autre guilde : introuvable.
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/device-codes",
            Some(&admin_token),
            Some(json!({"memberId": other_member["memberId"]})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "member_not_found");

    // L'appareil de l'autre guilde est resté intact.
    let stored = harness
        .db
        .overlay_clients()
        .get(other_device.id)
        .await
        .unwrap()
        .expect("appareil conservé");
    assert_eq!(stored.label, "Ailleurs");
    assert_eq!(stored.revoked_at, None);
}

#[tokio::test]
async fn device_label_and_session_mode_are_validated_and_applied() {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingAdminHub::default());
    let app = router_with_hub(&harness, hub.clone());
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let device = seed_device(&harness, &harness.guild_id, "Studio", "secret-studio", None).await;
    let path = format!("/overlay/admin/devices/{}", device.id.to_canonical());

    for payload in [
        json!({}),
        json!({"label": ""}),
        json!({"label": "   "}),
        json!({"label": "x".repeat(41)}),
        json!({"label": 42}),
        json!({"sessionMode": "guest"}),
    ] {
        let (status, body) = call_app(
            &app,
            Method::PATCH,
            &path,
            Some(&admin_token),
            &[],
            Some(payload.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{payload} -> {body}");
        assert_eq!(body["error"], "invalid_payload");
    }

    let (status, body) = call_app(
        &app,
        Method::PATCH,
        &path,
        Some(&admin_token),
        &[],
        Some(json!({"label": "  Salon  "})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["label"], "Salon");
    assert_eq!(body["memberId"], Value::Null);
    assert_eq!(body["role"], "member");
    assert_eq!(body["isSelf"], false);

    // Un nom de 40 caractères pile est accepté.
    let (status, _) = call_app(
        &app,
        Method::PATCH,
        &path,
        Some(&admin_token),
        &[],
        Some(json!({"label": "y".repeat(40)})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call_app(
        &app,
        Method::PATCH,
        &path,
        Some(&admin_token),
        &[],
        Some(json!({"sessionMode": "invite_read_only"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sessionMode"], "invite_read_only");
    // Sans membre, le rôle suit le mode de session (repli du panneau).
    assert_eq!(body["role"], "guest");

    let stored = harness
        .db
        .overlay_clients()
        .get(device.id)
        .await
        .unwrap()
        .expect("appareil conservé");
    assert_eq!(stored.label, "y".repeat(40));
    assert_eq!(stored.session_mode, OverlaySessionMode::InviteReadOnly);

    let sent = hub.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, device.id.as_uuid());
    match &sent[0].1 {
        ServerMessage::SessionChanged(payload) => {
            assert_eq!(payload.session_mode, SessionMode::InviteReadOnly);
        }
        other => panic!("message inattendu : {other:?}"),
    }
}

#[tokio::test]
async fn device_guild_settings_are_validated_and_delivered() {
    let harness = Harness::new().await;
    let hub = Arc::new(RecordingAdminHub::default());
    let app = router_with_hub(&harness, hub.clone());
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let device = seed_device(&harness, &harness.guild_id, "Studio", "secret-studio", None).await;
    let path = format!(
        "/overlay/admin/devices/{}/guild-settings",
        device.id.to_canonical()
    );

    for payload in [
        json!({}),
        json!({"volume": -0.1}),
        json!({"volume": 1.1}),
        json!({"volume": "0.5"}),
        json!({"mediaMuted": "yes"}),
    ] {
        let (status, body) = call_app(
            &app,
            Method::PATCH,
            &path,
            Some(&admin_token),
            &[],
            Some(payload.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{payload} -> {body}");
        assert_eq!(body["error"], "invalid_payload");
    }

    // Aucune socket : accepté, mais non délivré.
    let (status, body) = call_app(
        &app,
        Method::PATCH,
        &path,
        Some(&admin_token),
        &[],
        Some(json!({"volume": 0.4, "mediaMuted": true, "paused": true})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["delivered"], false);

    let sent = hub.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, device.id.as_uuid());
    match &sent[0].1 {
        ServerMessage::DeviceSettings(payload) => {
            assert_eq!(payload.volume, Some(0.4));
            assert_eq!(payload.media_muted, Some(true));
            assert_eq!(payload.paused, Some(true));
        }
        other => panic!("message inattendu : {other:?}"),
    }

    // Appareil en ligne : les réglages sont poussés en direct.
    hub.set_connected(device.id.as_uuid(), true);
    hub.set_deliver(true);
    let (status, body) = call_app(
        &app,
        Method::PATCH,
        &path,
        Some(&admin_token),
        &[],
        Some(json!({"paused": false})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["delivered"], true);
}

#[tokio::test]
async fn a_device_code_issued_for_another_member_is_redeemed_by_that_member() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    let admin_token = token(&admin);
    let lea = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let lea_id = member_id_of(&lea).to_canonical();

    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/device-codes",
            Some(&admin_token),
            Some(json!({"memberId": lea_id})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["created"], true);
    let invite = &body["invite"];
    assert_eq!(invite["role"], "member");
    assert_eq!(invite["memberId"], lea["memberId"]);
    assert_eq!(invite["maxUses"], 1);
    assert_eq!(
        invite["expiresAt"],
        lc_domain::time::to_rfc3339_millis(fixed_now() + time::Duration::minutes(10))
    );

    let code = invite["code"].as_str().unwrap().to_owned();
    let second = harness.join(&code, "Nom ignoré").await;
    assert_eq!(second["memberId"], lea["memberId"]);
    assert_eq!(second["displayName"], "Léa");
    assert_eq!(second["role"], "member");
    assert_ne!(second["clientId"], lea["clientId"]);

    // Usage unique : le code ne se consomme plus.
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

    // Corps invalide puis membre inconnu.
    for payload in [
        json!({}),
        json!({"memberId": "  "}),
        json!({"memberId": 42}),
    ] {
        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/admin/device-codes",
                Some(&admin_token),
                Some(payload.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{payload} -> {body}");
        assert_eq!(body["error"], "invalid_payload");
    }
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/admin/device-codes",
            Some(&admin_token),
            Some(json!({"memberId": uuid::Uuid::new_v4()})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "member_not_found");
}
