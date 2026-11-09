//! Payload « riche » du legacy (`richOverlayPayload.ts`).
//!
//! Le legacy transmettait des informations structurées (offset de départ d'une
//! vidéo, carte tweet) en les encodant dans le champ texte d'un
//! `PlaybackJob` : `__LC_OVERLAY_RICH__::` suivi du JSON en base64.
//!
//! Phase 6a ne porte que le type `media` (les cartes tweet nécessitent le
//! réseau et relèvent de la Phase 6b). Le format exact est conservé pour que
//! l'overlay legacy et le futur overlay Rust restent compatibles.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Préfixe littéral du payload riche.
pub const RICH_PREFIX: &str = "__LC_OVERLAY_RICH__::";

/// Payload riche décodable depuis le texte d'un job de lecture.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RichOverlayPayload {
    /// Lecture d'un média avec offset de départ et légende optionnelle.
    Media {
        /// Légende affichée sous la vidéo, `None` si vide.
        caption: Option<String>,
        /// Offset de départ en secondes (strictement positif).
        #[serde(rename = "startOffsetSec")]
        start_offset_sec: i64,
    },
}

/// Encode un payload au format legacy.
///
/// La sérialisation de ce type est infaillible (pas de flottants, pas de clés
/// non-chaînes) : en cas d'erreur théorique, une chaîne vide est retournée.
pub fn encode(payload: &RichOverlayPayload) -> String {
    let json = match serde_json::to_string(payload) {
        Ok(json) => json,
        Err(_) => return String::new(),
    };
    format!("{RICH_PREFIX}{}", STANDARD.encode(json))
}

/// Décode un payload riche, `None` si le préfixe, le base64 ou le JSON sont
/// invalides (le legacy renvoyait `null` dans ce cas).
pub fn decode(text: &str) -> Option<RichOverlayPayload> {
    let encoded = text.strip_prefix(RICH_PREFIX)?;
    let bytes = STANDARD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// `true` si le texte commence par le préfixe riche (sans valider le contenu).
pub fn is_rich(text: &str) -> bool {
    text.starts_with(RICH_PREFIX)
}

/// Construit le texte à stocker dans un job média :
/// - sans offset : le texte brut (comportement legacy) ;
/// - avec offset : payload riche `media` avec légende trimmée.
pub fn build_media_text_payload(
    text: Option<&str>,
    start_offset_sec: Option<i64>,
) -> Option<String> {
    match start_offset_sec {
        Some(offset) if offset > 0 => {
            let caption = text
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            Some(encode(&RichOverlayPayload::Media {
                caption,
                start_offset_sec: offset,
            }))
        }
        _ => text.map(str::to_owned),
    }
}

/// Offset de départ porté par un texte riche, si présent et positif.
pub fn media_start_offset_sec(text: &str) -> Option<i64> {
    match decode(text)? {
        RichOverlayPayload::Media {
            start_offset_sec, ..
        } if start_offset_sec > 0 => Some(start_offset_sec),
        RichOverlayPayload::Media { .. } => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    use super::{
        build_media_text_payload, decode, encode, is_rich, media_start_offset_sec,
        RichOverlayPayload, RICH_PREFIX,
    };

    #[test]
    fn prefixe_legacy_exact() {
        assert_eq!(RICH_PREFIX, "__LC_OVERLAY_RICH__::");
        assert!(is_rich("__LC_OVERLAY_RICH__::"));
        assert!(!is_rich("texte normal"));
    }

    #[test]
    fn aller_retour_media() {
        let payload = RichOverlayPayload::Media {
            caption: Some("salut".to_owned()),
            start_offset_sec: 42,
        };
        let encoded = encode(&payload);
        assert_eq!(
            encoded,
            format!(
                "{RICH_PREFIX}{}",
                STANDARD.encode(r#"{"type":"media","caption":"salut","startOffsetSec":42}"#)
            )
        );
        assert_eq!(decode(&encoded), Some(payload));
        assert_eq!(media_start_offset_sec(&encoded), Some(42));
    }

    #[test]
    fn decode_refuse_les_payloads_invalides() {
        assert_eq!(decode("pas un payload riche"), None);
        assert_eq!(decode("__LC_OVERLAY_RICH__::%%%"), None);
        assert_eq!(decode("__LC_OVERLAY_RICH__::eyJmb28iOjF9"), None);
        assert_eq!(decode("__LC_OVERLAY_RICH__::e30"), None);
    }

    #[test]
    fn construction_du_texte_media() {
        assert_eq!(build_media_text_payload(None, None), None);
        assert_eq!(
            build_media_text_payload(Some("salut"), None),
            Some("salut".to_owned())
        );
        // Un offset nul se comporte comme absent (legacy).
        assert_eq!(
            build_media_text_payload(Some("salut"), Some(0)),
            Some("salut".to_owned())
        );

        let encoded = build_media_text_payload(Some("  salut  "), Some(42)).unwrap();
        let decoded = decode(&encoded).unwrap();
        assert_eq!(
            decoded,
            RichOverlayPayload::Media {
                caption: Some("salut".to_owned()),
                start_offset_sec: 42,
            }
        );

        let encoded = build_media_text_payload(Some("   "), Some(42)).unwrap();
        assert_eq!(
            decode(&encoded),
            Some(RichOverlayPayload::Media {
                caption: None,
                start_offset_sec: 42,
            })
        );
    }
}
