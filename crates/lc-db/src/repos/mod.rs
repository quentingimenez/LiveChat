//! Repositories typés : une structure par table, exposée par [`Db`](crate::Db).
//!
//! Les repositories ne valident pas la machine d'état métier (c'est le rôle de
//! `lc-domain`/des services) sauf quand la garantie doit être atomique en base
//! (`PlaybackJobsRepository::claim_next`, `PairingCodesRepository::consume`).

mod guilds;
mod ingest_clients;
mod media_assets;
mod meme_board;
mod metadata;
mod overlay_clients;
mod pairing_codes;
mod playback_jobs;

pub use guilds::GuildsRepository;
pub use ingest_clients::IngestClientsRepository;
pub use media_assets::MediaAssetsRepository;
pub use meme_board::MemeBoardRepository;
pub use metadata::MetadataRepository;
pub use overlay_clients::OverlayClientsRepository;
pub use pairing_codes::PairingCodesRepository;
pub use playback_jobs::PlaybackJobsRepository;
