//! Routes HTTP `/overlay/*` — parité REST avec l'ancien serveur Node (Phase 3b).
//!
//! ## Périmètre
//!
//! Ce module remplace `server/src/components/overlay/overlayRoutes.ts` et
//! `server/src/services/overlayAuth.ts` (voir `docs/rust/server-overlay.md`).
//! Il expose :
//!
//! | Méthode | Chemin | Authentification |
//! |---------|--------|------------------|
//! | `POST` | `/overlay/pair/consume` | publique (code d'appairage) |
//! | `GET` | `/overlay/media/{assetId}` | URL signée HMAC (`exp`, `sig`, `clientId`) |
//! | `GET` | `/overlay/meme-board/items` | `Authorization: Bearer` |
//! | `POST` | `/overlay/meme-board/items` | `Authorization: Bearer` |
//! | `GET` | `/overlay/meme-board/media/{assetId}` | `Authorization: Bearer` |
//! | `PATCH` | `/overlay/meme-board/items/{itemId}` | `Authorization: Bearer` |
//! | `DELETE` | `/overlay/meme-board/items/{itemId}` | `Authorization: Bearer` |
//!
//! `GET /overlay/config` **n'existe pas** en protocole v2 (supprimé, cf.
//! `docs/rust/protocol-v2.md` §8) : la route tombe sur le `fallback` de
//! `crate::app`, qui répond `404 {"error":"request_error"}`.
//!
//! ## Conventions
//!
//! - AUCUN handler ne lit l'horloge système : tout instant vient de
//!   `state.clock` (tests déterministes, cf. `docs/rust/server-core.md` §9).
//! - Les erreurs applicatives sont déjà au format JSON minimal du legacy
//!   (`{"error": "<code>"}`, éventuellement enrichi de `message`/`code`) : le
//!   middleware `normalize_errors` laisse intact un corps JSON.
//! - Les accès base passent par `state.db` (`Option<Db>`) ; une base absente
//!   répond `503 {"error":"database_unavailable"}` plutôt que de paniquer.
//! - Aucun `unwrap`/`expect`/`panic` (lints workspace).
//!
//! ## Handoff Phase 4b / Phase 8
//!
//! - Les URLs signées sont générées par le hub WebSocket (Phase 4b) via
//!   [`crate::media_url`] ; la signature et sa vérification sont partagées.
//! - Le renderer board (Phase 8) devra envoyer un en-tête `Authorization`
//!   (le fallback `?token=` du legacy est volontairement supprimé).

// Les helpers (`db`, `resolve_media_file`, ...) et certains handlers renvoient
// `Result<_, Response>` : `Response` est le type d'erreur natif d'axum et le
// boxing systématique dégraderait la lisibilité sans gain mesurable sur ces
// chemins I/O. Le lint `result_large_err` est donc neutralisé à l'échelle du
// module (cf. décision dans `docs/rust/server-overlay.md` §2).
#![allow(clippy::result_large_err)]

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::error;

use crate::app::AppState;

mod auth;
mod media;
mod meme_board;
mod pair;

pub use media::sign_media_url;

#[cfg(test)]
mod tests;

/// Sous-routeur monté par `crate::http::mod` puis `crate::app::router`.
///
/// Sous-routeur `/overlay`, monté tel quel (fusion sans préfixe) par
/// [`crate::app::router`] : il porte donc lui-même le préfixe `/overlay`,
/// comme [`crate::http::ingest::router`].
///
/// Les routes du meme board exigent l'extracteur [`auth::AuthedOverlayClient`]
/// (Bearer uniquement, exécuté avant le handler) ; l'appairage et le média
/// signé ont leurs propres contrôles (code d'appairage / signature HMAC).
pub fn router() -> Router<AppState> {
    let authed = Router::new()
        .route(
            "/overlay/meme-board/items",
            get(meme_board::list_items).post(meme_board::create_item),
        )
        .route(
            "/overlay/meme-board/items/{item_id}",
            axum::routing::patch(meme_board::update_item).delete(meme_board::delete_item),
        )
        .route(
            "/overlay/meme-board/media/{asset_id}",
            get(meme_board::get_media),
        );

    Router::new()
        .route("/overlay/pair/consume", post(pair::consume_pairing))
        .route("/overlay/media/{asset_id}", get(media::get_media))
        .merge(authed)
}

/// Instant courant, toujours lu depuis l'horloge injectée.
fn now(state: &AppState) -> DateTime<Utc> {
    state.clock.now()
}

/// Accès à la base ou réponse `503` normalisée.
fn db(state: &AppState) -> Result<&lc_db::Db, Response> {
    state.db.as_ref().ok_or_else(database_unavailable)
}

/// Réponse JSON applicative.
fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Erreur JSON minimale du legacy (`{"error": code}`).
fn error(status: StatusCode, code: &'static str) -> Response {
    crate::http::error::json_error(status, code)
}

/// `401 {"error":"unauthorized"}`.
fn unauthorized() -> Response {
    error(StatusCode::UNAUTHORIZED, "unauthorized")
}

/// `500 {"error":"internal_error"}`.
fn internal_error() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}

/// `503 {"error":"database_unavailable"}` (serveur démarré sans `DATABASE_URL`).
fn database_unavailable() -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
}

/// Journalise une erreur `lc-db` et renvoie `500 internal_error`.
fn db_failure(operation: &'static str, db_error: &lc_db::DbError) -> Response {
    error!(operation, error = %db_error, "échec d'accès base sur une route overlay");
    internal_error()
}
