//! Integration tests of `GET` / `PUT /admin/api/rights` and of the enforcement
//! of the configurable rights.
//!
//! They reuse the admin harness of [`super::tests`] (complete router, in-memory
//! database, authenticated session) and add the overlay/extension fixtures the
//! surfaces need.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::body::Body;
use axum::http::{header, Method, Request as HttpRequest, StatusCode};
use chrono::Utc;
use lc_domain::{
    GuildId, GuildMember, GuildMemberId, GuildRole, IngestClient, IngestClientId, OverlayClient,
    OverlaySessionMode, PairingCode,
};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

use super::tests::{guild_member, json_or_null, overlay_client, Harness};

const GUILD: &str = "123456789012345678";

fn guild_id() -> GuildId {
    GuildId::parse(GUILD).expect("identifiant de guilde valide")
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Guest overlay (member role `GUEST`, read-only session) with a known token.
async fn guest_overlay(harness: &Harness) -> (OverlayClient, String) {
    let member = guild_member(&guild_id(), GuildRole::Guest);
    harness
        .db
        .guild_members()
        .create(&member)
        .await
        .expect("membre invité");
    let token = "a".repeat(64);
    let mut client = overlay_client(&guild_id(), OverlaySessionMode::InviteReadOnly);
    client.token_hash = sha256_hex(&token);
    harness
        .db
        .overlay_clients()
        .create(&client)
        .await
        .expect("client overlay");
    harness
        .db
        .overlay_clients()
        .set_member(client.id, member.id)
        .await
        .expect("rattachement du client");
    (client, token)
}

/// Member (with a linked Discord account) owning an ingest client.
async fn ingest_fixture(harness: &Harness) -> (GuildMember, String) {
    let member = GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild_id(),
        display_name: "Alice".to_owned(),
        avatar: None,
        role: GuildRole::Member,
        created_at: Utc::now(),
        removed_at: None,
        discord_user_id: Some("1001".to_owned()),
        profile_custom: false,
        discord_synced_at: None,
    };
    harness
        .db
        .guild_members()
        .create(&member)
        .await
        .expect("membre d'ingestion");
    let token = "b".repeat(64);
    let client = IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild_id(),
        label: "Extension-Test".to_owned(),
        token_hash: crate::media::tokens::hash_ingest_token(&token),
        default_author_name: "Alice".to_owned(),
        default_author_image: None,
        created_by_discord_user_id: "1001".to_owned(),
        last_seen_at: None,
        created_at: Utc::now(),
        revoked_at: None,
    };
    harness
        .db
        .ingest_clients()
        .create(&client)
        .await
        .expect("client d'ingestion");
    (member, token)
}

fn bearer_request(method: Method, uri: &str, token: &str) -> HttpRequest<Body> {
    HttpRequest::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("requête de test valide")
}

fn ingest_request(token: &str, body: &Value) -> HttpRequest<Body> {
    HttpRequest::builder()
        .method(Method::POST)
        .uri("/ingest")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("requête de test valide")
}

async fn overlay_call(
    harness: &Harness,
    method: Method,
    uri: &str,
    token: &str,
) -> (StatusCode, Value) {
    let (status, _, bytes) = harness.send(bearer_request(method, uri, token)).await;
    (status, json_or_null(&bytes))
}

/// `PUT /admin/api/rights` with a single cell.
async fn put_cell(
    harness: &Harness,
    surface: &str,
    action: &str,
    role: &str,
    allowed: bool,
) -> (StatusCode, Value) {
    harness
        .call_json(
            Method::PUT,
            "/admin/api/rights",
            &json!({"cells": [{
                "surface": surface,
                "action": action,
                "role": role,
                "allowed": allowed,
            }]}),
        )
        .await
}

fn find_action<'a>(body: &'a Value, surface: &str, action: &str) -> &'a Value {
    body["surfaces"]
        .as_array()
        .expect("surfaces")
        .iter()
        .find(|entry| entry["id"] == surface)
        .expect("surface")
        .get("actions")
        .and_then(Value::as_array)
        .expect("actions")
        .iter()
        .find(|entry| entry["id"] == action)
        .expect("action")
}

fn find_surface<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["surfaces"]
        .as_array()
        .expect("surfaces")
        .iter()
        .find(|entry| entry["id"] == id)
        .expect("surface")
}

