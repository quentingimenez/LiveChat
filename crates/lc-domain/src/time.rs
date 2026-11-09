//! Conversion entre les horodatages `INTEGER` (ms epoch UTC) de SQLite et
//! [`chrono::DateTime<Utc>`].
//!
//! Règle du contrat de schéma (`docs/rust/schema-v3.sql`) : **toutes** les
//! dates sont stockées en `INTEGER` = millisecondes Unix epoch UTC. Le legacy
//! Prisma/SQLite faisait de même (`docs/rust/prisma-dates.txt`) ; le piège
//! historique était le `DEFAULT CURRENT_TIMESTAMP` (TEXT secondes), interdit
//! dans le schéma v3.

use chrono::{DateTime, Utc};

/// Convertit une date UTC en millisecondes epoch (troncature à la ms).
pub fn datetime_to_ms(value: DateTime<Utc>) -> i64 {
    value.timestamp_millis()
}

/// Convertit des millisecondes epoch en date UTC.
///
/// Renvoie `None` si la valeur sort de la plage représentable par `chrono`
/// (données corrompues) ; les repositories `lc-db` la transforment en erreur
/// `Corrupt` plutôt que de fabriquer une date arbitraire.
pub fn ms_to_datetime(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
}

/// Horodatage courant en millisecondes epoch UTC.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{datetime_to_ms, ms_to_datetime};
    use chrono::DateTime;

    #[test]
    fn epoch_zero() {
        let epoch = DateTime::from_timestamp_millis(0).unwrap();
        assert_eq!(datetime_to_ms(epoch), 0);
        assert_eq!(ms_to_datetime(0), Some(epoch));
    }

    #[test]
    fn aller_retour_millisecondes() {
        // 2026-11-09T22:18:00.123Z
        let ms = 1_794_262_680_123;
        let date = ms_to_datetime(ms).unwrap();
        assert_eq!(datetime_to_ms(date), ms);
        assert_eq!(date.timestamp_subsec_millis(), 123);
    }

    #[test]
    fn valeurs_hors_plage_rejetees() {
        assert_eq!(ms_to_datetime(i64::MAX), None);
        assert_eq!(ms_to_datetime(i64::MIN), None);
    }
}
