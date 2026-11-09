//! Tests du repository `meme_board_items` : CRUD, unicité par guilde, FK.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_db::DbError;
use lc_domain::{DomainError, MediaAssetId};

#[tokio::test]
async fn crud_meme_board() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let other_guild = common::guild("987654321098765432");
    let asset = common::sample_asset("meme-asset", now);
    db.media_assets().insert(&asset).await.unwrap();

    let repo = db.meme_board();
    let item = common::sample_meme(&guild, asset.id, now);
    repo.create(&item).await.unwrap();

    assert_eq!(repo.get(item.id).await.unwrap().unwrap(), item);
    let by_pair = repo
        .get_by_guild_and_asset(&guild, asset.id)
        .await
        .unwrap()
        .expect("élément présent");
    assert_eq!(by_pair.id, item.id);
    assert!(repo
        .get_by_guild_and_asset(&other_guild, asset.id)
        .await
        .unwrap()
        .is_none());

    // Mise à jour : `None` efface la colonne.
    assert!(repo
        .update(item.id, Some("nouveau titre"), None)
        .await
        .unwrap());
    let updated = repo.get(item.id).await.unwrap().unwrap();
    assert_eq!(updated.title.as_deref(), Some("nouveau titre"));
    assert_eq!(updated.message, None);
    assert!(!repo
        .update(lc_domain::MemeBoardItemId::new_v4(), None, None)
        .await
        .unwrap());

    // Liste par guilde triée du plus récent au plus ancien.
    let later_asset = common::sample_asset("meme-asset-2", now);
    db.media_assets().insert(&later_asset).await.unwrap();
    let later_item =
        common::sample_meme(&guild, later_asset.id, common::date(common::NOW_MS + 1_000));
    repo.create(&later_item).await.unwrap();
    let listed = repo.list_by_guild(&guild).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, later_item.id);

    assert!(repo.delete(item.id).await.unwrap());
    assert!(repo.get(item.id).await.unwrap().is_none());
    assert!(!repo.delete(item.id).await.unwrap());
    assert!(repo.list_by_guild(&guild).await.unwrap().len() == 1);
}

#[tokio::test]
async fn doublon_guild_et_asset_refuse() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let asset = common::sample_asset("meme-unique", now);
    db.media_assets().insert(&asset).await.unwrap();

    let repo = db.meme_board();
    repo.create(&common::sample_meme(&guild, asset.id, now))
        .await
        .unwrap();

    let another = common::sample_meme(&guild, asset.id, now);
    let error = repo.create(&another).await.unwrap_err();
    match error {
        DbError::Domain(DomainError::Conflict(message)) => {
            assert!(message.contains("déjà présent"), "{message}");
        }
        other => panic!("erreur inattendue : {other}"),
    }
}

#[tokio::test]
async fn media_asset_inexistant_refuse() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let item = common::sample_meme(&guild, MediaAssetId::new_v4(), now);

    let error = db.meme_board().create(&item).await.unwrap_err();
    assert!(
        error.is_foreign_key_violation(),
        "la FK media_asset_id doit être vérifiée : {error}"
    );
}
