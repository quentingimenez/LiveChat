//! Tests du repository `guilds` : upsert complet, mises à jour partielles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::Duration;

#[tokio::test]
async fn upsert_get_et_liste() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.guilds();
    let now = common::now();
    let guild = common::guild("600000000000000000");

    assert!(repo.get(&guild).await.unwrap().is_none());

    let mut sample = common::sample_guild(&guild);
    sample.busy_until = Some(now + Duration::seconds(5));
    sample.default_media_time = Some(20);
    sample.max_media_time = Some(120);
    sample.display_media_full = true;
    repo.upsert(&sample).await.unwrap();

    let fetched = repo.get(&guild).await.unwrap().expect("guilde insérée");
    assert_eq!(fetched, sample);
    let listed = repo.list().await.unwrap();
    assert_eq!(listed, vec![sample.clone()]);

    // `upsert` remplace intégralement la ligne.
    let mut replaced = common::sample_guild(&guild);
    replaced.name = Some("Nouveau nom".to_owned());
    repo.upsert(&replaced).await.unwrap();
    let fetched = repo.get(&guild).await.unwrap().unwrap();
    assert_eq!(fetched, replaced);
    assert_eq!(fetched.busy_until, None);
    assert!(!fetched.display_media_full);
}

#[tokio::test]
async fn set_busy_until_cree_la_ligne_et_preserve_les_reglages() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.guilds();
    let now = common::now();
    let guild = common::guild("601111111111111111");

    // Création partielle : seule `busy_until` est posée.
    repo.set_busy_until(&guild, Some(now + Duration::seconds(10)))
        .await
        .unwrap();
    let created = repo.get(&guild).await.unwrap().unwrap();
    assert_eq!(created.busy_until, Some(now + Duration::seconds(10)));
    assert_eq!(created.name, None);
    assert_eq!(created.default_media_time, None);
    assert!(!created.display_media_full);

    // `update_settings` ne touche pas au verrou.
    repo.update_settings(&guild, Some(15), Some(90), true)
        .await
        .unwrap();
    let updated = repo.get(&guild).await.unwrap().unwrap();
    assert_eq!(updated.busy_until, Some(now + Duration::seconds(10)));
    assert_eq!(updated.default_media_time, Some(15));
    assert_eq!(updated.max_media_time, Some(90));
    assert!(updated.display_media_full);

    // Lever le verrou ne touche pas aux réglages.
    repo.set_busy_until(&guild, None).await.unwrap();
    let cleared = repo.get(&guild).await.unwrap().unwrap();
    assert_eq!(cleared.busy_until, None);
    assert_eq!(cleared.default_media_time, Some(15));
    assert_eq!(cleared.max_media_time, Some(90));
}

#[tokio::test]
async fn update_settings_cree_la_ligne_et_liste_ordre() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.guilds();

    let guild_a = common::guild("602222222222222222");
    let guild_b = common::guild("603333333333333333");
    repo.upsert(&common::sample_guild(&guild_b)).await.unwrap();
    repo.update_settings(&guild_a, None, None, true)
        .await
        .unwrap();

    let created = repo.get(&guild_a).await.unwrap().unwrap();
    assert!(created.display_media_full);
    assert_eq!(created.default_media_time, None);

    let listed = repo.list().await.unwrap();
    let ids: Vec<_> = listed.iter().map(|guild| guild.id.as_str()).collect();
    assert_eq!(ids, vec![guild_a.as_str(), guild_b.as_str()]);
}
