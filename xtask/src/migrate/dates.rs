//! Parseur tolérant des dates legacy `DateTime` (Prisma/SQLite) vers des
//! millisecondes Unix epoch UTC (`i64`).
//!
//! Le schéma Prisma déclare des colonnes `DateTime`, que SQLite stocke selon le
//! chemin d'écriture :
//!
//! - `INTEGER` : epoch millisecondes UTC — cas canonique de Prisma, à conserver
//!   tel quel ;
//! - `TEXT` `YYYY-MM-DD HH:MM:SS` : valeur écrite par un `DEFAULT
//!   CURRENT_TIMESTAMP` SQL ou par un `INSERT` brut, **UTC à la seconde** ;
//! - `TEXT` RFC 3339 : `2026-11-09T22:00:00Z` ou `...+01:00`, avec fuseau
//!   explicite.
//!
//! Sont refusés explicitement (comme le fait Prisma, et pour ne jamais inventer
//! de fuseau) : `T` sans fuseau, espace suivi d'une fraction de seconde, valeur
//! `REAL`, format arbitraire. L'appelant transforme le message d'erreur en
//! [`super::Error::InvalidDate`] avec table/colonne/ligne.

use std::fmt;

use chrono::{DateTime, NaiveDateTime, Utc};

/// Borne basse acceptée : `0001-01-01T00:00:00Z` en millisecondes.
///
/// Alignée sur la plage `DateTime` de Prisma ; toute valeur hors bornes est
/// considérée comme corrompue plutôt que silencieusement convertie.
const MIN_EPOCH_MS: i64 = -62_135_596_800_000;

/// Borne haute acceptée : `9999-12-31T23:59:59.999Z` en millisecondes.
const MAX_EPOCH_MS: i64 = 253_402_300_799_999;

/// Format source reconnu, conservé pour les statistiques du rapport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateFormat {
    /// `INTEGER` epoch millisecondes (cas canonique Prisma).
    IntegerMillis,
    /// `TEXT` `YYYY-MM-DD HH:MM:SS`, interprété en UTC à la seconde.
    TextSpaceUtcSeconds,
    /// `TEXT` RFC 3339 avec fuseau explicite (`Z` ou `±HH:MM`).
    Rfc3339,
}

/// Compteurs par format, affichés par le rapport de `--dry-run`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DateStats {
    pub integer: u64,
    pub text_space_utc: u64,
    pub rfc3339: u64,
    pub null: u64,
}

impl DateStats {
    /// Enregistre une date acceptée dans le compteur de son format.
    pub fn record(&mut self, format: DateFormat) {
        match format {
            DateFormat::IntegerMillis => self.integer += 1,
            DateFormat::TextSpaceUtcSeconds => self.text_space_utc += 1,
            DateFormat::Rfc3339 => self.rfc3339 += 1,
        }
    }

    /// Nombre total de valeurs rencontrées (NULL compris).
    pub fn total(&self) -> u64 {
        self.integer + self.text_space_utc + self.rfc3339 + self.null
    }
}

impl fmt::Display for DateStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} valeur(s) : {} INTEGER ms, {} TEXT « AAAA-MM-JJ HH:MM:SS » UTC, {} RFC 3339, {} NULL",
            self.total(),
            self.integer,
            self.text_space_utc,
            self.rfc3339,
            self.null
        )
    }
}

/// Valide une valeur `INTEGER` legacy (epoch millisecondes) et la renvoie telle
/// quelle.
pub fn check_integer(value: i64) -> Result<i64, String> {
    if (MIN_EPOCH_MS..=MAX_EPOCH_MS).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "valeur INTEGER {value} hors de la plage epoch ms de Prisma \
             ({MIN_EPOCH_MS}..={MAX_EPOCH_MS}, soit 0001-01-01..9999-12-31 UTC)"
        ))
    }
}

/// Parse une valeur `TEXT` legacy et renvoie `(millisecondes epoch, format)`.
///
/// Les `NULL` sont détectés en amont via `typeof(...)` : cette fonction ne les
/// voit jamais.
pub fn parse_text(raw: &str) -> Result<(i64, DateFormat), String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("chaîne vide".to_string());
    }
    if value.contains('T') {
        parse_rfc3339(value)
    } else {
        parse_space_datetime(value)
    }
}

