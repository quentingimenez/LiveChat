//! Routes `/overlay` — Phase 3b (owner : overlay-bridge).
//!
//! Sous-routeur monté par [`crate::app::router`] ; le module est vide tant que
//! la Phase 3b n'a pas posé ses routes (pair/consume, média signé, meme board).
//! L'agent de la lane possède ce répertoire (`http/overlay/**`) et doit
//! uniquement remplacer le corps de [`router`] (+ sous-modules) ; `app.rs`,
//! `http/mod.rs`, `Cargo.toml` et `lib.rs` appartiennent au Lead.

use axum::Router;

use crate::app::AppState;

/// Sous-routeur `/overlay` (contrat à compléter en Phase 3b).
pub fn router() -> Router<AppState> {
    Router::new()
}
