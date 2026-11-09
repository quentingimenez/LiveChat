//! Routes `/admin` — Phase 7b (owner : infra-rust).
//!
//! Sous-routeur monté par [`crate::app::router`] ; vide tant que la Phase 7b
//! n'a pas posé le service des assets statiques (`static/admin/`) et les
//! 13 handlers JSON d'administration. L'agent possède `http/admin/**` ;
//! `app.rs`, `http/mod.rs`, `Cargo.toml` et `lib.rs` appartiennent au Lead.

use axum::Router;

use crate::app::AppState;

/// Sous-routeur `/admin` (contrat à compléter en Phase 7b).
pub fn router() -> Router<AppState> {
    Router::new()
}
