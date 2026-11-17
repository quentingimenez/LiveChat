//! Migration 0008: members linked to their Discord account, and the profile
//! sync rules of `GuildMembersRepository`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_domain::entities::GuildMember;
use lc_domain::enums::GuildRole;
use lc_domain::ids::{GuildId, GuildMemberId};
use sqlx::sqlite::SqlitePoolOptions;
use time::Duration;

const MIGRATIONS_BEFORE_0008: [&str; 7] = [
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_playback_single_playing.sql"),
    include_str!("../migrations/0003_guild_members.sql"),
    include_str!("../migrations/0004_discord_link_codes.sql"),
    include_str!("../migrations/0005_media_board.sql"),
    include_str!("../migrations/0006_guild_discord_icon.sql"),
    include_str!("../migrations/0007_pairing_code_admin.sql"),
];

fn linked_member(guild_id: &GuildId, name: &str, discord_user_id: Option<&str>) -> GuildMember {
    GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild_id.clone(),
        display_name: name.to_owned(),
        avatar: None,
        role: GuildRole::Member,
        created_at: common::now(),
        removed_at: None,
        discord_user_id: discord_user_id.map(str::to_owned),
        profile_custom: discord_user_id.is_none(),
        discord_synced_at: None,
    }
}

#[tokio::test]
async fn migration_0008_links_existing_members_and_keeps_their_profile() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for migration in MIGRATIONS_BEFORE_0008 {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO guilds (id, name) VALUES ('111', 'Serveur')")
        .execute(&pool)
        .await
        .unwrap();
    // (member, created_at, removed_at, overlay session mode, code author)
    let rows = [
        ("m-own", 1_i64, None, "NORMAL", Some("4242")),
        // Same Discord account, newer member: loses the link.
        ("m-dup", 2, None, "NORMAL", Some("4242")),
        // Read-only invitation handed out by someone else: never linked.
        ("m-guest", 3, None, "INVITE_READ_ONLY", Some("5151")),
        // Browser-extension code author: not a Discord account.
        ("m-ext", 4, None, "NORMAL", Some("member:abc")),
        // Board invitation: no Discord author.
        ("m-board", 5, None, "NORMAL", None),
        // Removed member: linked, but outside the uniqueness rule.
        ("m-gone", 0, Some(9_i64), "NORMAL", Some("4242")),
    ];
    for (id, created_at, removed_at, mode, author) in rows {
        sqlx::query(
            "INSERT INTO guild_members (id, guild_id, display_name, role, created_at, removed_at) \
             VALUES (?, '111', ?, 'MEMBER', ?, ?)",
        )
        .bind(id)
        .bind(format!("Nom {id}"))
        .bind(created_at)
        .bind(removed_at)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO overlay_clients ( \
                id, guild_id, label, token_hash, session_mode, created_by_discord_user_id, \
                created_at, member_id \
             ) VALUES (?, '111', 'PC', ?, ?, ?, 1, ?)",
        )
        .bind(format!("c-{id}"))
        .bind(format!("hash-{id}"))
        .bind(mode)
        .bind(author)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }

    sqlx::raw_sql(include_str!(
        "../migrations/0008_member_discord_profile.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let linked: Vec<(String, Option<String>, i64)> =
        sqlx::query_as("SELECT id, discord_user_id, profile_custom FROM guild_members ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let expected = [
        ("m-board", None),
        ("m-dup", None),
        ("m-ext", None),
        ("m-gone", Some("4242")),
        ("m-guest", None),
        ("m-own", Some("4242")),
    ];
    assert_eq!(linked.len(), expected.len());
    for ((id, discord, custom), (expected_id, expected_discord)) in linked.iter().zip(expected) {
        assert_eq!(id, expected_id);
        assert_eq!(discord.as_deref(), expected_discord, "{id}");
        assert_eq!(*custom, 1, "{id}: existing profiles are kept as chosen");
    }
}

#[tokio::test]
async fn one_active_member_per_discord_account() {
    let db = common::memory_db().await;
    let guild = common::guild("111");
    let first = linked_member(&guild, "Quentin", Some("4242"));
    db.guild_members().create(&first).await.unwrap();
    let error = db
        .guild_members()
        .create(&linked_member(&guild, "Doublon", Some("4242")))
        .await
        .unwrap_err();
    assert!(error.is_unique_violation(), "{error}");

    // Another guild may link the same account.
    let other = common::guild("222");
    db.guild_members()
        .create(&linked_member(&other, "Quentin", Some("4242")))
        .await
        .unwrap();

    let found = db
        .guild_members()
        .find_active_by_discord_user(&guild, "4242")
        .await
        .unwrap()
        .expect("linked member");
    assert_eq!(found.id, first.id);
    assert!(found.follows_discord());

    // Once removed, the account can be linked to a new member.
    db.guild_members()
        .remove(first.id, common::now())
        .await
        .unwrap();
    assert!(db
        .guild_members()
        .find_active_by_discord_user(&guild, "4242")
        .await
        .unwrap()
        .is_none());
    db.guild_members()
        .create(&linked_member(&guild, "Retour", Some("4242")))
        .await
        .unwrap();
}

#[tokio::test]
async fn discord_sync_stops_once_the_member_chooses_a_profile() {
    let db = common::memory_db().await;
    let guild = common::guild("111");
    let member = linked_member(&guild, "Ancien pseudo", Some("4242"));
    db.guild_members().create(&member).await.unwrap();
    let mut overlay = common::sample_overlay(&guild, "hash-sync", common::now());
    overlay.default_author_name = Some("Ancien pseudo".to_owned());
    db.overlay_clients().create(&overlay).await.unwrap();
    db.overlay_clients()
        .set_member(overlay.id, member.id)
        .await
        .unwrap();

    let synced_at = common::now() + Duration::hours(1);
    assert!(db
        .guild_members()
        .sync_discord_profile(
            member.id,
            "Pseudo Discord",
            Some("https://cdn.test/a.png"),
            synced_at
        )
        .await
        .unwrap());
    let stored = db.guild_members().get(member.id).await.unwrap().unwrap();
    assert_eq!(stored.display_name, "Pseudo Discord");
    assert_eq!(stored.avatar.as_deref(), Some("https://cdn.test/a.png"));
    assert_eq!(stored.discord_synced_at, Some(synced_at));
    // The overlays play under the new name.
    let client = db.overlay_clients().get(overlay.id).await.unwrap().unwrap();
    assert_eq!(
        client.default_author_name.as_deref(),
        Some("Pseudo Discord")
    );
    assert_eq!(
        client.default_author_image.as_deref(),
        Some("https://cdn.test/a.png")
    );

    // « Mon profil »: the member's choice wins from now on.
    db.guild_members()
        .update_profile(member.id, "Choisi", None)
        .await
        .unwrap();
    assert!(!db
        .guild_members()
        .sync_discord_profile(member.id, "Pseudo Discord", None, synced_at)
        .await
        .unwrap());
    let stored = db.guild_members().get(member.id).await.unwrap().unwrap();
    assert_eq!(stored.display_name, "Choisi");
    assert!(stored.profile_custom);

    // « Reprendre mon profil Discord »: following again, next read forced.
    assert!(db.guild_members().follow_discord(member.id).await.unwrap());
    let stored = db.guild_members().get(member.id).await.unwrap().unwrap();
    assert!(stored.follows_discord());
    assert_eq!(stored.discord_synced_at, None);
}

#[tokio::test]
async fn an_unlinked_member_never_follows_discord() {
    let db = common::memory_db().await;
    let guild = common::guild("111");
    let member = linked_member(&guild, "Invité", None);
    db.guild_members().create(&member).await.unwrap();
    assert!(!db.guild_members().follow_discord(member.id).await.unwrap());
    assert!(!db
        .guild_members()
        .sync_discord_profile(member.id, "Autre", None, common::now())
        .await
        .unwrap());
    db.guild_members()
        .mark_discord_synced(member.id, common::now())
        .await
        .unwrap();
    let stored = db.guild_members().get(member.id).await.unwrap().unwrap();
    assert_eq!(stored.display_name, "Invité");
    assert!(!stored.follows_discord());
}
