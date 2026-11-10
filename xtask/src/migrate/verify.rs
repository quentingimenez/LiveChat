//! Vérification post-migration, exécutée sur le fichier cible AVANT son
//! installation définitive (rename) : intégrité, clés étrangères, registre sqlx,
//! version de schéma, comptes par table, sommes numériques et échantillon de
//! 20 lignes par table comparées champ à champ.
//!
//! Toute anomalie produit une erreur : le fichier temporaire est supprimé et
//! aucun fichier final n'est créé.

use sqlx::sqlite::SqliteRow;
use sqlx::SqliteConnection;

use super::dates;
use super::ledger::{self, LedgerReport};
use super::legacy::Legacy;
use super::read::LegacyData;
use super::tables::{
    Guild, IngestClient, MediaAsset, MemeBoardItem, OverlayClient, PairingCode, PlaybackJob,
};
use super::Error;

/// Nombre de lignes comparées champ à champ par table (contrat task-8).
pub const SAMPLE_SIZE: usize = 20;

/// Version de schéma attendue dans `app_metadata`.
pub const SCHEMA_VERSION: &str = "3.0.0";

/// Clés de métadonnées ajoutées par le migrateur (en plus de `schema_version`).
pub const EXTRA_METADATA_KEYS: [&str; 4] = [
    "legacy_import_at",
    "legacy_source_path",
    "legacy_source_sha256",
    "legacy_import_tool",
];

