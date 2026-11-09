//! Tests d'intégration du migrateur, sur des bases SQLite de synthèse créées en
//! SQL brut (aucune fixture binaire versionnée).
//!
//! Couverture (contrat task-8) :
//! - lecture seule de la source et normalisation des dates INTEGER/TEXT/RFC 3339 ;
//! - `--dry-run` n'écrit rien et ne modifie pas la source (comparaison SHA256) ;
//! - migration réelle complète, vérifiée et installée ;
//! - date invalide (`T` sans fuseau, `REAL`) => erreur détaillée, aucun fichier final ;
//! - booléen et énumération invalides rejetés ;
//! - cible existante refusée sans `--force`, remplacée avec `--force` ;
//! - fichier `<to>.tmp` résiduel refusé sans `--force` ;
//! - source et cible identiques refusées ;
//! - base laissée en mode WAL lue sans corruption.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{Connection, SqliteConnection};
use tempfile::{tempdir, TempDir};

use super::{execute, target, Error, Options};

/// DDL des 7 tables Prisma legacy, dans la forme produite par `prisma migrate`
/// (types `TEXT`/`INTEGER`/`DATETIME`, colonnes camelCase, FK).
const LEGACY_DDL: &str = r#"
CREATE TABLE "MediaAsset" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "sourceHash" TEXT NOT NULL,
    "sourceUrl" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "mime" TEXT NOT NULL,
    "durationSec" INTEGER,
    "width" INTEGER,
    "height" INTEGER,
    "isVertical" INTEGER NOT NULL DEFAULT 0,
    "storagePath" TEXT NOT NULL,
    "sizeBytes" INTEGER NOT NULL DEFAULT 0,
    "status" TEXT NOT NULL DEFAULT 'PROCESSING',
    "error" TEXT,
    "lastAccessedAt" DATETIME NOT NULL,
    "expiresAt" DATETIME NOT NULL,
    "createdAt" DATETIME NOT NULL
);

CREATE TABLE "MemeBoardItem" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "mediaAssetId" TEXT NOT NULL,
    "title" TEXT,
    "message" TEXT,
    "createdByDiscordUserId" TEXT,
    "createdByName" TEXT,
    "createdAt" DATETIME NOT NULL
);

CREATE TABLE "PlaybackJob" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "mediaAssetId" TEXT,
    "text" TEXT,
    "showText" INTEGER NOT NULL DEFAULT 0,
    "authorName" TEXT,
    "authorImage" TEXT,
    "durationSec" INTEGER NOT NULL,
    "priority" INTEGER NOT NULL DEFAULT 0,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "resumesAfterJobId" TEXT,
    "resumeOffsetSec" INTEGER DEFAULT 0,
    "remainingMsSnapshot" INTEGER,
    "lastPlaybackStateAt" DATETIME,
    "submissionDate" DATETIME NOT NULL,
    "executionDate" DATETIME NOT NULL,
    "scheduledAt" DATETIME NOT NULL,
    "startedAt" DATETIME,
    "finishedAt" DATETIME
);

CREATE TABLE "Guild" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "name" TEXT,
    "busyUntil" DATETIME,
    "defaultMediaTime" INTEGER,
    "maxMediaTime" INTEGER,
    "displayMediaFull" INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE "OverlayClient" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "label" TEXT NOT NULL,
    "tokenHash" TEXT NOT NULL,
    "sessionMode" TEXT NOT NULL DEFAULT 'NORMAL',
    "defaultAuthorName" TEXT,
    "defaultAuthorImage" TEXT,
    "createdByDiscordUserId" TEXT,
    "lastSeenAt" DATETIME,
    "createdAt" DATETIME NOT NULL,
    "revokedAt" DATETIME
);

CREATE TABLE "IngestClient" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "label" TEXT NOT NULL,
    "tokenHash" TEXT NOT NULL,
    "defaultAuthorName" TEXT NOT NULL,
    "defaultAuthorImage" TEXT,
    "createdByDiscordUserId" TEXT NOT NULL,
    "lastSeenAt" DATETIME,
    "createdAt" DATETIME NOT NULL,
    "revokedAt" DATETIME
);

CREATE TABLE "PairingCode" (
    "code" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "mode" TEXT NOT NULL DEFAULT 'NORMAL',
    "createdByDiscordUserId" TEXT NOT NULL,
    "authorName" TEXT,
    "authorImage" TEXT,
    "expiresAt" DATETIME,
    "usedAt" DATETIME,
    "createdAt" DATETIME NOT NULL
);
"#;

