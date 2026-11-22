//! Traduction des cartes de tweet.
//!
//! Le texte rendu dans une carte est figé au moment de l'ingestion : quand le
//! tweet n'est pas dans la langue du serveur (`I18N`), le texte est traduit
//! avant le rendu. La traduction est un enrichissement, jamais un prérequis :
//! toute erreur (réseau, quota, réponse inattendue) laisse l'original en place
//! et l'ingestion continue.
//!
//! Google gtx est utilisé sans clé, avec un corps de formulaire (`q`) pour ne
//! pas dépasser la taille d'une URL sur les notes longues. Les entités que X
//! doit conserver telles quelles (`@mention`, `#hashtag`, `$cashtag`, URL) sont
//! remplacées par des sentinelles de la zone privée Unicode avant l'envoi, puis
//! restaurées après la réponse : même si le fournisseur traduit un hashtag, la
//! carte garde le texte d'origine.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde_json::Value;

use crate::config::{DownloadConfig, MediaConfig};
use crate::error::{MediaError, MediaErrorCode, MediaResult};
use crate::http;
use crate::tweet_card::TweetCardContent;

/// Endpoint public de Google Translate (API non officielle, sans clé).
pub const GOOGLE_TRANSLATE_ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";

/// Longueur maximale d'une requête ; au-delà, la traduction est abandonnée.
const MAX_TRANSLATE_CHARS: usize = 8_000;
/// Taille visée d'un morceau quand le texte doit être découpé.
const MAX_CHUNK_CHARS: usize = 4_000;
/// Garde-fou sur la réponse : une traduction anormalement longue est rejetée.
const MAX_TRANSLATED_CHARS: usize = 12_000;
/// Sentinelle de début d'entité (zone privée Unicode, préservée par Google).
const ENTITY_START: char = '\u{E000}';
/// Sentinelle de fin d'entité.
const ENTITY_END: char = '\u{E001}';

/// Demande de traduction envoyée au transport.
pub(crate) struct TranslateRequest<'a> {
    /// Texte masqué à traduire.
    pub text: &'a str,
    /// Langue source normalisée (`en`, `ar`, `ja`…).
    pub source_language: &'a str,
    /// Langue cible normalisée (`fr` ou `en`).
    pub target_language: &'a str,
    /// Endpoint effectif (Google par défaut, surcharge de configuration sinon).
    pub endpoint: &'a str,
    /// Budget de la requête.
    pub timeout: Duration,
}

/// Transport de traduction, injectable pour tester sans réseau.
pub(crate) trait TranslateTransport {
    /// Traduit une demande bornée et renvoie le texte traduit.
    fn translate<'a>(
        &'a self,
        request: &'a TranslateRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = MediaResult<String>> + Send + 'a>>;
}

/// Transport HTTP réel (Google gtx, formulaire POST).
pub(crate) struct HttpTranslateTransport<'a> {
    download: &'a DownloadConfig,
}

impl TranslateTransport for HttpTranslateTransport<'_> {
    fn translate<'a>(
        &'a self,
        request: &'a TranslateRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = MediaResult<String>> + Send + 'a>> {
        Box::pin(async move {
            let form = [
                ("client", "gtx"),
                ("sl", request.source_language),
                ("tl", request.target_language),
                ("dt", "t"),
                ("q", request.text),
            ];
            let response =
                http::post_form_text(self.download, request.endpoint, request.timeout, &form)
                    .await?;
            parse_google_response(&response.body)
        })
    }
}

/// Traduit le texte racine, cité et parent d'une carte, sans jamais échouer.
///
/// `config.translate.enabled` et `config.resolve.language` sont les seuls
/// réglages lus ; les langues sources viennent des payloads (`lang`).
pub async fn localize_tweet_card(config: &MediaConfig, content: &mut TweetCardContent) {
    if !config.translate.enabled {
        return;
    }
    let transport = HttpTranslateTransport {
        download: &config.download,
    };
    localize_tweet_card_with(config, content, &transport).await;
}

