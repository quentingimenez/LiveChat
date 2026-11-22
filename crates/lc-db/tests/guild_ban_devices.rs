//! Migration 0020: device fingerprints attached to a guild ban.
//!
//! The table only stores server-side fingerprints; these tests pin the
//! idempotence, the guild scope and the `ON DELETE CASCADE` that lifts a ban's
//! devices with the ban itself.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_domain::entities::GuildBan;
use lc_domain::ids::{GuildBanId, GuildMemberId};

fn ban(guild: &str, name: &str) -> GuildBan {
    GuildBan {
        id: GuildBanId::new_v4(),
        guild_id: common::guild(guild),
        member_id: Some(GuildMemberId::new_v4()),
        discord_user_id: None,
        display_name: name.to_owned(),
        avatar: None,
        banned_by: Some("Admin".to_owned()),
        created_at: common::now(),
    }
}

#[tokio::test]
async fn devices_are_recorded_idempotently_and_lifted_with_the_ban() {
    let (db, _dir) = common::temp_db().await;
    let guild = common::guild("111");
    let now = common::now();
    let devices = db.guild_ban_devices();

    let banned = ban("111", "Léa");
    db.guild_bans().create(&banned).await.unwrap();

    let hashes = vec!["hash-a".to_owned(), "hash-b".to_owned()];
    assert_eq!(
        devices
            .record(banned.id, &guild, &hashes, now)
            .await
            .unwrap(),
        2
    );
    // Recording the same fingerprints again attaches nothing new: a ban
    // recorded twice must not fail on its devices.
    assert_eq!(
        devices
            .record(banned.id, &guild, &hashes, now)
            .await
            .unwrap(),
        0
    );
    assert_eq!(devices.count_by_ban(banned.id).await.unwrap(), 2);

    assert!(devices.is_device_banned(&guild, "hash-a").await.unwrap());
    assert!(!devices.is_device_banned(&guild, "hash-c").await.unwrap());
    // A device ban only applies to its guild.
    assert!(!devices
        .is_device_banned(&common::guild("222"), "hash-a")
        .await
        .unwrap());

    // Lifting the ban cascades to its devices (`foreign_keys` pragma).
    assert!(db.guild_bans().delete(&guild, banned.id).await.unwrap());
    assert!(!devices.is_device_banned(&guild, "hash-a").await.unwrap());
    assert_eq!(devices.count_by_ban(banned.id).await.unwrap(), 0);
}

#[tokio::test]
async fn create_with_devices_commits_both_and_reports_a_conflict() {
    let (db, _dir) = common::temp_db().await;
    let guild = common::guild("111");
    let now = common::now();

    let mut first = ban("111", "Léa");
    first.discord_user_id = Some("777".to_owned());
    let hashes = vec!["hash-a".to_owned(), "hash-b".to_owned()];
    let stored = db
        .guild_bans()
        .create_with_devices(&first, &hashes, now)
        .await
        .unwrap();
    assert_eq!(stored.as_ref().map(|entry| entry.id), Some(first.id));
    assert_eq!(
        db.guild_ban_devices().count_by_ban(first.id).await.unwrap(),
        2
    );
    assert_eq!(
        db.guild_bans().list_by_guild(&guild).await.unwrap().len(),
        1
    );

    // The same Discord account: the ban insert conflicts, no ban is added and
    // the caller gets `None` (it then attaches devices to the existing ban).
    let mut second = ban("111", "Léa bis");
    second.discord_user_id = Some("777".to_owned());
    let rejected = db
        .guild_bans()
        .create_with_devices(&second, &["hash-c".to_owned()], now)
        .await
        .unwrap();
    assert!(rejected.is_none());
    assert_eq!(
        db.guild_bans().list_by_guild(&guild).await.unwrap().len(),
        1
    );
    assert_eq!(
        db.guild_ban_devices()
            .count_by_ban(second.id)
            .await
            .unwrap(),
        0
    );
    assert!(!db
        .guild_ban_devices()
        .is_device_banned(&guild, "hash-c")
        .await
        .unwrap());
}

#[tokio::test]
async fn deleting_a_guild_cascades_through_its_bans_to_their_devices() {
    let (db, _dir) = common::temp_db().await;
    let guild = common::guild("111");
    let now = common::now();

    let first = ban("111", "Léa");
    let second = ban("111", "Sans compte");
    for entry in [&first, &second] {
        db.guild_bans().create(entry).await.unwrap();
    }
    db.guild_ban_devices()
        .record(first.id, &guild, &["hash-a".to_owned()], now)
        .await
        .unwrap();
    db.guild_ban_devices()
        .record(second.id, &guild, &["hash-b".to_owned()], now)
        .await
        .unwrap();

    assert_eq!(db.guild_bans().delete_by_guild(&guild).await.unwrap(), 2);
    assert!(!db
        .guild_ban_devices()
        .is_device_banned(&guild, "hash-a")
        .await
        .unwrap());
    assert!(!db
        .guild_ban_devices()
        .is_device_banned(&guild, "hash-b")
        .await
        .unwrap());
}
