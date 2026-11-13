//! Codes de liaison Discord : émission, consommation atomique et purge.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::{DateTime, Duration, Utc};
use lc_db::{Db, LinkOutcome};
use lc_domain::entities::DiscordLinkCode;
use lc_domain::ids::{GuildId, PairingCode};

fn link_code(guild_id: &GuildId, raw: &str, now: DateTime<Utc>) -> DiscordLinkCode {
    DiscordLinkCode {
        code: PairingCode::parse(raw).unwrap(),
        guild_id: guild_id.clone(),
        created_by_member_id: None,
        expires_at: now + Duration::minutes(10),
        used_at: None,
        created_at: now,
    }
}

async fn standalone(db: &Db, name: &str) -> GuildId {
    let guild_id = GuildId::generate();
    db.guilds()
        .create_standalone(&guild_id, name, common::now())
        .await
        .unwrap();
    guild_id
}

async fn code_count(db: &Db, guild_id: &GuildId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM discord_link_codes WHERE guild_id = ?")
        .bind(guild_id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap()
}

async fn used_at(db: &Db, raw: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT used_at FROM discord_link_codes WHERE code = ?")
        .bind(raw)
        .fetch_one(db.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn replace_supprime_les_anciens_codes_inutilises() {
    let db = common::memory_db().await;
    let guild = standalone(&db, "Les Sardines").await;
    let links = db.discord_link_codes();

    links
        .replace_for_guild(&link_code(&guild, "K7QXM2PD", common::now()))
        .await
        .unwrap();
    links
        .replace_for_guild(&link_code(&guild, "P3WN8RTA", common::now()))
        .await
        .unwrap();

    let stored: Vec<String> = sqlx::query_scalar("SELECT code FROM discord_link_codes")
        .fetch_all(db.pool())
        .await
        .unwrap();
    assert_eq!(stored, vec!["P3WN8RTA".to_owned()]);

    // Un code déjà consommé n'est jamais supprimé par un remplacement.
    let discord = common::guild("123456789012345678");
    links
        .redeem(
            &PairingCode::parse("P3WN8RTA").unwrap(),
            &discord,
            common::now(),
        )
        .await
        .unwrap();
    links
        .replace_for_guild(&link_code(&guild, "Z9LCV5HE", common::now()))
        .await
        .unwrap();
    assert_eq!(code_count(&db, &guild).await, 2);
    assert_eq!(used_at(&db, "P3WN8RTA").await, Some(common::NOW_MS));
    assert_eq!(used_at(&db, "Z9LCV5HE").await, None);
}

#[tokio::test]
async fn redeem_lie_la_guilde_et_marque_le_code() {
    let db = common::memory_db().await;
    let guild = standalone(&db, "Les Sardines").await;
    let discord = common::guild("123456789012345678");
    let links = db.discord_link_codes();
    let link = link_code(&guild, "K7QXM2PD", common::now());
    links.replace_for_guild(&link).await.unwrap();

    let outcome = links
        .redeem(&link.code, &discord, common::now())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        LinkOutcome::Linked {
            guild_id: guild.clone(),
            guild_name: Some("Les Sardines".to_owned()),
        }
    );
    assert_eq!(db.guilds().resolve_discord(&discord).await.unwrap(), guild);
    assert_eq!(
        db.guilds()
            .access(&guild)
            .await
            .unwrap()
            .unwrap()
            .discord_guild_id
            .as_deref(),
        Some(discord.as_str())
    );
    assert_eq!(used_at(&db, "K7QXM2PD").await, Some(common::NOW_MS));

    // Un code est à usage unique.
    assert_eq!(
        links
            .redeem(&link.code, &discord, common::now())
            .await
            .unwrap(),
        LinkOutcome::InvalidCode
    );
}

#[tokio::test]
async fn redeem_refuse_code_inconnu_ou_expire() {
    let db = common::memory_db().await;
    let guild = standalone(&db, "Les Sardines").await;
    let discord = common::guild("123456789012345678");
    let links = db.discord_link_codes();

    let unknown = PairingCode::parse("Z9LCV5HE").unwrap();
    assert_eq!(
        links
            .redeem(&unknown, &discord, common::now())
            .await
            .unwrap(),
        LinkOutcome::InvalidCode
    );

    // Expiré : `expires_at <= now`.
    let mut expired = link_code(&guild, "K7QXM2PD", common::now());
    expired.expires_at = common::now();
    links.replace_for_guild(&expired).await.unwrap();
    assert_eq!(
        links
            .redeem(&expired.code, &discord, common::now())
            .await
            .unwrap(),
        LinkOutcome::InvalidCode
    );
    assert_eq!(used_at(&db, "K7QXM2PD").await, None);
}

#[tokio::test]
async fn redeem_refuse_une_cible_native_ou_absente() {
    let db = common::memory_db().await;
    let discord = common::guild("123456789012345678");
    let links = db.discord_link_codes();

    // Guilde native (`standalone = 0`) : code invalide.
    let native = common::guild("111");
    db.guilds()
        .upsert(&common::sample_guild(&native))
        .await
        .unwrap();
    links
        .replace_for_guild(&link_code(&native, "K7QXM2PD", common::now()))
        .await
        .unwrap();
    assert_eq!(
        links
            .redeem(
                &PairingCode::parse("K7QXM2PD").unwrap(),
                &discord,
                common::now()
            )
            .await
            .unwrap(),
        LinkOutcome::InvalidCode
    );

    // Guilde cible inexistante : code invalide.
    let missing = GuildId::generate();
    links
        .replace_for_guild(&link_code(&missing, "P3WN8RTA", common::now()))
        .await
        .unwrap();
    assert_eq!(
        links
            .redeem(
                &PairingCode::parse("P3WN8RTA").unwrap(),
                &discord,
                common::now()
            )
            .await
            .unwrap(),
        LinkOutcome::InvalidCode
    );
}

#[tokio::test]
async fn redeem_refuse_un_serveur_deja_lie_puis_reessaie() {
    let db = common::memory_db().await;
    let first = standalone(&db, "Les Sardines").await;
    let second = standalone(&db, "Les Anchois").await;
    let discord = common::guild("123456789012345678");
    let links = db.discord_link_codes();

    db.guilds()
        .link_discord(&first, Some(&discord))
        .await
        .unwrap();
    let link = link_code(&second, "K7QXM2PD", common::now());
    links.replace_for_guild(&link).await.unwrap();

    assert_eq!(
        links
            .redeem(&link.code, &discord, common::now())
            .await
            .unwrap(),
        LinkOutcome::AlreadyLinkedElsewhere
    );
    // Le code n'a pas été consommé : il reste utilisable.
    assert_eq!(used_at(&db, "K7QXM2PD").await, None);

    db.guilds().link_discord(&first, None).await.unwrap();
    assert_eq!(
        links
            .redeem(&link.code, &discord, common::now())
            .await
            .unwrap(),
        LinkOutcome::Linked {
            guild_id: second.clone(),
            guild_name: Some("Les Anchois".to_owned()),
        }
    );
    assert_eq!(db.guilds().resolve_discord(&discord).await.unwrap(), second);
}

#[tokio::test]
async fn relink_remplace_l_ancien_serveur() {
    let db = common::memory_db().await;
    let guild = standalone(&db, "Les Sardines").await;
    let first = common::guild("123456789012345678");
    let second = common::guild("987654321098765432");
    let links = db.discord_link_codes();

    let first_code = link_code(&guild, "K7QXM2PD", common::now());
    links.replace_for_guild(&first_code).await.unwrap();
    links
        .redeem(&first_code.code, &first, common::now())
        .await
        .unwrap();

    let second_code = link_code(&guild, "P3WN8RTA", common::now());
    links.replace_for_guild(&second_code).await.unwrap();
    assert_eq!(
        links
            .redeem(&second_code.code, &second, common::now())
            .await
            .unwrap(),
        LinkOutcome::Linked {
            guild_id: guild.clone(),
            guild_name: Some("Les Sardines".to_owned()),
        }
    );

    // La nouvelle liaison remplace l'ancienne.
    assert_eq!(db.guilds().resolve_discord(&second).await.unwrap(), guild);
    assert_eq!(db.guilds().resolve_discord(&first).await.unwrap(), first);
    assert_eq!(
        db.guilds()
            .access(&guild)
            .await
            .unwrap()
            .unwrap()
            .discord_guild_id
            .as_deref(),
        Some(second.as_str())
    );
}

#[tokio::test]
async fn delete_by_guild_purge_les_codes_de_la_guilde() {
    let db = common::memory_db().await;
    let first = standalone(&db, "Les Sardines").await;
    let second = standalone(&db, "Les Anchois").await;
    let links = db.discord_link_codes();
    links
        .replace_for_guild(&link_code(&first, "K7QXM2PD", common::now()))
        .await
        .unwrap();
    links
        .replace_for_guild(&link_code(&second, "P3WN8RTA", common::now()))
        .await
        .unwrap();

    assert_eq!(links.delete_by_guild(&first).await.unwrap(), 1);
    assert_eq!(code_count(&db, &first).await, 0);
    assert_eq!(code_count(&db, &second).await, 1);
    assert_eq!(links.delete_by_guild(&first).await.unwrap(), 0);
}
