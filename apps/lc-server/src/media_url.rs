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
    MEDIA_URL_EXPIRY_MARGIN_SEC, MEDIA_URL_TTL_MAX_SEC, MEDIA_URL_TTL_SEC,
    media_signature_input, media_url_ttl_sec,
};

type HmacSha256 = Hmac<Sha256>;

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
        assert!(!verify(SECRET, ASSET, "00000000-0000-4000-8000-000000000000", EXP, &signature));
        assert!(!verify(SECRET, "ffffffff-ffff-4fff-8fff-ffffffffffff", CLIENT, EXP, &signature));
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
    fn expiry_is_inclusive() {
        assert!(!is_expired(EXP, EXP - 1));
        assert!(is_expired(EXP, EXP));
        assert!(is_expired(EXP, EXP + 1));
    }

    #[test]
    fn ttl_matches_protocol_contract() {
        assert_eq!(expiry_for(lc_protocol::MediaKind::Image, None, 1_000), 1_300);
        assert_eq!(expiry_for(lc_protocol::MediaKind::Video, Some(2), 1_000), 1_300);
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
