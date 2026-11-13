//! Signature et vérification des URLs média du protocole v2.
//!
//! Les URLs média remises aux overlays suivent le format figé dans
//! `docs/rust/protocol-v2.md` :
//!
//! ```text
//! {API_URL}/overlay/media/{assetId}?exp={unix_secs}&sig={hex}
//! ```
//!
//! La signature HMAC-SHA256 couvre `assetId|clientId|exp` (voir
//! [`lc_protocol::media_signature_input`]) : une URL signée pour un client ne
//! doit jamais être acceptée pour un autre client. Le TTL dépend du média
//! ([`lc_protocol::media_url_ttl_sec`]) et ne dépasse jamais une heure.
//!
//! Module partagé entre la lane REST `/overlay` (vérification de la signature
//! sur `GET /overlay/media/:assetId`) et la lane WebSocket (signature des URLs
//! injectées dans les trames `play`), d'où sa place dans la bibliothèque du
//! serveur plutôt que dans un sous-routeur.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub use lc_protocol::{
    media_signature_input, media_url_ttl_sec, MEDIA_URL_EXPIRY_MARGIN_SEC, MEDIA_URL_TTL_MAX_SEC,
    MEDIA_URL_TTL_SEC,
};

type HmacSha256 = Hmac<Sha256>;

/// Durée de validité d'une URL signée du board média (une heure).
pub const BOARD_MEDIA_URL_TTL_SEC: u64 = 3_600;

/// Board URL expiries are rounded down to this window so that the URLs stay
/// identical between two item reloads and the browser keeps its cached
/// thumbnails.
pub const BOARD_MEDIA_URL_WINDOW_SEC: u64 = 900;

/// Expiry of a board URL signed at `now_sec`: the start of the current
/// 15-minute window plus the TTL, i.e. 45 to 60 minutes ahead, never beyond
/// the maximum TTL checked by the media route.
#[must_use]
pub fn board_url_expiry(now_sec: u64) -> u64 {
    (now_sec - now_sec % BOARD_MEDIA_URL_WINDOW_SEC).saturating_add(BOARD_MEDIA_URL_TTL_SEC)
}

/// Préfixe de la clé signée d'un média du board.
///
/// Le préfixe sépare les espaces de signature : une URL de lecture
/// (`/overlay/media/{assetId}`) ne vérifie jamais sur la route du board, et
/// inversement.
const BOARD_SIGNATURE_PREFIX: &str = "board:";

/// Erreur de signature : secret HMAC inutilisable.
///
/// `Hmac::new_from_slice` n'échoue en pratique jamais pour SHA-256 (toutes les
/// longueurs de clé sont acceptées) ; l'erreur existe pour interdire tout
/// `unwrap` et documenter le cas théorique.
#[derive(Debug, thiserror::Error)]
pub enum MediaUrlError {
    /// Le secret fourni n'a pas pu être utilisé pour initialiser le HMAC.
    #[error("secret HMAC média invalide")]
    InvalidSecret,
}

