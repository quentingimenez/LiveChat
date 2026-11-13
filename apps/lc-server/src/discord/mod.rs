//! Runtime Discord du serveur (Phase 6b).
//!
//! Ce module raccorde le crate [`lc_discord`] (catalogue des commandes, logique
//! métier et glue serenity de la Phase 6a) au serveur HTTP [`crate`] :
//!
//! | fichier | rôle |
//! |---|---|
//! | [`runtime`] | démarrage/arrêt du client serenity et handle retourné au serveur |
//! | [`event_handler`] | handler gateway : `ready`, `guild_create`, `interaction_create`, `message` |
//! | [`registration`] | enregistrement global des commandes via le catalogue `lc_discord` |
//! | [`adapters`] | adaptateurs [`lc_discord::BotContext`] au-dessus de [`crate::AppState`] |
//! | [`message_commands`] | classification des messages entrants (no-op fidèle au legacy) |
//! | [`guild_leave`] | départ du bot d'un serveur Discord (trait injectable) |
//! | [`error`] | erreurs de démarrage/arrêt du runtime |
//!
//! # Câblage côté serveur (fait par le Lead, hors de ce périmètre)
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use lc_server::{AppState, Config, Shutdown};
//! # use lc_server::discord::DiscordRuntime;
//! # async fn wiring(config: Arc<Config>, mut state: AppState, shutdown: Shutdown) {
//! let runtime = match DiscordRuntime::start(Arc::clone(&config), state.clone()).await {
//!     Ok(handle) => {
//!         state = state.with_discord_probe(handle.probe());
//!         Some(handle)
//!     }
//!     Err(error) if error.is_disabled() => None,
//!     Err(error) => {
//!         tracing::error!(%error, "démarrage Discord impossible");
//!         None
//!     }
//! };
//!
//! // … boucle de service (axum ou attente du signal d'arrêt) …
//! let _ = shutdown.subscribe().wait().await;
//! if let Some(handle) = runtime {
//!     if let Err(error) = handle.shutdown().await {
//!         tracing::warn!(%error, "arrêt Discord incomplet");
//!     }
//! }
//! # }
//! ```
//!
//! `DISCORD_TOKEN` est optionnel côté `Config` : le runtime renvoie alors
//! [`DiscordError::Disabled`] (sans réseau ni blocage du boot), que le serveur
//! traduit en absence de runtime. L'état `/health` reste alimenté par
//! [`crate::http::health::DiscordProbe`] tant que le runtime n'a pas démarré ;
//! après démarrage, `DiscordHandle::probe()` fournit une sonde réelle branchée
//! sur le gateway.

#![forbid(unsafe_code)]

pub mod adapters;
pub mod error;
pub mod event_handler;
pub mod guild_leave;
pub mod message_commands;
pub mod registration;
pub mod runtime;

pub use adapters::{
    bot_config_from, DbMediaBoardBackend, DbOverlayDirectory, DiscordAdapters,
    UnavailableMediaBackend, UnavailablePlaybackBackend,
};
pub use error::DiscordError;
pub use event_handler::DiscordEventHandler;
pub use runtime::{DiscordHandle, DiscordRuntime, DiscordStatus, DiscordStatusProbe};
