//! Registre sqlx `_sqlx_migrations` écrit dans la cible 3.0.
//!
//! Sans ce registre, le premier démarrage de `lc-server` appelle `Db::open`, dont
//! la configuration par défaut exécute `Migrator::run` : sqlx considère alors la
//! migration `0001_init` comme non appliquée, rejoue son SQL sur un schéma déjà
//! présent et échoue (« table app_metadata already exists »).
//!
//! Le migrateur écrit donc lui-même le registre tel que sqlx l'aurait écrit :
//! la table `_sqlx_migrations` et une ligne `version = 1`,
//! `description = "init"`, `success = TRUE`, `execution_time` mesuré, et
//! `checksum = SHA-384` des octets de `lc_db::MIGRATION_SQL`.
//!
//! Le format doit rester strictement identique à celui de sqlx 0.8
//! (`sqlx-core/src/migrate/migration.rs` pour le checksum, et
//! `sqlx-sqlite/src/migrate.rs` pour le DDL et la ligne insérée) : toute
//! divergence provoque au démarrage soit un rejeu, soit une erreur
//! `VersionMismatch` de sqlx.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha384};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection as _, SqliteConnection};

use super::Error;

/// Nom de la table de suivi des migrations de sqlx.
pub const LEDGER_TABLE: &str = "_sqlx_migrations";

/// Version de la migration initiale embarquée par `lc-db` (`0001_init.sql`).
pub const MIGRATION_VERSION: i64 = 1;

/// Description que sqlx dérive du nom de fichier `0001_init.sql`.
pub const MIGRATION_DESCRIPTION: &str = "init";

/// DDL exact créé par sqlx 0.8
/// (`SqliteConnection::ensure_migrations_table`, `sqlx-sqlite/src/migrate.rs`).
const CREATE_LEDGER_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
"#;

/// Checksum attendu par sqlx : SHA-384 des octets de `lc_db::MIGRATION_SQL`.
pub fn expected_checksum() -> Vec<u8> {
    Sha384::digest(lc_db::MIGRATION_SQL.as_bytes()).to_vec()
}

/// Checksum attendu en hexadécimal (affichage, diagnostics et tests).
pub fn expected_checksum_hex() -> String {
    hex(&expected_checksum())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Résultat de la vérification du registre, intégré au rapport de migration.
#[derive(Debug, Clone, Default)]
pub struct LedgerReport {
    /// La table `_sqlx_migrations` existe dans la cible.
    pub table_exists: bool,
    /// Nombre de lignes du registre (attendu : 1).
    pub rows: i64,
    /// Version enregistrée.
    pub version: Option<i64>,
    /// Description enregistrée.
    pub description: Option<String>,
    /// Indicateur de succès enregistré (1 = réussie).
    pub success: Option<i64>,
    /// Le checksum enregistré correspond au SHA-384 attendu.
    pub checksum_matches: bool,
}

impl LedgerReport {
    /// Résumé d'une ligne pour le rapport de migration.
    pub fn summary(&self) -> String {
        let table = if self.table_exists {
            ""
        } else {
            "table absente ; "
        };
        let checksum = if self.checksum_matches {
            "SHA-384 conforme"
        } else {
            "SHA-384 DIFFÉRENT"
        };
        format!(
            "{table}{} ligne(s), version {:?}, description {:?}, success {:?}, {}",
            self.rows,
            self.version.unwrap_or_default(),
            self.description.as_deref().unwrap_or(""),
            self.success.unwrap_or_default(),
            checksum
        )
    }
}

/// Écrit le registre sqlx dans la cible : table + ligne `0001_init`.
///
/// À appeler après l'écriture du schéma et des données, et avant la vérification
/// puis le renommage atomique.
pub async fn write(conn: &mut SqliteConnection, execution_time_ms: i64) -> Result<(), Error> {
    sqlx::raw_sql(CREATE_LEDGER_SQL)
        .execute(&mut *conn)
        .await
        .map_err(Error::LedgerWrite)?;

    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (?1, ?2, 1, ?3, ?4)",
    )
    .bind(MIGRATION_VERSION)
    .bind(MIGRATION_DESCRIPTION)
    .bind(expected_checksum())
    .bind(execution_time_ms)
    .execute(&mut *conn)
    .await
    .map_err(Error::LedgerWrite)?;

    Ok(())
}