/// Résultat de la vérification, affiché par le migrateur.
#[derive(Debug, Default)]
pub struct VerifyReport {
    /// `(table, lignes legacy, lignes cible)`.
    pub counts: Vec<(&'static str, usize, i64)>,
    /// `(label, somme legacy, somme cible)`.
    pub sums: Vec<(&'static str, Option<i64>, Option<i64>)>,
    /// `(table, lignes comparées)`.
    pub samples: Vec<(&'static str, usize)>,
    /// Nombre de violations détectées par `PRAGMA foreign_key_check`.
    pub foreign_key_violations: i64,
    /// Résultat de `PRAGMA integrity_check` sur la cible.
    pub integrity: String,
    /// Valeur de `app_metadata.schema_version`.
    pub schema_version: String,
    /// Contenu du registre sqlx `_sqlx_migrations` (une ligne, `0001_init`).
    pub ledger: LedgerReport,
    /// Métadonnées legacy relues sur la cible.
    pub extra_metadata: Vec<(String, String)>,
}

impl VerifyReport {
    /// Nombre total de lignes comparées par échantillon.
    pub fn total_sampled(&self) -> usize {
        self.samples.iter().map(|(_, count)| count).sum()
    }

    /// Affiche le rapport (une section par contrôle).
    pub fn print(&self) {
        println!("      comptes par table (legacy -> cible) :");
        for (table, legacy_count, target_count) in &self.counts {
            println!("        - {table:<18} {legacy_count:>8} -> {target_count:>8}  OK");
        }
        println!("      sommes numériques : {} contrôlées", self.sums.len());
        for (label, legacy_sum, target_sum) in &self.sums {
            println!(
                "        - {label:<34} {} = {}  OK",
                format_sum(*legacy_sum),
                format_sum(*target_sum)
            );
        }
        println!(
            "      échantillon : {} ligne(s) comparée(s) champ à champ ({} par table max)",
            self.total_sampled(),
            SAMPLE_SIZE
        );
        println!(
            "      clés étrangères : {} violation(s) ; integrity_check = {} ; schema_version = {}",
            self.foreign_key_violations, self.integrity, self.schema_version
        );
        println!(
            "      registre sqlx : {} ({})",
            ledger::LEDGER_TABLE,
            self.ledger.summary()
        );
        if !self.extra_metadata.is_empty() {
            println!("      métadonnées d'import :");
            for (key, value) in &self.extra_metadata {
                let rendered = if key == "legacy_import_at" {
                    value
                        .parse::<i64>()
                        .map(dates::format_millis)
                        .unwrap_or_else(|_| value.clone())
                } else {
                    value.clone()
                };
                println!("        - {key} = {rendered}");
            }
        }
    }
}

fn format_sum(value: Option<i64>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "NULL".to_string(),
    }
}

/// Définition d'un contrôle de somme : identique des deux côtés après mapping.
struct SumCheckDef {
    label: &'static str,
    legacy_table: &'static str,
    legacy_sql: &'static str,
    target_sql: &'static str,
}

/// Sommes contrôlées : toutes les colonnes numériques sujettes au mapping
/// camelCase -> snake_case (les compteurs de lignes couvrent le reste).
const SUM_CHECKS: &[SumCheckDef] = &[
    SumCheckDef {
        label: "media_assets.size_bytes",
        legacy_table: MediaAsset::LEGACY_TABLE,
        legacy_sql: "SELECT SUM(sizeBytes) FROM \"MediaAsset\"",
        target_sql: "SELECT SUM(size_bytes) FROM media_assets",
    },
    SumCheckDef {
        label: "media_assets.duration_sec",
        legacy_table: MediaAsset::LEGACY_TABLE,
        legacy_sql: "SELECT SUM(durationSec) FROM \"MediaAsset\"",
        target_sql: "SELECT SUM(duration_sec) FROM media_assets",
    },
    SumCheckDef {
        label: "playback_jobs.duration_sec",
        legacy_table: PlaybackJob::LEGACY_TABLE,
        legacy_sql: "SELECT SUM(durationSec) FROM \"PlaybackJob\"",
        target_sql: "SELECT SUM(duration_sec) FROM playback_jobs",
    },
    SumCheckDef {
        label: "playback_jobs.priority",
        legacy_table: PlaybackJob::LEGACY_TABLE,
        legacy_sql: "SELECT SUM(priority) FROM \"PlaybackJob\"",
        target_sql: "SELECT SUM(priority) FROM playback_jobs",
    },
    SumCheckDef {
        label: "playback_jobs.resume_offset_sec",
        legacy_table: PlaybackJob::LEGACY_TABLE,
        legacy_sql: "SELECT SUM(COALESCE(resumeOffsetSec, 0)) FROM \"PlaybackJob\"",
        target_sql: "SELECT SUM(resume_offset_sec) FROM playback_jobs",
    },
    SumCheckDef {
        label: "guilds.default_media_time",
        legacy_table: Guild::LEGACY_TABLE,
        legacy_sql: "SELECT SUM(defaultMediaTime) FROM \"Guild\"",
        target_sql: "SELECT SUM(default_media_time) FROM guilds",
    },
    SumCheckDef {
        label: "guilds.max_media_time",
        legacy_table: Guild::LEGACY_TABLE,
        legacy_sql: "SELECT SUM(maxMediaTime) FROM \"Guild\"",
        target_sql: "SELECT SUM(max_media_time) FROM guilds",
    },
];

/// Exécute tous les contrôles post-migration sur la cible ouverte.
pub async fn verify(
    conn: &mut SqliteConnection,
    legacy: &Legacy,
    data: &LegacyData,
) -> Result<VerifyReport, Error> {
    let mut report = VerifyReport {
        integrity: query_text(conn, "PRAGMA integrity_check", "integrity_check").await?,
        ..VerifyReport::default()
    };
    if report.integrity != "ok" {
        return Err(Error::Verify(format!(
            "PRAGMA integrity_check de la cible = {:?} (attendu \"ok\")",
            report.integrity
        )));
    }

    report.foreign_key_violations = count_foreign_key_violations(conn).await?;
    if report.foreign_key_violations != 0 {
        return Err(Error::Verify(format!(
            "PRAGMA foreign_key_check de la cible révèle {} violation(s) de clé étrangère",
            report.foreign_key_violations
        )));
    }

    report.schema_version = query_text(
        conn,
        "SELECT value FROM app_metadata WHERE key = 'schema_version'",
        "app_metadata.schema_version",
    )
    .await?;
    if report.schema_version != SCHEMA_VERSION {
        return Err(Error::Verify(format!(
            "app_metadata.schema_version = {:?} (attendu {SCHEMA_VERSION})",
            report.schema_version
        )));
    }

    // Le registre sqlx doit être présent et conforme AVANT le rename, sinon le
    // premier démarrage du serveur rejouerait `0001_init` et échouerait.
    report.ledger = ledger::verify(conn).await?;

    for (table, legacy_count) in data.counts() {
        let target_count = count_rows(conn, table).await?;
        report.counts.push((table, legacy_count, target_count));
        if target_count != legacy_count as i64 {
            return Err(Error::Verify(format!(
                "compte de lignes incohérent pour {table} : {legacy_count} dans le legacy, \
                 {target_count} dans la cible"
            )));
        }
    }

    for check in SUM_CHECKS {
        let legacy_sum = legacy
            .scalar_sum(check.legacy_table, check.legacy_sql)
            .await?;
        let target_sum = query_optional_i64(conn, check.target_sql, check.label).await?;
        report.sums.push((check.label, legacy_sum, target_sum));
        if legacy_sum != target_sum {
            return Err(Error::Verify(format!(
                "somme incohérente pour {} : legacy {:?}, cible {:?}",
                check.label, legacy_sum, target_sum
            )));
        }
    }

    report.samples.push((
        MediaAsset::TABLE,
        sample_media_assets(conn, &data.media_assets).await?,
    ));
    report.samples.push((
        MemeBoardItem::TABLE,
        sample_meme_board_items(conn, &data.meme_board_items).await?,
    ));
    report.samples.push((
        PlaybackJob::TABLE,
        sample_playback_jobs(conn, &data.playback_jobs).await?,
    ));
    report
        .samples
        .push((Guild::TABLE, sample_guilds(conn, &data.guilds).await?));
    report.samples.push((
        OverlayClient::TABLE,
        sample_overlay_clients(conn, &data.overlay_clients).await?,
    ));
    report.samples.push((
        IngestClient::TABLE,
        sample_ingest_clients(conn, &data.ingest_clients).await?,
    ));
    report.samples.push((
        PairingCode::TABLE,
        sample_pairing_codes(conn, &data.pairing_codes).await?,
    ));

    for key in EXTRA_METADATA_KEYS {
        let value = query_optional_text(conn, "SELECT value FROM app_metadata WHERE key = ?", key)
            .await?
            .unwrap_or_default();
        report.extra_metadata.push((key.to_string(), value));
    }

    Ok(report)
}

async fn count_rows(conn: &mut SqliteConnection, table: &'static str) -> Result<i64, Error> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    sqlx::query_scalar::<_, i64>(&sql)
        .fetch_one(&mut *conn)
        .await
        .map_err(|source| {
            Error::Verify(format!(
                "comptage de la table {table} impossible : {source}"
            ))
        })
}

async fn count_foreign_key_violations(conn: &mut SqliteConnection) -> Result<i64, Error> {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&mut *conn)
        .await
        .map_err(|source| Error::Verify(format!("PRAGMA foreign_key_check impossible : {source}")))
}

async fn query_text(conn: &mut SqliteConnection, sql: &str, label: &str) -> Result<String, Error> {
    sqlx::query_scalar::<_, String>(sql)
        .fetch_one(&mut *conn)
        .await
        .map_err(|source| Error::Verify(format!("lecture de {label} impossible : {source}")))
}

async fn query_optional_text(
    conn: &mut SqliteConnection,
    sql: &str,
    key: &str,
) -> Result<Option<String>, Error> {
    sqlx::query_scalar::<_, String>(sql)
        .bind(key)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|source| {
            Error::Verify(format!(
                "lecture de app_metadata[{key}] impossible : {source}"
            ))
        })
}

