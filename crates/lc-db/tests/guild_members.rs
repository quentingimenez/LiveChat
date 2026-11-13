//! Migration 0003 : guildes indépendantes, membres, rôles et invitations.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::Duration;
use lc_db::MemberChange;
use lc_domain::entities::{GuildInvite, GuildMember};
use lc_domain::enums::{GuildRole, OverlaySessionMode};
use lc_domain::ids::{GuildId, GuildMemberId, PairingCode};
use lc_domain::DomainError;
use sqlx::sqlite::SqlitePoolOptions;

fn member(guild_id: &GuildId, name: &str, role: GuildRole) -> GuildMember {
    GuildMember {
        id: GuildMemberId::new_v4(),
        guild_id: guild_id.clone(),
        display_name: name.to_owned(),
        avatar: None,
        role,
        created_at: common::now(),
        removed_at: None,
    }
}

fn invite(guild_id: &GuildId, code: &str, role: GuildRole) -> GuildInvite {
    GuildInvite {
        code: PairingCode::parse(code).unwrap(),
        guild_id: guild_id.clone(),
        role,
        member_id: None,
        max_uses: Some(1),
        use_count: 0,
        created_by_member_id: None,
        expires_at: Some(common::now() + Duration::hours(24)),
        revoked_at: None,
        created_at: common::now(),
    }
}

#[tokio::test]
async fn migration_0003_reprend_les_overlays_existants() {
    // Base arrêtée à la migration 0002, peuplée comme une base en production.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../migrations/0001_init.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/0002_playback_single_playing.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO guilds (id, name) VALUES ('111', 'Legacy')")
        .execute(&pool)
        .await
        .unwrap();
    for (id, label, mode, author, revoked_at) in [
        ("c-normal", "Salon", "NORMAL", Some("Léa"), None),
        ("c-guest", "Invité-42", "INVITE_READ_ONLY", None, None),
        ("c-revoked", "Ancien", "NORMAL", Some("  "), Some(5_i64)),
    ] {
        sqlx::query(
            "INSERT INTO overlay_clients ( \
                id, guild_id, label, token_hash, session_mode, default_author_name, \
                created_at, revoked_at \
             ) VALUES (?, '111', ?, ?, ?, ?, 1, ?)",
        )
        .bind(id)
        .bind(label)
        .bind(format!("hash-{id}"))
        .bind(mode)
        .bind(author)
        .bind(revoked_at)
        .execute(&pool)
        .await
        .unwrap();
    }

    sqlx::raw_sql(include_str!("../migrations/0003_guild_members.sql"))
        .execute(&pool)
        .await
        .unwrap();

    let members: Vec<(String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT m.id, m.display_name, m.role, m.removed_at \
         FROM guild_members AS m JOIN overlay_clients AS c ON c.member_id = m.id \
         ORDER BY m.id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        members,
        vec![
            (
                "c-guest".to_owned(),
                "Invité-42".to_owned(),
                "GUEST".to_owned(),
                None
            ),
            (
                "c-normal".to_owned(),
                "Léa".to_owned(),
                "MEMBER".to_owned(),
                None
            ),
            (
                "c-revoked".to_owned(),
                "Ancien".to_owned(),
                "MEMBER".to_owned(),
                Some(5)
            ),
        ]
    );

    // Une guilde existante reste une guilde Discord, board visible des invités.
    let (standalone, guests): (i64, i64) =
        sqlx::query_as("SELECT standalone, guests_can_view_board FROM guilds WHERE id = '111'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((standalone, guests), (0, 1));
}

#[tokio::test]
async fn guilde_independante_et_liaison_discord() {
    let (db, _dir) = common::temp_db().await;
    let guilds = db.guilds();

    let id = GuildId::generate();
    guilds
        .create_standalone(&id, "Les Sardines", common::now())
        .await
        .unwrap();
    let error = guilds
        .create_standalone(&id, "Doublon", common::now())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        lc_db::DbError::Domain(DomainError::Conflict(_))
    ));

    let access = guilds.access(&id).await.unwrap().unwrap();
    assert!(access.standalone);
    assert_eq!(access.name.as_deref(), Some("Les Sardines"));
    assert!(access.guests_can_view_board);
    assert_eq!(access.created_at, Some(common::now()));
    assert_eq!(access.discord_guild_id, None);

    // Les réglages de lecture restent lisibles par l'entité historique.
    let settings = guilds.get(&id).await.unwrap().unwrap();
    assert_eq!(settings.name.as_deref(), Some("Les Sardines"));
    assert!(!settings.display_media_full);

    // Sans liaison, un serveur Discord se résout sur lui-même (legacy).
    let discord = common::guild("123456789012345678");
    assert_eq!(guilds.resolve_discord(&discord).await.unwrap(), discord);

    assert!(guilds.link_discord(&id, Some(&discord)).await.unwrap());
    assert_eq!(guilds.resolve_discord(&discord).await.unwrap(), id);

    // Un serveur Discord ne peut être lié qu'à une seule guilde.
    let other = GuildId::generate();
    guilds
        .create_standalone(&other, "Autre", common::now())
        .await
        .unwrap();
    let error = guilds
        .link_discord(&other, Some(&discord))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        lc_db::DbError::Domain(DomainError::Conflict(_))
    ));

    // Une guilde Discord legacy ne se lie pas.
    guilds
        .upsert(&common::sample_guild(&discord))
        .await
        .unwrap();
    assert!(!guilds.link_discord(&discord, Some(&discord)).await.unwrap());

    assert!(guilds.link_discord(&id, None).await.unwrap());
    assert_eq!(guilds.resolve_discord(&discord).await.unwrap(), discord);

    guilds.set_guests_can_view_board(&id, false).await.unwrap();
    assert!(guilds.set_name(&id, "Sardines").await.unwrap());
    let access = guilds.access(&id).await.unwrap().unwrap();
    assert!(!access.guests_can_view_board);
    assert_eq!(access.name.as_deref(), Some("Sardines"));
    assert_eq!(guilds.list_access().await.unwrap().len(), 3);
}

