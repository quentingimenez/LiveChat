//! Identifiants typés du domaine.
//!
//! Deux familles :
//! - les identifiants générés par le serveur (`MediaAssetId`, `MediaBoardItemId`,
//!   `PlaybackJobId`, `OverlayClientId`, `IngestClientId`) sont des **UUID v4**
//!   (stockés `TEXT` dans SQLite, comme le legacy Prisma `@default(uuid())`) ;
//! - `GuildId` (snowflake Discord, numérique) et `PairingCode` (code lisible
//!   sans ambiguïté) sont des identifiants **externes** validés, stockés `TEXT`.
//!
//! Le but est d'empêcher les mélanges d'identifiants dans les signatures : une
//! `MediaAssetId` ne peut pas être passée là où une `PlaybackJobId` est
//! attendue, même si les deux sont des UUID en base.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

/// Déclare un newtype UUID avec les conversions standard du domaine.
macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident, $entity:literal) => {
        $(#[$meta])*
        ///
        /// Sérialisé en JSON comme une chaîne UUID canonique (transparent).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Génère un nouvel identifiant aléatoire (UUID v4).
            pub fn new_v4() -> Self {
                Self(Uuid::new_v4())
            }

            /// Décode une chaîne UUID (`8-4-4-4-12`, casse indifférente).
            pub fn parse(raw: &str) -> Result<Self, DomainError> {
                Uuid::parse_str(raw)
                    .map(Self)
                    .map_err(|error| {
                        DomainError::validation(format!(
                            concat!($entity, " : UUID invalide ({}): {}"),
                            raw,
                            error
                        ))
                    })
            }

            /// Accès à l'UUID sous-jacent.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }

            /// Forme canonique `8-4-4-4-12`.
            pub fn to_canonical(self) -> String {
                self.0.to_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::parse(raw)
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl AsRef<Uuid> for $name {
            fn as_ref(&self) -> &Uuid {
                &self.0
            }
        }
    };
}

uuid_id!(
    /// Identifiant d'un média ingéré (`media_assets.id`).
    MediaAssetId,
    "media_asset"
);

uuid_id!(
    /// Identifiant d'un élément du board média (`media_board_items.id`).
    MediaBoardItemId,
    "media_board_item"
);

uuid_id!(
    /// Identifiant d'un job de lecture (`playback_jobs.id`).
    PlaybackJobId,
    "playback_job"
);

uuid_id!(
    /// Identifiant d'un overlay appairé (`overlay_clients.id`).
    OverlayClientId,
    "overlay_client"
);

uuid_id!(
    /// Identifiant d'un client d'ingestion (`ingest_clients.id`).
    IngestClientId,
    "ingest_client"
);

uuid_id!(
    /// Identifiant d'un membre de guilde (`guild_members.id`).
    GuildMemberId,
    "guild_member"
);

/// Identifiant de guilde (numérique, stocké `TEXT`).
///
/// Historiquement un snowflake Discord ; une guilde créée hors Discord reçoit
/// un identifiant numérique de même format, généré par [`GuildId::generate`].
///
/// Validé : chaîne non vide composée uniquement de chiffres ASCII et d'au plus
/// 20 caractères (`u64` maximal). Aucune conversion `u64` n'est imposée car le
/// legacy manipule les snowflakes comme des chaînes (précision JSON).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GuildId(String);

impl GuildId {
    /// Longueur maximale d'un snowflake `u64` en base 10.
    pub const MAX_LEN: usize = 20;

    /// Valide et normalise un identifiant de guilde (trim, aucune casse).
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let value = raw.trim();
        if value.is_empty() {
            return Err(DomainError::validation("guild_id vide"));
        }
        if value.len() > Self::MAX_LEN {
            return Err(DomainError::validation(format!(
                "guild_id trop long ({} > {} caractères)",
                value.len(),
                Self::MAX_LEN
            )));
        }
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(DomainError::validation(format!(
                "guild_id non numérique : {value}"
            )));
        }
        Ok(Self(value.to_owned()))
    }

    /// Génère l'identifiant d'une guilde créée hors Discord.
    ///
    /// 19 chiffres commençant par `9` : même format qu'un snowflake (aucun
    /// changement de validation ni de stockage), dans une plage que Discord
    /// n'atteindra pas avant plusieurs décennies. L'unicité réelle est garantie
    /// par la clé primaire `guilds.id` (l'appelant retente sur conflit).
    pub fn generate() -> Self {
        const SPAN: u128 = 1_000_000_000_000_000_000;
        let random = Uuid::new_v4().as_u128() % SPAN;
        Self((9 * SPAN + random).to_string())
    }

    /// Représentation texte (snowflake).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GuildId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for GuildId {
    type Err = DomainError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

