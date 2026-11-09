//! Extraction typée des lignes legacy : accès aux colonnes avec contexte
//! d'erreur (table, ligne, clé primaire) et détection du type SQLite réel des
//! colonnes `DateTime` via `typeof(...)`.
//!
//! Les requêtes de lecture sont construites par [`super::legacy`] : chaque ligne
//! expose `rowid AS lc_rowid` (ordre stable) et, pour chaque colonne de date,
//! une colonne calculée `__t_<colonne>` contenant le type stocké
//! (`integer`, `text`, `real`, `null`). [`RowCtx`] s'appuie dessus.

use std::collections::BTreeMap;

use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use super::dates::{self, DateStats};
use super::tables::{
    Guild, IngestClient, MediaAsset, MemeBoardItem, OverlayClient, PairingCode, PlaybackJob,
};
use super::Error;

/// Compteurs accumulés pendant la lecture d'une table legacy.
#[derive(Debug, Default)]
pub struct ReadOut {
    /// Statistiques des valeurs de dates rencontrées.
    pub dates: DateStats,
    /// Avertissements non bloquants, comptés par message.
    pub warnings: BTreeMap<String, u64>,
}

impl ReadOut {
    /// Incrémente le compteur d'un avertissement.
    pub fn warn(&mut self, key: impl Into<String>) {
        *self.warnings.entry(key.into()).or_insert(0) += 1;
    }
}

/// Données legacy normalisées, prêtes à être écrites dans le schéma 3.0.
#[derive(Debug, Default)]
pub struct LegacyData {
    pub media_assets: Vec<MediaAsset>,
    pub meme_board_items: Vec<MemeBoardItem>,
    pub playback_jobs: Vec<PlaybackJob>,
    pub guilds: Vec<Guild>,
    pub overlay_clients: Vec<OverlayClient>,
    pub ingest_clients: Vec<IngestClient>,
    pub pairing_codes: Vec<PairingCode>,
    /// Statistiques de dates agrégées sur les 7 tables.
    pub dates: DateStats,
    /// Avertissements agrégés sur les 7 tables.
    pub warnings: BTreeMap<String, u64>,
}

impl LegacyData {
    /// Comptes par table, dans l'ordre du rapport.
    pub fn counts(&self) -> [(&'static str, usize); 7] {
        [
            (MediaAsset::TABLE, self.media_assets.len()),
            (MemeBoardItem::TABLE, self.meme_board_items.len()),
            (PlaybackJob::TABLE, self.playback_jobs.len()),
            (Guild::TABLE, self.guilds.len()),
            (OverlayClient::TABLE, self.overlay_clients.len()),
            (IngestClient::TABLE, self.ingest_clients.len()),
            (PairingCode::TABLE, self.pairing_codes.len()),
        ]
    }

    /// Nombre total de lignes legacy lues.
    pub fn total_rows(&self) -> usize {
        self.media_assets.len()
            + self.meme_board_items.len()
            + self.playback_jobs.len()
            + self.guilds.len()
            + self.overlay_clients.len()
            + self.ingest_clients.len()
            + self.pairing_codes.len()
    }
}

/// Contexte d'une ligne legacy, pour produire des erreurs actionnables.
pub struct RowCtx {
    /// Nom de la table legacy (`MediaAsset`…) ou cible (`media_assets`).
    pub table: &'static str,
    /// `rowid` SQLite de la ligne (0 pour une lecture cible).
    pub rowid: i64,
    /// Clé primaire applicative de la ligne.
    pub row_id: String,
}

impl RowCtx {
    /// Crée le contexte d'une ligne.
    pub fn new(table: &'static str, rowid: i64, row_id: impl Into<String>) -> Self {
        Self {
            table,
            rowid,
            row_id: row_id.into(),
        }
    }

    fn column_error(&self, column: &'static str, source: sqlx::Error) -> Error {
        Error::ColumnRead {
            table: self.table,
            column,
            row: self.rowid,
            row_id: self.row_id.clone(),
            source,
        }
    }

    fn null_error(&self, column: &'static str) -> Error {
        Error::UnexpectedNull {
            table: self.table,
            column,
            row: self.rowid,
            row_id: self.row_id.clone(),
        }
    }

    fn invalid_date(&self, column: &'static str, value: String, detail: String) -> Error {
        Error::InvalidDate {
            table: self.table,
            column,
            row: self.rowid,
            row_id: self.row_id.clone(),
            value,
            detail,
        }
    }

    /// Colonne `TEXT` nullable.
    pub fn text_opt(&self, row: &SqliteRow, column: &'static str) -> Result<Option<String>, Error> {
        row.try_get::<Option<String>, _>(column)
            .map_err(|err| self.column_error(column, err))
    }

    /// Colonne `TEXT` non nullable.
    pub fn text_req(&self, row: &SqliteRow, column: &'static str) -> Result<String, Error> {
        self.text_opt(row, column)?
            .ok_or_else(|| self.null_error(column))
    }

