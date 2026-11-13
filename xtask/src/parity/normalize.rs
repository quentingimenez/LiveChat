//! Normalisation des réponses avant comparaison.
//!
//! La normalisation est volontairement conservatrice : elle ne touche qu'aux
//! valeurs intrinsèquement volatiles (dates, UUID, chemins de stockage, jetons)
//! et aux pointeurs explicitement déclarés par le cas (`ignore`, `volatile`,
//! `sort_arrays`). Les valeurs stables (codes d'erreur, libellés, statuts) sont
//! comparées telles quelles.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::parity::case::CompareRules;
use crate::parity::request::Body;

/// Valeur de remplacement d'un champ déclaré volatile.
pub const VOLATILE_PLACEHOLDER: &str = "<volatile>";
/// Valeur de remplacement d'un horodatage ISO 8601.
pub const TIMESTAMP_PLACEHOLDER: &str = "<timestamp>";
/// Valeur de remplacement d'un horodatage epoch millisecondes.
pub const TIMESTAMP_MS_PLACEHOLDER: &str = "<timestamp_ms>";
/// Valeur de remplacement d'un UUID.
pub const UUID_PLACEHOLDER: &str = "<uuid>";
/// Valeur de remplacement d'un chemin de stockage local.
pub const STORAGE_PATH_PLACEHOLDER: &str = "<storage_path>";
/// Valeur de remplacement d'un jeton ou d'une signature.
pub const REDACTED_PLACEHOLDER: &str = "<redacted>";
/// Valeur de remplacement de `x-ratelimit-remaining` (variable selon l'état).
pub const RATE_LIMIT_REMAINING_PLACEHOLDER: &str = "<rate_limit_remaining>";
/// Valeur de remplacement de `x-ratelimit-reset` (epoch ou durée variable).
pub const RATE_LIMIT_RESET_PLACEHOLDER: &str = "<rate_limit_reset>";
/// Valeur de remplacement de `retry-after` (durée variable).
pub const RETRY_AFTER_PLACEHOLDER: &str = "<retry_after>";

const ISO_TIMESTAMP_PATTERN: &str =
    r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?";
const UUID_PATTERN: &str =
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
const SIGNED_QUERY_PATTERN: &str =
    r#"([?&](?:sig|exp|signature|token|mediaToken|media_token)=)[^&\s"']+"#;
const BEARER_PATTERN: &str = r"Bearer\s+[A-Za-z0-9._~+/=-]+";
const STORAGE_PATH_PATTERN: &str = r#"(?:[A-Za-z]:\\[^"'\s<>]*|[A-Za-z]:/[^/"'\s<>][^"'\s<>]*|/(?:[^"'\s<>]*/)*data[\\/]media[^"'\s<>]*)"#;

/// Epoch millisecondes raisonnable : 2001-09-09 → 2100 (bornes larges).
const EPOCH_MS_MIN: i64 = 1_000_000_000_000;
/// Borne haute des horodatages epoch millisecondes normalisés.
const EPOCH_MS_MAX: i64 = 4_102_444_800_000;

fn compiled(cell: &'static OnceLock<Option<Regex>>, pattern: &str) -> Option<&'static Regex> {
    cell.get_or_init(|| Regex::new(pattern).ok()).as_ref()
}

fn iso_timestamp_regex() -> Option<&'static Regex> {
    static CELL: OnceLock<Option<Regex>> = OnceLock::new();
    compiled(&CELL, ISO_TIMESTAMP_PATTERN)
}

fn uuid_regex() -> Option<&'static Regex> {
    static CELL: OnceLock<Option<Regex>> = OnceLock::new();
    compiled(&CELL, UUID_PATTERN)
}

fn signed_query_regex() -> Option<&'static Regex> {
    static CELL: OnceLock<Option<Regex>> = OnceLock::new();
    compiled(&CELL, SIGNED_QUERY_PATTERN)
}

fn bearer_regex() -> Option<&'static Regex> {
    static CELL: OnceLock<Option<Regex>> = OnceLock::new();
    compiled(&CELL, BEARER_PATTERN)
}

fn storage_path_regex() -> Option<&'static Regex> {
    static CELL: OnceLock<Option<Regex>> = OnceLock::new();
    compiled(&CELL, STORAGE_PATH_PATTERN)
}

