//! `POST /overlay/profile/discord-link`: a member links their Discord account
//! with a `/pair-code` code.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use axum::http::{Method, StatusCode};
use lc_domain::{GuildId, GuildRole, PairingCode, PairingCodeEntry, PairingMode};
use serde_json::json;
use time::Duration;

use super::guild_tests::{fixed_now, token, Harness};

const URI: &str = "/overlay/profile/discord-link";

async fn pair_code(
    harness: &Harness,
    code: &str,
    guild_id: &GuildId,
    user: &str,
    mode: PairingMode,
    grants_admin: bool,
    author: Option<(&str, Option<&str>)>,
) {
    let entry = PairingCodeEntry {
        code: PairingCode::parse(code).unwrap(),
        guild_id: guild_id.clone(),
        mode,
        grants_admin,
        created_by_discord_user_id: user.to_owned(),
        author_name: author.map(|(name, _)| name.to_owned()),
        author_image: author.and_then(|(_, image)| image.map(str::to_owned)),
        expires_at: Some(fixed_now() + Duration::minutes(10)),
        used_at: None,
        created_at: fixed_now(),
    };
    harness.db.pairing_codes().create(&entry).await.unwrap();
}

#[tokio::test]
async fn a_member_links_their_discord_account_once() {
    let harness = Harness::new().await;
    let bob = token(
        &harness
            .join(&harness.server_invite(GuildRole::Member).await, "Bob")
            .await,
    );
    pair_code(
        &harness,
        "LINK2345",
        &harness.guild_id,
        "777",
        PairingMode::Normal,
        false,
        Some(("Pseudo Discord", Some("https://cdn.example/discord.png"))),
    )
    .await;

    let (status, body) = harness
        .call(
            Method::POST,
            URI,
            Some(&bob),
            Some(json!({"code": "link-2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["linked"], true);
    assert_eq!(body["promoted"], false);
    assert_eq!(body["member"]["discordLinked"], true);
    assert_eq!(body["member"]["role"], "member");
    // Proving the account hands the profile back to Discord: the pseudo and
    // avatar the code carries replace the typed ones.
    assert_eq!(body["member"]["displayName"], "Pseudo Discord");
    assert_eq!(body["member"]["avatar"], "https://cdn.example/discord.png");
    assert_eq!(body["member"]["profileSource"], "discord");
    let linked = harness
        .db
        .guild_members()
        .find_active_by_discord_user(&harness.guild_id, "777")
        .await
        .unwrap()
        .expect("membre lié");
    assert_eq!(linked.display_name, "Pseudo Discord");
    assert_eq!(
        linked.avatar.as_deref(),
        Some("https://cdn.example/discord.png")
    );
    assert!(!linked.profile_custom);
    assert_eq!(linked.discord_synced_at, Some(fixed_now()));

    // The code is used up.
    let (status, _) = harness
        .call(
            Method::POST,
            URI,
            Some(&bob),
            Some(json!({"code": "LINK2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_code_without_a_name_leaves_the_profile_to_the_next_connection() {
    let harness = Harness::new().await;
    let bob = token(
        &harness
            .join(&harness.server_invite(GuildRole::Member).await, "Bob")
            .await,
    );
    pair_code(
        &harness,
        "NONM2345",
        &harness.guild_id,
        "778",
        PairingMode::Normal,
        false,
        None,
    )
    .await;

    let (status, body) = harness
        .call(
            Method::POST,
            URI,
            Some(&bob),
            Some(json!({"code": "NONM2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["member"]["discordLinked"], true);
    // The account is proven, but the code carries no pseudo: the profile follows
    // Discord and is read at the next connection.
    assert_eq!(body["member"]["displayName"], "Bob");
    assert_eq!(body["member"]["profileSource"], "discord");
    let linked = harness
        .db
        .guild_members()
        .find_active_by_discord_user(&harness.guild_id, "778")
        .await
        .unwrap()
        .expect("membre lié");
    assert_eq!(linked.display_name, "Bob");
    assert!(!linked.profile_custom);
    assert_eq!(linked.discord_synced_at, None);
}

#[tokio::test]
async fn an_administrator_code_promotes_the_member() {
    let harness = Harness::new().await;
    let guest = token(
        &harness
            .join(&harness.server_invite(GuildRole::Guest).await, "Invité")
            .await,
    );
    pair_code(
        &harness,
        "ADMN2345",
        &harness.guild_id,
        "888",
        PairingMode::Normal,
        true,
        Some(("Pseudo Discord", None)),
    )
    .await;

    let (status, body) = harness
        .call(
            Method::POST,
            URI,
            Some(&guest),
            Some(json!({"code": "ADMN2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["promoted"], true);
    assert_eq!(body["member"]["role"], "admin");
}

#[tokio::test]
async fn refused_codes_are_not_consumed() {
    let harness = Harness::new().await;
    let alice = token(
        &harness
            .join(&harness.server_invite(GuildRole::Member).await, "Alice")
            .await,
    );
    let bob = token(
        &harness
            .join(&harness.server_invite(GuildRole::Member).await, "Bob")
            .await,
    );
    pair_code(
        &harness,
        "ALIC2345",
        &harness.guild_id,
        "999",
        PairingMode::Normal,
        false,
        Some(("Pseudo Discord", None)),
    )
    .await;
    let (status, _) = harness
        .call(
            Method::POST,
            URI,
            Some(&alice),
            Some(json!({"code": "ALIC2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Another member cannot take Alice's account.
    pair_code(
        &harness,
        "TAKE2345",
        &harness.guild_id,
        "999",
        PairingMode::Normal,
        false,
        Some(("Pseudo Discord", None)),
    )
    .await;
    let (status, body) = harness
        .call(
            Method::POST,
            URI,
            Some(&bob),
            Some(json!({"code": "TAKE2345"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "discord_already_linked");
    let stored = harness
        .db
        .pairing_codes()
        .get(&PairingCode::parse("TAKE2345").unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(stored.used_at.is_none(), "code refusé non consommé");

    // A read-only invitation, an extension code or another guild's code
    // never links an account.
    let other = GuildId::parse("999999999999999999").unwrap();
    pair_code(
        &harness,
        "RDNL2345",
        &harness.guild_id,
        "111",
        PairingMode::InviteReadOnly,
        false,
        Some(("Pseudo Discord", None)),
    )
    .await;
    pair_code(
        &harness,
        "EXTN2345",
        &harness.guild_id,
        "member:abc",
        PairingMode::Normal,
        false,
        Some(("Pseudo Discord", None)),
    )
    .await;
    pair_code(
        &harness,
        "OTHR2345",
        &other,
        "222",
        PairingMode::Normal,
        false,
        Some(("Pseudo Discord", None)),
    )
    .await;
    for code in ["RDNL2345", "EXTN2345", "OTHR2345", "NOPE2345"] {
        let (status, body) = harness
            .call(Method::POST, URI, Some(&bob), Some(json!({"code": code})))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{code}: {body}");
    }
    let (status, _) = harness
        .call(Method::POST, URI, Some(&bob), Some(json!({})))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