/// Jeu de données representative : dates INTEGER, TEXT espace UTC, RFC 3339,
/// NULL nullable, casse d'énumérations mélangée, `resumeOffsetSec` NULL.
const LEGACY_SEED: &str = r#"
INSERT INTO "MediaAsset" (
    "id", "sourceHash", "sourceUrl", "kind", "mime", "durationSec", "width",
    "height", "isVertical", "storagePath", "sizeBytes", "status", "error",
    "lastAccessedAt", "expiresAt", "createdAt"
) VALUES
    ('m1', 'hash-m1', 'https://cdn.example/m1.mp4', 'video', 'video/mp4', 42,
     1920, 1080, 1, 'media/m1.mp4', 1048576, 'READY', NULL,
     1704067200000, 1735689600000, 1704067200000),
    ('m2', 'hash-m2', 'https://cdn.example/m2.mp3', 'AUDIO', 'audio/mpeg', NULL,
     NULL, NULL, 0, 'media/m2.mp3', 2048, 'PROCESSING', 'en attente',
     '2024-03-01 12:00:00', '2025-03-01T12:00:00Z', '2024-03-01 12:00:00');

INSERT INTO "MemeBoardItem" (
    "id", "guildId", "mediaAssetId", "title", "message",
    "createdByDiscordUserId", "createdByName", "createdAt"
) VALUES
    ('mb1', 'g1', 'm1', NULL, 'hello', 'u1', 'Alice', 1704067200000),
    ('mb2', 'g1', 'm2', 'titre', NULL, NULL, NULL, '2024-03-01 12:00:00');

INSERT INTO "PlaybackJob" (
    "id", "guildId", "mediaAssetId", "text", "showText", "authorName",
    "authorImage", "durationSec", "priority", "status", "resumesAfterJobId",
    "resumeOffsetSec", "remainingMsSnapshot", "lastPlaybackStateAt",
    "submissionDate", "executionDate", "scheduledAt", "startedAt", "finishedAt"
) VALUES
    ('p1', 'g1', 'm1', 'message à afficher', 1, 'Alice', NULL, 42, 5, 'PLAYING',
     NULL, NULL, 12345, 1704067260000, 1704067200000, 1704067210000,
     1704067200000, 1704067205000, NULL),
    ('p2', 'g1', NULL, NULL, 0, NULL, NULL, 10, 0, 'pending', NULL, 7, NULL,
     NULL, '2024-03-01 12:00:00', '2024-03-01T12:00:00Z', 1709294400000,
     NULL, NULL);

INSERT INTO "Guild" (
    "id", "name", "busyUntil", "defaultMediaTime", "maxMediaTime", "displayMediaFull"
) VALUES
    ('g1', 'Guilde A', 1704067200000, 300, 600, 1),
    ('g2', NULL, '2024-03-01 12:00:00', NULL, NULL, 0);

INSERT INTO "OverlayClient" (
    "id", "guildId", "label", "tokenHash", "sessionMode", "defaultAuthorName",
    "defaultAuthorImage", "createdByDiscordUserId", "lastSeenAt", "createdAt",
    "revokedAt"
) VALUES
    ('o1', 'g1', 'salon', 'hash-o1', 'INVITE_READ_ONLY', 'Alice', NULL, 'u1',
     1704067200000, 1704067200000, NULL),
    ('o2', 'g1', 'normal', 'hash-o2', 'normal', NULL, NULL, NULL,
     '2024-03-01T12:00:00Z', '2024-03-01 12:00:00', NULL);

INSERT INTO "IngestClient" (
    "id", "guildId", "label", "tokenHash", "defaultAuthorName",
    "defaultAuthorImage", "createdByDiscordUserId", "lastSeenAt", "createdAt",
    "revokedAt"
) VALUES
    ('i1', 'g1', 'extension', 'hash-i1', 'Extension', NULL, 'u2', NULL,
     1704067200000, NULL);

INSERT INTO "PairingCode" (
    "code", "guildId", "mode", "createdByDiscordUserId", "authorName",
    "authorImage", "expiresAt", "usedAt", "createdAt"
) VALUES
    ('ABCD1234', 'g1', 'NORMAL', 'u1', 'Alice', NULL, 1704153600000, NULL,
     1704067200000),
    ('WXYZ5678', 'g1', 'invite_read_only', 'u2', NULL, NULL, NULL,
     '2024-03-01 12:00:00', '2024-03-01 12:00:00');
