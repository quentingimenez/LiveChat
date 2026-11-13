//! Utilitaires de texte repris du legacy (`text`, `mediaSourceResolver`).
//!
//! On y trouve :
//! - `start_case` : équivalent de `lodash.startCase` utilisé par `/alive` ;
//! - `to_optional_text` : équivalent de `toNonEmptyString(..).slice(0, max)` ;
//! - `youtube_start_offset_sec` : équivalent de `extractMediaStartOffsetSec`
//!   (seules les URLs YouTube sont prises en charge, comme le legacy).

/// Décompose une chaîne en mots comme `lodash.words` (approximation fidèle
/// pour les identifiants Discord usuels : séparateurs non alphanumériques,
/// transitions minuscule→majuscule, sigles et frontière lettres/chiffres).
///
/// Les diacritiques sont supprimés comme `lodash.deburr`, et les apostrophes
/// sont retirées avant découpage (`lodash.startCase` les considère comme des
/// séparateurs).
fn split_words(input: &str) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Class {
        Lower,
        Upper,
        Digit,
        Other,
    }

    fn class_of(ch: char) -> Class {
        if ch.is_ascii_digit() || (!ch.is_alphabetic() && ch.is_numeric()) {
            Class::Digit
        } else if ch.is_alphabetic() {
            if ch.is_uppercase() {
                Class::Upper
            } else {
                Class::Lower
            }
        } else {
            Class::Other
        }
    }

    let chars: Vec<(char, Class)> = input
        .chars()
        .filter(|ch| *ch != '\'' && *ch != '\u{2019}')
        .map(|ch| (ch, class_of(ch)))
        .collect();

    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, (ch, class)) in chars.iter().enumerate() {
        let boundary = match index {
            0 => false,
            _ => {
                let previous = chars[index - 1].1;
                match (previous, class) {
                    (Class::Lower, Class::Upper) | (Class::Digit, Class::Upper) => true,
                    (Class::Upper, Class::Lower) => {
                        index >= 2 && chars[index - 2].1 == Class::Upper
                    }
                    (Class::Digit, Class::Lower) | (Class::Lower, Class::Digit) => true,
                    _ => false,
                }
            }
        };
        if *class == Class::Other {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if boundary && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(*ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Supprime les diacritiques latins usuels (équivalent partiel de
/// `lodash.deburr`).
fn deburr(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => {
                'a'
            }
            'ç' | 'Ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => 'i',
            'ñ' | 'Ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => 'u',
            'ý' | 'ÿ' | 'Ý' | 'Ÿ' => 'y',
            'œ' | 'Œ' => 'e',
            'æ' | 'Æ' => 'a',
            other => other,
        })
        .collect()
}

/// Met la première lettre de chaque mot en majuscule et joint par une espace
/// (équivalent de `lodash.startCase`, utilisé par le message de `/alive`).
pub fn start_case(input: &str) -> String {
    let deburred = deburr(input);
    split_words(&deburred)
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let mut out = String::with_capacity(word.len());
                    for upper in first.to_uppercase() {
                        out.push(upper);
                    }
                    out.push_str(chars.as_str());
                    out
                }
            }
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Compacte les espaces et supprime ceux de début et de fin.
pub fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Équivalent de `toNonEmptyString(value)?.slice(0, maxChars)`.
///
/// `None` si la valeur vaut `None`, est vide après `trim`, ou si `max_chars`
/// vaut 0. La troncature compte des caractères Unicode (le legacy comptait des
/// unités UTF-16 ; la différence ne concerne que les caractères hors BMP).
pub fn to_optional_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() || max_chars == 0 {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

/// Parse une durée « lisible » : `90`, `90s`, `1h2m3s`, `1:30`, `1:02:03`.
///
/// Retourne `None` si la valeur est invalide ou nulle (le legacy ne conserve
/// que les offsets strictement positifs).
pub fn parse_start_offset(raw: &str) -> Option<i64> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let seconds = if value.contains(':') {
        parse_clock_offset(value)?
    } else {
        parse_duration_offset(value)?
    };
    if seconds > 0 {
        Some(seconds)
    } else {
        None
    }
}

fn parse_clock_offset(value: &str) -> Option<i64> {
    let parts: Vec<&str> = value.split(':').collect();
    let seconds = match parts.as_slice() {
        [minutes, seconds] => {
            let minutes = parse_i64(minutes)?;
            let seconds = parse_i64(seconds)?;
            minutes.checked_mul(60)?.checked_add(seconds)?
        }
        [hours, minutes, seconds] => {
            let hours = parse_i64(hours)?;
            let minutes = parse_i64(minutes)?;
            let seconds = parse_i64(seconds)?;
            hours
                .checked_mul(3_600)?
                .checked_add(minutes.checked_mul(60)?)?
                .checked_add(seconds)?
        }
        _ => return None,
    };
    Some(seconds)
}

fn parse_duration_offset(value: &str) -> Option<i64> {
    // Le legacy accepte un entier nu (secondes), ex. `t=90`.
    if !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()) {
        return parse_i64(value);
    }
    let mut digits = String::new();
    let mut total: i64 = 0;
    let mut has_unit = false;
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let unit = match ch.to_ascii_lowercase() {
            'h' => 3_600,
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        if digits.is_empty() {
            return None;
        }
        let amount = parse_i64(&digits)?;
        total = total.checked_add(amount.checked_mul(unit)?)?;
        digits.clear();
        has_unit = true;
    }
    if !digits.is_empty() {
        // « 2m30 » : les chiffres finaux sont des secondes implicites.
        total = total.checked_add(parse_i64(&digits)?)?;
    }
    if has_unit {
        Some(total)
    } else {
        None
    }
}

