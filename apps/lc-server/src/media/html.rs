//! Nettoyage des fragments HTML de carte tweet servis aux overlays.
//!
//! Le HTML de citation fourni par oEmbed n'est jamais servi tel quel : il est
//! filtré par une allowlist stricte de balises et d'attributs, sans script,
//! sans style, sans gestionnaire d'événement et sans URL `javascript:`
//! (AUD-COR-15 / AUD-A2-015). Ce module est autonome (il ne dépend pas du
//! nettoyeur de `lc-media`, destiné aux réponses HTTP internes) pour garder
//! une surface minimale côté serveur.

/// Balises conservées dans un fragment de carte tweet.
const ALLOWED_TAGS: &[&str] = &[
    "p",
    "a",
    "span",
    "br",
    "strong",
    "em",
    "b",
    "i",
    "blockquote",
];

/// Attributs conservés dans un fragment de carte tweet.
const ALLOWED_ATTRIBUTES: &[&str] = &["href", "src", "alt", "title", "class", "dir", "lang"];

/// Blocs retirés intégralement (contenu compris) avant filtrage.
const REMOVED_BLOCKS: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "form", "video",
];

/// Nettoie un fragment HTML de carte tweet.
///
/// Les balises hors allowlist sont retirées en conservant leur contenu
/// textuel ; seules les URL `http(s)` sont acceptées pour `href`/`src`, et un
/// lien reçoit toujours `rel="nofollow noopener noreferrer" target="_blank"`.
pub fn sanitize_tweet_embed(html: &str) -> String {
    let mut cleaned = html.to_string();
    for block in REMOVED_BLOCKS {
        cleaned = remove_block(&cleaned, block);
    }

    let mut output = String::with_capacity(cleaned.len());
    let bytes = cleaned.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            let start = index;
            while index < bytes.len() && bytes[index] != b'<' {
                index += 1;
            }
            output.push_str(&cleaned[start..index]);
            continue;
        }
        if cleaned[index..].starts_with("<!--") {
            match cleaned[index + 4..].find("-->") {
                Some(offset) => index = index + 4 + offset + 3,
                None => break,
            }
            continue;
        }
        match tag_end(bytes, index) {
            Some(end) => {
                emit_tag(&cleaned[index..=end], &mut output);
                index = end + 1;
            }
            None => {
                output.push_str(&cleaned[index..]);
                break;
            }
        }
    }
    output
}

/// Supprime les blocs `<tag>…</tag>` (insensible à la casse).
fn remove_block(html: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(relative_start) = find_ci(&html[cursor..], &open) {
        let start = cursor + relative_start;
        output.push_str(&html[cursor..start]);
        match find_ci(&html[start..], &close) {
            Some(relative_end) => cursor = start + relative_end + close.len(),
            None => {
                cursor = html.len();
                break;
            }
        }
    }
    output.push_str(&html[cursor..]);
    output
}

/// Recherche ASCII insensible à la casse.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Indice du `>` fermant la balise, en ignorant les `>` entre guillemets.
fn tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(expected) => {
                if byte == expected {
                    quote = None;
                }
            }
            None => {
                if byte == b'"' || byte == b'\'' {
                    quote = Some(byte);
                } else if byte == b'>' {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

/// Ré-émet la balise si elle est autorisée, en filtrant ses attributs.
fn emit_tag(tag: &str, output: &mut String) {
    let lower = tag.to_ascii_lowercase();
    let is_closing = lower.starts_with("</");
    let name = lower
        .trim_start_matches('<')
        .trim_start_matches('/')
        .split(|character: char| {
            character.is_ascii_whitespace() || character == '>' || character == '/'
        })
        .next()
        .unwrap_or_default()
        .to_string();
    if name.is_empty() || !ALLOWED_TAGS.contains(&name.as_str()) {
        return;
    }
    if is_closing {
        output.push_str("</");
        output.push_str(&name);
        output.push('>');
        return;
    }

    output.push('<');
    output.push_str(&name);
    let mut has_href = false;
    for (attribute, value) in attributes(tag) {
        if !ALLOWED_ATTRIBUTES.contains(&attribute.as_str()) {
            continue;
        }
        if attribute == "href" || attribute == "src" {
            let value_lower = value.to_ascii_lowercase();
            if !(value_lower.starts_with("http://") || value_lower.starts_with("https://")) {
                continue;
            }
            if attribute == "href" {
                has_href = true;
            }
        }
        output.push(' ');
        output.push_str(&attribute);
        output.push_str("=\"");
        escape_attribute(&value, output);
        output.push('"');
    }
    if name == "a" && has_href {
        output.push_str(" rel=\"nofollow noopener noreferrer\" target=\"_blank\"");
    } else if name == "br" {
        output.push_str(" /");
    }
    output.push('>');
}

/// Extrait les attributs d'une balise, en décodant les entités usuelles.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let bytes = tag.as_bytes();
    let mut result = Vec::new();
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
                result.push((name, decode_entities(&value)));
            } else {
                let value_start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && bytes[index] != b'>'
                {
                    index += 1;
                }
                let value = tag[value_start..index].to_string();
                result.push((name, decode_entities(&value)));
            }
        } else {
            result.push((name, String::new()));
        }
    }
    result
}

/// Échappe une valeur d'attribut avant ré-émission.
fn escape_attribute(value: &str, output: &mut String) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            other => output.push(other),
        }
    }
}

/// Décode les entités HTML courantes des valeurs d'attributs.
fn decode_entities(raw: &str) -> String {
    raw.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
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
    fn garde_les_balises_autorisees() {
        let sanitized = sanitize_tweet_embed(
            r#"<blockquote class="twitter-tweet"><p>Bonjour <strong>monde</strong></p></blockquote>"#,
        );
        assert_eq!(
            sanitized,
            r#"<blockquote class="twitter-tweet"><p>Bonjour <strong>monde</strong></p></blockquote>"#
        );
    }

    #[test]
    fn supprime_scripts_et_blocs_actifs_avec_leur_contenu() {
        let html = r#"<p>a</p><script>alert(1)</script><iframe src="x">fallback</iframe><style>p{}</style>"#;
        let sanitized = sanitize_tweet_embed(html);
        assert!(sanitized.contains("<p>a</p>"));
        for forbidden in ["alert(1)", "fallback", "iframe", "style", "p{}"] {
            assert!(!sanitized.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn retire_les_gestionnaires_et_urls_dangereuses() {
        let html = r#"<p onclick="alert(1)">x</p><a href="javascript:alert(1)">bad</a><a href="https://ok.test/a?x=1&amp;y=2">ok</a><img src="https://x.test/a.png"><span onmouseover="x">s</span>"#;
        let sanitized = sanitize_tweet_embed(html);
        for forbidden in ["onclick", "onmouseover", "javascript:", "<img"] {
            assert!(!sanitized.to_ascii_lowercase().contains(forbidden));
        }
        assert!(sanitized.contains(
            r#"<a href="https://ok.test/a?x=1&amp;y=2" rel="nofollow noopener noreferrer" target="_blank">ok</a>"#
        ));
    }

    #[test]
    fn conserve_le_texte_des_balises_retirees() {
        assert_eq!(
            sanitize_tweet_embed("<div>texte</div>"),
            "texte".to_string()
        );
    }
}