/// Variante testable de [`localize_tweet_card`] avec un transport injecté.
async fn localize_tweet_card_with<T: TranslateTransport>(
    config: &MediaConfig,
    content: &mut TweetCardContent,
    transport: &T,
) {
    if !config.translate.enabled {
        return;
    }
    let target = target_language(&config.resolve.language);

    // Les trois textes sont indépendants : une seule requête lente ne doit pas
    // retarder les deux autres. Le scope ferme les emprunts avant les remplacements.
    let (root_translation, quoted_translation, parent_translation) = {
        let root_future = translate_if_needed(
            config,
            &content.text,
            content.source_language.as_deref(),
            target,
            transport,
        );
        let quoted_future = async {
            let quoted = content.quoted.as_ref()?;
            translate_if_needed(
                config,
                &quoted.text,
                quoted.source_language.as_deref(),
                target,
                transport,
            )
            .await
        };
        let parent_future = async {
            let parent = content.reply_parent.as_ref()?;
            translate_if_needed(
                config,
                &parent.text,
                parent.source_language.as_deref(),
                target,
                transport,
            )
            .await
        };
        tokio::join!(root_future, quoted_future, parent_future)
    };

    apply_translation(
        &mut content.text,
        &mut content.translation_label,
        root_translation,
        target,
    );
    if let Some(quoted) = content.quoted.as_mut() {
        apply_translation(
            &mut quoted.text,
            &mut quoted.translation_label,
            quoted_translation,
            target,
        );
    }
    if let Some(parent) = content.reply_parent.as_mut() {
        apply_translation(
            &mut parent.text,
            &mut parent.translation_label,
            parent_translation,
            target,
        );
    }
}

/// Résultat d'une traduction réussie.
struct Translation {
    text: String,
    source_language: String,
}

/// Copie le texte traduit et son libellé quand une traduction existe.
fn apply_translation(
    text: &mut String,
    label: &mut Option<String>,
    translation: Option<Translation>,
    target: &str,
) {
    let Some(translation) = translation else {
        return;
    };
    *text = translation.text;
    *label = Some(translation_label(&translation.source_language, target));
}

/// Traduit un texte si sa langue est connue, différente de la cible et courte
/// assez. Toute anomalie renvoie `None` : l'appelant garde l'original.
async fn translate_if_needed<T: TranslateTransport>(
    config: &MediaConfig,
    text: &str,
    source_language: Option<&str>,
    target: &str,
    transport: &T,
) -> Option<Translation> {
    let source = source_language.map(normalize_language_code)?;
    if source.is_empty() || source == "und" || source == "auto" {
        return None;
    }
    if source == target || text.trim().is_empty() {
        return None;
    }

    let chunks = split_chunks(text)?;
    let endpoint = config
        .translate
        .endpoint
        .as_deref()
        .unwrap_or(GOOGLE_TRANSLATE_ENDPOINT);
    // Un seul budget pour tout le texte : une note découpée en deux morceaux ne
    // doit pas doubler le temps d'ingestion.
    let deadline = std::time::Instant::now() + config.translate.timeout;
    let mut pieces = Vec::new();
    let mut translated_any = false;

    for chunk in chunks {
        let masked = mask_entities(chunk);
        if !masked.text.chars().any(char::is_alphabetic) {
            pieces.push(chunk.to_string());
            continue;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            tracing::debug!("traduction ignorée : délai dépassé");
            return None;
        }
        let request = TranslateRequest {
            text: &masked.text,
            source_language: &source,
            target_language: target,
            endpoint,
            timeout: remaining,
        };
        let response = match tokio::time::timeout(remaining, transport.translate(&request)).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::debug!(code = error.code().as_str(), "traduction ignorée");
                return None;
            }
            Err(_) => {
                tracing::debug!("traduction ignorée : délai dépassé");
                return None;
            }
        };
        let restored = restore_entities(&response, &masked.entities)?;
        if restored.trim().is_empty() {
            return None;
        }
        pieces.push(restored);
        translated_any = true;
    }

    let translated = pieces.join(" ");
    if !translated_any || translated.chars().count() > MAX_TRANSLATED_CHARS {
        return None;
    }
    Some(Translation {
        text: translated,
        source_language: source,
    })
}

/// Normalise la langue cible : `en` exact, sinon `fr` (comme le bot Discord).
fn target_language(config_language: &str) -> &'static str {
    if normalize_language_code(config_language) == "en" {
        "en"
    } else {
        "fr"
    }
}

