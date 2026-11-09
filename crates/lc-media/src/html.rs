//! Petits utilitaires de parsing HTML (aucune dépendance DOM).
//!
//! Les résolveurs TikTok et X lisent quelques balises `<meta>`, des scripts JSON
//! embarqués et des URL cachées dans du JavaScript. Ces helpers volontairement
//! minimalistes évitent d'introduire un parseur HTML complet pour si peu.

use serde_json::Value;

/// Caractères qui délimitent une URL dans du HTML ou du JavaScript.
const URL_BOUNDARIES: &[char] = &[
    '"', '\'', '<', '>', '(', ')', '`', ' ', '\n', '\r', '\t', '{', '}', '[', ']', ',', ';',
];

/// Cherche `needle` (insensible à la casse ASCII) à partir de `from`.
///
/// Renvoie un index d'octet garanti sur une frontière de caractère : `needle`
/// ne contient que de l'ASCII, ses octets ne peuvent pas tomber sur une
/// continuation UTF-8.
pub fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(haystack.len()));
    }
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if from >= bytes.len() || needle_bytes.len() > bytes.len().saturating_sub(from) {
        return None;
    }
    bytes[from..]
        .windows(needle_bytes.len())
        .position(|window| window.eq_ignore_ascii_case(needle_bytes))
        .map(|offset| from + offset)
}

/// Extrait les attributs d'une balise (`<meta name="x" content='y'>`).
pub fn attributes(tag: &str) -> Vec<(String, String)> {
    let bytes = tag.as_bytes();
    let mut result: Vec<(String, String)> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && !is_attribute_name_start(bytes[index]) {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let name_start = index;
        while index < bytes.len() && is_attribute_name_char(bytes[index]) {
            index += 1;
        }
        let name = tag[name_start..index].to_ascii_lowercase();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index < bytes.len() && (bytes[index] == b'"' || bytes[index] == b'\'') {
                let quote = bytes[index];
                index += 1;
                let value_start = index;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                let value = tag[value_start..index].to_string();
                index += 1;
                result.push((name, decode_html_entities(&value)));
            } else {
                let value_start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && bytes[index] != b'>'
                {
                    index += 1;
                }
                let value = tag[value_start..index].to_string();
                result.push((name, decode_html_entities(&value)));
            }
        } else {
            result.push((name, String::new()));
        }
    }
    result
}

/// Première valeur d'attribut (nom insensible à la casse).
pub fn attribute(tag: &str, name: &str) -> Option<String> {
    attributes(tag)
        .into_iter()
        .find(|(attribute_name, _)| attribute_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value)
}

/// Contenu d'une balise `<meta property|name="…" content="…">`.
pub fn meta_content(html: &str, key: &str) -> Option<String> {
    let key = key.to_ascii_lowercase();
    let mut from = 0;
    while let Some(position) = find_ci(html, "<meta", from) {
        let tag_start = position;
        let Some(end) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + end;
        let tag = &html[tag_start..=tag_end];
        let attributes = attributes(tag);
        let matches_key = attributes.iter().any(|(name, value)| {
            (name == "property" || name == "name") && value.eq_ignore_ascii_case(&key)
        });
        if matches_key {
            if let Some((_, content)) = attributes.iter().find(|(name, _)| name == "content") {
                return Some(content.clone());
            }
        }
        from = tag_end + 1;
    }
    None
}

/// Contenu d'un `<script id="…" type="application/json">…</script>`.
pub fn script_payload(html: &str, id: &str) -> Option<String> {
    let mut from = 0;
    while let Some(position) = find_ci(html, "<script", from) {
        let tag_end = html[position..].find('>').map(|offset| position + offset)?;
        let tag = &html[position..=tag_end];
        let script_id = attribute(tag, "id");
        let body_start = tag_end + 1;
        let close_offset = find_ci(html, "</script>", body_start)?;
        if script_id
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(id))
        {
            return Some(html[body_start..close_offset].to_string());
        }
        from = close_offset + "</script>".len();
    }
    None
}

/// Contenu JSON d'un script embarqué (décodé en [`serde_json::Value`]).
pub fn script_json(html: &str, id: &str) -> Option<Value> {
    let payload = script_payload(html, id)?;
    serde_json::from_str(&payload).ok()
}

