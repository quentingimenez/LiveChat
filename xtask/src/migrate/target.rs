//! Écriture du fichier cible 3.0, toujours dans un fichier temporaire
//! (`<to>.tmp`) : DDL de `docs/rust/schema-v3.sql`, métadonnées d'import puis
//! les 7 tables dans l'ordre des clés étrangères.
//!
//! Le fichier temporaire utilise un journal de type `DELETE` (et non `WAL`) :
//! l'écriture doit être entièrement contenue dans un seul fichier, car SQLite
//! nomme les fichiers `-wal`/`-shm` d'après le chemin de la base — un renommage
//! ultérieur perdrait ces annexes. `synchronous=FULL` garantit que les données
//! sont bien sur disque avant la vérification puis le renommage atomique.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{Connection, SqliteConnection};

use super::legacy::SourceMeta;
use super::read::LegacyData;
use super::schema::SCHEMA_V3_SQL;
use super::Error;

/// Crée le fichier `<tmp>` et y écrit le schéma 3.0, les métadonnées d'import et
/// toutes les lignes legacy déjà normalisées.
pub async fn write(tmp: &Path, data: &LegacyData, source: &SourceMeta) -> Result<(), Error> {
    let options = SqliteConnectOptions::new()
        .filename(tmp)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Delete)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(Duration::from_secs(10))
        .pragma("foreign_keys", "ON");

    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .map_err(|source| Error::TargetInit {
            path: tmp.to_path_buf(),
            source,
        })?;

    sqlx::raw_sql(SCHEMA_V3_SQL)
        .execute(&mut conn)
        .await
        .map_err(|source| Error::TargetInit {
            path: tmp.to_path_buf(),
            source,
        })?;

    insert_metadata(&mut conn, source).await?;
    insert_tables(&mut conn, data).await?;

    conn.close().await.map_err(|source| Error::TargetInit {
        path: tmp.to_path_buf(),
        source,
    })?;

    Ok(())
}

/// Ajoute les métadonnées d'import (traçabilité) à `app_metadata`.
async fn insert_metadata(conn: &mut SqliteConnection, source: &SourceMeta) -> Result<(), Error> {
    let imported_at = chrono::Utc::now().timestamp_millis().to_string();
    let source_path = source.path.display().to_string();
    let tool = concat!("xtask ", env!("CARGO_PKG_VERSION"), " migrate-legacy");
    let entries: [(&'static str, String); 4] = [
        ("legacy_import_at", imported_at),
        ("legacy_source_path", source_path),
        ("legacy_source_sha256", source.sha256.clone()),
        ("legacy_import_tool", tool.to_string()),
    ];

    for (key, value) in entries {
        sqlx::query("INSERT OR REPLACE INTO app_metadata (key, value) VALUES (?1, ?2)")
            .bind(key)
            .bind(value)
            .execute(&mut *conn)
            .await
            .map_err(|source| Error::TargetMetadata { key, source })?;
    }
    Ok(())
}

/// Insère une ligne à la fois, une transaction par table, dans l'ordre des FK :
/// `media_assets` -> `meme_board_items` -> `playback_jobs` -> `guilds` ->
/// `overlay_clients` -> `ingest_clients` -> `pairing_codes`.
async fn insert_tables(conn: &mut SqliteConnection, data: &LegacyData) -> Result<(), Error> {
    macro_rules! insert_table {
        ($rows:expr) => {{
            let mut tx = conn.begin().await?;
            for row in $rows {
                row.insert(&mut tx).await?;
            }
            tx.commit().await?;
        }};
    }

    insert_table!(data.media_assets.iter());
    insert_table!(data.meme_board_items.iter());
    insert_table!(data.playback_jobs.iter());
    insert_table!(data.guilds.iter());
    insert_table!(data.overlay_clients.iter());
    insert_table!(data.ingest_clients.iter());
    insert_table!(data.pairing_codes.iter());

    Ok(())
}

/// Installe le fichier temporaire en fichier final, de façon atomique.
///
/// - cible absente : simple `rename` ;
/// - cible présente (`--force` déjà validé) : la cible est renommée en `.old`,
///   le temporaire prend sa place, puis le `.old` est supprimé ; en cas d'échec
///   du second renommage, l'ancienne cible est restaurée.
pub fn install(tmp: &Path, target: &Path, force: bool) -> Result<(), Error> {
    if target.exists() {
        if !force {
            return Err(Error::TargetExists(target.to_path_buf()));
        }
        let backup = super::append_suffix(target, ".old");
        if backup.exists() {
            std::fs::remove_file(&backup)?;
        }
        std::fs::rename(target, &backup)?;
        match std::fs::rename(tmp, target) {
            Ok(()) => {
                drop(std::fs::remove_file(&backup));
                Ok(())
            }
            Err(err) => {
                drop(std::fs::rename(&backup, target));
                Err(Error::Io(err))
            }
        }
    } else {
        std::fs::rename(tmp, target)?;
        Ok(())
    }
}

/// Chemin de la cible temporaire pour `target` (`<target>.tmp`).
pub fn tmp_path(target: &Path) -> PathBuf {
    super::append_suffix(target, ".tmp")
}