/// Réduit un code BCP 47 à sa langue de base, minuscule (`en-US` → `en`).
fn normalize_language_code(raw: &str) -> String {
    let base = raw
        .trim()
        .to_ascii_lowercase()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_string();
    match base.as_str() {
        // Codes legacy de l'API syndication / des fournisseurs.
        "in" => "id".to_string(),
        "iw" => "he".to_string(),
        "ji" => "yi".to_string(),
        "jw" => "jv".to_string(),
        _ => base,
    }
}

/// Libellé affiché sous un texte traduit.
fn translation_label(source: &str, target: &str) -> String {
    match language_names(source) {
        Some((_, en)) if target == "en" => format!("Translated from {en}"),
        Some((fr, _)) => french_label(fr),
        None if target == "en" => format!("Translated ({source})"),
        None => format!("Traduit ({source})"),
    }
}

/// Compose « Traduit de l'anglais » ou « Traduit du japonais ».
fn french_label(name: &str) -> String {
    // « hongrois » commence par un h aspiré : pas d'élision, contrairement à
    // « hébreu » ou « hindi ».
    if name == "hongrois" {
        return format!("Traduit du {name}");
    }
    match name.chars().next() {
        Some(
            'a' | 'e' | 'i' | 'o' | 'u' | 'é' | 'è' | 'ê' | 'à' | 'â' | 'î' | 'ï' | 'ô' | 'û' | 'ù'
            | 'h',
        ) => format!("Traduit de l'{name}"),
        _ => format!("Traduit du {name}"),
    }
}

/// Noms complets d'une langue, sans article (le libellé le compose).
fn language_names(code: &str) -> Option<(&'static str, &'static str)> {
    let names = match code {
        "en" => ("anglais", "English"),
        "fr" => ("français", "French"),
        "ar" => ("arabe", "Arabic"),
        "es" => ("espagnol", "Spanish"),
        "de" => ("allemand", "German"),
        "it" => ("italien", "Italian"),
        "pt" => ("portugais", "Portuguese"),
        "ja" => ("japonais", "Japanese"),
        "ko" => ("coréen", "Korean"),
        "zh" => ("chinois", "Chinese"),
        "ru" => ("russe", "Russian"),
        "uk" => ("ukrainien", "Ukrainian"),
        "tr" => ("turc", "Turkish"),
        "nl" => ("néerlandais", "Dutch"),
        "pl" => ("polonais", "Polish"),
        "sv" => ("suédois", "Swedish"),
        "da" => ("danois", "Danish"),
        "no" => ("norvégien", "Norwegian"),
        "fi" => ("finnois", "Finnish"),
        "cs" => ("tchèque", "Czech"),
        "ro" => ("roumain", "Romanian"),
        "hu" => ("hongrois", "Hungarian"),
        "el" => ("grec", "Greek"),
        "he" => ("hébreu", "Hebrew"),
        "hi" => ("hindi", "Hindi"),
        "th" => ("thaï", "Thai"),
        "vi" => ("vietnamien", "Vietnamese"),
        "id" => ("indonésien", "Indonesian"),
        "ms" => ("malais", "Malay"),
        "fa" => ("persan", "Persian"),
        "bn" => ("bengali", "Bengali"),
        "ur" => ("ourdou", "Urdu"),
        "ta" => ("tamoul", "Tamil"),
        "te" => ("télougou", "Telugu"),
        "sr" => ("serbe", "Serbian"),
        "hr" => ("croate", "Croatian"),
        "sk" => ("slovaque", "Slovak"),
        "bg" => ("bulgare", "Bulgarian"),
        "lt" => ("lituanien", "Lithuanian"),
        "lv" => ("letton", "Latvian"),
        "et" => ("estonien", "Estonian"),
        "sl" => ("slovène", "Slovenian"),
        "ca" => ("catalan", "Catalan"),
        "tl" => ("tagalog", "Tagalog"),
        "sw" => ("swahili", "Swahili"),
        "af" => ("afrikaans", "Afrikaans"),
        "yi" => ("yiddish", "Yiddish"),
        "jv" => ("javanais", "Javanese"),
        "la" => ("latin", "Latin"),
        "is" => ("islandais", "Icelandic"),
        "ga" => ("irlandais", "Irish"),
        "cy" => ("gallois", "Welsh"),
        "gl" => ("galicien", "Galician"),
        "eu" => ("basque", "Basque"),
        "ka" => ("géorgien", "Georgian"),
        "hy" => ("arménien", "Armenian"),
        "az" => ("azéri", "Azerbaijani"),
        "kk" => ("kazakh", "Kazakh"),
        "uz" => ("ouzbek", "Uzbek"),
        "ne" => ("népalais", "Nepali"),
        "si" => ("cinghalais", "Sinhala"),
        "km" => ("khmer", "Khmer"),
        "lo" => ("lao", "Lao"),
        "my" => ("birman", "Burmese"),
        "mn" => ("mongol", "Mongolian"),
        "am" => ("amharique", "Amharic"),
        "so" => ("somali", "Somali"),
        "zu" => ("zoulou", "Zulu"),
        _ => return None,
    };
    Some(names)
}

