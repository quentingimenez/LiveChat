//! Tests de l'ouverture en lecture seule (`?mode=ro`).
//!
//! AUD-A5-012 : `Db::connect` acceptait `mode=ro` sans l'appliquer. Ces tests
//! vérifient que la demande est honorée : pas de migration, pas de passage en
//! WAL, et toute écriture est refusée par SQLite (`SQLITE_READONLY`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;

use lc_db::{Db, DbConfig, DbError, SCHEMA_VERSION};
use sqlx::Connection;

/// Prépare une base migrée en journal `DELETE`.
///
/// Une connexion strictement en lecture seule ne peut pas créer les fichiers
/// `-shm`/`-wal` : la base de test est donc créée directement en journal
/// classique (sans passer par [`Db::open`], qui impose WAL), puis le schéma
/// complet (0001 + 0002) y est appliqué.
async fn prepare_database(path: &Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete)
        .foreign_keys(true);
    let mut connection = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();

    let schema = format!(
        "{}{}",
        lc_db::MIGRATION_SQL,
        include_str!("../migrations/0002_playback_single_playing.sql")
    );
    sqlx::raw_sql(&schema)
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_metadata (key, value) VALUES ('read_only_probe', 'ok')")
        .execute(&mut connection)
        .await
        .unwrap();

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(
        journal_mode.to_ascii_lowercase(),
        "delete",
        "la base de test doit être en journal DELETE"
    );
    connection.close().await.unwrap();
}

#[tokio::test]
async fn mode_ro_autorise_la_lecture_et_refuse_les_ecritures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("read-only.db");
    prepare_database(&path).await;

    let url = format!("file:{}?mode=ro", path.display());
    let db = Db::connect(&url, &DbConfig::default())
        .await
        .expect("une base saine doit s'ouvrir en lecture seule");

    // Les lectures fonctionnent et la version est vérifiée sans écriture.
    assert_eq!(
        db.get_metadata("schema_version").await.unwrap().as_deref(),
        Some(SCHEMA_VERSION)
    );
    assert_eq!(
        db.get_metadata("read_only_probe").await.unwrap().as_deref(),
        Some("ok")
    );

    // Les migrations et la création du fichier sont désactivées pour cette
    // connexion : la configuration effective le reflète.
    assert!(!db.config().run_migrations);
    assert!(!db.config().create_if_missing);
    assert!(!db.config().create_parent_dirs);

    // Aucun passage en WAL : les pragmas d'écriture ne sont pas appliqués.
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        journal_mode.to_ascii_lowercase(),
        "delete",
        "une connexion read-only ne doit pas basculer la base en WAL"
    );

    // Toute écriture est refusée par le moteur SQLite.
    let error = sqlx::query("INSERT INTO app_metadata (key, value) VALUES ('ro', 'refusé')")
        .execute(db.pool())
        .await
        .unwrap_err();
    let error = DbError::from(error);
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains("readonly") || message.contains("read-only"),
        "l'écriture doit échouer en SQLITE_READONLY : {message}"
    );

    db.close().await;
}

#[tokio::test]
async fn mode_ro_sur_fichier_absent_ne_cree_rien() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.db");
    let url = format!("file:{}?mode=ro", path.display());

    let error = Db::connect(&url, &DbConfig::default())
        .await
        .expect_err("un fichier absent ne peut pas s'ouvrir en lecture seule");
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains("unable to open") || message.contains("no such file"),
        "erreur d'ouverture attendue : {message}"
    );
    assert!(!path.exists(), "la connexion read-only ne doit rien créer");
}
