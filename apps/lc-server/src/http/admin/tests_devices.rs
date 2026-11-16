//! Tests of the remote device-control admin routes
//! (`PATCH /admin/api/overlay-clients/{id}` and its `guild-settings` route).
//!
//! They reuse the integration harness of [`super::tests`] (full router, SQLite
//! in memory, real admin session) and the recording fake hub.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use lc_domain::{GuildId, OverlayClientId, OverlaySessionMode};
use lc_protocol::{DeviceSettingsStatePayload, ServerMessage, SessionMode};
use serde_json::{json, Value};

use super::tests::{guild, overlay_client, Harness, RecordingAdminHub};

/// Creates the guild and one active overlay client in the harness database.
async fn seed_overlay(harness: &Harness) -> OverlayClientId {
    let guild_id = GuildId::parse("42").expect("guild id");
    harness
        .db()
        .guilds()
        .upsert(&guild("42"))
        .await
        .expect("guilde");
    let client = overlay_client(&guild_id, OverlaySessionMode::Normal);
    harness
        .db()
        .overlay_clients()
        .create(&client)
        .await
        .expect("client overlay");
    client.id
}

fn overlay_path(client_id: OverlayClientId) -> String {
    format!("/admin/api/overlay-clients/{}", client_id.to_canonical())
}

#[tokio::test]
async fn patch_client_persists_label_and_mode_and_notifies_the_socket() {
    let hub = Arc::new(RecordingAdminHub::default());
    let harness = Harness::with_hub(hub.clone()).await;
    let client_id = seed_overlay(&harness).await;

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &overlay_path(client_id),
            &json!({ "label": "  Studio  ", "sessionMode": "invite_read_only" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], client_id.to_canonical());
    assert_eq!(body["label"], "Studio");
    assert_eq!(body["sessionMode"], "invite_read_only");
    assert!(body
        .as_object()
        .is_some_and(|entry| entry.contains_key("overlayVersion")));
    assert!(body
        .as_object()
        .is_some_and(|entry| entry.contains_key("platform")));
    assert!(body
        .as_object()
        .is_some_and(|entry| entry.contains_key("guildSettings")));

    let stored = harness
        .db()
        .overlay_clients()
        .get(client_id)
        .await
        .expect("lecture client")
        .expect("client présent");
    assert_eq!(stored.label, "Studio");
    assert_eq!(stored.session_mode, OverlaySessionMode::InviteReadOnly);

    let sent = hub.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, client_id.as_uuid());
    match &sent[0].1 {
        ServerMessage::SessionChanged(payload) => {
            assert_eq!(payload.session_mode, SessionMode::InviteReadOnly);
        }
        other => panic!("message inattendu : {other:?}"),
    }
    assert_eq!(
        hub.session_modes().get(&client_id.as_uuid()),
        Some(&SessionMode::InviteReadOnly),
        "le mode doit aussi être appliqué à la session vivante"
    );
}