/// Calcule la signature hexadécimale d'une URL média.
///
/// `asset_id` et `client_id` doivent être les chaînes canoniques utilisées par
/// le protocole (UUID en forme hyphenée). `exp_sec` est l'horodatage Unix en
/// secondes inclus dans l'URL.
pub fn sign(
    secret: &[u8],
    asset_id: &str,
    client_id: &str,
    exp_sec: u64,
) -> Result<String, MediaUrlError> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| MediaUrlError::InvalidSecret)?;
    mac.update(media_signature_input(asset_id, client_id, exp_sec).as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Vérifie en temps constant la signature d'une URL média.
///
/// Retourne `false` (sans divulgâcher la raison) si la signature est mal
/// formée, de longueur incorrecte ou différente de la valeur attendue.
pub fn verify(
    secret: &[u8],
    asset_id: &str,
    client_id: &str,
    exp_sec: u64,
    signature_hex: &str,
) -> bool {
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(media_signature_input(asset_id, client_id, exp_sec).as_bytes());
    let expected = mac.finalize().into_bytes();
    let Ok(provided) = hex::decode(signature_hex) else {
        return false;
    };
    provided.ct_eq(&expected).into()
}

/// Clé signée d'un asset du board média (`board:{assetId}`).
#[must_use]
pub fn board_signature_key(asset_id: &str) -> String {
    format!("{BOARD_SIGNATURE_PREFIX}{asset_id}")
}

/// Signe une URL de média du board pour un client et une expiration donnés.
///
/// La clé inclut le préfixe [`board_signature_key`] : la signature diffère de
/// celle d'une URL de lecture pour le même asset.
pub fn sign_board(
    secret: &[u8],
    asset_id: &str,
    client_id: &str,
    exp_sec: u64,
) -> Result<String, MediaUrlError> {
    sign(secret, &board_signature_key(asset_id), client_id, exp_sec)
}

/// Vérifie en temps constant la signature d'une URL de média du board.
pub fn verify_board(
    secret: &[u8],
    asset_id: &str,
    client_id: &str,
    exp_sec: u64,
    signature_hex: &str,
) -> bool {
    verify(
        secret,
        &board_signature_key(asset_id),
        client_id,
        exp_sec,
        signature_hex,
    )
}

/// Construit une URL absolue de média du board, signée pour un client.
///
/// ```text
/// {apiUrl}/overlay/media-board/media/{assetId}?clientId={clientId}&exp={exp}&sig={hex}
/// ```
pub fn sign_board_url(
    secret: &[u8],
    api_url: &str,
    asset_id: &str,
    client_id: &str,
    exp_sec: u64,
) -> Result<String, MediaUrlError> {
    let signature = sign_board(secret, asset_id, client_id, exp_sec)?;
    Ok(format!(
        "{}/overlay/media-board/media/{}?clientId={}&exp={}&sig={}",
        api_url.trim_end_matches('/'),
        asset_id,
        client_id,
        exp_sec,
        signature
    ))
}

/// Vérifie que l'expiration fournie reste dans la politique serveur.
///
/// Une URL signée ne doit jamais expirer au-delà de
/// `now + MEDIA_URL_TTL_MAX_SEC` : sinon un `exp` forgé (ou prolongé) porterait
/// la durée de validité d'un lien média au-delà de la politique (AUD-A2-002).
/// Le test d'expiration (`now >= exp`) reste distinct et s'applique après.
pub fn is_expiry_within_max(exp_sec: u64, now_sec: u64) -> bool {
    exp_sec <= now_sec.saturating_add(MEDIA_URL_TTL_MAX_SEC)
}

/// Indique si une URL signée est expirée à l'instant `now_sec`.
///
/// L'expiration est inclusive : à `exp_sec`, l'URL n'est plus valide.
pub fn is_expired(exp_sec: u64, now_sec: u64) -> bool {
    now_sec >= exp_sec
}

/// Construit la valeur `exp` d'une URL média signée.
///
/// `now_sec` est l'horloge serveur ; le TTL est calculé par le protocole à
/// partir du type et de la durée du média.
pub fn expiry_for(kind: lc_protocol::MediaKind, duration_sec: Option<u64>, now_sec: u64) -> u64 {
    now_sec.saturating_add(media_url_ttl_sec(kind, duration_sec))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const SECRET: &[u8] = b"secret-de-test";
    const ASSET: &str = "e3b7e1a0-4c1d-4b1f-9a5e-2b6c7d8e9f00";
    const CLIENT: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
    const EXP: u64 = 1_700_000_300;

    #[test]
    fn sign_and_verify_roundtrip() {
        let signature = sign(SECRET, ASSET, CLIENT, EXP).expect("signature");
        assert_eq!(signature.len(), 64);
        assert!(verify(SECRET, ASSET, CLIENT, EXP, &signature));
    }

    #[test]
    fn verify_rejects_other_client_asset_or_expiry() {
        let signature = sign(SECRET, ASSET, CLIENT, EXP).expect("signature");
        assert!(!verify(
            SECRET,
            ASSET,
            "00000000-0000-4000-8000-000000000000",
            EXP,
            &signature
        ));
        assert!(!verify(
            SECRET,
            "ffffffff-ffff-4fff-8fff-ffffffffffff",
            CLIENT,
            EXP,
            &signature
        ));
        assert!(!verify(SECRET, ASSET, CLIENT, EXP + 1, &signature));
    }

    #[test]
    fn verify_rejects_other_secret_and_malformed_values() {
        let signature = sign(SECRET, ASSET, CLIENT, EXP).expect("signature");
        assert!(!verify(b"autre-secret", ASSET, CLIENT, EXP, &signature));
        assert!(!verify(SECRET, ASSET, CLIENT, EXP, "pas-hex"));
        assert!(!verify(SECRET, ASSET, CLIENT, EXP, "00"));
        assert!(!verify(SECRET, ASSET, CLIENT, EXP, ""));
    }

    #[test]
    fn board_url_expiry_is_stable_within_a_window_and_within_the_max_ttl() {
        let start = 1_794_182_400; // multiple of the 15-minute window
        assert_eq!(board_url_expiry(start), start + BOARD_MEDIA_URL_TTL_SEC);
        assert_eq!(
            board_url_expiry(start + BOARD_MEDIA_URL_WINDOW_SEC - 1),
            start + BOARD_MEDIA_URL_TTL_SEC
        );
        assert_eq!(
            board_url_expiry(start + BOARD_MEDIA_URL_WINDOW_SEC),
            start + BOARD_MEDIA_URL_WINDOW_SEC + BOARD_MEDIA_URL_TTL_SEC
        );
        for now in [start, start + 1, start + BOARD_MEDIA_URL_WINDOW_SEC - 1] {
            assert!(is_expiry_within_max(board_url_expiry(now), now));
            assert!(!is_expired(board_url_expiry(now), now));
        }
    }

    #[test]
    fn board_url_uses_the_board_signature_namespace() {
        let url = sign_board_url(SECRET, "https://api.example.test/", ASSET, CLIENT, EXP)
            .expect("URL board signée");
        assert!(url.starts_with(&format!(
            "https://api.example.test/overlay/media-board/media/{ASSET}?clientId={CLIENT}&exp={EXP}&sig="
        )));
        let signature = url.rsplit("sig=").next().expect("signature");
        assert!(verify_board(SECRET, ASSET, CLIENT, EXP, signature));
        // Une URL de lecture (sans `board:`) ne vérifie pas sur le board.
        let playback = sign(SECRET, ASSET, CLIENT, EXP).expect("signature lecture");
        assert!(!verify_board(SECRET, ASSET, CLIENT, EXP, &playback));
        assert!(!verify(
            SECRET,
            &board_signature_key(ASSET),
            CLIENT,
            EXP,
            &playback
        ));
        assert_eq!(board_signature_key(ASSET), format!("board:{ASSET}"));
    }

    #[test]
    fn expiry_is_inclusive() {
        assert!(!is_expired(EXP, EXP - 1));
        assert!(is_expired(EXP, EXP));
        assert!(is_expired(EXP, EXP + 1));
    }

    #[test]
    fn expiry_beyond_policy_max_is_rejected() {
        let now = 1_700_000_000;
        assert!(is_expiry_within_max(now, now));
        assert!(is_expiry_within_max(now + MEDIA_URL_TTL_MAX_SEC, now));
        assert!(!is_expiry_within_max(now + MEDIA_URL_TTL_MAX_SEC + 1, now));
        // Une URL déjà expirée reste acceptée par le plafond : le refus
        // « expiré » est un contrôle distinct appliqué ensuite.
        assert!(is_expiry_within_max(now - 10, now));
        // Saturation : pas de panique sur une valeur extrême.
        assert!(!is_expiry_within_max(u64::MAX, now));
    }

    #[test]
    fn ttl_matches_protocol_contract() {
        assert_eq!(
            expiry_for(lc_protocol::MediaKind::Image, None, 1_000),
            1_300
        );
        assert_eq!(
            expiry_for(lc_protocol::MediaKind::Video, Some(2), 1_000),
            1_300
        );
        assert_eq!(
            expiry_for(lc_protocol::MediaKind::Video, Some(600), 1_000),
            1_720
        );
        assert_eq!(
            expiry_for(lc_protocol::MediaKind::Video, Some(10_000), 1_000),
            4_600
        );
    }
}
