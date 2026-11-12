//! Couche réseau de l'overlay (Phase 8b).
//!
//! - [`ws`] : client WebSocket v2 (handshake `hello`/`welcome`, heartbeat,
//!   reconnexion avec backoff, trames `play`/`stop`/`peers` → renderer et
//!   commandes du bridge → serveur).
//! - [`pairing`] : appairage HTTP (`POST {serverUrl}/overlay/pair/consume`).

pub mod api;
pub mod pairing;
mod tls;
pub mod ws;

pub use pairing::{consume as consume_pairing, PairingOutcome, PairingRequest};
pub use ws::{
    refresh_tray_from_any_thread, request_reconnect, request_shutdown, send_command,
    ConnectionStatus, NetCommand, NetSnapshot,
};
