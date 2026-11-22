//! Empreinte matérielle stable : normalisation et empreinte serveur.
//!
//! L'overlay transmet une empreinte matérielle brute sur la connexion TLS
//! (`deviceId` : UUID SMBIOS de la carte mère, série du disque système, modèle
//! du processeur). Le serveur n'en conserve qu'une empreinte SHA-256 salée par
//! la guilde : le brut ne doit jamais être stocké ni journalisé.
//!
//! Ce n'est pas une authentification : le client est falsifiable, un changement
//! de matériel produit une autre empreinte, et les clients antérieurs au champ
//! `deviceId` n'en envoient pas. C'est une défense en profondeur contre le
//! retour immédiat d'un appareil banni ; elle survit à une réinstallation de
//! Windows tant que le matériel est inchangé.

use lc_domain::GuildId;
use sha2::{Digest, Sha256};

/// Préfixe de domaine de l'empreinte (versionné avec l'algorithme).
const DEVICE_HASH_PREFIX: &str = "lc-device-v2";

/// Longueur maximale admise pour l'identifiant brut, en caractères.
const MAX_DEVICE_ID_CHARS: usize = 256;

/// Normalise l'identifiant d'appareil facultatif reçu à l'appairage.
///
/// - absent ou vide après `trim` → `None` (les anciens clients n'en envoient
///   pas ; aucun appareil n'est alors identifiable) ;
/// - trop long ou contenant un caractère de contrôle → `Err(())`, traduit en
///   `400 invalid_payload` par l'appelant ;
/// - sinon → `Some(identifiant)` débarrassé de ses espaces de bord.
pub(super) fn normalize_device_id(raw: Option<&str>) -> Result<Option<String>, ()> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_DEVICE_ID_CHARS || trimmed.chars().any(char::is_control) {
        return Err(());
    }
    Ok(Some(trimmed.to_owned()))
}

/// Empreinte SHA-256 de l'identifiant d'appareil, salée par la guilde.
///
/// Le sel lie l'empreinte à une guilde : le même poste n'y a pas la même
/// empreinte d'une guilde à l'autre. Seule cette empreinte est stockée
/// (`overlay_clients.device_hash`, `guild_ban_devices.device_hash`).
pub(super) fn device_hash(guild_id: &GuildId, device_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DEVICE_HASH_PREFIX.as_bytes());
    hasher.update(b"|");
    hasher.update(guild_id.as_str().as_bytes());
    hasher.update(b"|");
    hasher.update(device_id.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{device_hash, normalize_device_id};
    use lc_domain::GuildId;

    #[test]
    fn normalizes_only_usable_device_ids() {
        assert_eq!(normalize_device_id(None).unwrap(), None);
        assert_eq!(normalize_device_id(Some("   ")).unwrap(), None);
        assert_eq!(
            normalize_device_id(Some("  board=abc;cpu=xyz  "))
                .unwrap()
                .as_deref(),
            Some("board=abc;cpu=xyz")
        );
        assert!(normalize_device_id(Some(&"x".repeat(256))).is_ok());
        assert!(normalize_device_id(Some(&"x".repeat(257))).is_err());
        // Un saut de ligne est un caractère de contrôle : il n'a pas sa place
        // dans une empreinte d'appareil.
        assert!(normalize_device_id(Some("board=abc\ncpu=xyz")).is_err());
    }

    #[test]
    fn fingerprint_is_stable_guild_scoped_and_never_the_raw_id() {
        let guild = GuildId::parse("123456789012345678").unwrap();
        let other_guild = GuildId::parse("987654321098765432").unwrap();

        let hash = device_hash(&guild, "board=abc;disk=def;cpu=ghi");
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));

        assert_eq!(hash, device_hash(&guild, "board=abc;disk=def;cpu=ghi"));
        assert_ne!(
            hash,
            device_hash(&other_guild, "board=abc;disk=def;cpu=ghi")
        );
        assert_ne!(hash, device_hash(&guild, "board=abc;disk=def;cpu=jkl"));
        assert!(!hash.contains("board=abc"));
    }
}