"#;

/// Date RFC 3339 -> millisecondes epoch UTC (référence indépendante du code testé).
fn ms(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .expect("date de référence valide")
        .timestamp_millis()
}

/// Options prêtes à exécuter.
fn options(from: &Path, to: &Path, dry_run: bool, force: bool) -> Options {
    Options {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        dry_run,
        force,
    }
}

/// Crée une base legacy vide avec le DDL Prisma et renvoie son chemin.
async fn new_legacy(dir: &TempDir) -> (PathBuf, SqliteConnection) {
    let path = dir.path().join("legacy.sqlite");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Delete);
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .expect("ouverture de la fixture legacy");
    sqlx::raw_sql(LEGACY_DDL)
        .execute(&mut conn)
        .await
        .expect("DDL legacy");
    (path, conn)
}

/// Base legacy avec le jeu de données standard.
async fn seeded_legacy(dir: &TempDir) -> (PathBuf, SqliteConnection) {
    let (path, mut conn) = new_legacy(dir).await;
    sqlx::raw_sql(LEGACY_SEED)
        .execute(&mut conn)
        .await
        .expect("insertion des données legacy");
    (path, conn)
}

/// Exécute un ordre SQL brut sur une connexion de test.
async fn exec(conn: &mut SqliteConnection, sql: &str) {
    sqlx::raw_sql(sql)
        .execute(conn)
        .await
        .expect("SQL de fixture");
}

/// Ouvre un fichier existant en lecture seule (vérifications post-migration).
async fn open_readonly(path: &Path) -> SqliteConnection {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false);
    SqliteConnection::connect_with(&options)
        .await
        .expect("ouverture en lecture seule")
}

async fn scalar_i64(conn: &mut SqliteConnection, sql: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(conn)
        .await
        .expect("requête scalaire i64")
}

async fn scalar_opt_i64(conn: &mut SqliteConnection, sql: &str) -> Option<i64> {
    sqlx::query_scalar::<_, Option<i64>>(sql)
        .fetch_one(conn)
        .await
        .expect("requête scalaire i64 optionnelle")
}

async fn scalar_text(conn: &mut SqliteConnection, sql: &str) -> String {
    sqlx::query_scalar::<_, String>(sql)
        .fetch_one(conn)
        .await
        .expect("requête scalaire texte")
}

/// Empreinte SHA256 d'un fichier, via le helper de `legacy.rs`.
fn sha256(path: &Path) -> String {
    super::legacy::hash_file(path).expect("empreinte SHA256")
}

/// Vérifie qu'aucun fichier final ni temporaire n'a été créé.
fn assert_rien_ecrit(to: &Path) {
    assert!(
        !to.exists(),
        "le fichier final ne doit pas exister : {}",
        to.display()
    );
    assert!(
        !target::tmp_path(to).exists(),
        "le fichier temporaire ne doit pas exister : {}",
        target::tmp_path(to).display()
    );
}

