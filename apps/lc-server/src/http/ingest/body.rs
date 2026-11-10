//! Normalisation du corps JSON de `POST /ingest`.
//!
//! Le legacy validait avec zod puis appliquait des conversions maison
//! (`stringUtils.ts`) : seules les chaînes non vides sont retenues, les
//! booléens ne sont acceptés que s'ils sont de vrais booléens (`showText`), les
//! drapeaux tolèrent `1`/`"true"`/`"yes"`/`"on"` (`forceRefresh`,
//! `saveToBoard`), et les durées sont des nombres finis strictement positifs
//! arrondis au supérieur.
//!
//! Aucun rejet ici : la validation applicative (champs obligatoires, conflits)
//! reste dans le handler pour préserver l'ordre exact des réponses legacy.

use serde_json::Value;

/// Corps de `POST /ingest` après normalisation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NormalizedIngest {
    /// `guildId` demandé (doit correspondre à la guilde du client).
    pub requested_guild_id: Option<String>,
    /// URL de média à ingérer.
    pub url: Option<String>,
    /// Source média directe (prioritaire sur `url`).
    pub media: Option<String>,
    /// Texte à afficher dans l'overlay.
    pub text: Option<String>,
    /// `showText` explicite (vrai booléen uniquement).
    pub show_text: Option<bool>,
    /// Force une ré-ingestion même si un asset en cache existe.
    pub force_refresh: bool,
    /// Enregistre le média sur le meme board au lieu de créer un job.
    pub save_to_board: bool,
    /// Auteur demandé (refusé pour un client d'ingestion).
    pub author_name: Option<String>,
    /// Image d'auteur demandée (refusée pour un client d'ingestion).
    pub author_image: Option<String>,
    /// Durée demandée en secondes (ceil, > 0).
    pub duration_sec: Option<i64>,
}

impl NormalizedIngest {
    /// Normalise un corps JSON (objet ou `null`).
    pub fn from_value(value: &Value) -> Self {
        Self {
            requested_guild_id: non_empty_string(value.get("guildId")),
            url: non_empty_string(value.get("url")),
            media: non_empty_string(value.get("media")),
            text: non_empty_string(value.get("text")),
            show_text: optional_boolean(value.get("showText")),
            force_refresh: boolean_flag(value.get("forceRefresh")),
            save_to_board: boolean_flag(value.get("saveToBoard")),
            author_name: non_empty_string(value.get("authorName")),
            author_image: non_empty_string(value.get("authorImage")),
            duration_sec: optional_duration_sec(value.get("durationSec")),
        }
    }
}

/// Nom d'appareil demandé lors de l'appairage (`deviceName`).
pub fn device_name(value: &Value) -> Option<String> {
    non_empty_string(value.get("deviceName"))
}

/// Chaîne JSON non vide après `trim` ; tout autre type → `None`.
pub fn non_empty_string(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_owned())
    }
}

/// Booléen strict (`showText`) : seul un vrai booléen JSON est accepté.
pub fn optional_boolean(value: Option<&Value>) -> Option<bool> {
    value?.as_bool()
}

/// Drapeau tolérant du legacy : booléen, `1`, `"1"`, `"true"`, `"yes"`, `"on"`.
pub fn boolean_flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_i64() == Some(1),
        Some(Value::String(raw)) => matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        _ => false,
    }
}

/// Durée en secondes : nombre fini strictement positif, arrondi au supérieur.
pub fn optional_duration_sec(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_f64()?;
    if !number.is_finite() || number <= 0.0 {
        return None;
    }
    let ceil = number.ceil();
    if ceil > i64::MAX as f64 {
        return None;
    }
    Some(ceil as i64)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use serde_json::json;

    use super::*;

    #[test]
    fn normalizes_a_complete_body() {
        let value = json!({
            "guildId": " 123456789012345678 ",
            "url": " https://example.com/video ",
            "text": "  bonjour  ",
            "showText": false,
            "forceRefresh": "true",
            "saveToBoard": 1,
            "authorName": "Jack",
            "durationSec": 4.2
        });
        let normalized = NormalizedIngest::from_value(&value);

        assert_eq!(
            normalized.requested_guild_id.as_deref(),
            Some("123456789012345678")
        );
        assert_eq!(normalized.url.as_deref(), Some("https://example.com/video"));
        assert_eq!(normalized.text.as_deref(), Some("bonjour"));
        assert_eq!(normalized.show_text, Some(false));
        assert!(normalized.force_refresh);
        assert!(normalized.save_to_board);
        assert_eq!(normalized.author_name.as_deref(), Some("Jack"));
        assert_eq!(normalized.duration_sec, Some(5));
    }

    #[test]
    fn empty_and_wrong_types_are_ignored() {
        let value = json!({
            "guildId": "   ",
            "url": 42,
            "showText": "true",
            "durationSec": -1,
            "authorName": null
        });
        let normalized = NormalizedIngest::from_value(&value);

        assert!(normalized.requested_guild_id.is_none());
        assert!(normalized.url.is_none());
        assert!(normalized.show_text.is_none());
        assert!(normalized.duration_sec.is_none());
        assert!(normalized.author_name.is_none());
    }

    #[test]
    fn boolean_flags_follow_legacy_rules() {
        assert!(boolean_flag(Some(&json!(true))));
        assert!(boolean_flag(Some(&json!(1))));
        assert!(boolean_flag(Some(&json!("TRUE"))));
        assert!(boolean_flag(Some(&json!("yes"))));
        assert!(boolean_flag(Some(&json!(" on "))));
        assert!(!boolean_flag(Some(&json!(false))));
        assert!(!boolean_flag(Some(&json!(0))));
        assert!(!boolean_flag(Some(&json!("non"))));
        assert!(!boolean_flag(None));
    }

    #[test]
    fn duration_is_ceiled_and_positive_only() {
        assert_eq!(optional_duration_sec(Some(&json!(4.2))), Some(5));
        assert_eq!(optional_duration_sec(Some(&json!(4.0))), Some(4));
        assert_eq!(optional_duration_sec(Some(&json!(0))), None);
        assert_eq!(optional_duration_sec(Some(&json!(-3))), None);
        assert_eq!(optional_duration_sec(Some(&json!("4"))), None);
        assert_eq!(optional_duration_sec(None), None);
    }
}
