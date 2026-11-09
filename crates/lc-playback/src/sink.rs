//! Port de sortie vers les overlays.
//!
//! Le moteur ne connaît ni le WebSocket ni l'application serveur : il remet les
//! payloads `play`/`stop` à une implémentation de [`PlaybackSink`]. En
//! production, l'implémentation est branchée sur le hub v2
//! (`apps/lc-server`) ; dans les tests, un `FakeSink` enregistre les appels.

use std::future::Future;

use lc_domain::GuildId;
use lc_protocol::{PlayPayload, StopPayload};

/// Cible d'émission des messages de lecture.
///
/// Les méthodes renvoient un [`Future`] pour que l'implémentation réelle puisse
/// attendre la diffusion réseau. `on_play` renvoie le nombre de clients
/// ciblés : `0` signifie qu'aucun overlay n'est connecté, le job est alors
/// marqué `FAILED` (parité avec le scheduler legacy qui échouait sur une room
/// vide).
pub trait PlaybackSink: Send + Sync {
    /// Diffuse un message `play` (unicast par socket côté hub).
    fn on_play(
        &self,
        guild_id: &GuildId,
        payload: PlayPayload,
    ) -> impl Future<Output = usize> + Send;

    /// Diffuse un message `stop` (broadcast de la room de la guilde).
    fn on_stop(&self, guild_id: &GuildId, payload: StopPayload) -> impl Future<Output = ()> + Send;
}