#[tokio::test]
async fn migration_complete_installe_et_normalise() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, legacy) = seeded_legacy(&dir).await;
    let source_hash = sha256(&from);
    let to = dir.path().join("v3.sqlite");

    execute(&options(&from, &to, false, false))
        .await
        .expect("migration complète");

    assert!(to.is_file(), "la cible doit être installée");
    assert!(
        !target::tmp_path(&to).exists(),
        "le temporaire doit avoir disparu"
    );
    assert_eq!(sha256(&from), source_hash, "la source doit rester intacte");
    drop(legacy);

    let mut target = open_readonly(&to).await;
    // Comptes par table.
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM media_assets").await,
        2
    );
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM meme_board_items").await,
        2
    );
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM playback_jobs").await,
        2
    );
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM guilds").await,
        2
    );
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM overlay_clients").await,
        2
    );
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM ingest_clients").await,
        1
    );
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM pairing_codes").await,
        2
    );

    // Version de schéma et métadonnées de traçabilité.
    assert_eq!(
        scalar_text(
            &mut target,
            "SELECT value FROM app_metadata WHERE key = 'schema_version'"
        )
        .await,
        "3.0.0"
    );
    assert_eq!(
        scalar_text(
            &mut target,
            "SELECT value FROM app_metadata WHERE key = 'legacy_source_sha256'"
        )
        .await,
        source_hash
    );
    assert_eq!(
        scalar_text(
            &mut target,
            "SELECT value FROM app_metadata WHERE key = 'legacy_import_tool'"
        )
        .await,
        concat!("xtask ", env!("CARGO_PKG_VERSION"), " migrate-legacy")
    );

    // Dates INTEGER conservées.
    assert_eq!(
        scalar_i64(
            &mut target,
            "SELECT created_at FROM media_assets WHERE id = 'm1'"
        )
        .await,
        ms("2024-01-01T00:00:00Z")
    );
    // TEXT `YYYY-MM-DD HH:MM:SS` interprété en UTC.
    assert_eq!(
        scalar_i64(
            &mut target,
            "SELECT created_at FROM media_assets WHERE id = 'm2'"
        )
        .await,
        ms("2024-03-01T12:00:00Z")
    );
    // RFC 3339 avec `Z`.
    assert_eq!(
        scalar_i64(
            &mut target,
            "SELECT expires_at FROM media_assets WHERE id = 'm2'"
        )
        .await,
        ms("2025-03-01T12:00:00Z")
    );
    // RFC 3339 avec offset.
    assert_eq!(
        scalar_i64(
            &mut target,
            "SELECT submission_date FROM playback_jobs WHERE id = 'p2'"
        )
        .await,
        ms("2024-03-01T12:00:00Z")
    );

    // Énumérations normalisées (minuscules legacy -> majuscules v3).
    assert_eq!(
        scalar_text(&mut target, "SELECT kind FROM media_assets WHERE id = 'm1'").await,
        "VIDEO"
    );
    assert_eq!(
        scalar_text(
            &mut target,
            "SELECT status FROM playback_jobs WHERE id = 'p2'"
        )
        .await,
        "PENDING"
    );
    assert_eq!(
        scalar_text(
            &mut target,
            "SELECT session_mode FROM overlay_clients WHERE id = 'o2'"
        )
        .await,
        "NORMAL"
    );
    assert_eq!(
        scalar_text(
            &mut target,
            "SELECT mode FROM pairing_codes WHERE code = 'WXYZ5678'"
        )
        .await,
        "INVITE_READ_ONLY"
    );

    // `resumeOffsetSec` NULL remplacé par 0 ; valeur non NULL conservée.
    assert_eq!(
        scalar_i64(
            &mut target,
            "SELECT resume_offset_sec FROM playback_jobs WHERE id = 'p1'"
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            &mut target,
            "SELECT resume_offset_sec FROM playback_jobs WHERE id = 'p2'"
        )
        .await,
        7
    );

    // Booléens legacy 0/1 conservés.
    assert_eq!(
        scalar_opt_i64(&mut target, "SELECT busy_until FROM guilds WHERE id = 'g2'").await,
        Some(ms("2024-03-01T12:00:00Z"))
    );
}

#[tokio::test]
async fn dry_run_n_ecrit_rien_et_source_intacte() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, legacy) = seeded_legacy(&dir).await;
    let source_hash = sha256(&from);
    let to = dir.path().join("v3.sqlite");

    execute(&options(&from, &to, true, false))
        .await
        .expect("dry-run");

    assert_rien_ecrit(&to);
    assert_eq!(sha256(&from), source_hash, "la source doit rester intacte");
    drop(legacy);
}

#[tokio::test]
async fn date_t_sans_fuseau_rejetee_sans_fichier_final() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, mut legacy) = seeded_legacy(&dir).await;
    exec(
        &mut legacy,
        r#"UPDATE "MediaAsset" SET "createdAt" = '2024-01-01T00:00:00' WHERE id = 'm1';"#,
    )
    .await;
    let to = dir.path().join("v3.sqlite");

    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("une date `T` sans fuseau doit être rejetée");
    let message = err.to_string();
    match &err {
        Error::InvalidDate {
            table,
            column,
            row,
            row_id,
            ..
        } => {
            assert_eq!(*table, "MediaAsset");
            assert_eq!(*column, "createdAt");
            assert!(*row >= 1);
            assert_eq!(row_id, "m1");
        }
        other => panic!("InvalidDate attendu, obtenu : {other:?}"),
    }
    assert!(message.contains("MediaAsset"), "message : {message}");
    assert!(message.contains("createdAt"), "message : {message}");
    assert!(message.contains("ligne 1"), "message : {message}");
    assert_rien_ecrit(&to);
    drop(legacy);
}

