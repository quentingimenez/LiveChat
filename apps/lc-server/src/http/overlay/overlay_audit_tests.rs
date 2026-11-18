//! Journal of the administrative actions done from an overlay.
//!
//! The tests use the full router of [`super::guild_tests::Harness`]: overlay
//! admin routes, the board moderation paths and the owner console all read and
//! write the same `admin_audit` table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::{Method, StatusCode};
use lc_domain::{GuildMemberId, GuildRole};
use serde_json::{json, Value};

use crate::http::admin::{create_secret_session, SESSION_COOKIE_NAME};

use super::guild_admin_tests::{seed_asset, seed_item};
use super::guild_tests::{call_app, fixed_now, token, Harness};

/// `(action, actor_method, actor_name, method, path, status, guild_id)`, oldest
/// first.
type AuditRow = (
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    i64,
    Option<String>,
);

async fn clear_audit(harness: &Harness) {
    sqlx::query("DELETE FROM admin_audit")
        .execute(harness.db.pool())
        .await
        .unwrap();
}

async fn audit_rows(harness: &Harness) -> Vec<AuditRow> {
    sqlx::query_as(
        "SELECT action, actor_method, actor_name, method, path, status, guild_id \
           FROM admin_audit ORDER BY id",
    )
    .fetch_all(harness.db.pool())
    .await
    .unwrap()
}

async fn admin_session(harness: &Harness) -> (String, Value) {
    let body = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Alice")
        .await;
    (token(&body), body)
}

