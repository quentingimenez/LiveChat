//! Jetons des clients d'ingestion.
//!
//! Le legacy générait 32 octets aléatoires encodés en hexadécimal (64
//! caractères) puis ne stockait que leur empreinte SHA-256 : le jeton en clair
//! n'existe qu'une fois, dans la réponse d'appairage. Ces deux primitives sont
//! reprises telles quelles pour rester compatibles avec les jetons déjà émis.

use rand::RngCore as _;
use sha2::{Digest, Sha256};

/// Longueur en octets du jeton brut (avant encodage hexadécimal).
pub const INGEST_TOKEN_BYTES: usize = 32;

/// Génère un jeton d'ingestion en clair (64 caractères hexadécimaux).
pub fn generate_ingest_token() -> String {
    let mut bytes = [0_u8; INGEST_TOKEN_BYTES];
    // `ThreadRng` est un générateur cryptographique infaillible côté API.
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Empreinte SHA-256 hexadécimale d'un jeton, seule valeur persistée en base.
pub fn hash_ingest_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_hex_and_unique() {
        let first = generate_ingest_token();
        let second = generate_ingest_token();

        assert_eq!(first.len(), INGEST_TOKEN_BYTES * 2);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn hash_matches_known_sha256() {
        // Empreinte SHA-256 de la chaîne vide, valeur de référence publique.
        assert_eq!(
            hash_ingest_token(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(hash_ingest_token("abc").len(), 64);
        assert_ne!(hash_ingest_token("abc"), hash_ingest_token("abd"));
    }
}
