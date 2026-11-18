//! Tests du repository `media_board_items` : CRUD, unicité par guilde, FK et
//! migration 0005.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::borrow::Cow;

use lc_db::{BoardItemEdit, Db, DbConfig, DbError};
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
        .update(
            item.id,
            &BoardItemEdit {
                title: Some("nouveau titre"),
                ..BoardItemEdit::default()
            },
        )
        .await
        .unwrap());
    let updated = repo.get(item.id).await.unwrap().unwrap();
    assert_eq!(updated.title.as_deref(), Some("nouveau titre"));
    assert_eq!(updated.message, None);
    assert!(!repo
        .update(MediaBoardItemId::new_v4(), &BoardItemEdit::default())
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
async fn display_duration_round_trips() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let asset = common::sample_asset("media-asset", now);
    db.media_assets().insert(&asset).await.unwrap();
    let repo = db.media_board();
    let mut item = common::sample_board_item(&guild, asset.id, now);
    item.display_duration_sec = Some(8);
    repo.create(&item).await.unwrap();
    assert_eq!(
        repo.get(item.id)
            .await
            .unwrap()
            .unwrap()
            .display_duration_sec,
        Some(8)
    );

    assert!(repo.set_display_duration(item.id, None).await.unwrap());
    assert_eq!(
        repo.get(item.id)
            .await
            .unwrap()
            .unwrap()
            .display_duration_sec,
        None
    );
    assert!(repo.set_display_duration(item.id, Some(600)).await.unwrap());
    assert_eq!(
        repo.get(item.id)
            .await
            .unwrap()
            .unwrap()
            .display_duration_sec,
        Some(600)
    );
    assert!(!repo
        .set_display_duration(MediaBoardItemId::new_v4(), Some(8))
        .await
        .unwrap());
}

#[tokio::test]
async fn display_duration_check_rejects_out_of_range() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let asset = common::sample_asset("media-asset", now);
    db.media_assets().insert(&asset).await.unwrap();
    let repo = db.media_board();
    let item = common::sample_board_item(&guild, asset.id, now);
    repo.create(&item).await.unwrap();
    assert!(repo.set_display_duration(item.id, Some(0)).await.is_err());
    assert!(repo.set_display_duration(item.id, Some(601)).await.is_err());
}

#[tokio::test]
async fn display_scale_round_trips() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let asset = common::sample_asset("media-asset", now);
    db.media_assets().insert(&asset).await.unwrap();
    let repo = db.media_board();
    let mut item = common::sample_board_item(&guild, asset.id, now);
    item.display_scale_percent = Some(200);
    repo.create(&item).await.unwrap();
    assert_eq!(
        repo.get(item.id)
            .await
            .unwrap()
            .unwrap()
            .display_scale_percent,
        Some(200)
    );

    assert!(repo.set_display_scale(item.id, None).await.unwrap());
    assert_eq!(
        repo.get(item.id)
            .await
            .unwrap()
            .unwrap()
            .display_scale_percent,
        None
    );
    assert!(repo.set_display_scale(item.id, Some(1000)).await.unwrap());
    assert_eq!(
        repo.get(item.id)
            .await
            .unwrap()
            .unwrap()
            .display_scale_percent,
        Some(1000)
    );
    assert!(!repo
        .set_display_scale(MediaBoardItemId::new_v4(), Some(200))
        .await
        .unwrap());
}

#[tokio::test]
async fn display_scale_check_rejects_out_of_range() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let asset = common::sample_asset("media-asset", now);
    db.media_assets().insert(&asset).await.unwrap();
    let repo = db.media_board();
    let item = common::sample_board_item(&guild, asset.id, now);
    repo.create(&item).await.unwrap();
    assert!(repo.set_display_scale(item.id, Some(9)).await.is_err());
    assert!(repo.set_display_scale(item.id, Some(1001)).await.is_err());
}

