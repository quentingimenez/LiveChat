//! Contrat d'ouverture de la base : migration, idempotence, pragmas,
//! intégrité référentielle et vérification de version.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use lc_db::{Db, DbConfig, DbError, SCHEMA_VERSION};
use lc_domain::{MediaAssetId, MediaBoardItemId};

#[tokio::test]
async fn migration_fraiche_puis_idempotente() {
    let (db, dir) = common::temp_db().await;
    let path = dir.path().join("livechat.db");

    assert_eq!(
        db.get_metadata("schema_version").await.unwrap().as_deref(),
        Some(SCHEMA_VERSION)
    );

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations' \
         ORDER BY name",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    for expected in [
        "app_metadata",
        "discord_link_codes",
        "guild_invites",
        "guild_members",
        "guilds",
        "ingest_clients",
        "media_assets",
        "media_board_items",
        "overlay_clients",
        "pairing_codes",
        "playback_jobs",
    ] {
        assert!(
            tables.iter().any(|table| table == expected),
            "table `{expected}` absente (tables : {tables:?})"
        );
    }
    assert_eq!(tables.len(), 11, "tables inattendues : {tables:?}");

    // L'invariant « un seul PLAYING par guilde » est porté par un index unique
    // partiel créé par la migration 0002.
    let index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
         AND name = 'idx_playback_jobs_single_playing'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        index_count, 1,
        "la migration 0002 doit créer l'index unique partiel de l'invariant PLAYING"
    );

    // Relancer les migrations ne rejoue rien et ne casse rien.
    db.migrate().await.unwrap();
    db.migrate().await.unwrap();
    assert_eq!(
        db.get_metadata("schema_version").await.unwrap().as_deref(),
        Some(SCHEMA_VERSION)
    );

    // Rouvrir une base existante réapplique zéro migration et retrouve la version.
    let reopened = Db::open(&path, &DbConfig::default()).await.unwrap();
    assert_eq!(
        reopened
            .get_metadata("schema_version")
            .await
            .unwrap()
            .as_deref(),
        Some(SCHEMA_VERSION)
    );
}

#[tokio::test]
async fn pragmas_du_contrat_sont_actifs() {
    let (db, _dir) = common::temp_db().await;

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);

    let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(busy_timeout, 5_000);

    // `synchronous = NORMAL` (1) et non FULL (2).
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(synchronous, 1);
}

#[tokio::test]
async fn version_de_schema_inattendue_detectee() {
    let (db, _dir) = common::temp_db().await;

    db.set_metadata("schema_version", "2.9.9").await.unwrap();
    let error = db.ensure_schema_version().await.unwrap_err();
    match error {
        DbError::SchemaVersion { expected, found } => {
            assert_eq!(expected, SCHEMA_VERSION);
            assert_eq!(found, "2.9.9");
        }
        other => panic!("erreur inattendue : {other}"),
    }

    sqlx::query("DELETE FROM app_metadata WHERE key = 'schema_version'")
        .execute(db.pool())
        .await
        .unwrap();
    let error = db.ensure_schema_version().await.unwrap_err();
    match error {
        DbError::SchemaVersion { expected, found } => {
            assert_eq!(expected, SCHEMA_VERSION);
            assert_eq!(found, "<absente>");
        }
        other => panic!("erreur inattendue : {other}"),
    }
}

#[tokio::test]
async fn violation_de_cle_etrangere_rejetee() {
    let (db, _dir) = common::temp_db().await;

    let error = sqlx::query(
        "INSERT INTO media_board_items (id, guild_id, media_asset_id, created_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(MediaBoardItemId::new_v4().to_canonical())
    .bind("123456789012345678")
    .bind(MediaAssetId::new_v4().to_canonical())
    .bind(common::NOW_MS)
    .execute(db.pool())
    .await
    .unwrap_err();

    let error = DbError::from(error);
    assert!(
        error.is_foreign_key_violation(),
        "la FK media_asset_id aurait dû être refusée : {error}"
    );
}

#[tokio::test]
async fn violation_de_contrainte_check_rejetee() {
    let (db, _dir) = common::temp_db().await;

    let error = sqlx::query(
        "INSERT INTO media_assets ( \
            id, source_hash, source_url, kind, mime, storage_path, \
            last_accessed_at, expires_at, created_at \
         ) VALUES (?, ?, ?, 'GIF', 'image/gif', 'x', 0, 0, 0)",
    )
    .bind(MediaAssetId::new_v4().to_canonical())
    .bind("check-hash")
    .bind("https://example.test/x.gif")
    .execute(db.pool())
    .await
    .unwrap_err();
    assert!(
        DbError::from(error).is_check_violation(),
        "kind='GIF' viole le CHECK du contrat"
    );

    let error = sqlx::query(
        "INSERT INTO media_assets ( \
            id, source_hash, source_url, kind, mime, is_vertical, storage_path, \
            last_accessed_at, expires_at, created_at \
         ) VALUES (?, ?, ?, 'IMAGE', 'image/png', 2, 'x', 0, 0, 0)",
    )
    .bind(MediaAssetId::new_v4().to_canonical())
    .bind("check-hash-2")
    .bind("https://example.test/y.png")
    .execute(db.pool())
    .await
    .unwrap_err();
    assert!(
        DbError::from(error).is_check_violation(),
        "is_vertical=2 viole le CHECK 0/1"
    );
}

#[tokio::test]
async fn dossier_parent_cree_et_urls_acceptees() {
    let dir = tempfile::tempdir().unwrap();

    // `open` crée les dossiers parents.
    let nested = dir.path().join("data").join("nested").join("livechat.db");
    let db = Db::open(&nested, &DbConfig::default()).await.unwrap();
    assert!(nested.exists(), "le fichier SQLite n'a pas été créé");
    db.close().await;

    // `connect` accepte une URL `file:` façon Prisma et crée aussi le dossier.
    let url_path = dir
        .path()
        .join("url")
        .join("prisma-style")
        .join("livechat.db");
    let url = format!("file:{}", url_path.display());
    let db = Db::connect(&url, &DbConfig::default()).await.unwrap();
    assert!(
        url_path.exists(),
        "le fichier SQLite n'a pas été créé via URL"
    );
    assert_eq!(
        db.get_metadata("schema_version").await.unwrap().as_deref(),
        Some(SCHEMA_VERSION)
    );
}

#[tokio::test]
async fn base_en_memoire_utilisable() {
    let db = common::memory_db().await;
    assert_eq!(
        db.get_metadata("schema_version").await.unwrap().as_deref(),
        Some(SCHEMA_VERSION)
    );
    assert_eq!(db.config().max_connections, 1);
}
