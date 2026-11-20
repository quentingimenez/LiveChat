//! Durée d'affichage des cartes tweet selon la quantité de texte.
//!
//! Une carte tweet est une image : sans règle dédiée, [`super::jobs`] lui
//! applique la durée par défaut des images (5 s), quelle que soit la longueur
//! du texte. Le titre de source d'un tweet contient ce texte (tronqué à
//! `lc_media::title::TITLE_MAX_CHARS` par le pipeline) : on s'en sert pour
//! moduler la durée.
//!
//! Règles :
//!
//! - 5 s de base ;
//! - +1 s par tranche de 12 caractères, arrondie à la tranche supérieure ;
//! - plafond à 30 s ;
//! - un titre saturé (`>= TITLE_MAX_CHARS`, donc possiblement tronqué) prend
//!   directement le plafond ;
//! - jamais moins que le défaut applicable (guilde ou serveur).
//!
//! Seules les cartes tweet **image** sont concernées : une carte vidéo de tweet
//! garde la durée de la vidéo, et un média non-tweet garde la résolution
//! habituelle.

use lc_domain::entities::MediaAsset;
use lc_domain::enums::MediaKind;

/// Caractères par seconde supplémentaire.
pub const TWEET_CARD_CHARS_PER_SEC: usize = 12;
/// Durée de base d'une carte tweet, en secondes.
pub const TWEET_CARD_BASE_SEC: i64 = 5;
/// Durée maximale d'une carte tweet, en secondes.
pub const TWEET_CARD_MAX_SEC: i64 = 30;
/// Durée d'affichage d'une carte tweet, si `asset` en est une.
///
/// `minimum_sec` est le défaut applicable (guilde ou serveur) : la carte ne
/// s'affiche jamais moins longtemps que ce défaut. Renvoie `None` pour un
/// média qui n'est pas une carte tweet image.
pub fn tweet_card_duration_sec(asset: &MediaAsset, minimum_sec: i64) -> Option<i64> {
    if asset.kind != MediaKind::Image || !lc_media::source::is_twitter_url(&asset.source_url) {
        return None;
    }
    let chars = asset
        .source_title
        .as_deref()
        .map_or(0, |title| title.chars().count());
    Some(tweet_card_seconds(chars).max(minimum_sec))
}

/// Durée d'une carte tweet pour `chars` caractères, hors minimum de guilde.
fn tweet_card_seconds(chars: usize) -> i64 {
    // Le titre stocké est tronqué à `TITLE_MAX_CHARS` : à la limite, le tweet
    // est peut-être plus long que le texte conservé, donc on applique le
    // plafond plutôt qu'une durée calculée sur un texte partiel.
    if chars >= lc_media::title::TITLE_MAX_CHARS {
        return TWEET_CARD_MAX_SEC;
    }
    let extra = i64::try_from(chars.div_ceil(TWEET_CARD_CHARS_PER_SEC)).unwrap_or(i64::MAX);
    TWEET_CARD_BASE_SEC
        .saturating_add(extra)
        .min(TWEET_CARD_MAX_SEC)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use lc_domain::enums::MediaStatus;
    use lc_domain::ids::MediaAssetId;
    use time::UtcDateTime;

    use super::*;

    const TWEET_URL: &str = "https://x.com/jeanne/status/1234567890";

    fn asset(kind: MediaKind, source_url: &str, source_title: Option<&str>) -> MediaAsset {
        let now = UtcDateTime::now();
        MediaAsset {
            id: MediaAssetId::new_v4(),
            source_hash: "hash-de-test".into(),
            source_url: source_url.into(),
            source_title: source_title.map(str::to_owned),
            kind,
            mime: "image/png".into(),
            duration_sec: None,
            width: None,
            height: None,
            is_vertical: false,
            storage_path: "/tmp/carte.png".into(),
            size_bytes: 10,
            status: MediaStatus::Ready,
            error: None,
            last_accessed_at: now,
            expires_at: now + time::Duration::hours(1),
            created_at: now,
        }
    }

    #[test]
    fn tweet_card_duration_grows_with_text_length() {
        let short = asset(MediaKind::Image, TWEET_URL, Some("Bonjour"));
        let medium = asset(MediaKind::Image, TWEET_URL, Some(&"a".repeat(120)));
        let long = asset(MediaKind::Image, TWEET_URL, Some(&"a".repeat(239)));

        assert_eq!(tweet_card_duration_sec(&short, 5), Some(6));
        assert_eq!(tweet_card_duration_sec(&medium, 5), Some(15));
        assert_eq!(tweet_card_duration_sec(&long, 5), Some(25));
    }

    #[test]
    fn saturated_titles_use_the_cap() {
        let long = asset(MediaKind::Image, TWEET_URL, Some(&"a".repeat(240)));
        assert_eq!(tweet_card_duration_sec(&long, 5), Some(TWEET_CARD_MAX_SEC));
    }

    #[test]
    fn tweet_card_duration_is_capped() {
        let very_long = asset(MediaKind::Image, TWEET_URL, Some(&"a".repeat(10_000)));
        assert_eq!(
            tweet_card_duration_sec(&very_long, 5),
            Some(TWEET_CARD_MAX_SEC)
        );
    }

    #[test]
    fn tweet_card_duration_never_goes_below_the_default() {
        let short = asset(MediaKind::Image, TWEET_URL, Some("Bonjour"));
        assert_eq!(tweet_card_duration_sec(&short, 12), Some(12));
    }

    #[test]
    fn empty_title_uses_the_base_duration() {
        let textless = asset(MediaKind::Image, TWEET_URL, None);
        assert_eq!(
            tweet_card_duration_sec(&textless, 5),
            Some(TWEET_CARD_BASE_SEC)
        );
    }

    #[test]
    fn non_tweet_images_keep_the_default_duration() {
        let image = asset(
            MediaKind::Image,
            "https://example.com/photo.png",
            Some(&"a".repeat(240)),
        );
        assert_eq!(tweet_card_duration_sec(&image, 12), None);
    }

    #[test]
    fn tweet_videos_keep_their_own_duration() {
        let video = asset(MediaKind::Video, TWEET_URL, Some(&"a".repeat(240)));
        assert_eq!(tweet_card_duration_sec(&video, 12), None);
    }
}
