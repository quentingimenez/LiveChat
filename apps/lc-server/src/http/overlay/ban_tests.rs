//! Guild bans: a banned member loses every device and code, and their Discord
//! account cannot come back until the ban is lifted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use axum::http::{Method, StatusCode};
use lc_domain::{
    GuildBan, GuildBanId, GuildId, GuildInvite, GuildMemberId, GuildRole, IngestClient,
    IngestClientId, OverlayClientId, PairingCode, PairingCodeEntry, PairingMode,
};
use lc_protocol::close_code;
use serde_json::json;
use time::Duration;

use super::device_id::device_hash;
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

/// Pairs an open invitation while presenting a stable device identifier, the
/// way the desktop overlay does.
async fn join_with_device(
    harness: &Harness,
    code: &str,
    name: &str,
    device_id: &str,
) -> (StatusCode, serde_json::Value) {
    harness
        .call(
            Method::POST,
            "/overlay/pair/consume",
            None,
            Some(json!({"code": code, "displayName": name, "deviceId": device_id})),
        )
        .await
}

/// Bans an active member and returns the ban id, as the admin panel sees it.
async fn ban_and_get_id(harness: &Harness, admin_token: &str, member_id: &str) -> String {
    let (status, body) = harness
        .call(
            Method::POST,
            &format!("/overlay/admin/members/{member_id}/ban"),
            Some(admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = harness
        .call(Method::GET, "/overlay/admin/bans", Some(admin_token), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"][0]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_ban_records_the_devices_of_the_member_and_unban_lifts_them() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let code = harness.server_invite(GuildRole::Member).await;
    let (status, lea) = join_with_device(&harness, &code, "Léa", "lea-machine-guid").await;
    assert_eq!(status, StatusCode::OK, "{lea}");
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();
    let fingerprint = device_hash(&harness.guild_id, "lea-machine-guid");

    // The pairing stored the fingerprint on the overlay client, not the raw id.
    let clients = harness
        .db
        .overlay_clients()
        .list_active_by_member(lea_id)
        .await
        .unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(
        clients[0].device_hash.as_deref(),
        Some(fingerprint.as_str())
    );

    // The devices screen never leaks the fingerprint.
    let (status, devices) = harness
        .call(
            Method::GET,
            "/overlay/admin/devices",
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{devices}");
    assert!(
        !devices.to_string().contains("deviceHash"),
        "deviceHash exposé : {devices}"
    );
    assert!(
        !devices.to_string().contains(fingerprint.as_str()),
        "empreinte exposée : {devices}"
    );

    let ban_id = ban_and_get_id(&harness, &admin_token, lea_id.to_canonical().as_str()).await;
    assert!(harness
        .db
        .guild_ban_devices()
        .is_device_banned(&harness.guild_id, &fingerprint)
        .await
        .unwrap());
    assert_eq!(
        harness
            .db
            .guild_ban_devices()
            .count_by_ban(ban_id.parse().unwrap())
            .await
            .unwrap(),
        1
    );

    // Lifting the ban drops its device fingerprints: the same machine could
    // join again, and a stale ban row must not keep refusing it.
    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/bans/{ban_id}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!harness
        .db
        .guild_ban_devices()
        .is_device_banned(&harness.guild_id, &fingerprint)
        .await
        .unwrap());
}

#[tokio::test]
async fn a_banned_device_cannot_redeem_an_invite_or_a_pairing_code() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);

    let code = harness.server_invite(GuildRole::Member).await;
    let (status, lea) = join_with_device(&harness, &code, "Léa", "lea-machine-guid").await;
    assert_eq!(status, StatusCode::OK, "{lea}");
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();
    let code = harness.server_invite(GuildRole::Member).await;
    join_with_device(&harness, &code, "Bob", "bob-machine-guid").await;

    let ban_id = ban_and_get_id(&harness, &admin_token, lea_id.to_canonical().as_str()).await;

    // Léa's machine is refused on an open invitation before the invitation is
    // spent, while another machine still joins normally.
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
    let invite = stored_invite(&harness, None, None).await;
    let (status, body) =
        join_with_device(&harness, invite.code.as_str(), "Léa", "lea-machine-guid").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");
    assert!(
        body.get("clientToken").is_none(),
        "un refus ne doit pas émettre de jeton : {body}"
    );
    assert_eq!(
        harness
            .db
            .guild_members()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        members_before,
        "un refus ne doit créer aucun membre"
    );
    assert_eq!(
        harness
            .db
            .overlay_clients()
            .list_by_guild(&harness.guild_id)
            .await
            .unwrap()
            .len(),
        clients_before,
        "un refus ne doit créer aucun client overlay"
    );
    let stored = harness
        .db
        .guild_invites()
        .get(&invite.code)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.use_count, 0);
    assert!(stored.is_usable_at(fixed_now()));

    let (status, body) =
        join_with_device(&harness, invite.code.as_str(), "Bob", "bob-machine-guid").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The same fingerprint is also refused on a Discord pairing code, without
    // consuming the code.
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
    discord_code(&harness, "DEVI2345", &harness.guild_id, DISCORD_USER).await;
    let (status, body) = join_with_device(&harness, "DEVI2345", "Léa", "lea-machine-guid").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "member_banned");
    assert!(
        body.get("clientToken").is_none(),
        "un refus ne doit pas émettre de jeton : {body}"
    );
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
    let kept = harness
        .db
        .pairing_codes()
        .get(&PairingCode::parse("DEVI2345").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(kept.used_at.is_none());

    // Lifting the ban removes the fingerprint and the machine can join again.
    let (status, body) = harness
        .call(
            Method::DELETE,
            &format!("/overlay/admin/bans/{ban_id}"),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let invite = stored_invite(&harness, None, None).await;
    let (status, body) =
        join_with_device(&harness, invite.code.as_str(), "Léa", "lea-machine-guid").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_invalid_device_id_is_rejected_before_the_invite_is_used() {
    let harness = Harness::new().await;
    let invite = stored_invite(&harness, None, None).await;

    for device_id in ["bad\u{7}id".to_owned(), "x".repeat(257)] {
        let (status, body) = harness
            .call(
                Method::POST,
                "/overlay/pair/consume",
                None,
                Some(json!({
                    "code": invite.code.as_str(),
                    "displayName": "Léa",
                    "deviceId": device_id,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "invalid_payload");
    }

    // Refusals happened before consumption: the invitation is still usable.
    let stored = harness
        .db
        .guild_invites()
        .get(&invite.code)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.use_count, 0);
    assert!(stored.is_usable_at(fixed_now()));
    let (status, body) =
        join_with_device(&harness, invite.code.as_str(), "Léa", "lea-machine-guid").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_ban_records_the_device_of_a_client_revoked_just_before() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let code = harness.server_invite(GuildRole::Member).await;
    let (status, lea) = join_with_device(&harness, &code, "Léa", "lea-revoked-guid").await;
    assert_eq!(status, StatusCode::OK, "{lea}");
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();
    let client_id: OverlayClientId = lea["clientId"].as_str().unwrap().parse().unwrap();

    // A device revoked just before the ban must still be retained: it was the
    // member's machine even though it is no longer active.
    assert!(harness
        .db
        .overlay_clients()
        .revoke(client_id, fixed_now())
        .await
        .unwrap());

    ban_and_get_id(&harness, &admin_token, lea_id.to_canonical().as_str()).await;

    let fingerprint = device_hash(&harness.guild_id, "lea-revoked-guid");
    assert!(
        harness
            .db
            .guild_ban_devices()
            .is_device_banned(&harness.guild_id, &fingerprint)
            .await
            .unwrap(),
        "l'appareil d'un client révoqué juste avant le ban doit être retenu"
    );
}

#[tokio::test]
async fn a_second_ban_keeps_the_existing_ban_and_attaches_its_devices() {
    let harness = Harness::new().await;
    let admin = harness
        .join(&harness.server_invite(GuildRole::Admin).await, "Quentin")
        .await;
    let admin_token = token(&admin);
    let code = harness.server_invite(GuildRole::Member).await;
    let (status, lea) = join_with_device(&harness, &code, "Léa", "lea-conflict-guid").await;
    assert_eq!(status, StatusCode::OK, "{lea}");
    let lea_id: GuildMemberId = lea["memberId"].as_str().unwrap().parse().unwrap();

    // Un ban importé couvre déjà le compte Discord lié, sans `member_id` : le
    // ban du membre entre en conflit et doit réutiliser cette ligne, pas
    // renvoyer l'identifiant d'un ban jamais inséré.
    harness
        .db
        .guild_members()
        .link_discord(lea_id, DISCORD_USER)
        .await
        .unwrap();
    let existing = GuildBan {
        id: GuildBanId::new_v4(),
        guild_id: harness.guild_id.clone(),
        member_id: None,
        discord_user_id: Some(DISCORD_USER.to_owned()),
        display_name: "Exilé".to_owned(),
        avatar: None,
        banned_by: None,
        created_at: fixed_now(),
    };
    harness.db.guild_bans().create(&existing).await.unwrap();

    let (status, body) = harness
        .call(
            Method::POST,
            &format!(
                "/overlay/admin/members/{}/ban",
                lea_id.to_canonical().as_str()
            ),
            Some(&admin_token),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ban"]["id"], existing.id.to_canonical());
    assert_eq!(
        harness
            .db
            .guild_ban_devices()
            .count_by_ban(existing.id)
            .await
            .unwrap(),
        1
    );
    let fingerprint = device_hash(&harness.guild_id, "lea-conflict-guid");
    assert!(harness
        .db
        .guild_ban_devices()
        .is_device_banned(&harness.guild_id, &fingerprint)
        .await
        .unwrap());
}