#[tokio::test]
async fn date_reelle_ou_fraction_espace_rejetee() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, mut legacy) = seeded_legacy(&dir).await;
    exec(
        &mut legacy,
        r#"UPDATE "MediaAsset" SET "lastAccessedAt" = 1704067200.5 WHERE id = 'm1';"#,
    )
    .await;
    let to = dir.path().join("v3.sqlite");

    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("un REAL ne doit pas être accepté comme date");
    let message = err.to_string();
    assert!(message.contains("REAL"), "message : {message}");
    assert!(message.contains("lastAccessedAt"), "message : {message}");
    assert_rien_ecrit(&to);

    exec(
        &mut legacy,
        r#"UPDATE "MediaAsset" SET "lastAccessedAt" = 1704067200000, "createdAt" = '2024-01-01 00:00:00.123' WHERE id = 'm1';"#,
    )
    .await;
    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("une fraction après espace doit être rejetée");
    assert!(err.to_string().contains("createdAt"), "message : {err}");
    assert_rien_ecrit(&to);
    drop(legacy);
}

#[tokio::test]
async fn booleen_invalide_rejete() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, mut legacy) = seeded_legacy(&dir).await;
    exec(
        &mut legacy,
        r#"UPDATE "Guild" SET "displayMediaFull" = 2 WHERE id = 'g1';"#,
    )
    .await;
    let to = dir.path().join("v3.sqlite");

    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("un booléen legacy hors 0/1 doit être rejeté");
    match err {
        Error::InvalidBoolean {
            table,
            column,
            value,
            ..
        } => {
            assert_eq!(table, "Guild");
            assert_eq!(column, "displayMediaFull");
            assert_eq!(value, 2);
        }
        other => panic!("InvalidBoolean attendu, obtenu : {other:?}"),
    }
    assert_rien_ecrit(&to);
    drop(legacy);
}

#[tokio::test]
async fn enumeration_invalide_rejetee() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, mut legacy) = seeded_legacy(&dir).await;
    exec(
        &mut legacy,
        r#"UPDATE "OverlayClient" SET "sessionMode" = 'BOGUS' WHERE id = 'o1';"#,
    )
    .await;
    let to = dir.path().join("v3.sqlite");

    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("une valeur d'énumération hors contrat doit être rejetée");
    match err {
        Error::InvalidEnum {
            table,
            column,
            value,
            allowed,
            ..
        } => {
            assert_eq!(table, "OverlayClient");
            assert_eq!(column, "sessionMode");
            assert_eq!(value, "BOGUS");
            assert!(allowed.contains("NORMAL"));
            assert!(allowed.contains("INVITE_READ_ONLY"));
        }
        other => panic!("InvalidEnum attendu, obtenu : {other:?}"),
    }
    assert_rien_ecrit(&to);
    drop(legacy);
}

#[tokio::test]
async fn cible_existante_refusee_sans_force_puis_remplacee() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, legacy) = seeded_legacy(&dir).await;
    let to = dir.path().join("v3.sqlite");
    std::fs::write(&to, b"ancienne cible").expect("écriture de l'ancienne cible");

    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("sans --force, une cible existante doit être refusée");
    match err {
        Error::TargetExists(path) => assert_eq!(path, to),
        other => panic!("TargetExists attendu, obtenu : {other:?}"),
    }
    assert_eq!(
        std::fs::read(&to).expect("lecture cible"),
        b"ancienne cible"
    );

    execute(&options(&from, &to, false, true))
        .await
        .expect("--force remplace la cible");
    let mut target = open_readonly(&to).await;
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM guilds").await,
        2
    );
    assert!(!super::append_suffix(&to, ".old").exists());
    drop(legacy);
}

#[tokio::test]
async fn temporaire_residuel_refuse_sans_force() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, legacy) = seeded_legacy(&dir).await;
    let to = dir.path().join("v3.sqlite");
    let tmp = target::tmp_path(&to);
    std::fs::write(&tmp, b"residuel").expect("écriture du temporaire");

    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("un temporaire résiduel doit être refusé");
    match err {
        Error::TmpExists(path) => assert_eq!(path, tmp),
        other => panic!("TmpExists attendu, obtenu : {other:?}"),
    }
    assert!(!to.exists());

    execute(&options(&from, &to, false, true))
        .await
        .expect("--force remplace le temporaire résiduel");
    assert!(to.is_file());
    assert!(!tmp.exists());
    drop(legacy);
}

