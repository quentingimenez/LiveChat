//! Conversions entre lignes SQLite et entités de domaine.
//!
//! Toutes les dates traversent la frontière persistance sous forme de `i64`
//! millisecondes epoch UTC. Les helpers de ce module centralisent les erreurs
//! [`DbError::Corrupt`] : une valeur illisible en base ne doit jamais faire
//! paniquer le serveur ni être silencieusement remplacée.

use chrono::{DateTime, Utc};
use lc_domain::{datetime_to_ms, ms_to_datetime, DomainError};

use crate::error::{DbError, DbResult};

/// Construit une erreur de ligne corrompue.
pub(crate) fn corrupt(table: &'static str, detail: impl Into<String>) -> DbError {
    DbError::Corrupt {
        table,
        detail: detail.into(),
    }
}

/// Convertit une date du domaine en millisecondes epoch UTC (stockage).
pub(crate) fn to_ms(value: DateTime<Utc>) -> i64 {
    datetime_to_ms(value)
}

/// Convertit une date optionnelle en millisecondes epoch UTC (stockage).
pub(crate) fn to_opt_ms(value: Option<DateTime<Utc>>) -> Option<i64> {
    value.map(datetime_to_ms)
}

/// Convertit des millisecondes epoch UTC en date du domaine (lecture).
pub(crate) fn from_ms(
    table: &'static str,
    column: &'static str,
    millis: i64,
) -> DbResult<DateTime<Utc>> {
    ms_to_datetime(millis).ok_or_else(|| {
        corrupt(
            table,
            format!("{column} : {millis} ms hors plage supportée"),
        )
    })
}

/// Convertit des millisecondes optionnelles en date optionnelle du domaine.
pub(crate) fn from_opt_ms(
    table: &'static str,
    column: &'static str,
    millis: Option<i64>,
) -> DbResult<Option<DateTime<Utc>>> {
    match millis {
        Some(millis) => from_ms(table, column, millis).map(Some),
        None => Ok(None),
    }
}

/// Décode un booléen stocké `INTEGER` (0/1).
pub(crate) fn from_bool(value: i64) -> bool {
    value != 0
}

/// Encode un booléen en `INTEGER` (0/1).
pub(crate) fn to_bool(value: bool) -> i64 {
    i64::from(value)
}

/// Décode une valeur énumérée (texte) via le parseur du domaine.
pub(crate) fn decode<T>(
    table: &'static str,
    column: &'static str,
    raw: &str,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> DbResult<T> {
    parse(raw).map_err(|error| corrupt(table, format!("{column} : {error}")))
}

/// Décode une valeur optionnelle via le parseur du domaine.
pub(crate) fn decode_opt<T>(
    table: &'static str,
    column: &'static str,
    raw: Option<&str>,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> DbResult<Option<T>> {
    match raw {
        Some(raw) => decode(table, column, raw, parse).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{decode_opt, from_bool, from_ms, from_opt_ms, to_bool, to_ms, to_opt_ms};
    use lc_domain::time::ms_to_datetime;
    use lc_domain::{DomainError, MediaKind};

    #[test]
    fn dates_font_l_aller_retour() {
        let date = ms_to_datetime(1_700_000_123_456).unwrap();
        assert_eq!(to_ms(date), 1_700_000_123_456);
        assert_eq!(from_ms("t", "c", 1_700_000_123_456).unwrap(), date);
        assert_eq!(to_opt_ms(Some(date)), Some(1_700_000_123_456));
        assert_eq!(to_opt_ms(None), None);
        assert_eq!(from_opt_ms("t", "c", None).unwrap(), None);
    }

    #[test]
    fn dates_hors_plage_signalees() {
        let error = from_ms("media_assets", "created_at", i64::MAX).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("created_at"), "{message}");
    }

    #[test]
    fn bool_et_enum() {
        assert!(from_bool(1));
        assert!(!from_bool(0));
        assert_eq!(to_bool(true), 1);
        assert_eq!(to_bool(false), 0);

        let kind = decode_opt(
            "media_assets",
            "kind",
            Some("VIDEO"),
            MediaKind::from_db_str,
        )
        .unwrap()
        .unwrap();
        assert_eq!(kind, MediaKind::Video);
        assert_eq!(
            decode_opt::<MediaKind>("media_assets", "kind", None, MediaKind::from_db_str).unwrap(),
            None
        );

        let error =
            decode_opt("media_assets", "kind", Some("GIF"), MediaKind::from_db_str).unwrap_err();
        assert!(matches!(error, super::DbError::Corrupt { .. }));
        let domain_error = MediaKind::from_db_str("GIF").unwrap_err();
        assert!(matches!(domain_error, DomainError::Validation(_)));
    }
}
