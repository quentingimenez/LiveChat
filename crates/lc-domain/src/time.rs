//! Conversion entre les horodatages `INTEGER` (ms epoch UTC) de SQLite et
//! [`UtcDateTime`] (crate `time`), plus le format RFC 3339 des payloads JSON.
//!
//! Règle du contrat de schéma (`docs/rust/schema-v3.sql`) : **toutes** les
//! dates sont stockées en `INTEGER` = millisecondes Unix epoch UTC. Le legacy
//! Prisma/SQLite faisait de même (`docs/rust/prisma-dates.txt`) ; le piège
//! historique était le `DEFAULT CURRENT_TIMESTAMP` (TEXT secondes), interdit
//! dans le schéma v3.

use ::time::{Duration, UtcDateTime};

/// Convertit une date UTC en millisecondes epoch (troncature à la ms, vers
/// le passé pour les dates antérieures à 1970).
pub fn datetime_to_ms(value: UtcDateTime) -> i64 {
    // `unix_timestamp` est arrondi vers le bas et `millisecond` est dans
    // 0..1000 : la somme est le plancher en ms (même sémantique que l'ancien
    // `chrono::DateTime::timestamp_millis`). La plage de `UtcDateTime`
    // (±9999 ans) tient largement dans un `i64` en ms.
    value.unix_timestamp() * 1000 + i64::from(value.millisecond())
}

/// Convertit des millisecondes epoch en date UTC.
///
/// Renvoie `None` si la valeur sort de la plage représentable par `time`
/// (données corrompues) ; les repositories `lc-db` la transforment en erreur
/// `Corrupt` plutôt que de fabriquer une date arbitraire.
pub fn ms_to_datetime(ms: i64) -> Option<UtcDateTime> {
    UtcDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()
}

/// Horodatage courant en millisecondes epoch UTC.
pub fn now_ms() -> i64 {
    datetime_to_ms(UtcDateTime::now())
}

/// Formate une date en RFC 3339 à la milliseconde, suffixe `Z`
/// (`2026-11-09T22:18:00.123Z`).
///
/// Octet pour octet le format de l'ancien
/// `to_rfc3339_opts(SecondsFormat::Millis, true)` de `chrono` : toujours trois
/// décimales (tronquées), jamais d'offset numérique.
pub fn to_rfc3339_millis(value: UtcDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.millisecond()
    )
}

/// Durée en millisecondes entières (troncature vers zéro), saturée à la plage
/// `i64`.
pub fn duration_to_ms(value: Duration) -> i64 {
    let millis = value.whole_milliseconds();
    i64::try_from(millis).unwrap_or(if millis < 0 { i64::MIN } else { i64::MAX })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{datetime_to_ms, duration_to_ms, ms_to_datetime, to_rfc3339_millis};
    use ::time::{Duration, UtcDateTime};

    #[test]
    fn epoch_zero() {
        let epoch = UtcDateTime::UNIX_EPOCH;
        assert_eq!(datetime_to_ms(epoch), 0);
        assert_eq!(ms_to_datetime(0), Some(epoch));
    }

    #[test]
    fn aller_retour_millisecondes() {
        // 2026-11-09T22:18:00.123Z
        let ms = 1_794_262_680_123;
        let date = ms_to_datetime(ms).unwrap();
        assert_eq!(datetime_to_ms(date), ms);
        assert_eq!(date.millisecond(), 123);
    }

    #[test]
    fn avant_1970_arrondi_vers_le_passe() {
        assert_eq!(datetime_to_ms(ms_to_datetime(-1).unwrap()), -1);
        assert_eq!(datetime_to_ms(ms_to_datetime(-1_500).unwrap()), -1_500);
        let date = UtcDateTime::UNIX_EPOCH - Duration::nanoseconds(1);
        assert_eq!(datetime_to_ms(date), -1);
    }

    #[test]
    fn valeurs_hors_plage_rejetees() {
        assert_eq!(ms_to_datetime(i64::MAX), None);
        assert_eq!(ms_to_datetime(i64::MIN), None);
    }

    #[test]
    fn rfc3339_millis_identique_a_chrono() {
        // Valeurs produites par `chrono` 0.4 `to_rfc3339_opts(Millis, true)`.
        let cases = [
            (0, "1970-01-01T00:00:00.000Z"),
            (1_794_262_680_123, "2026-11-09T22:18:00.123Z"),
            (1_704_067_200_000, "2024-01-01T00:00:00.000Z"),
            (1_704_067_200_120, "2024-01-01T00:00:00.120Z"),
            (-1, "1969-12-31T23:59:59.999Z"),
        ];
        for (ms, expected) in cases {
            assert_eq!(to_rfc3339_millis(ms_to_datetime(ms).unwrap()), expected);
        }
        // Sous-milliseconde tronquée, pas arrondie.
        let date = ms_to_datetime(1_704_067_200_120).unwrap() + Duration::microseconds(999);
        assert_eq!(to_rfc3339_millis(date), "2024-01-01T00:00:00.120Z");
    }

    #[test]
    fn duree_en_ms() {
        assert_eq!(duration_to_ms(Duration::seconds(2)), 2_000);
        assert_eq!(duration_to_ms(Duration::microseconds(-1_500)), -1);
        assert_eq!(duration_to_ms(Duration::MAX), i64::MAX);
    }
}