/// Découpe un texte trop long sans jamais couper au milieu d'un mot.
///
/// Au-delà de [`MAX_TRANSLATE_CHARS`], ou sans espace où couper, la traduction
/// est abandonnée : le texte original reste affiché.
fn split_chunks(text: &str) -> Option<Vec<&str>> {
    let total = text.chars().count();
    if total > MAX_TRANSLATE_CHARS {
        return None;
    }
    if total <= MAX_CHUNK_CHARS {
        return Some(vec![text]);
    }

    let mut count = 0usize;
    let mut cut = None;
    for (index, _) in text.char_indices() {
        count += 1;
        if count > MAX_CHUNK_CHARS {
            cut = Some(index);
            break;
        }
    }
    let mut cut = cut?;
    let head = &text[..cut];
    cut = head.rfind(char::is_whitespace)?;
    let first = text[..cut].trim_end();
    let second = text[cut..].trim_start();
    if first.is_empty() || second.is_empty() {
        return None;
    }
    Some(vec![first, second])
}

/// Texte masqué et entités correspondantes, dans l'ordre des sentinelles.
struct MaskedText {
    text: String,
    entities: Vec<String>,
}

/// Remplace les entités à préserver par des sentinelles numérotées.
///
/// La ponctuation collée à l'entité (`(@NASA),` par exemple) reste dans le
/// texte visible : seule l'entité elle-même est protégée.
fn mask_entities(text: &str) -> MaskedText {
    let mut masked = String::with_capacity(text.len());
    let mut entities = Vec::new();
    let mut token_start = None;

    for (index, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = token_start.take() {
                push_masked_token(&text[start..index], &mut masked, &mut entities);
            }
            masked.push(ch);
        } else if token_start.is_none() {
            token_start = Some(index);
        }
    }
    if let Some(start) = token_start {
        push_masked_token(&text[start..], &mut masked, &mut entities);
    }

    MaskedText {
        text: masked,
        entities,
    }
}

/// Ajoute un jeton au texte masqué, en protégeant son éventuelle entité.
fn push_masked_token(token: &str, masked: &mut String, entities: &mut Vec<String>) {
    let leading = token
        .chars()
        .take_while(|ch| is_leading_punctuation(*ch))
        .count();
    let trailing = token
        .chars()
        .rev()
        .take_while(|ch| is_trailing_punctuation(*ch))
        .count();
    let lead_bytes: usize = token.chars().take(leading).map(char::len_utf8).sum();
    let core_end = token.len()
        - token
            .chars()
            .rev()
            .take(trailing)
            .map(char::len_utf8)
            .sum::<usize>();
    if core_end <= lead_bytes {
        masked.push_str(token);
        return;
    }
    let core = &token[lead_bytes..core_end];

    masked.push_str(&token[..lead_bytes]);
    if is_entity(core) {
        masked.push(ENTITY_START);
        masked.push_str(&entities.len().to_string());
        masked.push(ENTITY_END);
        entities.push(core.to_string());
    } else {
        masked.push_str(core);
    }
    masked.push_str(&token[core_end..]);
}