#[tokio::test]
async fn patch_client_rejects_invalid_bodies_and_unknown_clients() {
    let hub = Arc::new(RecordingAdminHub::default());
    let harness = Harness::with_hub(hub).await;
    let client_id = seed_overlay(&harness).await;
    let path = overlay_path(client_id);

    for body in [
        json!({}),
        json!({ "label": "" }),
        json!({ "label": "   " }),
        json!({ "label": "x".repeat(65) }),
        json!({ "sessionMode": "guest" }),
        json!({ "label": 42 }),
    ] {
        let (status, payload) = harness.call_json(Method::PATCH, &path, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {payload}");
        assert_eq!(payload["error"], "invalid_payload", "{body}");
    }

    // A label of exactly 64 characters is accepted.
    let (status, _) = harness
        .call_json(Method::PATCH, &path, &json!({ "label": "x".repeat(64) }))
        .await;
    assert_eq!(status, StatusCode::OK);

    let unknown = format!("/admin/api/overlay-clients/{}", uuid::Uuid::new_v4());
    let (status, body) = harness
        .call_json(Method::PATCH, &unknown, &json!({ "label": "Studio" }))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "overlay_client_not_found");

    let invalid = "/admin/api/overlay-clients/not-a-uuid";
    let (status, _) = harness
        .call_json(Method::PATCH, invalid, &json!({ "label": "Studio" }))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn guild_settings_are_delivered_when_a_socket_accepts_them() {
    let hub = Arc::new(RecordingAdminHub::default());
    hub.set_deliver(true);
    let harness = Harness::with_hub(hub.clone()).await;
    let client_id = seed_overlay(&harness).await;

    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("{}/guild-settings", overlay_path(client_id)),
            &json!({ "volume": 0.25, "mediaMuted": true, "paused": false }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body, json!({ "delivered": true }));

    let sent = hub.sent();
    assert_eq!(sent.len(), 1);
    match &sent[0].1 {
        ServerMessage::DeviceSettings(payload) => {
            assert_eq!(payload.volume, Some(0.25));
            assert_eq!(payload.media_muted, Some(true));
            assert_eq!(payload.paused, Some(false));
        }
        other => panic!("message inattendu : {other:?}"),
    }

    hub.set_deliver(false);
    let (status, body) = harness
        .call_json(
            Method::PATCH,
            &format!("{}/guild-settings", overlay_path(client_id)),
            &json!({ "paused": true }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body, json!({ "delivered": false }));
}

#[tokio::test]
async fn guild_settings_validate_the_body_and_the_client() {
    let hub = Arc::new(RecordingAdminHub::default());
    let harness = Harness::with_hub(hub).await;
    let client_id = seed_overlay(&harness).await;
    let path = format!("{}/guild-settings", overlay_path(client_id));

    for body in [
        json!({}),
        json!({ "volume": -0.1 }),
        json!({ "volume": 1.1 }),
        json!({ "volume": "0.5" }),
        json!({ "mediaMuted": "yes" }),
    ] {
        let (status, payload) = harness.call_json(Method::PATCH, &path, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {payload}");
        assert_eq!(payload["error"], "invalid_payload", "{body}");
    }

    let unknown = format!(
        "/admin/api/overlay-clients/{}/guild-settings",
        uuid::Uuid::new_v4()
    );
    let (status, body) = harness
        .call_json(Method::PATCH, &unknown, &json!({ "paused": true }))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "overlay_client_not_found");
}

#[tokio::test]
async fn device_routes_require_an_admin_session() {
    let harness = Harness::new().await;
    let client_id = seed_overlay(&harness).await;

    for (path, body) in [
        (overlay_path(client_id), json!({ "label": "Studio" })),
        (
            format!("{}/guild-settings", overlay_path(client_id)),
            json!({ "paused": true }),
        ),
    ] {
        let request = harness.request_json_without_session(Method::PATCH, &path, &body);
        let (status, _, _) = harness.send(request).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[tokio::test]
async fn overview_exposes_versions_platforms_and_last_device_state() {
    let hub = Arc::new(RecordingAdminHub::default());
    let harness = Harness::with_hub(hub.clone()).await;
    let client_id = seed_overlay(&harness).await;
    harness
        .db()
        .overlay_clients()
        .update_client_info(
            client_id,
            Some("3.4.1"),
            Some(lc_protocol::Platform::Windows.as_str()),
        )
        .await
        .expect("informations client");
    hub.set_connected(true);
    hub.set_device_settings(
        client_id.as_uuid(),
        DeviceSettingsStatePayload {
            volume: 0.5,
            media_muted: true,
            paused: false,
        },
    );

    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::OK);
    let clients = body["guilds"][0]["overlays"]["clients"]
        .as_array()
        .expect("clients");
    assert_eq!(clients.len(), 1);
    let client = &clients[0];
    assert_eq!(client["id"], client_id.to_canonical());
    assert_eq!(client["overlayVersion"], "3.4.1");
    assert_eq!(client["platform"], "windows");
    assert_eq!(client["guildSettings"]["volume"], 0.5);
    assert_eq!(client["guildSettings"]["mediaMuted"], true);
    assert_eq!(client["guildSettings"]["paused"], false);
    assert_eq!(client["connected"], true);
}

#[tokio::test]
async fn overview_reports_null_guild_settings_without_a_report() {
    let hub = Arc::new(RecordingAdminHub::default());
    let harness = Harness::with_hub(hub).await;
    let _client_id = seed_overlay(&harness).await;

    let (status, body) = harness.call(Method::GET, "/admin/api/overview").await;
    assert_eq!(status, StatusCode::OK);
    let client = &body["guilds"][0]["overlays"]["clients"][0];
    assert_eq!(client["guildSettings"], Value::Null);
    assert_eq!(client["overlayVersion"], Value::Null);
    assert_eq!(client["platform"], Value::Null);
}