    /// Colonne `INTEGER` nullable.
    pub fn int_opt(&self, row: &SqliteRow, column: &'static str) -> Result<Option<i64>, Error> {
        row.try_get::<Option<i64>, _>(column)
            .map_err(|err| self.column_error(column, err))
    }

    /// Colonne `INTEGER` non nullable.
    pub fn int_req(&self, row: &SqliteRow, column: &'static str) -> Result<i64, Error> {
        self.int_opt(row, column)?
            .ok_or_else(|| self.null_error(column))
    }

    /// Colonne booléenne legacy (`Boolean` Prisma stocké en 0/1).
    pub fn bool_req(&self, row: &SqliteRow, column: &'static str) -> Result<i64, Error> {
        let value = self.int_req(row, column)?;
        if value == 0 || value == 1 {
            Ok(value)
        } else {
            Err(Error::InvalidBoolean {
                table: self.table,
                column,
                row: self.rowid,
                row_id: self.row_id.to_string(),
                value,
            })
        }
    }

    /// Colonne énumérée legacy : la casse peut varier selon les chemins
    /// d'écriture (`NORMAL` attendu, mais le serveur JS a déjà utilisé
    /// minuscules) → normalisation `trim` + ASCII majuscules puis validation
    /// contre les contraintes `CHECK` du schéma 3.0.
    pub fn enum_req(
        &self,
        row: &SqliteRow,
        column: &'static str,
        allowed: &[&'static str],
    ) -> Result<String, Error> {
        let raw = self.text_req(row, column)?;
        let normalized = raw.trim().to_ascii_uppercase();
        if allowed.contains(&normalized.as_str()) {
            Ok(normalized)
        } else {
            Err(Error::InvalidEnum {
                table: self.table,
                column,
                row: self.rowid,
                row_id: self.row_id.to_string(),
                value: raw,
                allowed: allowed.join(", "),
            })
        }
    }

    /// Colonne `DateTime` nullable : détecte le type SQLite stocké puis
    /// normalise en millisecondes epoch UTC.
    pub fn date_opt(
        &self,
        row: &SqliteRow,
        column: &'static str,
        out: &mut ReadOut,
    ) -> Result<Option<i64>, Error> {
        let alias = format!("__t_{column}");
        let storage_type: String = row
            .try_get(alias.as_str())
            .map_err(|err| self.column_error(column, err))?;
        match storage_type.as_str() {
            "null" => {
                out.dates.null += 1;
                Ok(None)
            }
            "integer" => {
                let value = self.int_req(row, column)?;
                match dates::check_integer(value) {
                    Ok(millis) => {
                        out.dates.record(dates::DateFormat::IntegerMillis);
                        Ok(Some(millis))
                    }
                    Err(detail) => {
                        Err(self.invalid_date(column, format!("INTEGER {value}"), detail))
                    }
                }
            }
            "text" => {
                let raw = self.text_req(row, column)?;
                match dates::parse_text(&raw) {
                    Ok((millis, format)) => {
                        out.dates.record(format);
                        Ok(Some(millis))
                    }
                    Err(detail) => Err(self.invalid_date(column, format!("TEXT {raw:?}"), detail)),
                }
            }
            "real" => {
                let value = row
                    .try_get::<f64, _>(column)
                    .map_err(|err| self.column_error(column, err))?;
                Err(self.invalid_date(
                    column,
                    format!("REAL {value}"),
                    "les REAL ne sont pas des dates valides (Prisma les rejette aussi) ; \
                     utiliser INTEGER epoch ms ou TEXT RFC 3339"
                        .to_string(),
                ))
            }
            other => Err(self.invalid_date(
                column,
                format!("type SQLite {other}"),
                "type stocké inattendu pour une colonne DateTime".to_string(),
            )),
        }
    }

    /// Colonne `DateTime` non nullable : une valeur NULL est une anomalie.
    pub fn date_req(
        &self,
        row: &SqliteRow,
        column: &'static str,
        out: &mut ReadOut,
    ) -> Result<i64, Error> {
        self.date_opt(row, column, out)?
            .ok_or_else(|| self.null_error(column))
    }
}

/// Lit `rowid` (alias `lc_rowid`) et l'enveloppe dans une erreur typée.
pub fn read_rowid(row: &SqliteRow, table: &'static str) -> Result<i64, Error> {
    row.try_get::<i64, _>("lc_rowid")
        .map_err(|source| Error::ColumnRead {
            table,
            column: "rowid",
            row: 0,
            row_id: String::new(),
            source,
        })
}

/// Lit la clé primaire applicative d'une ligne et l'enveloppe dans une erreur typée.
pub fn read_pk(
    row: &SqliteRow,
    table: &'static str,
    column: &'static str,
) -> Result<String, Error> {
    row.try_get::<String, _>(column)
        .map_err(|source| Error::ColumnRead {
            table,
            column,
            row: 0,
            row_id: String::new(),
            source,
        })
}