/// Rend le texte traduit en replaçant chaque sentinelle par son entité.
///
/// Une sentinelle manquante, dupliquée ou mal formée invalide toute la
/// traduction : mieux vaut l'original qu'une carte au texte tronqué.
fn restore_entities(translated: &str, entities: &[String]) -> Option<String> {
    let mut restored = String::with_capacity(translated.len());
    let mut seen = vec![false; entities.len()];
    let mut rest = translated;

    while let Some(start) = rest.find(ENTITY_START) {
        restored.push_str(&rest[..start]);
        let after = &rest[start + ENTITY_START.len_utf8()..];
        let end = after.find(ENTITY_END)?;
        let number = after[..end].parse::<usize>().ok()?;
        let entity = entities.get(number)?;
        if seen[number] {
            return None;
        }
        seen[number] = true;
        restored.push_str(entity);
        rest = &after[end + ENTITY_END.len_utf8()..];
    }
    if rest.contains(ENTITY_END) {
        return None;
    }
    restored.push_str(rest);
    if seen.iter().any(|seen| !seen) {
        return None;
    }
    Some(restored)
}

/// Vrai si le jeton est une entité que X affiche telle quelle.
fn is_entity(token: &str) -> bool {
    if token.len() <= 1 {
        return false;
    }
    token.starts_with('@')
        || token.starts_with('#')
        || token
            .strip_prefix('$')
            .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_alphabetic()))
        || token.starts_with("http://")
        || token.starts_with("https://")
}

/// Ponctuation autorisée avant une entité (`(@NASA`).
fn is_leading_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '(' | '[' | '{' | '<' | '"' | '\'' | '«' | '“' | '‘' | '「' | '（' | '《'
    )
}

/// Ponctuation autorisée après une entité (`@NASA,`).
fn is_trailing_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '.' | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
            | '>'
            | '"'
            | '\''
            | '»'
            | '”'
            | '’'
            | '」'
            | '。'
            | '、'
            | '！'
            | '？'
            | '）'
            | '…'
    )
}

