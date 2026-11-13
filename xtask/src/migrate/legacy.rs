//! Accès à la base legacy : ouverture strictement en lecture seule du snapshot
//! cohérent produit par [`super::snapshot`], contrôles d'intégrité, lecture des
//! 7 tables Prisma et empreintes SHA256.
//!
//! La base legacy elle-même n'est **jamais ouverte** par ce module : le
//! migrateur travaille sur le snapshot temporaire (copie + `VACUUM INTO`), qui
//! est en lecture seule (`read_only(true)` + `PRAGMA query_only=ON`, connexion
//! unique, aucun `ATTACH`/`VACUUM`/`PRAGMA` d'écriture).

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::SqlitePool;

use super::read::{LegacyData, ReadOut};
use super::tables::{
    Guild, IngestClient, MediaAsset, MediaBoardItem, OverlayClient, PairingCode, PlaybackJob,
};
use super::Error;

/// État des fichiers annexes SQLite (`-wal`, `-shm`) de la base legacy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalInfo {
    pub exists: bool,
    pub bytes: u64,
    pub shm_exists: bool,
}

/// Métadonnées de la source, collectées avant la migration.
#[derive(Debug, Clone)]
pub struct SourceMeta {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub wal: WalInfo,
}

/// Tables legacy attendues, dans l'ordre de migration (FK d'abord).
const EXPECTED_TABLES: [&str; 7] = [
    MediaAsset::LEGACY_TABLE,
    MediaBoardItem::LEGACY_TABLE,
    PlaybackJob::LEGACY_TABLE,
    Guild::LEGACY_TABLE,
    OverlayClient::LEGACY_TABLE,
    IngestClient::LEGACY_TABLE,
    PairingCode::LEGACY_TABLE,
];

/// Empreinte SHA256 d'un fichier (lecture par blocs de 1 Mio).
pub fn hash_file(path: &Path) -> Result<String, Error> {
    use std::io::Read as _;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
}

/// Détecte la présence des fichiers annexes `-wal`/`-shm`.
pub fn wal_info(path: &Path) -> WalInfo {
    let wal = super::append_suffix(path, "-wal");
    let shm = super::append_suffix(path, "-shm");
    let bytes = std::fs::metadata(&wal)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    WalInfo {
        exists: wal.is_file(),
        bytes,
        shm_exists: shm.is_file(),
    }
}

/// Collecte les métadonnées de la source avant lecture.
pub fn source_meta(path: &Path) -> Result<SourceMeta, Error> {
    let metadata = std::fs::metadata(path)?;
    Ok(SourceMeta {
        path: path.to_path_buf(),
        bytes: metadata.len(),
        sha256: hash_file(path)?,
        wal: wal_info(path),
    })
}

/// Connexion en lecture seule à la base legacy.
pub struct Legacy {
    pool: SqlitePool,
}