#[tokio::test]
async fn update_applies_every_field_or_none() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("123456789012345678");
    let asset = common::sample_asset("media-asset", now);
    db.media_assets().insert(&asset).await.unwrap();
    let repo = db.media_board();
    let item = common::sample_board_item(&guild, asset.id, now);
    repo.create(&item).await.unwrap();

    let edit = BoardItemEdit {
        title: Some("Titre"),
        message: Some("Message"),
        artwork_on_overlay: Some(true),
        display_duration_sec: Some(Some(8)),
        display_scale_percent: Some(Some(250)),
    };
    assert!(repo.update(item.id, &edit).await.unwrap());
    let saved = repo.get(item.id).await.unwrap().unwrap();
    assert_eq!(saved.title.as_deref(), Some("Titre"));
    assert_eq!(saved.message.as_deref(), Some("Message"));
    assert!(saved.artwork_on_overlay);
    assert_eq!(saved.display_duration_sec, Some(8));
    assert_eq!(saved.display_scale_percent, Some(250));

    // Absent fields are kept.
    let keep = BoardItemEdit {
        title: Some("Autre"),
        message: None,
        artwork_on_overlay: None,
        display_duration_sec: None,
        display_scale_percent: None,
    };
    assert!(repo.update(item.id, &keep).await.unwrap());
    let saved = repo.get(item.id).await.unwrap().unwrap();
    assert_eq!(saved.title.as_deref(), Some("Autre"));
    assert!(saved.message.is_none());
    assert!(saved.artwork_on_overlay);
    assert_eq!(saved.display_duration_sec, Some(8));
    assert_eq!(saved.display_scale_percent, Some(250));

    // A value the database refuses leaves the whole item untouched.
    let refused = BoardItemEdit {
        title: Some("Jamais"),
        message: None,
        artwork_on_overlay: Some(false),
        display_duration_sec: Some(Some(0)),
        display_scale_percent: None,
    };
    assert!(repo.update(item.id, &refused).await.is_err());
    let saved = repo.get(item.id).await.unwrap().unwrap();
    assert_eq!(saved.title.as_deref(), Some("Autre"));
    assert!(saved.artwork_on_overlay);
    assert_eq!(saved.display_duration_sec, Some(8));
    assert_eq!(saved.display_scale_percent, Some(250));

    assert!(!repo
        .update(MediaBoardItemId::new_v4(), &keep)
        .await
        .unwrap());
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
async fn replace_media_keeps_the_item_and_releases_the_previous_asset() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let pinned = common::date(common::NOW_MS + 1_000_000_000);
    let released = common::date(common::NOW_MS + 5_000);
    let guild = common::guild("123456789012345678");
    let other_guild = common::guild("987654321098765432");

    let mut original = common::sample_asset("replace-original", now);
    original.expires_at = pinned;
    let mut edited = common::sample_asset("replace-edited", now);
    edited.expires_at = pinned;
    let mut taken = common::sample_asset("replace-taken", now);
    taken.expires_at = pinned;
    for asset in [&original, &edited, &taken] {
        db.media_assets().insert(asset).await.unwrap();
    }

    let repo = db.media_board();
    let mut item = common::sample_board_item(&guild, original.id, now);
    item.title = Some("Titre gardé".to_owned());
    repo.create(&item).await.unwrap();
    let other = common::sample_board_item(&guild, taken.id, now);
    repo.create(&other).await.unwrap();

    // Unknown item, or an item of another guild: nothing changes.
    assert_eq!(
        repo.replace_media(MediaBoardItemId::new_v4(), &guild, edited.id, now, released)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repo.replace_media(item.id, &other_guild, edited.id, now, released)
            .await
            .unwrap(),
        None
    );

    // The same asset: no change, nothing released.
    assert_eq!(
        repo.replace_media(item.id, &guild, original.id, now, released)
            .await
            .unwrap(),
        Some(original.id)
    );
    let untouched = db.media_assets().get(original.id).await.unwrap().unwrap();
    assert_eq!(untouched.expires_at, pinned);

    // An asset already shown by another item of the guild is refused, and the
    // transaction leaves the item as it was.
    let error = repo
        .replace_media(item.id, &guild, taken.id, now, released)
        .await
        .unwrap_err();
    assert!(
        matches!(error, DbError::Domain(DomainError::Conflict(_))),
        "{error}"
    );
    assert_eq!(
        repo.get(item.id).await.unwrap().unwrap().media_asset_id,
        original.id
    );

    // A real replacement.
    assert_eq!(
        repo.replace_media(item.id, &guild, edited.id, now, released)
            .await
            .unwrap(),
        Some(original.id)
    );
    let replaced = repo.get(item.id).await.unwrap().unwrap();
    assert_eq!(replaced.media_asset_id, edited.id);
    assert_eq!(replaced.id, item.id);
    assert_eq!(replaced.title.as_deref(), Some("Titre gardé"));
    assert_eq!(replaced.created_at, item.created_at);
    let previous = db.media_assets().get(original.id).await.unwrap().unwrap();
    assert_eq!(previous.expires_at, released, "previous asset released");
    let current = db.media_assets().get(edited.id).await.unwrap().unwrap();
    assert_eq!(current.expires_at, pinned);

    // A previous asset still used as another item's picture stays pinned.
    repo.set_artwork(other.id, Some(edited.id)).await.unwrap();
    let mut third = common::sample_asset("replace-third", now);
    third.expires_at = pinned;
    db.media_assets().insert(&third).await.unwrap();
    assert_eq!(
        repo.replace_media(item.id, &guild, third.id, now, released)
            .await
            .unwrap(),
        Some(edited.id)
    );
    let still_shown = db.media_assets().get(edited.id).await.unwrap().unwrap();
    assert_eq!(still_shown.expires_at, pinned);
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
    // The 3.0.0 database receives the later migrations too (0011 to 0014): the historical baseline stays the same.
    assert_eq!(&versions[..10], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert!(
        [11, 12, 13, 14].iter().all(|v| versions.contains(v)),
        "les migrations 0011 à 0014 doivent être appliquées : {versions:?}"
    );

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