#[tokio::test]
async fn dernier_admin_protege() {
    let (db, _dir) = common::temp_db().await;
    let members = db.guild_members();
    let guild_id = common::guild("111");

    let admin = member(&guild_id, "Quentin", GuildRole::Admin);
    let other = member(&guild_id, "Léa", GuildRole::Member);
    members.create(&admin).await.unwrap();
    members.create(&other).await.unwrap();
    assert_eq!(members.count_active_admins(&guild_id).await.unwrap(), 1);

    // Seul admin : ni rétrogradation ni retrait.
    assert_eq!(
        members.set_role(admin.id, GuildRole::Member).await.unwrap(),
        MemberChange::LastAdmin
    );
    assert_eq!(
        members.remove(admin.id, common::now()).await.unwrap(),
        MemberChange::LastAdmin
    );
    // Reconduire son rôle d'admin reste permis.
    assert_eq!(
        members.set_role(admin.id, GuildRole::Admin).await.unwrap(),
        MemberChange::Applied
    );

    // Avec un second admin, le premier peut partir.
    assert_eq!(
        members.set_role(other.id, GuildRole::Admin).await.unwrap(),
        MemberChange::Applied
    );
    assert_eq!(
        members.remove(admin.id, common::now()).await.unwrap(),
        MemberChange::Applied
    );
    assert_eq!(
        members.remove(admin.id, common::now()).await.unwrap(),
        MemberChange::NotFound
    );
    assert_eq!(
        members.set_role(other.id, GuildRole::Guest).await.unwrap(),
        MemberChange::LastAdmin
    );
    assert_eq!(
        members
            .set_role(GuildMemberId::new_v4(), GuildRole::Guest)
            .await
            .unwrap(),
        MemberChange::NotFound
    );

    // Un admin d'une autre guilde ne compte pas.
    let elsewhere = member(&common::guild("222"), "Max", GuildRole::Admin);
    members.create(&elsewhere).await.unwrap();
    assert_eq!(
        members.set_role(other.id, GuildRole::Member).await.unwrap(),
        MemberChange::LastAdmin
    );

    let listed = members.list_by_guild(&guild_id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, other.id);
    assert!(members
        .update_profile(other.id, "Léa B.", Some("https://example.test/a.png"))
        .await
        .unwrap());
    let reloaded = members.get(other.id).await.unwrap().unwrap();
    assert_eq!(reloaded.display_name, "Léa B.");
    assert_eq!(
        reloaded.avatar.as_deref(),
        Some("https://example.test/a.png")
    );
}