/// Décode les entités HTML courantes (`&amp;`, `&#38;`, `&#x26;`, …).
pub fn decode_html_entities(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(current) = chars.next() {
        if current != '&' {
            output.push(current);
            continue;
        }
        let mut entity = String::new();
        let mut terminated = false;
        while let Some(&next) = chars.peek() {
            if next == ';' {
                chars.next();
                terminated = true;
                break;
            }
            if !(next.is_ascii_alphanumeric() || next == '#') || entity.len() >= 10 {
                break;
            }
            entity.push(next);
            chars.next();
        }
        match entity.as_str() {
            "amp" => output.push('&'),
            "lt" => output.push('<'),
            "gt" => output.push('>'),
            "quot" => output.push('"'),
            "apos" => output.push('\''),
            "nbsp" => output.push(' '),
            _ => {
                if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    if let Ok(code) = u32::from_str_radix(hex, 16) {
                        if let Some(character) = char::from_u32(code) {
                            output.push(character);
                            continue;
                        }
                    }
                }
                if let Some(decimal) = entity.strip_prefix('#') {
                    if let Ok(code) = decimal.parse::<u32>() {
                        if let Some(character) = char::from_u32(code) {
                            output.push(character);
                            continue;
                        }
                    }
                }
                output.push('&');
                output.push_str(&entity);
                if terminated {
                    output.push(';');
                }
            }
        }
    }
    output
}

/// Décode les échappements JavaScript/JSON (`\/`, `\u0026`, `\n`, …).
pub fn decode_json_escapes(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(current) = chars.next() {
        if current != '\\' {
            output.push(current);
            continue;
        }
        match chars.next() {
            Some('/') => output.push('/'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('t') => output.push('\t'),
            Some('"') => output.push('"'),
            Some('\'') => output.push('\''),
            Some('\\') => output.push('\\'),
            Some('u') => {
                let mut hex = String::with_capacity(4);
                for _ in 0..4 {
                    if let Some(digit) = chars.next() {
                        hex.push(digit);
                    }
                }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(character) => output.push(character),
                    None => {
                        output.push_str("\\u");
                        output.push_str(&hex);
                    }
                }
            }
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }
    output
}

/// Nettoie une URL trouvée dans du HTML/JS (entités, échappements, ponctuation).
pub fn clean_url(raw: &str) -> String {
    let decoded = decode_json_escapes(&decode_html_entities(raw));
    decoded
        .trim()
        .trim_matches(|character: char| matches!(character, '"' | '\'' | ',' | ';' | ')' | ']'))
        .to_string()
}

/// Toutes les URL d'un document dont le texte contient `marker`.
pub fn urls_containing(html: &str, marker: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(position) = find_ci(html, marker, from) {
        let start = html[..position]
            .rfind(|character: char| URL_BOUNDARIES.contains(&character))
            .map(|index| index + 1)
            .unwrap_or(0);
        let end = html[position..]
            .find(|character: char| URL_BOUNDARIES.contains(&character))
            .map(|offset| position + offset)
            .unwrap_or(html.len());
        let cleaned = clean_url(&html[start..end]);
        if cleaned.starts_with("http") && !urls.iter().any(|url| url == &cleaned) {
            urls.push(cleaned);
        }
        from = position + marker.len();
    }
    urls
}

/// Toutes les URL commençant par `prefix` (ex. `https://video.twimg.com/`).
pub fn urls_starting_with(html: &str, prefix: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(position) = find_ci(html, prefix, from) {
        let end = html[position..]
            .find(|character: char| URL_BOUNDARIES.contains(&character) && character != ':')
            .map(|offset| position + offset)
            .unwrap_or(html.len());
        let cleaned = clean_url(&html[position..end]);
        if !urls.iter().any(|url| url == &cleaned) {
            urls.push(cleaned);
        }
        from = position + prefix.len();
    }
    urls
}

/// Supprime tous les blocs `<tag …>…</tag>` (insensible à la casse).
pub fn remove_tag_blocks(html: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(relative_start) = find_ci(&html[cursor..], &open, 0) {
        let start = cursor + relative_start;
        output.push_str(&html[cursor..start]);
        match find_ci(&html[start..], &close, 0) {
            Some(relative_end) => {
                cursor = start + relative_end + close.len();
            }
            None => {
                cursor = html.len();
                break;
            }
        }
    }
    output.push_str(&html[cursor..]);
    output
}

/// Nettoie un HTML de citation (retire `<script>` et `<iframe>`).
pub fn sanitize_embed_html(html: &str) -> String {
    let without_scripts = remove_tag_blocks(html, "script");
    remove_tag_blocks(&without_scripts, "iframe")
}

fn is_attribute_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b':'
}

