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
        "admin_audit",
        "admin_sessions",
        "app_metadata",
        "discord_link_codes",
        "guild_ban_devices",
        "guild_bans",
        "guild_invites",
        "guild_members",
        "guilds",
        "ingest_clients",
        "media_assets",
        "media_board_favorites",
        "media_board_items",
        "overlay_clients",
        "pairing_codes",
        "playback_jobs",
        "role_rights",
        "server_flags",
    ] {
        assert!(
            tables.iter().any(|table| table == expected),
            "table `{expected}` absente (tables : {tables:?})"
        );
    }
    assert_eq!(tables.len(), 18, "tables inattendues : {tables:?}");

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
async fn pairing_code_admin_flag_default_and_check() {
    let (db, _dir) = common::temp_db().await;

    // La colonne ajoutée par la migration 0007 est optionnelle à l'insertion
    // (DEFAULT 0) et contrainte à 0/1.
    sqlx::query(
        "INSERT INTO pairing_codes ( \
            code, guild_id, mode, created_by_discord_user_id, created_at \
         ) VALUES ('DEF001', '123456789012345678', 'NORMAL', '42', ?)",
    )
    .bind(common::NOW_MS)
    .execute(db.pool())
    .await
    .unwrap();
    let grants_admin: i64 =
        sqlx::query_scalar("SELECT grants_admin FROM pairing_codes WHERE code = 'DEF001'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(grants_admin, 0, "DEFAULT 0 de grants_admin");

    let error = sqlx::query(
        "INSERT INTO pairing_codes ( \
            code, guild_id, mode, grants_admin, created_by_discord_user_id, created_at \
         ) VALUES ('BAD001', '123456789012345678', 'NORMAL', 2, '42', ?)",
    )
    .bind(common::NOW_MS)
    .execute(db.pool())
    .await
    .unwrap_err();
    assert!(
        DbError::from(error).is_check_violation(),
        "grants_admin=2 doit violer le CHECK"
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

#[tokio::test]
async fn migration_0020_vers_0021_ajoute_les_appareils_sans_perdre_les_clients() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("livechat.db");
    let all_migrations = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let old_migrations = dir.path().join("migrations-0020");
    std::fs::create_dir(&old_migrations).unwrap();
    for version in 1..=20 {
        let prefix = format!("{version:04}_");
        let file = std::fs::read_dir(&all_migrations)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
            })
            .unwrap_or_else(|| panic!("migration {prefix}* introuvable"));
        std::fs::copy(&file, old_migrations.join(file.file_name().unwrap())).unwrap();
    }

    // Base au schéma 0020, avec un client existant.
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::migrate::Migrator::new(old_migrations.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO overlay_clients ( \
            id, guild_id, label, token_hash, session_mode, default_author_name, \
            default_author_image, created_by_discord_user_id, last_seen_at, \
            created_at, revoked_at \
         ) VALUES ('client-avant-0021', '123456789012345678', 'PC', 'hash', \
            'NORMAL', NULL, NULL, NULL, NULL, ?, NULL)",
    )
    .bind(common::NOW_MS)
    .execute(&pool)
    .await
    .unwrap();
    let before: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('overlay_clients')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!before.iter().any(|name| name == "device_hash"));
    pool.close().await;

    // Ouvre la même base avec les migrations embarquées : seule 0021 manque.
    let db = Db::open(&path, &DbConfig::default()).await.unwrap();

    let device_hash: Option<String> = sqlx::query_scalar(
        "SELECT device_hash FROM overlay_clients WHERE id = 'client-avant-0021'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert!(
        device_hash.is_none(),
        "la ligne existante obtient device_hash NULL"
    );

    let strict: i64 = sqlx::query_scalar(
        "SELECT \"strict\" FROM pragma_table_list WHERE name = 'guild_ban_devices'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(strict, 1, "guild_ban_devices doit être STRICT");

    sqlx::query(
        "INSERT INTO guild_bans ( \
            id, guild_id, member_id, discord_user_id, display_name, avatar, \
            banned_by, created_at \
         ) VALUES ('ban-0021', '123456789012345678', NULL, '777', 'Exilé', \
            NULL, NULL, ?)",
    )
    .bind(common::NOW_MS)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO guild_ban_devices (ban_id, guild_id, device_hash, created_at) \
         VALUES ('ban-0021', '123456789012345678', 'empreinte', ?)",
    )
    .bind(common::NOW_MS)
    .execute(db.pool())
    .await
    .unwrap();

    // La cascade de 0021 supprime les appareils avec leur ban.
    sqlx::query("DELETE FROM guild_bans WHERE id = 'ban-0021'")
        .execute(db.pool())
        .await
        .unwrap();
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guild_ban_devices")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(remaining, 0, "la cascade doit supprimer les appareils");

    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(db.pool())
        .await
        .unwrap();
    assert!(violations.is_empty(), "PRAGMA foreign_key_check non vide");
}