async fn query_optional_i64(
    conn: &mut SqliteConnection,
    sql: &str,
    label: &str,
) -> Result<Option<i64>, Error> {
    sqlx::query_scalar::<_, Option<i64>>(sql)
        .fetch_one(&mut *conn)
        .await
        .map_err(|source| Error::Verify(format!("somme de {label} impossible : {source}")))
}

/// Lit une ligne cible par clé primaire, pour l'échantillon.
async fn fetch_row(
    conn: &mut SqliteConnection,
    table: &'static str,
    pk: &'static str,
    key: &str,
) -> Result<SqliteRow, Error> {
    let sql = format!("SELECT * FROM {table} WHERE {pk} = ?");
    sqlx::query(&sql)
        .bind(key)
        .fetch_optional(&mut *conn)
        .await
        .map_err(|source| {
            Error::Verify(format!(
                "relecture de {table} (clé {key}) impossible : {source}"
            ))
        })?
        .ok_or_else(|| Error::Verify(format!("ligne absente de la cible : {table}, clé {key}")))
}

fn sample_mismatch<T: std::fmt::Debug>(
    table: &'static str,
    key: &str,
    expected: &T,
    actual: &T,
) -> Error {
    Error::SampleMismatch {
        table,
        key: key.to_string(),
        legacy: format!("{expected:#?}"),
        target: format!("{actual:#?}"),
    }
}

/// Génère la fonction de contrôle d'échantillon d'une table.
macro_rules! sample_check {
    ($fn_name:ident, $ty:ty, $table:expr, $pk:expr) => {
        async fn $fn_name(
            conn: &mut SqliteConnection,
            legacy_rows: &[$ty],
        ) -> Result<usize, Error> {
            let mut checked = 0;
            for expected in legacy_rows.iter().take(SAMPLE_SIZE) {
                let row = fetch_row(conn, $table, $pk, expected.primary_key()).await?;
                let actual = <$ty>::from_target(&row)?;
                if actual != *expected {
                    return Err(sample_mismatch(
                        $table,
                        expected.primary_key(),
                        expected,
                        &actual,
                    ));
                }
                checked += 1;
            }
            Ok(checked)
        }
    };
}

sample_check!(
    sample_media_assets,
    MediaAsset,
    MediaAsset::TABLE,
    MediaAsset::PK
);
sample_check!(
    sample_meme_board_items,
    MemeBoardItem,
    MemeBoardItem::TABLE,
    MemeBoardItem::PK
);
sample_check!(
    sample_playback_jobs,
    PlaybackJob,
    PlaybackJob::TABLE,
    PlaybackJob::PK
);
sample_check!(sample_guilds, Guild, Guild::TABLE, Guild::PK);
sample_check!(
    sample_overlay_clients,
    OverlayClient,
    OverlayClient::TABLE,
    OverlayClient::PK
);
sample_check!(
    sample_ingest_clients,
    IngestClient,
    IngestClient::TABLE,
    IngestClient::PK
);
sample_check!(
    sample_pairing_codes,
    PairingCode,
    PairingCode::TABLE,
    PairingCode::PK
);
