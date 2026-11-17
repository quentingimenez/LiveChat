//! Migration 0015: guild bans and the revocations a ban relies on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_domain::entities::GuildBan;
use lc_domain::ids::{GuildBanId, GuildMemberId};
use lc_domain::DomainError;

fn ban(guild: &str, discord_user_id: Option<&str>, name: &str) -> GuildBan {
    GuildBan {
        id: GuildBanId::new_v4(),
        guild_id: common::guild(guild),
        member_id: Some(GuildMemberId::new_v4()),
        discord_user_id: discord_user_id.map(str::to_owned),
        display_name: name.to_owned(),
        avatar: None,
        banned_by: Some("Admin".to_owned()),
        created_at: common::now(),
    }
}

#[tokio::test]
async fn ban_is_listed_checked_and_lifted() {
    let (db, _dir) = common::temp_db().await;
    let bans = db.guild_bans();
    let guild = common::guild("111");

    let first = ban("111", Some("42"), "Léa");
    bans.create(&first).await.unwrap();
    // Two members without a Discord account can both be banned.
    bans.create(&ban("111", None, "Sans compte")).await.unwrap();
    bans.create(&ban("111", None, "Autre")).await.unwrap();

    assert!(bans.is_discord_user_banned(&guild, "42").await.unwrap());
    assert!(!bans.is_discord_user_banned(&guild, "43").await.unwrap());
    // A ban only applies to its guild.
    assert!(!bans
        .is_discord_user_banned(&common::guild("222"), "42")
        .await
        .unwrap());

    // The same account cannot be banned twice from one guild.
    let duplicate = bans.create(&ban("111", Some("42"), "Léa")).await;
    assert!(matches!(
        duplicate,
        Err(lc_db::DbError::Domain(DomainError::Conflict(_)))
    ));

    let listed = bans.list_by_guild(&guild).await.unwrap();
    assert_eq!(listed.len(), 3);
    assert!(listed.iter().any(|entry| entry == &first));

    // Lifting needs the right guild.
    assert!(!bans.delete(&common::guild("222"), first.id).await.unwrap());
    assert!(bans.delete(&guild, first.id).await.unwrap());
    assert!(!bans.delete(&guild, first.id).await.unwrap());
    assert!(!bans.is_discord_user_banned(&guild, "42").await.unwrap());

    assert_eq!(bans.delete_by_guild(&guild).await.unwrap(), 2);
    assert!(bans.list_by_guild(&guild).await.unwrap().is_empty());
}

#[tokio::test]
async fn revocations_target_only_the_given_authors() {
    let (db, _dir) = common::temp_db().await;
    let guild = common::guild("111");
    let other_guild = common::guild("222");
    let now = common::now();

    let mine = common::sample_ingest(&guild, "hash-mine", now);
    let mut by_member = common::sample_ingest(&guild, "hash-member", now);
    by_member.created_by_discord_user_id = "member:abc".to_owned();
    let mut someone_else = common::sample_ingest(&guild, "hash-else", now);
    someone_else.created_by_discord_user_id = "43".to_owned();
    let elsewhere = common::sample_ingest(&other_guild, "hash-elsewhere", now);
    for client in [&mine, &by_member, &someone_else, &elsewhere] {
        db.ingest_clients().create(client).await.unwrap();
    }
    let authors = vec!["42".to_owned(), "member:abc".to_owned()];
    assert_eq!(
        db.ingest_clients()
            .revoke_by_authors(&guild, &authors, now)
            .await
            .unwrap(),
        2
    );
    let active = |id| {
        let db = db.clone();
        async move {
            db.ingest_clients()
                .get(id)
                .await
                .unwrap()
                .unwrap()
                .is_active()
        }
    };
    assert!(!active(mine.id).await);
    assert!(!active(by_member.id).await);
    assert!(active(someone_else.id).await);
    assert!(active(elsewhere.id).await);

    let pending = common::sample_pairing("AAAAAA", &guild, now);
    let mut foreign = common::sample_pairing("BBBBBB", &guild, now);
    foreign.created_by_discord_user_id = "43".to_owned();
    let pending_elsewhere = common::sample_pairing("CCCCCC", &other_guild, now);
    for code in [&pending, &foreign, &pending_elsewhere] {
        db.pairing_codes().create(code).await.unwrap();
    }
    assert_eq!(
        db.pairing_codes()
            .revoke_by_authors(&guild, &authors, now)
            .await
            .unwrap(),
        1
    );
    let usable = |code: lc_domain::PairingCode| {
        let db = db.clone();
        async move {
            db.pairing_codes()
                .get(&code)
                .await
                .unwrap()
                .unwrap()
                .is_usable_at(now)
        }
    };
    assert!(!usable(pending.code.clone()).await);
    assert!(usable(foreign.code.clone()).await);
    assert!(usable(pending_elsewhere.code.clone()).await);
}