impl AsRef<str> for GuildId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GuildId {
    type Error = DomainError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw)
    }
}

/// Code d'appairage lisible (`pairing_codes.code`, clé primaire).
///
/// Le générateur legacy (`server/src/components/discord/clientCommand.ts:8`)
/// utilise l'alphabet sans ambiguïté `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` sur
/// 6 caractères. La validation accepte tout code alphanumérique MAJUSCULE de
/// 1 à 32 caractères, et **normalise** l'entrée (`trim` + `to_ascii_uppercase`)
/// comme le faisaient les routes legacy (`overlayRoutes.ts:203`,
/// `ingestRoutes.ts:86`) : un utilisateur peut saisir `ab12cd` sans casser le
/// lookup.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PairingCode(String);

impl PairingCode {
    /// Longueur maximale acceptée (large marge au-dessus des 6 legacy).
    pub const MAX_LEN: usize = 32;

    /// Normalise (`trim` + MAJUSCULES ASCII) puis valide un code d'appairage.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let value = raw.trim().to_ascii_uppercase();
        if value.is_empty() {
            return Err(DomainError::validation("pairing code vide"));
        }
        if value.len() > Self::MAX_LEN {
            return Err(DomainError::validation(format!(
                "pairing code trop long ({} > {} caractères)",
                value.len(),
                Self::MAX_LEN
            )));
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(DomainError::validation(format!(
                "pairing code invalide (alphanumérique attendu) : {value}"
            )));
        }
        Ok(Self(value))
    }

    /// Représentation texte normalisée (MAJUSCULES).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PairingCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for PairingCode {
    type Err = DomainError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

impl AsRef<str> for PairingCode {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PairingCode {
    type Error = DomainError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{GuildId, MediaAssetId, PairingCode, PlaybackJobId};
    use crate::error::DomainError;

    #[test]
    fn uuid_parse_et_affichage() {
        let id = MediaAssetId::new_v4();
        let text = id.to_string();
        let parsed = MediaAssetId::parse(&text).unwrap();
        assert_eq!(id, parsed);

        let uppercase = text.to_ascii_uppercase();
        assert_eq!(MediaAssetId::parse(&uppercase).unwrap(), id);
    }

    #[test]
    fn uuid_invalide_rejete() {
        assert!(matches!(
            MediaAssetId::parse("pas-un-uuid"),
            Err(DomainError::Validation(_))
        ));
        assert!(matches!(
            "pas-un-uuid".parse::<PlaybackJobId>(),
            Err(DomainError::Validation(_))
        ));
    }

    #[test]
    fn ids_uuid_distincts_ne_se_melangent_pas() {
        // Compile uniquement parce que les types sont distincts : garantit
        // qu'aucune conversion implicite n'est possible entre identifiants.
        let media = MediaAssetId::new_v4();
        let job = PlaybackJobId::from(media.as_uuid());
        assert_eq!(media.to_string(), job.to_string());
    }

    #[test]
    fn guild_id_valide_et_normalise() {
        let guild = GuildId::parse(" 123456789012345678 ").unwrap();
        assert_eq!(guild.as_str(), "123456789012345678");
        assert_eq!(guild.to_string(), "123456789012345678");
    }

    #[test]
    fn guild_id_genere_est_valide() {
        let first = GuildId::generate();
        let second = GuildId::generate();
        assert_eq!(first.as_str().len(), 19);
        assert!(first.as_str().starts_with('9'));
        assert_eq!(GuildId::parse(first.as_str()).unwrap(), first);
        assert_ne!(first, second);
    }

    #[test]
    fn guild_id_invalide_rejete() {
        assert!(matches!(
            GuildId::parse(""),
            Err(DomainError::Validation(_))
        ));
        assert!(matches!(
            GuildId::parse("12ab"),
            Err(DomainError::Validation(_))
        ));
        assert!(matches!(
            GuildId::parse("123456789012345678901"),
            Err(DomainError::Validation(_))
        ));
    }

    #[test]
    fn pairing_code_normalise_lowercase() {
        let code = PairingCode::parse(" ab12cd ").unwrap();
        assert_eq!(code.as_str(), "AB12CD");
        assert_eq!(code.to_string(), "AB12CD");
    }

    #[test]
    fn pairing_code_invalide_rejete() {
        assert!(matches!(
            PairingCode::parse("A-B"),
            Err(DomainError::Validation(_))
        ));
        assert!(matches!(
            PairingCode::parse(""),
            Err(DomainError::Validation(_))
        ));
        let too_long = "A".repeat(PairingCode::MAX_LEN + 1);
        assert!(matches!(
            PairingCode::parse(&too_long),
            Err(DomainError::Validation(_))
        ));
    }
}
