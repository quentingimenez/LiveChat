//! Domaine LiveChat : entités, identifiants, erreurs métier et horloge.
//!
//! Ce crate ne dépend d'aucun runtime réseau ni d'aucun stockage : il ne
//! contient que des types partagés par `lc-db`, `lc-media`, `lc-discord`,
//! `lc-admin` et le serveur (`apps/lc-server`).
//!
//! Conventions :
//! - dates en mémoire : [`chrono::DateTime<Utc>`] (conversion ms epoch dans
//!   [`time`], la base stocke des `INTEGER` ms — voir `docs/rust/lc-db.md`) ;
//! - chaînes « de base » : enums stockés en `TEXT` MAJUSCULE (`IMAGE`,
//!   `INVITE_READ_ONLY`…), conversions centralisées dans [`enums`] ;
//! - JSON : `camelCase`, dates en ms epoch (`chrono::serde::ts_milliseconds`)
//!   pour rester compatible avec les payloads de reprise v2 ;
//! - identifiants : newtypes typés dans [`ids`], jamais de `String` brut dans
//!   les signatures publiques.
#![forbid(unsafe_code)]

pub mod clock;
pub mod entities;
pub mod enums;
pub mod error;
pub mod ids;
pub mod time;

pub use clock::{Clock, FixedClock, SystemClock};
pub use entities::{
    Guild, IngestClient, MediaAsset, MemeBoardItem, OverlayClient, PairingCodeEntry, PlaybackJob,
};
pub use enums::{JobStatus, MediaKind, MediaStatus, OverlaySessionMode, PairingMode};
pub use error::DomainError;
pub use ids::{
    GuildId, IngestClientId, MediaAssetId, MemeBoardItemId, OverlayClientId, PairingCode,
    PlaybackJobId,
};
pub use time::{datetime_to_ms, ms_to_datetime, now_ms};