fn parse_i64(value: &str) -> Option<i64> {
    value.parse::<i64>().ok()
}

/// Extrait un offset de départ depuis une URL YouTube, comme le legacy
/// (`extractMediaStartOffsetSec`) : paramètres `t`/`start` en query string ou
/// fragment (`#t=1m30s`), uniquement pour YouTube, et seulement si > 0.
pub fn youtube_start_offset_sec(url: Option<&str>, media: Option<&str>) -> Option<i64> {
    let source = media.or(url)?.trim();
    if source.is_empty() {
        return None;
    }
    let lower = source.to_ascii_lowercase();
    if !(lower.contains("youtube.com") || lower.contains("youtu.be")) {
        return None;
    }
    if let Some((_, query)) = source.split_once('?') {
        let query = query.split('#').next().unwrap_or(query);
        if let Some(offset) = offset_from_pairs(query.split('&')) {
            return Some(offset);
        }
    }
    if let Some((_, fragment)) = source.split_once('#') {
        if let Some(offset) = offset_from_pairs(fragment.split('&')) {
            return Some(offset);
        }
    }
    None
}

fn offset_from_pairs<'a>(pairs: impl Iterator<Item = &'a str>) -> Option<i64> {
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if !(key.eq_ignore_ascii_case("t") || key.eq_ignore_ascii_case("start")) {
            continue;
        }
        if let Some(offset) = parse_start_offset(value) {
            return Some(offset);
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        normalize_whitespace, parse_start_offset, start_case, to_optional_text,
        youtube_start_offset_sec,
    };

    #[test]
    fn start_case_comme_lodash() {
        assert_eq!(start_case("john"), "John");
        assert_eq!(start_case("john_doe"), "John Doe");
        assert_eq!(start_case("JOHN DOE"), "JOHN DOE");
        assert_eq!(start_case("jean-luc"), "Jean Luc");
        assert_eq!(start_case("fooBar"), "Foo Bar");
        assert_eq!(start_case("LiveChatBot"), "Live Chat Bot");
        assert_eq!(start_case("abc123def"), "Abc 123 Def");
        assert_eq!(start_case("élodie"), "Elodie");
        assert_eq!(start_case("renée o'connor"), "Renee Oconnor");
        assert_eq!(start_case(""), "");
        assert_eq!(start_case("   "), "");
    }

    #[test]
    fn normalisation_espaces() {
        assert_eq!(normalize_whitespace("  a   b \n c "), "a b c");
        assert_eq!(normalize_whitespace(""), "");
    }

    #[test]
    fn texte_optionnel_tronque() {
        assert_eq!(
            to_optional_text(Some("  hello  "), 240),
            Some("hello".to_owned())
        );
        assert_eq!(to_optional_text(Some(""), 240), None);
        assert_eq!(to_optional_text(Some("   "), 240), None);
        assert_eq!(to_optional_text(None, 240), None);
        assert_eq!(to_optional_text(Some("abcdef"), 3), Some("abc".to_owned()));
        assert_eq!(to_optional_text(Some("abc"), 0), None);
    }

    #[test]
    fn parse_offsets() {
        assert_eq!(parse_start_offset("90"), Some(90));
        assert_eq!(parse_start_offset("90s"), Some(90));
        assert_eq!(parse_start_offset("1h2m3s"), Some(3_723));
        assert_eq!(parse_start_offset("2m30"), Some(150));
        assert_eq!(parse_start_offset("1:30"), Some(90));
        assert_eq!(parse_start_offset("1:02:03"), Some(3_723));
        assert_eq!(parse_start_offset("0"), None);
        assert_eq!(parse_start_offset("0s"), None);
        assert_eq!(parse_start_offset("abc"), None);
        assert_eq!(parse_start_offset("1h2x"), None);
        assert_eq!(parse_start_offset("  "), None);
    }

    #[test]
    fn offsets_youtube() {
        assert_eq!(
            youtube_start_offset_sec(Some("https://www.youtube.com/watch?v=abc&t=90"), None),
            Some(90)
        );
        assert_eq!(
            youtube_start_offset_sec(None, Some("https://youtu.be/abc?start=1m30s")),
            Some(90)
        );
        assert_eq!(
            youtube_start_offset_sec(Some("https://www.youtube.com/watch?v=abc#t=1:02"), None),
            Some(62)
        );
        assert_eq!(
            youtube_start_offset_sec(Some("https://example.com/watch?t=90"), None),
            None
        );
        assert_eq!(
            youtube_start_offset_sec(Some("https://www.youtube.com/watch?v=abc"), None),
            None
        );
        assert_eq!(youtube_start_offset_sec(None, None), None);
    }
}