impl Legacy {
    /// Ouvre la base legacy en lecture seule (aucune écriture possible, même par
    /// erreur de code : `read_only` + `query_only`).
    pub async fn open(path: &Path) -> Result<Self, Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false)
            .busy_timeout(Duration::from_secs(10))
            .pragma("query_only", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|source| {
                let wal = wal_info(path);
                let hint = if wal.exists {
                    "un fichier -wal est présent à côté du snapshot : répertoire temporaire \
                     inaccessible en écriture ou snapshot incomplet"
                } else {
                    "vérifier que le snapshot est une base SQLite valide et que le répertoire \
                     temporaire est accessible en lecture"
                };
                Error::LegacyOpen {
                    path: path.to_path_buf(),
                    source,
                    hint: hint.to_string(),
                }
            })?;
        Ok(Self { pool })
    }

    /// Ferme la connexion SQLite (libère les verrous de lecture).
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// `PRAGMA integrity_check` : doit valoir `ok`.
    pub async fn integrity_check(&self) -> Result<String, Error> {
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&self.pool)
            .await
            .map_err(Error::Sqlx)
    }

    /// Mode de journalisation effectif de la base legacy (informatif).
    pub async fn journal_mode(&self) -> Result<String, Error> {
        sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(Error::Sqlx)
    }

    /// Vérifie que les 7 tables Prisma attendues existent.
    pub async fn require_tables(&self) -> Result<(), Error> {
        let names: Vec<String> = sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(Error::Sqlx)?;
        for expected in EXPECTED_TABLES {
            if !names.iter().any(|name| name == expected) {
                return Err(Error::MissingLegacyTable(expected));
            }
        }
        Ok(())
    }

    /// Lit et normalise les 7 tables legacy.
    pub async fn read_tables(&self) -> Result<LegacyData, Error> {
        self.require_tables().await?;
        self.check_orphan_foreign_keys().await?;

        let mut out = ReadOut::default();
        let media_assets = self
            .read_table(
                MediaAsset::LEGACY_TABLE,
                MediaAsset::DATE_COLUMNS_LEGACY,
                MediaAsset::from_legacy,
                &mut out,
            )
            .await?;
        let media_board_items = self
            .read_table(
                MediaBoardItem::LEGACY_TABLE,
                MediaBoardItem::DATE_COLUMNS_LEGACY,
                MediaBoardItem::from_legacy,
                &mut out,
            )
            .await?;
        let playback_jobs = self
            .read_table(
                PlaybackJob::LEGACY_TABLE,
                PlaybackJob::DATE_COLUMNS_LEGACY,
                PlaybackJob::from_legacy,
                &mut out,
            )
            .await?;
        let guilds = self
            .read_table(
                Guild::LEGACY_TABLE,
                Guild::DATE_COLUMNS_LEGACY,
                Guild::from_legacy,
                &mut out,
            )
            .await?;
        let overlay_clients = self
            .read_table(
                OverlayClient::LEGACY_TABLE,
                OverlayClient::DATE_COLUMNS_LEGACY,
                OverlayClient::from_legacy,
                &mut out,
            )
            .await?;
        let ingest_clients = self
            .read_table(
                IngestClient::LEGACY_TABLE,
                IngestClient::DATE_COLUMNS_LEGACY,
                IngestClient::from_legacy,
                &mut out,
            )
            .await?;
        let pairing_codes = self
            .read_table(
                PairingCode::LEGACY_TABLE,
                PairingCode::DATE_COLUMNS_LEGACY,
                PairingCode::from_legacy,
                &mut out,
            )
            .await?;

        Ok(LegacyData {
            media_assets,
            media_board_items,
            playback_jobs,
            guilds,
            overlay_clients,
            ingest_clients,
            pairing_codes,
            dates: out.dates,
            warnings: out.warnings,
        })
    }

    /// Somme d'une colonne numérique legacy (`SUM(...)`), `None` si vide.
    pub async fn scalar_sum(&self, table: &'static str, sql: &str) -> Result<Option<i64>, Error> {
        sqlx::query_scalar::<_, Option<i64>>(sql)
            .fetch_one(&self.pool)
            .await
            .map_err(|source| Error::LegacyQuery { table, source })
    }

    /// Lit une table legacy en construisant la requête avec les alias `typeof`.
    async fn read_table<T>(
        &self,
        table: &'static str,
        date_columns: &'static [&'static str],
        extract: fn(&SqliteRow, &mut ReadOut) -> Result<T, Error>,
        out: &mut ReadOut,
    ) -> Result<Vec<T>, Error> {
        let sql = select_sql(table, date_columns);
        let rows = sqlx::query(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|source| Error::LegacySchema { table, source })?;
        let mut values = Vec::with_capacity(rows.len());
        for row in &rows {
            values.push(extract(row, out)?);
        }
        Ok(values)
    }

    /// Refuse les clés étrangères orphelines que le schéma 3.0 rejetterait.
    async fn check_orphan_foreign_keys(&self) -> Result<(), Error> {
        let orphan_memes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM \"MemeBoardItem\" m \
             LEFT JOIN \"MediaAsset\" a ON a.id = m.mediaAssetId \
             WHERE a.id IS NULL",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(Error::Sqlx)?;
        let orphan_jobs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM \"PlaybackJob\" j \
             LEFT JOIN \"MediaAsset\" a ON a.id = j.mediaAssetId \
             WHERE j.mediaAssetId IS NOT NULL AND a.id IS NULL",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(Error::Sqlx)?;
        if orphan_memes > 0 || orphan_jobs > 0 {
            return Err(Error::OrphanForeignKeys {
                detail: format!(
                    "{orphan_memes} MemeBoardItem.mediaAssetId et {orphan_jobs} \
                     PlaybackJob.mediaAssetId pointent vers un MediaAsset absent de la base legacy"
                ),
            });
        }
        Ok(())
    }
}

/// Construit la requête de lecture : `rowid` stable, `typeof` de chaque colonne
/// `DateTime`, puis toutes les colonnes applicatives.
pub fn select_sql(table: &'static str, date_columns: &[&'static str]) -> String {
    let mut sql = String::from("SELECT rowid AS lc_rowid");
    for column in date_columns {
        sql.push_str(&format!(", typeof(\"{column}\") AS \"__t_{column}\""));
    }
    sql.push_str(&format!(", * FROM \"{table}\" ORDER BY rowid"));
    sql
}
