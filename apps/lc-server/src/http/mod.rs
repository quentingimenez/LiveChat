//! Couche HTTP : sous-routeurs métier et middlewares transverses.
//!
//! Chaque module est indépendant et fournit soit un sous-routeur
//! (`pub fn router() -> Router<AppState>`), soit un middleware :
//!
//! - [`root`] : `GET /` (identité de service) ;
//! - [`health`] : `GET /health` enrichi (SQLite, Discord, jobs actifs) ;
//! - [`socket_io`] : garde-fou 426 pour les clients protocolaire v1 ;
//! - [`security`] : CSP et en-têtes de sécurité ;
//! - [`cors`] : origines autorisées explicites ;
//! - [`rate_limit`] : 100 requêtes / 15 min par IP ;
//! - [`error`] : normalisation JSON des erreurs et repli 404.
//!
//! L'assemblage (ordre des couches, état partagé) est centralisé dans
//! [`crate::app::router`] : les Phases 3b à 7 ajoutent leurs sous-routeurs à cet
//! assemblage sans modifier `main.rs`.

pub mod admin;
pub mod cors;
pub mod error;
pub mod health;
pub mod ingest;
pub mod overlay;
pub mod rate_limit;
pub mod root;
pub mod security;
pub mod socket_io;