#[tokio::test]
async fn get_returns_the_complete_default_table() {
    let harness = Harness::new().await;
    let (status, body) = harness.call(Method::GET, "/admin/api/rights").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordAdminIsAdmin"], true);
    assert_eq!(body["surfaces"].as_array().map(Vec::len), Some(3));

    let overlay = find_surface(&body, "overlay");
    assert_eq!(overlay["columns"], json!(["guest", "member", "admin"]));
    assert_eq!(
        overlay["actions"].as_array().map(Vec::len),
        Some(13),
        "one entry per Permission variant"
    );
    let add_media = find_action(&body, "overlay", "addMedia");
    assert_eq!(
        add_media["cells"]["guest"],
        json!({"allowed": false, "fixed": false, "default": false})
    );
    assert_eq!(
        add_media["cells"]["member"],
        json!({"allowed": true, "fixed": false, "default": true})
    );
    assert_eq!(
        add_media["cells"]["admin"],
        json!({"allowed": true, "fixed": true, "default": true})
    );
    let receive = find_action(&body, "overlay", "receiveMedia");
    for role in ["guest", "member", "admin"] {
        assert_eq!(receive["cells"][role]["fixed"], true);
        assert_eq!(receive["cells"][role]["allowed"], true);
    }

    let extension = find_surface(&body, "extension");
    assert_eq!(extension["columns"], json!(["guest", "member", "admin"]));
    let pair = find_action(&body, "extension", "pair");
    assert_eq!(pair["cells"]["guest"]["allowed"], false);
    assert_eq!(pair["cells"]["member"]["allowed"], true);
    assert_eq!(pair["cells"]["admin"]["fixed"], true);

    let discord = find_surface(&body, "discord");
    assert_eq!(
        discord["columns"],
        json!(["none", "guest", "member", "admin"])
    );
    let msg = find_action(&body, "discord", "msg");
    assert_eq!(msg["cells"]["none"]["allowed"], false);
    assert_eq!(msg["cells"]["member"]["allowed"], true);
    let stop = find_action(&body, "discord", "stop");
    assert_eq!(stop["cells"]["member"]["allowed"], false);
    assert_eq!(stop["cells"]["admin"]["allowed"], true);
    let info = find_action(&body, "discord", "info");
    for role in ["none", "guest", "member", "admin"] {
        assert_eq!(info["cells"][role]["fixed"], true);
        assert_eq!(info["cells"][role]["allowed"], true);
    }
}

#[tokio::test]
async fn put_override_then_get_returns_it() {
    let harness = Harness::new().await;
    let (status, body) = put_cell(&harness, "overlay", "triggerPlayback", "guest", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        find_action(&body, "overlay", "triggerPlayback")["cells"]["guest"],
        json!({"allowed": true, "fixed": false, "default": false})
    );
    // The response is already the same payload as the GET.
    let (status, get) = harness.call(Method::GET, "/admin/api/rights").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(get, body);

    // The override is persisted.
    let stored: i64 = sqlx::query_scalar(
        "SELECT allowed FROM role_rights \
         WHERE surface = 'overlay' AND action = 'triggerPlayback' AND role = 'guest'",
    )
    .fetch_one(harness.db().pool())
    .await
    .expect("surcharge persistée");
    assert_eq!(stored, 1);
}