/// `YYYY-MM-DD HH:MM:SS` (UTC, seconde), tel qu'écrit par `CURRENT_TIMESTAMP`.
fn parse_space_datetime(value: &str) -> Result<(i64, DateFormat), String> {
    let Some(index) = value.find(' ') else {
        return Err(
            "format TEXT non reconnu ; attendu « AAAA-MM-JJ HH:MM:SS » UTC, \
             RFC 3339 avec fuseau, ou INTEGER epoch ms"
                .to_string(),
        );
    };
    let time_part = &value[index + 1..];
    if time_part.contains('.') {
        return Err(
            "fraction de seconde refusée pour un TEXT sans fuseau (ambigu) : \
             convertir en RFC 3339 avec fuseau ou en INTEGER ms"
                .to_string(),
        );
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").map_err(|err| {
        format!(
            "format TEXT non reconnu ({err}) ; attendu « AAAA-MM-JJ HH:MM:SS » UTC, \
             RFC 3339 avec fuseau, ou INTEGER epoch ms"
        )
    })?;
    Ok((
        naive.and_utc().timestamp_millis(),
        DateFormat::TextSpaceUtcSeconds,
    ))
}

/// RFC 3339 avec fuseau explicite (`Z` ou `±HH:MM`).
fn parse_rfc3339(value: &str) -> Result<(i64, DateFormat), String> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|err| {
        format!("RFC 3339 invalide ({err}) ; un fuseau explicite (Z ou ±HH:MM) est obligatoire")
    })?;
    let millis = parsed.timestamp_millis();
    check_integer(millis)?;
    Ok((millis, DateFormat::Rfc3339))
}

/// Formate des millisecondes epoch pour les messages de vérification.
pub fn format_millis(millis: i64) -> String {
    match DateTime::<Utc>::from_timestamp_millis(millis) {
        Some(value) => value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        None => format!("{millis} ms (hors plage chrono)"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const JANVIER_2024_UTC: i64 = 1_704_067_200_000;

    #[test]
    fn integer_valide() {
        assert_eq!(check_integer(0), Ok(0));
        assert_eq!(check_integer(JANVIER_2024_UTC), Ok(JANVIER_2024_UTC));
        assert_eq!(check_integer(MIN_EPOCH_MS), Ok(MIN_EPOCH_MS));
        assert_eq!(check_integer(MAX_EPOCH_MS), Ok(MAX_EPOCH_MS));
        assert!(check_integer(MIN_EPOCH_MS - 1).is_err());
        assert!(check_integer(MAX_EPOCH_MS + 1).is_err());
    }

    #[test]
    fn text_espace_est_utc() {
        assert_eq!(
            parse_text("2024-01-01 00:00:00"),
            Ok((JANVIER_2024_UTC, DateFormat::TextSpaceUtcSeconds))
        );
        // Espaces autour tolérés (valeur sale mais non ambiguë).
        assert_eq!(
            parse_text("  2024-01-01 00:00:00  "),
            Ok((JANVIER_2024_UTC, DateFormat::TextSpaceUtcSeconds))
        );
    }

    #[test]
    fn rfc3339_z_et_offset() {
        assert_eq!(
            parse_text("2024-01-01T00:00:00Z"),
            Ok((JANVIER_2024_UTC, DateFormat::Rfc3339))
        );
        assert_eq!(
            parse_text("2024-01-01T01:00:00+01:00"),
            Ok((JANVIER_2024_UTC, DateFormat::Rfc3339))
        );
        assert_eq!(
            parse_text("2024-01-01T00:00:00.500Z"),
            Ok((JANVIER_2024_UTC + 500, DateFormat::Rfc3339))
        );
    }

    #[test]
    fn text_fraction_refuse() {
        let err = parse_text("2024-01-01 00:00:00.123").unwrap_err();
        assert!(err.contains("fraction de seconde"), "{err}");
    }

    #[test]
    fn t_sans_fuseau_refuse() {
        let err = parse_text("2024-01-01T00:00:00").unwrap_err();
        assert!(err.contains("fuseau"), "{err}");
    }

    #[test]
    fn formats_invalides_refuses() {
        assert!(parse_text("").is_err());
        assert!(parse_text("2024-01-01").is_err());
        assert!(parse_text("01/01/2024 00:00:00").is_err());
        assert!(parse_text("2024-13-01 00:00:00").is_err());
        assert!(parse_text("2024-01-01 24:00:00").is_err());
        assert!(parse_text("pas une date").is_err());
    }

    #[test]
    fn stats_display() {
        let mut stats = DateStats::default();
        stats.record(DateFormat::IntegerMillis);
        stats.record(DateFormat::Rfc3339);
        stats.null = 2;
        assert_eq!(stats.total(), 4);
        let rendered = stats.to_string();
        assert!(rendered.contains("4 valeur(s)"));
        assert!(rendered.contains("2 NULL"));
    }

    #[test]
    fn format_millis_rend_du_rfc3339() {
        assert_eq!(format_millis(JANVIER_2024_UTC), "2024-01-01T00:00:00.000Z");
    }
}
