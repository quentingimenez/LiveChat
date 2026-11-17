//! Guild bans: a banned member loses every device and code, and their Discord
//! account cannot come back until the ban is lifted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use chrono::Duration;
use lc_domain::{
    GuildId, GuildMemberId, GuildRole, IngestClient, IngestClientId, PairingCode, PairingCodeEntry,
    PairingMode,
};
use lc_protocol::close_code;
use serde_json::json;

use super::guild_tests::{call_app, fixed_now, token, Harness, RecordingAdminHub};
use crate::http::admin::hub::AdminHub;

const DISCORD_USER: &str = "777";

/// A `/pair-code` issued from Discord by `user`.
async fn discord_code(harness: &Harness, code: &str, guild_id: &GuildId, user: &str) {
    let entry = PairingCodeEntry {
        code: PairingCode::parse(code).unwrap(),
        guild_id: guild_id.clone(),
        mode: PairingMode::Normal,
        grants_admin: false,
        created_by_discord_user_id: user.to_owned(),
        author_name: Some("Bob Discord".to_owned()),
        author_image: None,
        expires_at: Some(fixed_now() + Duration::minutes(10)),
        used_at: None,
        created_at: fixed_now(),
    };
    harness.db.pairing_codes().create(&entry).await.unwrap();
}

fn ingest_client(guild_id: &GuildId, author: &str, hash: &str) -> IngestClient {
    IngestClient {
        id: IngestClientId::new_v4(),
        guild_id: guild_id.clone(),
        label: format!("Extension {hash}"),
        token_hash: hash.to_owned(),
        default_author_name: "Bob".to_owned(),
        default_author_image: None,
        created_by_discord_user_id: author.to_owned(),
        last_seen_at: None,
        created_at: fixed_now(),
        revoked_at: None,
    }
}

async fn pair(harness: &Harness, code: &str) -> (StatusCode, serde_json::Value) {
    harness
        .call(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(json!({"code": code, "deviceName": "PC"})),
        )
        .await
}

#[tokio::test]
async fn a_ban_revokes_everything_and_keeps_the_discord_account_out() {
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
    let admin_id = admin["memberId"].as_str().unwrap().to_owned();

    // Bob pairs with his own `/pair-code`: his member is linked to Discord.
    discord_code(&harness, "BOBA2345", &harness.guild_id, DISCORD_USER).await;
    let (status, bob) = pair(&harness, "BOBA2345").await;
    assert_eq!(status, StatusCode::OK, "{bob}");
    let bob_token = token(&bob);
    let bob_id = bob["memberId"].as_str().unwrap().to_owned();
    let bob_client = uuid::Uuid::parse_str(bob["clientId"].as_str().unwrap()).unwrap();
    let bob_member_id: GuildMemberId = bob_id.parse().unwrap();

    // Two extensions (one paired from Discord, one from the overlay) and a
    // code Bob kept for later.
    let from_discord = ingest_client(&harness.guild_id, DISCORD_USER, "hash-discord");
    let from_overlay = ingest_client(
        &harness.guild_id,
        &format!("member:{bob_id}"),
        "hash-overlay",
    );
    let someone_else = ingest_client(&harness.guild_id, "999", "hash-else");
    for client in [&from_discord, &from_overlay, &someone_else] {
        harness.db.ingest_clients().create(client).await.unwrap();
    }
    discord_code(&harness, "KEPT2345", &harness.guild_id, DISCORD_USER).await;

    // An admin cannot ban themselves.
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/members/{admin_id}/ban"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "cannot_ban_self");

    // Bob cannot ban (no `manageMembers`).
    let (status, _) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/members/{admin_id}/ban"),
            Some(&bob_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = call_app(
        &app,
        Method::POST,
        &format!("/overlay/admin/members/{bob_id}/ban"),
        Some(&admin_token),
        &[],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["banned"], true);
    assert_eq!(body["ban"]["discordUserId"], DISCORD_USER);
    assert_eq!(body["ban"]["bannedBy"], "Quentin");
    assert_eq!(
        hub.closes(),
        vec![(bob_client, close_code::REVOKED, "member_banned".to_owned())]
    );

    // Bob's overlay token, extensions and kept code are dead.
    let (status, _) = harness
        .call(Method::GET, "/overlay/guild", Some(&bob_token), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let active = |id| {
        let db = harness.db.clone();
        async move {
            db.ingest_clients()
                .get(id)
                .await
                .unwrap()
                .unwrap()
                .is_active()
        }
    };
    assert!(!active(from_discord.id).await);
    assert!(!active(from_overlay.id).await);
    assert!(active(someone_else.id).await);
    let (status, _) = pair(&harness, "KEPT2345").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(harness
        .db
        .guild_members()
        .get(bob_member_id)
        .await
        .unwrap()
        .is_some_and(|member| !member.is_active()));

    // A new `/pair-code` is refused without being used up, for the overlay
    // and for the extension.
    discord_code(&harness, "BACK2345", &harness.guild_id, DISCORD_USER).await;
    let (status, body) = pair(&harness, "BACK2345").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");
    let (status, body) = harness
        .call(
            Method::POST,
            "/ingest/pair/consume",
            None,
            Some(json!({"code": "BACK2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");

    // Nor can another member link the banned account to themselves.
    let carl = token(
        &harness
            .join(&harness.server_invite(GuildRole::Member).await, "Carl")
            .await,
    );
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/profile/discord-link",
            Some(&carl),
            Some(json!({"code": "BACK2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");

    // The ban is listed, then lifted: the same code now works.
    let (status, body) = harness
        .call(Method::GET, "/overlay/admin/bans", Some(&admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 1);
    assert_eq!(body["items"][0]["displayName"], "Bob Discord");
    assert_eq!(body["items"][0]["memberId"], bob_id.as_str());
    let ban_id = body["items"][0]["id"].as_str().unwrap().to_owned();

    let (status, _) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/bans/{ban_id}"),
            Some(&carl),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/bans/{ban_id}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["unbanned"], true);
    let (status, _) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/bans/{ban_id}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = pair(&harness, "BACK2345").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Back as a new member: the old one stays removed.
    assert_ne!(body["memberId"], bob_id.as_str());
}

#[tokio::test]
async fn a_ban_is_scoped_to_its_guild() {
    let harness = Harness::new().await;
    let admin_token = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await,
    );
    discord_code(&harness, "BOBA2345", &harness.guild_id, DISCORD_USER).await;
    let (_, bob) = pair(&harness, "BOBA2345").await;
    let bob_id = bob["memberId"].as_str().unwrap().to_owned();
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/members/{bob_id}/ban"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Another guild knows nothing of this ban.
    let other = harness.second_guild("Les Thons").await;
    discord_code(&harness, "THON2345", &other, DISCORD_USER).await;
    let (status, body) = pair(&harness, "THON2345").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_member_without_discord_is_removed_by_a_ban() {
    let harness = Harness::new().await;
    let admin_token = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await,
    );
    let max = harness.invite_and_join(&admin_token, "member", "Max").await;
    let max_id = max["memberId"].as_str().unwrap().to_owned();
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/members/{max_id}/ban"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ban"]["discordUserId"], serde_json::Value::Null);
    let (status, _) = harness
        .call(Method::GET, "/overlay/guild", Some(&token(&max)), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
