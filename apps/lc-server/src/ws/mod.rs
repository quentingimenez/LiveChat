//! Hub WebSocket v2 (`GET /ws`) — Phase 4b.
//!
//! Ce module remplace socket.io 1.x (cf. `docs/rust/protocol-v2.md`) par un
//! WebSocket JSON natif :
//!
//! - une seule route HTTP: [`router`] monte `GET /ws` et partage un [`Hub`]
//!   (registre des overlays connectés, ordre de diffusion) via
//!   `Extension<Arc<Hub>>` ;
//! - le handshake est un message `hello` applicatif (token d'appairage en
//!   clair, comparé côté serveur au SHA-256 stocké) sous 5 secondes, qui donne
//!   lieu à un `welcome` puis à un `settings` et à un `peers` ;
//! - les erreurs fatales ferment la connexion avec les codes `4401`
//!   (authentification), `4402` (client révoqué), `4403` (protocole
//!   obsolète), `4408` (trame trop grande) ou `4409` (débit excessif).
//!
//! ## Intégration avec `lc-playback`
//!
//! Le hub construit paresseusement un unique [`PlaybackEngine`]
//! (cf. [`Hub::ensure_runtime`]) au premier `hello` réussi, à partir de la
//! base `lc-db` et de la configuration du serveur, puis :
//!
//! - implémente [`PlaybackSink`] pour `lc-playback` : les `play` sont émis en
//!   **unicast** (un par socket, avec une URL média signée pour le `clientId`
//!   de la connexion) et les `stop` en **broadcast** à la room de la guilde ;
//! - alimente le moteur avec les messages `playback.state`, `playback.ended`,
//!   `playback.ended`, `meme.trigger` et `stop.request` ;
//! - lance la boucle de fond du moteur (`run_until_shutdown` via `run`) et une
//!   purge de rétention périodique, toutes deux arrêtées par
//!   `AppState::shutdown`.
//!
//! ## API d'administration
//!
//! [`Hub`] expose les opérations consommées par la couche HTTP admin
//! (trait `AdminHub` de `http/admin/hub.rs`) : [`Hub::connected_count`],
//! [`Hub::connected_for_guild`], [`Hub::is_connected`], [`Hub::close_client`],
//! [`Hub::broadcast_stop`] et [`Hub::broadcast_settings`].
//!
//! ## Limites
//!
//! - l'URL média est signée par le hub juste avant l'envoi ; si
//!   `MEDIA_URL_SECRET` est absent, le `play` n'est pas remis au client et le
//!   job échoue côté moteur (`dispatch_failed`) ;
//! - l'inscription d'une connexion se fait après authentification et
//!   `welcome` ; les erreurs de `hello` ne laissent aucune entrée dans le
//!   registre ;
//! - le hub ne modifie pas `AppState` : il s'initialise au premier `hello`.

mod hub;
mod session;

#[cfg(test)]
mod tests;

pub use hub::{Hub, HubEngine, HubError, HubSink};

use std::sync::Arc;

use axum::extract::Extension;
use axum::routing::get;
use axum::Router;

use crate::app::AppState;

/// Construit le routeur WebSocket v2 avec un hub neuf.
///
/// Utilisé par `app::router()` ; les tests peuvent passer un hub existant à
/// [`router_with_hub`] pour injecter une base de test.
pub fn router() -> Router<AppState> {
    router_with_hub(Arc::new(Hub::new()))
}

/// Construit le routeur WebSocket v2 autour d'un hub existant.
pub fn router_with_hub(hub: Arc<Hub>) -> Router<AppState> {
    let router: Router<AppState> = Router::new().route("/ws", get(session::ws_handler));
    router.layer(Extension(hub))
}
