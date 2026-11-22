//! Migration 0020: device fingerprints attached to a guild ban.
//!
//! The table only stores server-side fingerprints; these tests pin the
//! idempotence, the guild scope and the `ON DELETE CASCADE` that lifts a ban's
//! devices with the ban itself. They also pin the serialised creation: a
//! `BEGIN IMMEDIATE` transaction relits an existing ban of the member before
//! inserting, so two bans for the same unlinked member never create two rows.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_db::BanWithDevices;
use lc_domain::entities::{GuildBan, GuildMember};
use lc_domain::ids::{GuildBanId, GuildMemberId};
use lc_domain::GuildRole;
use time::UtcDateTime;

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

fn member(guild: &str, name: &str, removed_at: Option<UtcDateTime>) -> GuildMember {
    GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: common::guild(guild),
        display_name: name.to_owned(),
        avatar: None,
        role: GuildRole::Member,
        created_at: common::now(),
        removed_at,
        discord_user_id: None,
        profile_custom: true,
        discord_synced_at: None,
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
async fn create_with_devices_commits_both_and_reuses_the_existing_ban() {
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
    match stored {
        BanWithDevices::Created(created) => assert_eq!(created.id, first.id),
        BanWithDevices::Existing(existing) => panic!("unexpected existing ban {existing:?}"),
    }
    assert_eq!(
        db.guild_ban_devices().count_by_ban(first.id).await.unwrap(),
        2
    );
    assert_eq!(
        db.guild_bans().list_by_guild(&guild).await.unwrap().len(),
        1
    );

    // The same Discord account: the transaction finds the existing ban, adds
    // its devices to it and never inserts a second row.
    let mut second = ban("111", "Léa bis");
    second.discord_user_id = Some("777".to_owned());
    let reused = db
        .guild_bans()
        .create_with_devices(&second, &["hash-c".to_owned()], now)
        .await
        .unwrap();
    match reused {
        BanWithDevices::Existing(existing) => assert_eq!(existing.id, first.id),
        BanWithDevices::Created(created) => panic!("unexpected new ban {created:?}"),
    }
    assert_eq!(
        db.guild_bans().list_by_guild(&guild).await.unwrap().len(),
        1
    );
    assert_eq!(
        db.guild_ban_devices().count_by_ban(first.id).await.unwrap(),
        3
    );
    assert!(db
        .guild_ban_devices()
        .is_device_banned(&guild, "hash-c")
        .await
        .unwrap());
    // The rejected ban id itself owns nothing: devices went to the existing ban.
    assert_eq!(
        db.guild_ban_devices()
            .count_by_ban(second.id)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_second_ban_for_the_same_unlinked_member_reuses_the_first_one() {
    let (db, _dir) = common::temp_db().await;
    let guild = common::guild("111");
    let now = common::now();

    // Two concurrent bans of a member without a Discord account target the
    // same `member_id`: the second transaction must find the first row
    // instead of inserting a duplicate (there is no unique index on
    // `member_id`).
    let member_id = GuildMemberId::new_v4();
    let mut first = ban("111", "Sans compte");
    first.member_id = Some(member_id);
    let mut second = ban("111", "Sans compte");
    second.member_id = Some(member_id);

    let created = db
        .guild_bans()
        .create_with_devices(&first, &["hash-a".to_owned()], now)
        .await
        .unwrap();
    assert!(matches!(created, BanWithDevices::Created(_)));

    let reused = db
        .guild_bans()
        .create_with_devices(&second, &["hash-b".to_owned()], now)
        .await
        .unwrap();
    match reused {
        BanWithDevices::Existing(existing) => assert_eq!(existing.id, first.id),
        BanWithDevices::Created(created) => panic!("unexpected new ban {created:?}"),
    }

    let bans = db.guild_bans().list_by_guild(&guild).await.unwrap();
    assert_eq!(bans.len(), 1);
    assert_eq!(bans[0].id, first.id);
    assert_eq!(
        db.guild_ban_devices().count_by_ban(first.id).await.unwrap(),
        2
    );
    assert!(db
        .guild_ban_devices()
        .is_device_banned(&guild, "hash-b")
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

#[tokio::test]
async fn startup_reconciliation_drops_bans_left_on_active_members() {
    let (db, _dir) = common::temp_db().await;
    let guild = common::guild("111");
    let now = common::now();

    // A crash between the ban transaction and the member removal leaves an
    // active member with an active ban and its devices.
    let active = member("111", "Actif", None);
    let removed = member("111", "Retiré", Some(now));
    db.guild_members().create(&active).await.unwrap();
    db.guild_members().create(&removed).await.unwrap();

    let mut broken = ban("111", "Actif");
    broken.member_id = Some(active.id);
    let mut healthy = ban("111", "Retiré");
    healthy.member_id = Some(removed.id);
    // An imported Discord ban has no member: reconciliation must keep it.
    let mut imported = ban("111", "Discord");
    imported.member_id = None;
    imported.discord_user_id = Some("777".to_owned());
    for entry in [&broken, &healthy, &imported] {
        db.guild_bans().create(entry).await.unwrap();
    }
    db.guild_ban_devices()
        .record(broken.id, &guild, &["hash-a".to_owned()], now)
        .await
        .unwrap();
    db.guild_ban_devices()
        .record(healthy.id, &guild, &["hash-b".to_owned()], now)
        .await
        .unwrap();

    assert_eq!(
        db.guild_bans()
            .delete_bans_of_active_members()
            .await
            .unwrap(),
        1
    );
    let bans = db.guild_bans().list_by_guild(&guild).await.unwrap();
    assert_eq!(bans.len(), 2);
    assert!(!bans.iter().any(|entry| entry.id == broken.id));
    assert!(bans.iter().any(|entry| entry.id == healthy.id));
    assert!(bans.iter().any(|entry| entry.id == imported.id));

    // The removed ban's devices survive; the inconsistent one went with its ban.
    assert!(!db
        .guild_ban_devices()
        .is_device_banned(&guild, "hash-a")
        .await
        .unwrap());
    assert!(db
        .guild_ban_devices()
        .is_device_banned(&guild, "hash-b")
        .await
        .unwrap());
}