/// Normalise un texte libre : horodatages, UUID, signatures, chemins de stockage.
pub fn normalize_text(text: &str) -> String {
    let mut normalized = text.to_string();
    if let Some(regex) = iso_timestamp_regex() {
        normalized = regex
            .replace_all(&normalized, TIMESTAMP_PLACEHOLDER)
            .into_owned();
    }
    if let Some(regex) = uuid_regex() {
        normalized = regex
            .replace_all(&normalized, UUID_PLACEHOLDER)
            .into_owned();
    }
    if let Some(regex) = signed_query_regex() {
        normalized = regex
            .replace_all(&normalized, format!("${{1}}{REDACTED_PLACEHOLDER}"))
            .into_owned();
    }
    if let Some(regex) = bearer_regex() {
        normalized = regex
            .replace_all(&normalized, format!("Bearer {REDACTED_PLACEHOLDER}"))
            .into_owned();
    }
    if let Some(regex) = storage_path_regex() {
        normalized = regex
            .replace_all(&normalized, STORAGE_PATH_PLACEHOLDER)
            .into_owned();
    }
    normalized
}

/// Normalise un corps complet (JSON, texte ou vide).
pub fn normalize_body(body: &Body, rules: &CompareRules) -> Body {
    match body {
        Body::Empty => Body::Empty,
        Body::Text { value } => Body::Text {
            value: normalize_text(value),
        },
        Body::Json { value } => Body::Json {
            value: normalize_value(value, "", rules).unwrap_or(Value::Null),
        },
    }
}

/// Normalise récursivement une valeur JSON ; `None` signifie « supprimée » (ignore).
pub fn normalize_value(value: &Value, pointer: &str, rules: &CompareRules) -> Option<Value> {
    if rules
        .ignore
        .iter()
        .any(|pattern| matches_pattern(pattern, pointer))
    {
        return None;
    }
    if rules
        .volatile
        .iter()
        .any(|pattern| matches_pattern(pattern, pointer))
    {
        return Some(Value::String(VOLATILE_PLACEHOLDER.to_string()));
    }

    let normalized = match value {
        Value::Object(map) => {
            let mut output = serde_json::Map::new();
            for (key, child) in map {
                let child_pointer = format!("{pointer}/{}", escape_pointer_token(key));
                if let Some(normalized_child) = normalize_value(child, &child_pointer, rules) {
                    output.insert(key.clone(), normalized_child);
                }
            }
            Value::Object(output)
        }
        Value::Array(items) => {
            let mut output = Vec::with_capacity(items.len());
            for (index, child) in items.iter().enumerate() {
                let child_pointer = format!("{pointer}/{index}");
                if let Some(normalized_child) = normalize_value(child, &child_pointer, rules) {
                    output.push(normalized_child);
                }
            }
            let mut array = Value::Array(output);
            if rules
                .sort_arrays
                .iter()
                .any(|pattern| matches_pattern(pattern, pointer))
            {
                if let Value::Array(items) = &mut array {
                    items.sort_by_key(Value::to_string);
                }
            }
            array
        }
        Value::String(text) => Value::String(normalize_text(text)),
        Value::Number(number) => normalize_number(number),
        other => other.clone(),
    };

    Some(normalized)
}

fn normalize_number(number: &serde_json::Number) -> Value {
    if let Some(value) = number.as_i64() {
        if (EPOCH_MS_MIN..=EPOCH_MS_MAX).contains(&value) {
            return Value::String(TIMESTAMP_MS_PLACEHOLDER.to_string());
        }
    }
    Value::Number(number.clone())
}

fn escape_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Indique si un pointeur JSON correspond à un motif (`*` = un segment,
/// `**` = zéro ou plusieurs segments).
pub fn matches_pattern(pattern: &str, pointer: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
    let pointer_segments: Vec<&str> = pointer.split('/').filter(|part| !part.is_empty()).collect();
    segments_match(&pattern_segments, &pointer_segments)
}

fn segments_match(pattern: &[&str], pointer: &[&str]) -> bool {
    match pattern.split_first() {
        None => pointer.is_empty(),
        Some((head, rest)) => match *head {
            "**" => (0..=pointer.len()).any(|skip| segments_match(rest, &pointer[skip..])),
            "*" => match pointer.split_first() {
                Some((_, tail)) => segments_match(rest, tail),
                None => false,
            },
            literal => match pointer.split_first() {
                Some((actual, tail)) => *actual == literal && segments_match(rest, tail),
                None => false,
            },
        },
    }
}
