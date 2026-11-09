//! Hub WebSocket v2 (`GET /ws`) — Phase 4b (owner : protocole-v2).
//!
//! Sous-routeur monté par [`crate::app::router`] ; vide tant que la Phase 4b
//! n'a pas posé le handshake `hello`/`welcome`, l'enveloppe `{v:2,type,id,ts,payload}`,
//! les limites (64 Kio, 20 msg/s, ping 15 s/pong 45 s), les close codes
//! 4401/4402/4403/4408/4409 et l'adaptateur `lc-playback` (`PlaybackSink`).
//! L'agent possède `src/ws/**` ; `app.rs`, `http/mod.rs`, `Cargo.toml` et
//! `lib.rs` appartiennent au Lead.

use axum::Router;

use crate::app::AppState;

/// Sous-routeur `/ws` (contrat à compléter en Phase 4b).
pub fn router() -> Router<AppState> {
    Router::new()
}