fn is_attribute_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn find_ci_is_case_insensitive_and_bounded() {
        assert_eq!(find_ci("Hello <META name", "<meta", 0), Some(6));
        assert_eq!(find_ci("Hello", "meta", 0), None);
        assert_eq!(find_ci("Hello", "", 99), Some(5));
        assert_eq!(find_ci("héllo", "hello", 0), None);
    }

    #[test]
    fn attribute_parsing_handles_quotes_and_entities() {
        let tag = r#"<meta property="og:video:width" content="1920" data-x=7 name='a&amp;b'>"#;
        let attributes = attributes(tag);
        assert!(attributes
            .iter()
            .any(|(name, value)| name == "property" && value == "og:video:width"));
        assert_eq!(attribute(tag, "content").as_deref(), Some("1920"));
        assert_eq!(attribute(tag, "data-x").as_deref(), Some("7"));
        assert_eq!(attribute(tag, "name").as_deref(), Some("a&b"));
        assert_eq!(attribute(tag, "absent"), None);
    }

    #[test]
    fn meta_content_supports_property_and_name() {
        let html = r#"<html><head>
            <meta charset="utf-8">
            <meta property="og:video:secure_url" content="https://video.twimg.com/a.mp4?x=1&amp;y=2">
            <meta name="twitter:player:stream" content='https://video.twimg.com/b.mp4'>
        </head></html>"#;
        assert_eq!(
            meta_content(html, "og:video:secure_url").as_deref(),
            Some("https://video.twimg.com/a.mp4?x=1&y=2")
        );
        assert_eq!(
            meta_content(html, "TWITTER:PLAYER:STREAM").as_deref(),
            Some("https://video.twimg.com/b.mp4")
        );
        assert_eq!(meta_content(html, "og:missing"), None);
    }

    #[test]
    fn script_payload_returns_body_and_json() {
        let html = r#"<html><body>
            <script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{"a":1}</script>
            <script id="other">ignored</script>
        </body></html>"#;
        assert_eq!(
            script_payload(html, "__UNIVERSAL_DATA_FOR_REHYDRATION__").as_deref(),
            Some(r#"{"a":1}"#)
        );
        assert_eq!(
            script_json(html, "__UNIVERSAL_DATA_FOR_REHYDRATION__").unwrap()["a"],
            1
        );
        assert_eq!(script_payload(html, "missing"), None);
        assert_eq!(script_json(html, "other"), None);
    }

    #[test]
    fn entity_decoding_handles_named_and_numeric_entities() {
        assert_eq!(
            decode_html_entities("a &amp; b &lt;c&gt; &quot;d&quot; &#39;e&#39;"),
            "a & b <c> \"d\" 'e'"
        );
        assert_eq!(decode_html_entities("&#x26; &#38;"), "& &");
        assert_eq!(
            decode_html_entities("unknown &nope; and & alone"),
            "unknown &nope; and & alone"
        );
        assert_eq!(decode_html_entities("no entities"), "no entities");
    }

    #[test]
    fn json_escape_decoding_handles_unicode_and_slashes() {
        assert_eq!(
            decode_json_escapes(r"https:\/\/x.test\/a.mp4?a=1\u0026b=2"),
            "https://x.test/a.mp4?a=1&b=2"
        );
        assert_eq!(decode_json_escapes(r"ligne\nsuivante"), "ligne\nsuivante");
        assert_eq!(decode_json_escapes(r"inconnu \q"), r"inconnu \q");
        assert_eq!(decode_json_escapes(r"caf\u00e9"), "café");
    }

    #[test]
    fn urls_containing_extracts_js_escaped_urls() {
        let html = r#"<script>var x = {"url":"https:\/\/p16-sign.tiktokcdn.com\/abc~tplv-photomode-image.jpeg?x=1\u0026y=2"};</script>"#;
        let urls = urls_containing(html, "~tplv-photomode-image");
        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0],
            "https://p16-sign.tiktokcdn.com/abc~tplv-photomode-image.jpeg?x=1&y=2"
        );
    }

    #[test]
    fn urls_containing_dedupes() {
        let html = r#"<img src="https://a.test/thumb~tplv-photomode-image.jpg"><img src="https://a.test/thumb~tplv-photomode-image.jpg">"#;
        assert_eq!(urls_containing(html, "~tplv-photomode-image").len(), 1);
    }

    #[test]
    fn urls_starting_with_collects_cdn_urls() {
        let html = r#"<meta content="https://video.twimg.com/ext/a.mp4?tag=1">
            <script>var u="https://video.twimg.com/ext/b.mp4";</script>"#;
        let urls = urls_starting_with(html, "https://video.twimg.com/");
        assert_eq!(urls.len(), 2);
        assert!(urls[0].starts_with("https://video.twimg.com/ext/"));
        assert!(urls[1].ends_with("b.mp4"));
    }

    #[test]
    fn sanitize_embed_html_strips_scripts_and_iframes() {
        let html = r#"<blockquote class="twitter-tweet"><p>Hello</p><script async src="https://platform.twitter.com/widgets.js"></script><iframe src="x"></iframe></blockquote>"#;
        let sanitized = sanitize_embed_html(html);
        assert!(sanitized.contains("<p>Hello</p>"));
        assert!(!sanitized.to_ascii_lowercase().contains("<script"));
        assert!(!sanitized.to_ascii_lowercase().contains("<iframe"));
        assert!(!sanitized.contains("widgets.js"));
    }

    #[test]
    fn remove_tag_blocks_is_case_insensitive() {
        let html = "<div>A<SCRIPT type=\"text/javascript\">x</SCRIPT>B<script>y</script>C</div>";
        assert_eq!(remove_tag_blocks(html, "script"), "<div>ABC</div>");
        assert_eq!(remove_tag_blocks(html, "style"), html);
    }
}
