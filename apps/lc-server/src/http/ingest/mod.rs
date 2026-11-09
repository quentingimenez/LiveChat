//! Routes `/ingest` — Phases 3c/5b (owner : http-core).
//!
//! Sous-routeur monté par [`crate::app::router`] ; vide tant que la Phase 3c
//! n'a pas posé ses routes (pair/consume invité, POST /ingest) et le worker
//! média Phase 5b. L'agent possède `http/ingest/**` et `src/media/**` ;
//! `app.rs`, `http/mod.rs`, `Cargo.toml` et `lib.rs` appartiennent au Lead.

use axum::Router;

use crate::app::AppState;

/// Sous-routeur `/ingest` (contrat à compléter en Phase 3c).
pub fn router() -> Router<AppState> {
    Router::new()
}