#[tokio::test]
async fn overlay_admin_mutations_are_journalled_once_with_the_member_actor() {
    let harness = Harness::new().await;
    let (admin_token, _admin) = admin_session(&harness).await;
    let member = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let member_id = member["memberId"].as_str().unwrap();
    let device_id = member["clientId"].as_str().unwrap();
    // Seeded directly (not through the admin route) so only the revoke below
    // writes a row.
    let invite_code = harness.server_invite(GuildRole::Member).await;

    clear_audit(&harness).await;

    let (status, body) = harness
        .call(
            Method::PATCH,
            &format!("/overlay/admin/devices/{device_id}"),
            Some(&admin_token),
            Some(json!({"label": "Salon de Léa"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/invites/{invite_code}/revoke"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"name": "Les Sardines"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/members/{member_id}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows = audit_rows(&harness).await;
    assert_eq!(rows.len(), 4, "{rows:?}");
    let actions: Vec<&str> = rows.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(
        actions,
        [
            "overlay.device_update",
            "overlay.invite_revoke",
            "overlay.settings",
            "overlay.member_remove",
        ]
    );
    for (action, actor_method, actor_name, method, path, status, guild) in &rows {
        assert_eq!(actor_method.as_deref(), Some("overlay"), "{action}");
        assert_eq!(actor_name.as_deref(), Some("Alice"), "{action}");
        assert_eq!(*status, 200, "{action}");
        assert_eq!(
            guild.as_deref(),
            Some(harness.guild_id.as_str()),
            "{action}"
        );
        assert!(method == "PATCH" || method == "POST" || method == "DELETE");
        assert!(path.starts_with("/overlay/admin/"), "{path}");
    }
}

#[tokio::test]
async fn refused_and_unknown_overlay_actions_leave_no_row() {
    let harness = Harness::new().await;
    let (admin_token, _admin) = admin_session(&harness).await;
    let member = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let member_token = token(&member);

    clear_audit(&harness).await;

    // 403: a plain member has no `manageSettings` right.
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&member_token),
            Some(json!({"name": "Hack"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // 404: the member id is well formed but unknown.
    let unknown = GuildMemberId::new_v4().to_canonical();
    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/members/{unknown}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    assert!(audit_rows(&harness).await.is_empty());
}

#[tokio::test]
async fn board_moderation_of_another_members_item_is_journalled_not_own_items() {
    let harness = Harness::new().await;
    let (admin_token, admin) = admin_session(&harness).await;
    let admin_id = GuildMemberId::parse(admin["memberId"].as_str().unwrap()).unwrap();
    let member = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let member_id = GuildMemberId::parse(member["memberId"].as_str().unwrap()).unwrap();

    let (foreign_asset, _) = seed_asset(&harness, "foreign.bin", 1024).await;
    let foreign = seed_item(&harness, &harness.guild_id, &foreign_asset, "À Léa").await;
    harness
        .db
        .media_board()
        .set_creator_member(foreign.id, member_id)
        .await
        .unwrap();
    let (own_asset, _) = seed_asset(&harness, "own.bin", 1024).await;
    let own = seed_item(&harness, &harness.guild_id, &own_asset, "À moi").await;
    harness
        .db
        .media_board()
        .set_creator_member(own.id, admin_id)
        .await
        .unwrap();

    clear_audit(&harness).await;

    // Elevated edit and delete of the other member's item.
    let foreign_uri = format!("/overlay/media-board/items/{}", foreign.id.to_canonical());
    let (status, body) = harness
        .call(
            Method::PATCH,
            &foreign_uri,
            Some(&admin_token),
            Some(json!({"title": "Modéré"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = harness
        .call(Method::DELETE, &foreign_uri, Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Same operations on the admin's own item: normal use, no journal row.
    let own_uri = format!("/overlay/media-board/items/{}", own.id.to_canonical());
    let (status, body) = harness
        .call(
            Method::PATCH,
            &own_uri,
            Some(&admin_token),
            Some(json!({"title": "Mon média"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = harness
        .call(Method::DELETE, &own_uri, Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows = audit_rows(&harness).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    let actions: Vec<&str> = rows.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(actions, ["overlay.media_update", "overlay.media_remove"]);
    for (action, actor_method, actor_name, method, path, status, guild) in &rows {
        assert_eq!(actor_method.as_deref(), Some("overlay"), "{action}");
        assert_eq!(actor_name.as_deref(), Some("Alice"), "{action}");
        assert_eq!(
            guild.as_deref(),
            Some(harness.guild_id.as_str()),
            "{action}"
        );
        assert_eq!(*status, 200, "{action}");
        assert_eq!(path, &foreign_uri);
        assert!(method == "PATCH" || method == "DELETE");
    }
}

#[tokio::test]
async fn both_journals_list_overlay_and_console_entries_of_the_guild_only() {
    let harness = Harness::new().await;
    let (admin_token, _admin) = admin_session(&harness).await;
    clear_audit(&harness).await;

    // One action from the overlay.
    let (status, body) = harness
        .call(
            Method::PATCH,
            "/overlay/admin/settings",
            Some(&admin_token),
            Some(json!({"name": "Les Sardines"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // One action from the owner console on the same guild, and one on another
    // guild (which the overlay journal must hide).
    let (session_token, _session) =
        create_secret_session(&harness.db, lc_domain::datetime_to_ms(fixed_now()))
            .await
            .unwrap();
    let cookie = format!("{SESSION_COOKIE_NAME}={session_token}");
    let stop_self = format!("/admin/api/guilds/{}/stop", harness.guild_id);
    let (status, _) = call_app(
        &harness.app,
        Method::POST,
        &stop_self,
        None,
        &[
            ("cookie", cookie.as_str()),
            ("origin", "http://127.0.0.1:8099"),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let other = harness.second_guild("Autre guilde").await;
    let stop_other = format!("/admin/api/guilds/{other}/stop");
    let (status, _) = call_app(
        &harness.app,
        Method::POST,
        &stop_other,
        None,
        &[
            ("cookie", cookie.as_str()),
            ("origin", "http://127.0.0.1:8099"),
        ],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Overlay journal: both entries of this guild, none of the other guild.
    let (status, body) = harness
        .call(
            Method::GET,
            "/overlay/admin/audit?limit=50",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    let actions: Vec<&str> = items
        .iter()
        .map(|item| item["action"].as_str().unwrap())
        .collect();
    assert!(actions.contains(&"overlay.settings"), "{actions:?}");
    assert!(actions.contains(&"guild.stop"), "{actions:?}");
    for item in items {
        assert_eq!(item["guildId"], harness.guild_id.as_str());
    }
    assert!(
        items.iter().all(|item| item["guildId"] != other.as_str()),
        "{items:?}"
    );

    // Owner journal: the overlay entry is listed too, with its actor kind.
    let (status, body) = call_app(
        &harness.app,
        Method::GET,
        "/admin/api/audit?limit=100",
        None,
        &[("cookie", cookie.as_str())],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let overlay_entry = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["action"] == "overlay.settings")
        .unwrap_or_else(|| panic!("overlay entry missing from the owner journal: {body}"));
    assert_eq!(overlay_entry["actor"]["method"], "overlay");
    assert_eq!(overlay_entry["actor"]["name"], "Alice");
    assert_eq!(overlay_entry["guildId"], harness.guild_id.as_str());
}