/// Extrait le texte traduit d'une réponse `translate_a/single` de Google.
///
/// La réponse est une liste de segments `[traduit, original, …]` : le texte
/// final est la concaténation des premiers éléments.
fn parse_google_response(body: &str) -> MediaResult<String> {
    let payload: Value = serde_json::from_str(body).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::InvalidMedia,
            "réponse de traduction illisible",
            error.to_string(),
        )
    })?;
    let segments = payload
        .get(0)
        .and_then(Value::as_array)
        .ok_or_else(|| MediaError::new(MediaErrorCode::InvalidMedia, "segments absents"))?;
    let mut translated = String::new();
    for segment in segments {
        if let Some(text) = segment.get(0).and_then(Value::as_str) {
            translated.push_str(text);
        }
    }
    if translated.trim().is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::InvalidMedia,
            "traduction vide",
        ));
    }
    Ok(translated)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::tweet_card::{QuotedTweetContent, TweetCardContent};

    enum FakeReply {
        Text(&'static str),
        Error,
        Slow,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Seen {
        text: String,
        source: String,
        target: String,
        endpoint: String,
    }

    struct FakeTransport {
        reply: FakeReply,
        calls: Mutex<Vec<Seen>>,
    }

    impl FakeTransport {
        fn new(reply: FakeReply) -> Self {
            Self {
                reply,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Seen> {
            self.calls.lock().unwrap().drain(..).collect()
        }
    }

    impl TranslateTransport for FakeTransport {
        fn translate<'a>(
            &'a self,
            request: &'a TranslateRequest<'a>,
        ) -> Pin<Box<dyn Future<Output = MediaResult<String>> + Send + 'a>> {
            self.calls.lock().unwrap().push(Seen {
                text: request.text.to_string(),
                source: request.source_language.to_string(),
                target: request.target_language.to_string(),
                endpoint: request.endpoint.to_string(),
            });
            Box::pin(async move {
                match self.reply {
                    FakeReply::Text(text) => Ok(text.to_string()),
                    FakeReply::Error => Err(MediaError::new(
                        MediaErrorCode::DownloadFailed,
                        "panne simulée",
                    )),
                    FakeReply::Slow => {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Ok("trop tard".to_string())
                    }
                }
            })
        }
    }

    fn config_with_language(language: &str) -> MediaConfig {
        MediaConfig::from_pairs([("I18N", language)])
    }

    fn content(text: &str, source_language: &str) -> TweetCardContent {
        TweetCardContent {
            text: text.to_string(),
            source_language: Some(source_language.to_string()),
            ..TweetCardContent::default()
        }
    }

    #[test]
    fn parses_google_segments() {
        let body =
            r#"[[["Bonjour ","Hello ",null,null,10],["le monde !",null,null,null,10]],null,"en"]"#;
        assert_eq!(parse_google_response(body).unwrap(), "Bonjour le monde !");

        assert!(parse_google_response("[]").is_err());
        assert!(parse_google_response("[[],null,\"en\"]").is_err());
        assert!(parse_google_response("pas du json").is_err());
    }

    #[test]
    fn masks_and_restores_entities() {
        let masked = mask_entities("Salut @NASA, regarde https://t.co/abc et #OpenAI !");
        assert!(!masked.text.contains("@NASA"));
        assert!(!masked.text.contains("https://t.co/abc"));
        assert!(!masked.text.contains("#OpenAI"));
        assert!(masked.text.contains(ENTITY_START));
        assert_eq!(masked.entities.len(), 3);

        let translated = masked.text.replace("Salut", "Hello");
        let restored = restore_entities(&translated, &masked.entities).unwrap();
        assert_eq!(
            restored,
            "Hello @NASA, regarde https://t.co/abc et #OpenAI !"
        );
    }

    #[test]
    fn protects_punctuation_around_entities() {
        let masked = mask_entities("(@user), « #tag »");
        let restored = restore_entities(&masked.text, &masked.entities).unwrap();
        assert_eq!(restored, "(@user), « #tag »");
    }

    #[test]
    fn leaves_punctuation_only_tokens_untouched() {
        let masked = mask_entities("() . ,");
        assert_eq!(masked.text, "() . ,");
        assert!(masked.entities.is_empty());
    }

    #[test]
    fn rejects_missing_or_duplicated_entities() {
        let masked = mask_entities("Salut @NASA et #OpenAI");
        let missing = masked
            .text
            .replace(&format!("{ENTITY_START}0{ENTITY_END}"), "");
        assert!(restore_entities(&missing, &masked.entities).is_none());

        let duplicated = format!("{}{}", masked.text, masked.text);
        assert!(restore_entities(&duplicated, &masked.entities).is_none());
    }

    #[test]
    fn splits_long_text_at_a_whitespace() {
        let text = format!("{} fin", "mot ".repeat(1_100));
        let chunks = split_chunks(&text).unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].chars().count() <= MAX_CHUNK_CHARS);
        assert!(chunks[1].chars().count() <= MAX_CHUNK_CHARS + 1);
        assert_eq!(format!("{} {}", chunks[0], chunks[1]), text);
    }

    #[test]
    fn refuses_text_without_a_place_to_split() {
        let text = "a".repeat(MAX_TRANSLATE_CHARS + 1);
        assert!(split_chunks(&text).is_none());
        let text = "a".repeat(5_000);
        assert!(split_chunks(&text).is_none());
    }

    #[test]
    fn skips_the_target_language_and_unknown_sources() {
        let config = config_with_language("fr");
        let transport = FakeTransport::new(FakeReply::Text("Bonjour"));
        let mut content = content("Bonjour", "fr");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));
        assert_eq!(content.text, "Bonjour");
        assert!(content.translation_label.is_none());
        assert!(transport.calls().is_empty());

        let mut content = TweetCardContent {
            text: "Hello".to_string(),
            ..TweetCardContent::default()
        };
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));
        assert_eq!(content.text, "Hello");
        assert!(content.translation_label.is_none());
        assert!(transport.calls().is_empty());
    }

    #[test]
    fn translates_root_quoted_and_parent() {
        let config = config_with_language("fr");
        let transport = FakeTransport::new(FakeReply::Text("Bonjour"));
        let mut content = content("Hello", "en");
        content.quoted = Some(QuotedTweetContent {
            text: "Hi".to_string(),
            source_language: Some("en".to_string()),
            ..QuotedTweetContent::default()
        });
        content.reply_parent = Some(QuotedTweetContent {
            text: "Yo".to_string(),
            source_language: Some("en".to_string()),
            ..QuotedTweetContent::default()
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));

        assert_eq!(content.text, "Bonjour");
        assert_eq!(
            content.translation_label.as_deref(),
            Some("Traduit de l'anglais")
        );
        let quoted = content.quoted.as_ref().unwrap();
        assert_eq!(quoted.text, "Bonjour");
        assert_eq!(
            quoted.translation_label.as_deref(),
            Some("Traduit de l'anglais")
        );
        let parent = content.reply_parent.as_ref().unwrap();
        assert_eq!(parent.text, "Bonjour");
        assert_eq!(
            parent.translation_label.as_deref(),
            Some("Traduit de l'anglais")
        );
        assert_eq!(transport.calls().len(), 3);
    }

    #[test]
    fn failure_keeps_the_original_text() {
        let config = config_with_language("fr");
        let transport = FakeTransport::new(FakeReply::Error);
        let mut content = content("Hello", "en");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));
        assert_eq!(content.text, "Hello");
        assert!(content.translation_label.is_none());
    }

    #[test]
    fn disabled_translation_does_not_call_the_provider() {
        let mut config = config_with_language("fr");
        config.translate.enabled = false;
        let transport = FakeTransport::new(FakeReply::Text("Bonjour"));
        let mut content = content("Hello", "en");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));
        assert_eq!(content.text, "Hello");
        assert!(transport.calls().is_empty());
    }

    #[test]
    fn a_slow_provider_is_abandoned() {
        let mut config = config_with_language("fr");
        config.translate.timeout = Duration::from_millis(10);
        let transport = FakeTransport::new(FakeReply::Slow);
        let mut content = content("Hello", "en");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));
        assert_eq!(content.text, "Hello");
        assert!(content.translation_label.is_none());
    }

    #[test]
    fn labels_follow_the_target_language() {
        assert_eq!(translation_label("en", "fr"), "Traduit de l'anglais");
        assert_eq!(translation_label("en", "en"), "Translated from English");
        assert_eq!(translation_label("ja", "fr"), "Traduit du japonais");
        assert_eq!(translation_label("hu", "fr"), "Traduit du hongrois");
        assert_eq!(translation_label("xx", "fr"), "Traduit (xx)");
    }

    #[test]
    fn target_is_en_only_when_the_server_language_is_en() {
        assert_eq!(target_language("en"), "en");
        assert_eq!(target_language("en-US"), "en");
        assert_eq!(target_language("fr"), "fr");
        assert_eq!(target_language("de"), "fr");
    }

    #[test]
    fn normalizes_bcp47_and_legacy_codes() {
        assert_eq!(normalize_language_code("EN-us"), "en");
        assert_eq!(normalize_language_code("in"), "id");
        assert_eq!(normalize_language_code("iw"), "he");
    }

    #[test]
    fn keeps_long_notes_under_the_request_limit() {
        let config = config_with_language("fr");
        let transport = FakeTransport::new(FakeReply::Text("Traduit"));
        let text = "mot ".repeat(1_001);
        let mut content = content(&text, "en");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));
        let calls = transport.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|call| call.text.chars().count() <= 4_001));
        assert_eq!(calls[0].source, "en");
        assert_eq!(calls[0].target, "fr");
        assert_eq!(calls[0].endpoint, GOOGLE_TRANSLATE_ENDPOINT);
        assert_eq!(content.text, "Traduit Traduit");
    }

    #[test]
    fn uses_the_configured_endpoint() {
        let mut config = config_with_language("fr");
        config.translate.endpoint = Some("https://translate.example/gtx".to_string());
        let transport = FakeTransport::new(FakeReply::Text("Bonjour"));
        let mut content = content("Hello", "en");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(localize_tweet_card_with(&config, &mut content, &transport));
        assert_eq!(
            transport.calls()[0].endpoint,
            "https://translate.example/gtx"
        );
    }

    #[test]
    fn parse_ignores_a_null_source_segment() {
        let body = json!([[["Bonjour", null, null, null, 10]], null, "en"]).to_string();
        assert_eq!(parse_google_response(&body).unwrap(), "Bonjour");
    }
}
