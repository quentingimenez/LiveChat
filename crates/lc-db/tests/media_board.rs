//! Tests du repository `media_board_items` : CRUD, unicité par guilde, FK et
//! migration 0005.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::borrow::Cow;

use lc_db::{Db, DbConfig, DbError};
use lc_domain::{DomainError, MediaAssetId, MediaBoardItemId};
use sqlx::migrate::{Migration, Migrator};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn crud_media_board() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let other_guild = common::guild("987654321098765432");
    let asset = common::sample_asset("media-asset", now);
    db.media_assets().insert(&asset).await.unwrap();

    let repo = db.media_board();
    let item = common::sample_board_item(&guild, asset.id, now);
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
        .update(MediaBoardItemId::new_v4(), None, None)
        .await
        .unwrap());

    // Liste par guilde triée du plus récent au plus ancien.
    let later_asset = common::sample_asset("media-asset-2", now);
    db.media_assets().insert(&later_asset).await.unwrap();
    let later_item =
        common::sample_board_item(&guild, later_asset.id, common::date(common::NOW_MS + 1_000));
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
    let asset = common::sample_asset("media-unique", now);
    db.media_assets().insert(&asset).await.unwrap();

    let repo = db.media_board();
    repo.create(&common::sample_board_item(&guild, asset.id, now))
        .await
        .unwrap();

    let another = common::sample_board_item(&guild, asset.id, now);
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
    let item = common::sample_board_item(&guild, MediaAssetId::new_v4(), now);

    let error = db.media_board().create(&item).await.unwrap_err();
    assert!(
        error.is_foreign_key_violation(),
        "la FK media_asset_id doit être vérifiée : {error}"
    );
}

/// Migration 0005 : une base arrêtée à 0004 (données écrites dans l'ancienne
/// table) est migrée par sqlx, puis l'élément est relu via `Db::media_board()`.
#[tokio::test]
async fn migration_0005_renomme_la_table_et_conserve_les_elements() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade-0005.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();

    // Migrations 0001..0004 uniquement, avec le registre sqlx complet, pour que
    // `Db::open` n'applique ensuite que 0005 (même technique que le migrateur
    // `xtask`, qui contrôle les checksums déjà enregistrés).
    let all = sqlx::migrate!("./migrations");
    let migrations: Vec<Migration> = all
        .iter()
        .filter(|migration| migration.version <= 4)
        .cloned()
        .collect();
    let partial = Migrator {
        migrations: Cow::Owned(migrations),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    partial.run(&pool).await.unwrap();

    let guild = common::guild("123456789012345678");
    let asset = common::sample_asset("migration-0005-asset", common::now());
    sqlx::query(
        "INSERT INTO media_assets ( \
            id, source_hash, source_url, kind, mime, duration_sec, width, height, \
            is_vertical, storage_path, size_bytes, status, error, last_accessed_at, \
            expires_at, created_at \
         ) VALUES (?, ?, ?, 'VIDEO', 'video/mp4', 12, 1920, 1080, 0, ?, 1024, 'READY', NULL, ?, ?, ?)",
    )
    .bind(asset.id.to_canonical())
    .bind(&asset.source_hash)
    .bind(&asset.source_url)
    .bind(&asset.storage_path)
    .bind(common::NOW_MS)
    .bind(common::NOW_MS)
    .bind(common::NOW_MS)
    .execute(&pool)
    .await
    .unwrap();

    let item_id = MediaBoardItemId::new_v4();
    sqlx::query(
        "INSERT INTO meme_board_items ( \
            id, guild_id, media_asset_id, title, message, \
            created_by_discord_user_id, created_by_name, created_at \
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(item_id.to_canonical())
    .bind(guild.as_str())
    .bind(asset.id.to_canonical())
    .bind("Titre migration")
    .bind("Message migration")
    .bind("42")
    .bind("Auteur")
    .bind(common::NOW_MS)
    .execute(&pool)
    .await
    .unwrap();

    pool.close().await;

    // Ouvre la base « 3.0.0 » : sqlx applique la migration 0005.
    let db = Db::open(&path, &DbConfig::default()).await.unwrap();

    let old_table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'meme_board_items'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(old_table, 0, "l'ancienne table ne doit plus exister");

    let new_indexes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
         AND name IN ('idx_media_board_items_guild_created', 'idx_media_board_items_media_asset')",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(new_indexes, 2, "les deux index doivent être recréés");

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(versions, vec![1, 2, 3, 4, 5, 6, 7]);

    let item = db
        .media_board()
        .get(item_id)
        .await
        .unwrap()
        .expect("l'élément 0004 doit survivre au renommage");
    assert_eq!(item.id, item_id);
    assert_eq!(item.guild_id, guild);
    assert_eq!(item.title.as_deref(), Some("Titre migration"));
    assert_eq!(item.message.as_deref(), Some("Message migration"));

    db.close().await;
}