#[tokio::test]
async fn source_et_cible_identiques_refusees() {
    let dir = tempdir().expect("répertoire temporaire");
    let (from, legacy) = seeded_legacy(&dir).await;
    let source_hash = sha256(&from);

    let err = execute(&options(&from, &from, false, false))
        .await
        .expect_err("source et cible identiques doivent être refusées");
    assert!(matches!(err, Error::SameSourceAndTarget(_)));
    assert_eq!(sha256(&from), source_hash);
    drop(legacy);
}

#[tokio::test]
async fn source_absente_refusee() {
    let dir = tempdir().expect("répertoire temporaire");
    let from = dir.path().join("absent.sqlite");
    let to = dir.path().join("v3.sqlite");
    let err = execute(&options(&from, &to, false, false))
        .await
        .expect_err("une source absente doit être refusée");
    assert!(matches!(err, Error::LegacyNotFound(_)));
}

#[tokio::test]
async fn base_en_wal_lue_sans_corruption() {
    let dir = tempdir().expect("répertoire temporaire");
    let from = dir.path().join("legacy.sqlite");
    let options = SqliteConnectOptions::new()
        .filename(&from)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let mut writer = SqliteConnection::connect_with(&options)
        .await
        .expect("ouverture de la fixture WAL");
    sqlx::raw_sql(LEGACY_DDL)
        .execute(&mut writer)
        .await
        .expect("DDL legacy");
    sqlx::raw_sql(LEGACY_SEED)
        .execute(&mut writer)
        .await
        .expect("données legacy");

    let wal = super::legacy::wal_info(&from);
    assert!(
        wal.exists,
        "le fichier -wal doit exister pendant la migration"
    );
    let source_hash = sha256(&from);
    let to = dir.path().join("v3.sqlite");

    execute(&options_ref(&from, &to))
        .await
        .expect("lecture d'une base laissée en WAL");
    assert_eq!(
        sha256(&from),
        source_hash,
        "la source WAL doit rester intacte"
    );
    let mut target = open_readonly(&to).await;
    assert_eq!(
        scalar_i64(&mut target, "SELECT COUNT(*) FROM media_assets").await,
        2
    );
    drop(writer);
}

/// Alias d'options pour le test WAL (nom de variable `options` déjà pris).
fn options_ref(from: &Path, to: &Path) -> Options {
    options(from, to, false, false)
}

/// Benchmark manuel (ignoré par défaut) : 20 000 jobs pour extrapoler le temps de
/// migration sur une base legacy de taille réelle.
///
/// `cargo test -p xtask -- --ignored benchmark_volume --nocapture`
#[tokio::test]
#[ignore = "benchmark manuel : génère 20 000 lignes"]
async fn benchmark_volume() {
    const ROWS: usize = 20_000;

    let dir = tempdir().expect("répertoire temporaire");
    let (from, mut legacy) = seeded_legacy(&dir).await;
    let mut tx = legacy.begin().await.expect("transaction fixture");
    for index in 0..ROWS {
        sqlx::query(
            r#"INSERT INTO "PlaybackJob" (
                "id", "guildId", "mediaAssetId", "text", "showText", "authorName",
                "authorImage", "durationSec", "priority", "status",
                "resumesAfterJobId", "resumeOffsetSec", "remainingMsSnapshot",
                "lastPlaybackStateAt", "submissionDate", "executionDate",
                "scheduledAt", "startedAt", "finishedAt"
            ) VALUES (?, 'g1', NULL, 'bench', 0, NULL, NULL, 10, 0, 'PENDING',
                NULL, 0, NULL, NULL, 1704067200000, 1704067200000,
                1704067200000, NULL, NULL)"#,
        )
        .bind(format!("bench-{index}"))
        .execute(&mut *tx)
        .await
        .expect("insertion du jeu de benchmark");
    }
    tx.commit().await.expect("commit fixture");
    drop(legacy);

    let to = dir.path().join("v3-grande.sqlite");
    let started = std::time::Instant::now();
    execute(&options(&from, &to, false, false))
        .await
        .expect("migration de la grande base");
    let elapsed = started.elapsed();
    let total_rows = ROWS + 13;
    let per_row = elapsed.as_secs_f64() / total_rows as f64;
    println!(
        "benchmark : {total_rows} lignes en {:.2} s ({:.0} lignes/s, {:.3} ms/ligne)",
        elapsed.as_secs_f64(),
        total_rows as f64 / elapsed.as_secs_f64(),
        per_row * 1000.0
    );
    assert!(to.is_file());
}