/// Vérifie le registre de la cible ouverte : présence, une seule ligne,
/// `version = 1`, `description = "init"`, `success = 1` et checksum SHA-384
/// identique à celui attendu par sqlx.
pub async fn verify(conn: &mut SqliteConnection) -> Result<LedgerReport, Error> {
    let table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(LEDGER_TABLE)
    .fetch_one(&mut *conn)
    .await
    .map_err(Error::LedgerVerify)?
        > 0;

    if !table_exists {
        return Err(Error::LedgerInvalid(format!(
            "table {LEDGER_TABLE} absente de la cible (sqlx rejouerait 0001_init au \
             démarrage et échouerait sur un schéma déjà présent)"
        )));
    }

    let rows = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&mut *conn)
        .await
        .map_err(Error::LedgerVerify)?;

    let mut report = LedgerReport {
        table_exists,
        rows,
        ..LedgerReport::default()
    };

    if rows != 1 {
        return Err(Error::LedgerInvalid(format!(
            "table {LEDGER_TABLE} : {rows} ligne(s) (attendu : 1)"
        )));
    }

    let row = sqlx::query(
        "SELECT version, description, success, checksum FROM _sqlx_migrations WHERE version = ?1",
    )
    .bind(MIGRATION_VERSION)
    .fetch_one(&mut *conn)
    .await
    .map_err(Error::LedgerVerify)?;

    use sqlx::Row as _;
    let version: i64 = row.try_get("version").map_err(Error::LedgerVerify)?;
    let description: String = row.try_get("description").map_err(Error::LedgerVerify)?;
    let success: i64 = row.try_get("success").map_err(Error::LedgerVerify)?;
    let checksum: Vec<u8> = row.try_get("checksum").map_err(Error::LedgerVerify)?;

    report.version = Some(version);
    report.description = Some(description.clone());
    report.success = Some(success);
    report.checksum_matches = checksum == expected_checksum();

    if version != MIGRATION_VERSION {
        return Err(Error::LedgerInvalid(format!(
            "version enregistrée = {version} (attendu {MIGRATION_VERSION})"
        )));
    }
    if description != MIGRATION_DESCRIPTION {
        return Err(Error::LedgerInvalid(format!(
            "description enregistrée = {description:?} (attendu {MIGRATION_DESCRIPTION:?})"
        )));
    }
    if success != 1 {
        return Err(Error::LedgerInvalid(format!(
            "migration enregistrée avec success = {success} (sqlx la considérerait « dirty »)"
        )));
    }
    if !report.checksum_matches {
        return Err(Error::LedgerInvalid(format!(
            "checksum enregistré {} = {} (attendu SHA-384 {})",
            hex(&checksum),
            hex(&expected_checksum()),
            expected_checksum_hex()
        )));
    }

    Ok(report)
}

/// Compare le registre attendu avec un fichier cible existant, sans rien écrire
/// (mode `--dry-run`). Renvoie un compte rendu lisible, jamais d'erreur.
pub async fn compare_existing(path: &Path) -> String {
    if !path.exists() {
        return format!(
            "table {LEDGER_TABLE} sera créée (version {MIGRATION_VERSION}, \
             description \"{MIGRATION_DESCRIPTION}\", checksum SHA-384 {})",
            expected_checksum_hex()
        );
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(Duration::from_secs(5));
    let mut conn = match SqliteConnection::connect_with(&options).await {
        Ok(conn) => conn,
        Err(source) => {
            return format!("cible existante illisible ({source}) : registre non comparable");
        }
    };
    let outcome = verify(&mut conn).await;
    drop(conn.close().await);

    match outcome {
        Ok(report) => format!(
            "cible existante : table {LEDGER_TABLE} conforme ({})",
            report.summary()
        ),
        Err(err) => format!("cible existante : registre NON conforme ({err})"),
    }
}
