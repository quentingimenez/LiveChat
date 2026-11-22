//! Guild bans: a banned member loses every device and code, and their Discord
//! account cannot come back until the ban is lifted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use lc_domain::{
    GuildBan, GuildBanId, GuildId, GuildInvite, GuildMemberId, GuildRole, IngestClient,
    IngestClientId, PairingCode, PairingCodeEntry, PairingMode,
};
use lc_protocol::close_code;
use serde_json::json;
use time::Duration;

use super::guild::{create_invite, NewInvite};
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

/// Insère une invitation utilisable directement en base, sans passer par
/// l'API : simule une invitation que la révocation du bannissement n'a pas
/// atteinte (course, révocation manquée).
async fn stored_invite(
    harness: &Harness,
    member_id: Option<GuildMemberId>,
    created_by_member_id: Option<GuildMemberId>,
) -> GuildInvite {
    create_invite(
        &harness.db,
        NewInvite {
            guild_id: harness.guild_id.clone(),
            role: GuildRole::Member,
            member_id,
            max_uses: Some(1),
            created_by_member_id,
            expires_at: None,
        },
        fixed_now(),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn a_device_code_for_a_banned_member_is_refused_without_consuming_it() {
    let harness = Harness::new().await;
    let admin_token = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await,
    );
    let lea = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/members/{}/ban", lea_id.to_canonical()),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The ban removed Léa and revoked her codes, but a device code that
    // still points at her must not let her back in with a second overlay.
    let invite = stored_invite(&harness, Some(lea_id), None).await;
    let members_before = harness
        .db
        .guild_members()
        .list_by_guild(&harness.guild_id)
        .await
        .unwrap()
        .len();
    let clients_before = harness
        .db
        .overlay_clients()
        .list_by_guild(&harness.guild_id)
        .await
        .unwrap()
        .len();

    let (status, body) = pair(&harness, invite.code.as_str()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");

    // Refused before consumption: the code is still spendable and nothing
    // was attached to the banned member.
    let stored = harness
        .db
        .guild_invites()
        .get(&invite.code)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.use_count, 0);
    assert!(stored.is_usable_at(fixed_now()));
    assert_eq!(
        harness
            .db
            .guild_members()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        members_before
    );
    assert_eq!(
        harness
            .db
            .overlay_clients()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        clients_before
    );
}

#[tokio::test]
async fn an_invite_issued_by_a_banned_member_is_refused_without_consuming_it() {
    let harness = Harness::new().await;
    let admin_token = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await,
    );
    let lea = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/members/{}/ban", lea_id.to_canonical()),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // An open invite she issued before being banned is revoked with her; if
    // one survives, a stranger must not join through her invitation.
    let invite = stored_invite(&harness, None, Some(lea_id)).await;
    let members_before = harness
        .db
        .guild_members()
        .list_by_guild(&harness.guild_id)
        .await
        .unwrap()
        .len();
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(json!({"code": invite.code.as_str(), "displayName": "Imposteur"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");

    let stored = harness
        .db
        .guild_invites()
        .get(&invite.code)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.use_count, 0);
    assert!(stored.is_usable_at(fixed_now()));
    assert_eq!(
        harness
            .db
            .guild_members()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        members_before
    );
}

#[tokio::test]
async fn an_invite_from_an_active_member_still_joins() {
    let harness = Harness::new().await;
    let admin_token = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await,
    );
    // The control only refuses banned members: a regular invite still works.
    let joined = harness
        .invite_and_join(&admin_token, "member", "Nina")
        .await;
    assert_eq!(joined["role"], "member");
}

#[tokio::test]
async fn a_device_code_for_a_removed_member_is_refused_without_consuming_it() {
    let harness = Harness::new().await;
    let admin_token = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await,
    );
    let lea = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();
    // Kicked, not banned: the member is removed and its invites revoked.
    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/members/{}", lea_id.to_canonical()),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A device code that survived the removal still names a removed member:
    // it is refused with the generic "invalid" status, and not spent.
    let invite = stored_invite(&harness, Some(lea_id), None).await;
    let members_before = harness
        .db
        .guild_members()
        .list_by_guild(&harness.guild_id)
        .await
        .unwrap()
        .len();
    let clients_before = harness
        .db
        .overlay_clients()
        .list_by_guild(&harness.guild_id)
        .await
        .unwrap()
        .len();
    let (status, body) = pair(&harness, invite.code.as_str()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "pairing_code_invalid_or_expired");

    let stored = harness
        .db
        .guild_invites()
        .get(&invite.code)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.use_count, 0);
    assert!(stored.is_usable_at(fixed_now()));
    assert_eq!(
        harness
            .db
            .guild_members()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        members_before
    );
    assert_eq!(
        harness
            .db
            .overlay_clients()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        clients_before
    );
}

#[tokio::test]
async fn an_imported_ban_without_member_id_matches_the_linked_discord_account() {
    let harness = Harness::new().await;
    let admin_token = token(
        &harness
            .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
            .await,
    );
    let lea = harness.invite_and_join(&admin_token, "member", "Léa").await;
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();
    assert!(harness
        .db
        .guild_members()
        .link_discord(lea_id, DISCORD_USER)
        .await
        .unwrap());

    // An imported ban carries the Discord account alone (no member row id):
    // only the account half of `is_member_banned` can match it.
    harness
        .db
        .guild_bans()
        .create(&GuildBan {
            id: GuildBanId::new_v4(),
            guild_id: harness.guild_id.clone(),
            member_id: None,
            discord_user_id: Some(DISCORD_USER.to_owned()),
            display_name: "Léa".to_owned(),
            avatar: None,
            banned_by: None,
            created_at: fixed_now(),
        })
        .await
        .unwrap();

    let invite = stored_invite(&harness, None, Some(lea_id)).await;
    let members_before = harness
        .db
        .guild_members()
        .list_by_guild(&harness.guild_id)
        .await
        .unwrap()
        .len();
    let (status, body) = harness
        .call(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(json!({"code": invite.code.as_str(), "displayName": "Imposteur"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");

    let stored = harness
        .db
        .guild_invites()
        .get(&invite.code)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.use_count, 0);
    assert_eq!(
        harness
            .db
            .guild_members()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        members_before
    );
}