#[tokio::test]
async fn discord_admin_flag_round_trip() {
    let harness = Harness::new().await;
    let (status, body) = harness
        .call_json(
            Method::PUT,
            "/admin/api/rights",
            &json!({"cells": [], "discordAdminIsAdmin": false}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordAdminIsAdmin"], false);

    let (status, body) = harness.call(Method::GET, "/admin/api/rights").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["discordAdminIsAdmin"], false);

    // A PUT without the flag keeps the current value.
    let (status, body) = put_cell(&harness, "discord", "stop", "guest", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["discordAdminIsAdmin"], false);
}

#[tokio::test]
async fn fixed_cells_are_refused() {
    let harness = Harness::new().await;
    for (surface, action, role) in [
        ("overlay", "addMedia", "admin"),
        ("overlay", "receiveMedia", "guest"),
        ("extension", "send", "admin"),
        ("discord", "info", "none"),
    ] {
        let (status, body) = put_cell(&harness, surface, action, role, false).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{surface}.{action}.{role}");
        assert_eq!(body["error"], "fixed_right");
    }
    // Nothing was written.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM role_rights")
        .fetch_one(harness.db().pool())
        .await
        .expect("comptage");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn invalid_cells_are_refused() {
    let harness = Harness::new().await;
    let invalid = [
        json!({"surface": "inconnue", "action": "msg", "role": "member", "allowed": true}),
        json!({"surface": "overlay", "action": "inconnue", "role": "member", "allowed": true}),
        json!({"surface": "overlay", "action": "addMedia", "role": "owner", "allowed": true}),
        // Known ids, but the action does not belong to the surface.
        json!({"surface": "overlay", "action": "send", "role": "member", "allowed": true}),
        json!({"surface": "extension", "action": "msg", "role": "member", "allowed": true}),
        json!({"surface": "discord", "action": "pair", "role": "member", "allowed": true}),
        // Missing `allowed`.
        json!({"surface": "overlay", "action": "addMedia", "role": "guest"}),
    ];
    for cell in invalid {
        let (status, body) = harness
            .call_json(Method::PUT, "/admin/api/rights", &json!({"cells": [cell]}))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_right", "{body}");
    }

    let (status, body) = harness
        .call_json(Method::PUT, "/admin/api/rights", &json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_payload");
}

#[tokio::test]
async fn rights_routes_require_a_session() {
    let harness = Harness::new().await;
    for method in [Method::GET, Method::PUT] {
        let (status, _, bytes) = harness
            .send(harness.request_without_session(method.clone(), "/admin/api/rights"))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method}");
        assert_eq!(json_or_null(&bytes)["error"], "unauthorized");
    }
}

#[tokio::test]
async fn overlay_enforcement_follows_an_override() {
    let harness = Harness::new().await;
    let (_client, token) = guest_overlay(&harness).await;

    // A guest has neither `triggerPlayback` nor `moderateQueue` by default.
    let (status, body) = overlay_call(&harness, Method::GET, "/overlay/guild", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["permissions"]["play"], false);
    assert_eq!(body["permissions"]["moderateQueue"], false);

    let (status, body) = overlay_call(&harness, Method::DELETE, "/overlay/queue", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "forbidden");

    // Allowing `triggerPlayback` to guests is reported by `GET /overlay/guild`.
    let (status, body) = put_cell(&harness, "overlay", "triggerPlayback", "guest", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = overlay_call(&harness, Method::GET, "/overlay/guild", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["permissions"]["play"], true);

    // Allowing `moderateQueue` lets the previously refused request through.
    let (status, body) = put_cell(&harness, "overlay", "moderateQueue", "guest", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = overlay_call(&harness, Method::DELETE, "/overlay/queue", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["removedCount"], 0);

    // The extension surface is separate: pairing still needs `extension.pair`.
    let (status, body) = overlay_call(&harness, Method::POST, "/overlay/ingest-code", &token).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "forbidden");
    let (status, body) = put_cell(&harness, "extension", "pair", "guest", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = overlay_call(&harness, Method::POST, "/overlay/ingest-code", &token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["code"]
        .as_str()
        .is_some_and(|code| PairingCode::parse(code).is_ok()));
}

#[tokio::test]
async fn ingest_is_refused_when_extension_send_is_denied_or_the_member_is_removed() {
    let harness = Harness::new().await;
    let (member, token) = ingest_fixture(&harness).await;

    // Linked member: the default `extension.send` lets the submission through.
    let payload = json!({"text": "bonjour"});
    let (status, _, bytes) = harness.send(ingest_request(&token, &payload)).await;
    assert_eq!(status, StatusCode::CREATED, "{}", json_or_null(&bytes));

    let (status, body) = put_cell(&harness, "extension", "send", "member", false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, bytes) = harness.send(ingest_request(&token, &payload)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json_or_null(&bytes)["error"], "forbidden_role");

    // Re-allow the action, then remove the member: the client has no owner
    // any more, so the submission is refused as well.
    let (status, body) = put_cell(&harness, "extension", "send", "member", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    harness
        .db
        .guild_members()
        .remove(member.id, Utc::now())
        .await
        .expect("exclusion du membre");
    let (status, _, bytes) = harness.send(ingest_request(&token, &payload)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json_or_null(&bytes)["error"], "forbidden_role");
}

/// A `PUT` carries only the cells the operator changed: previous overrides are
/// kept.
#[tokio::test]
async fn put_merges_with_previous_overrides() {
    let harness = Harness::new().await;
    let (status, _) = put_cell(&harness, "overlay", "triggerPlayback", "guest", true).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = put_cell(&harness, "overlay", "addMedia", "guest", true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        find_action(&body, "overlay", "triggerPlayback")["cells"]["guest"]["allowed"],
        true
    );
    assert_eq!(
        find_action(&body, "overlay", "addMedia")["cells"]["guest"]["allowed"],
        true
    );
}