#[tokio::test]
async fn overlays_d_un_membre() {
    let (db, _dir) = common::temp_db().await;
    let guild_id = common::guild("111");
    let owner = member(&guild_id, "Léa", GuildRole::Member);
    db.guild_members().create(&owner).await.unwrap();

    let overlays = db.overlay_clients();
    let first = common::sample_overlay(&guild_id, "hash-1", common::now());
    let second = common::sample_overlay(&guild_id, "hash-2", common::now());
    let orphan = common::sample_overlay(&guild_id, "hash-3", common::now());
    for client in [&first, &second, &orphan] {
        overlays.create(client).await.unwrap();
    }
    assert!(overlays.set_member(first.id, owner.id).await.unwrap());
    assert!(overlays.set_member(second.id, owner.id).await.unwrap());

    let found = db
        .guild_members()
        .get_for_overlay_client(first.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, owner.id);
    assert!(db
        .guild_members()
        .get_for_overlay_client(orphan.id)
        .await
        .unwrap()
        .is_none());

    assert_eq!(
        overlays
            .update_mode_by_member(owner.id, OverlaySessionMode::InviteReadOnly)
            .await
            .unwrap(),
        2
    );
    let listed = overlays.list_active_by_member(owner.id).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .all(|client| client.session_mode == OverlaySessionMode::InviteReadOnly));

    let mut revoked = overlays
        .revoke_by_member(owner.id, common::now())
        .await
        .unwrap();
    revoked.sort();
    let mut expected = vec![first.id, second.id];
    expected.sort();
    assert_eq!(revoked, expected);
    assert!(overlays
        .list_active_by_member(owner.id)
        .await
        .unwrap()
        .is_empty());
    assert!(overlays
        .get_by_token_hash("hash-3")
        .await
        .unwrap()
        .is_some());

    // Un membre retiré ne donne plus accès par son overlay.
    let third = common::sample_overlay(&guild_id, "hash-4", common::now());
    overlays.create(&third).await.unwrap();
    overlays.set_member(third.id, owner.id).await.unwrap();
    db.guild_members()
        .remove(owner.id, common::now())
        .await
        .unwrap();
    assert!(db
        .guild_members()
        .get_for_overlay_client(third.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn invitation_usages_expiration_revocation() {
    let (db, _dir) = common::temp_db().await;
    let invites = db.guild_invites();
    let guild_id = common::guild("111");
    let now = common::now();

    // Usage unique.
    let single = invite(&guild_id, "K7QXM2PD", GuildRole::Admin);
    invites.create(&single).await.unwrap();
    let error = invites.create(&single).await.unwrap_err();
    assert!(matches!(
        error,
        lc_db::DbError::Domain(DomainError::Conflict(_))
    ));
    let consumed = invites.consume(&single.code, now).await.unwrap().unwrap();
    assert_eq!(consumed.role, GuildRole::Admin);
    assert_eq!(consumed.use_count, 1);
    assert!(invites.consume(&single.code, now).await.unwrap().is_none());

    // Usages multiples puis épuisement.
    let mut shared = invite(&guild_id, "P3WN8RTA", GuildRole::Guest);
    shared.max_uses = Some(2);
    invites.create(&shared).await.unwrap();
    assert!(invites.consume(&shared.code, now).await.unwrap().is_some());
    assert!(invites.consume(&shared.code, now).await.unwrap().is_some());
    assert!(invites.consume(&shared.code, now).await.unwrap().is_none());

    // Illimité, sans expiration.
    let mut open = invite(&guild_id, "Z9LCV5HE", GuildRole::Member);
    open.max_uses = None;
    open.expires_at = None;
    invites.create(&open).await.unwrap();
    for _ in 0..3 {
        assert!(invites.consume(&open.code, now).await.unwrap().is_some());
    }

    // Expirée.
    let expired = invite(&guild_id, "EXP1RED2", GuildRole::Member);
    invites.create(&expired).await.unwrap();
    assert!(invites
        .consume(&expired.code, now + Duration::hours(24))
        .await
        .unwrap()
        .is_none());

    // Révocation, limitée à la guilde propriétaire.
    assert!(!invites
        .revoke(&common::guild("222"), &open.code, now)
        .await
        .unwrap());
    assert!(invites.revoke(&guild_id, &open.code, now).await.unwrap());
    assert!(!invites.revoke(&guild_id, &open.code, now).await.unwrap());
    assert!(invites.consume(&open.code, now).await.unwrap().is_none());

    let listed = invites.list_by_guild(&guild_id).await.unwrap();
    assert_eq!(listed.len(), 4);
    assert_eq!(invites.delete_by_guild(&guild_id).await.unwrap(), 4);
    assert!(invites.get(&single.code).await.unwrap().is_none());
}

#[tokio::test]
async fn auteur_d_un_item_de_board() {
    let (db, _dir) = common::temp_db().await;
    let guild_id = common::guild("111");
    let asset = common::sample_asset("hash-board-item", common::now());
    db.media_assets().insert(&asset).await.unwrap();
    let item = common::sample_board_item(&guild_id, asset.id, common::now());
    db.media_board().create(&item).await.unwrap();

    assert_eq!(
        db.media_board().creator_member(item.id).await.unwrap(),
        None
    );
    let author = GuildMemberId::new_v4();
    assert!(db
        .media_board()
        .set_creator_member(item.id, author)
        .await
        .unwrap());
    assert_eq!(
        db.media_board().creator_member(item.id).await.unwrap(),
        Some(author)
    );
    // L'entité historique se relit sans changement.
    assert_eq!(db.media_board().get(item.id).await.unwrap().unwrap(), item);
}
