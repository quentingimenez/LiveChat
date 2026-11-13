//! Pont entre le panneau d'administration et le hub WebSocket.
//!
//! Le panneau ne dépend pas du type concret du hub (`crate::ws`) : il ne parle
//! qu'à ce trait, injecté dans [`AppState`] au démarrage par le câblage
//! applicatif (`AppState::with_admin_hub`). Par défaut, [`NoopAdminHub`] répond
//! « rien de connecté » et ne diffuse rien : le panneau reste testable sans
//! serveur WebSocket, et les handlers ne peuvent pas échouer faute de hub.
//!
//! Les méthodes synchrones (`connected_count`, `connected_for_guild`,
//! `is_connected`, `close_client`) sont volontairement non bloquantes : elles
//! sont appelées pour construire l'aperçu (`/admin/api/overview`) et pour
//! fermer une session révoquée, où un `await` n'apporterait rien. Les deux
//! diffusions (`broadcast_stop`, `broadcast_settings`) sont asynchrones car
//! elles peuvent attendre l'envoi sur des sockets.
//!
//! # Exemple de câblage (côté serveur)
//!
//! ```ignore
//! let state = AppState::new(/* … */).with_admin_hub(Arc::new(mon_hub_websocket));
//! ```
//!
//! AUD-COR-14 : le hub est porté par [`AppState`] (plus de `OnceLock` global),
//! ce qui rend les tests hermétiques et évite qu'un second démarrage dans le
//! même processus ne réutilise silencieusement l'ancien hub.

use std::sync::Arc;

use async_trait::async_trait;
use lc_domain::GuildId;
use uuid::Uuid;

use crate::app::AppState;

/// Ensemble minimal d'opérations WebSocket dont le panneau admin a besoin.
#[async_trait]
pub trait AdminHub: Send + Sync {
    /// Nombre total de sockets overlay connectées (toutes guildes confondues).
    fn connected_count(&self) -> usize;

    /// Nombre de sockets connectées pour une guilde donnée.
    fn connected_for_guild(&self, guild_id: &GuildId) -> usize;

    /// Vrai si le client overlay `client_id` a au moins une socket connectée.
    fn is_connected(&self, client_id: &Uuid) -> bool;

    /// Ferme toutes les sockets du client overlay `client_id`.
    ///
    /// Retourne `true` si au moins une socket a été fermée ; `code` est le code
    /// de fermeture WebSocket transmis au client (4402 pour une révocation).
    fn close_client(&self, client_id: &Uuid, code: u16) -> bool;

    /// Closes every socket of `client_id` with an explicit close reason.
    fn close_client_with_reason(&self, client_id: &Uuid, code: u16, reason: &str) -> bool;

    /// Diffuse un arrêt manuel à la guilde ; retourne le nombre de sockets
    /// touchées (0 si le hub n'expose pas cette information).
    async fn broadcast_stop(&self, guild_id: &GuildId) -> usize;

    /// Purge administrateur : arrête tous les jobs avec
    /// `stop {reason:"purge", jobId:null}`.
    ///
    /// Implémentation par défaut : retombe sur [`Self::broadcast_stop`] tant
    /// que le hub concret n'expose pas la raison dédiée.
    async fn broadcast_purge(&self, guild_id: &GuildId) -> usize {
        self.broadcast_stop(guild_id).await
    }

    /// Diffuse un changement de réglages à la guilde ; retourne le nombre de
    /// sockets touchées (0 si le hub n'expose pas cette information).
    async fn broadcast_settings(&self, guild_id: &GuildId) -> usize;
}

impl std::fmt::Debug for dyn AdminHub {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AdminHub")
    }
}

/// Hub inerte : aucune socket, aucune diffusion.
///
/// C'est l'implémentation par défaut tant que le serveur n'a pas de hub
/// WebSocket réel (ou dans les tests).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopAdminHub;

impl NoopAdminHub {
    /// Construit le hub inerte.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AdminHub for NoopAdminHub {
    fn connected_count(&self) -> usize {
        0
    }

    fn connected_for_guild(&self, _guild_id: &GuildId) -> usize {
        0
    }

    fn is_connected(&self, _client_id: &Uuid) -> bool {
        false
    }

    fn close_client(&self, _client_id: &Uuid, _code: u16) -> bool {
        false
    }

    fn close_client_with_reason(&self, _client_id: &Uuid, _code: u16, _reason: &str) -> bool {
        false
    }

    async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
        0
    }

    async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
        0
    }
}

/// Hub actif porté par l'état applicatif (Noop par défaut).
pub fn admin_hub(state: &AppState) -> Arc<dyn AdminHub> {
    Arc::clone(&state.admin_hub)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[tokio::test]
    async fn noop_hub_ne_connecte_rien_et_ne_diffuse_rien() {
        let hub = NoopAdminHub::new();
        let guild_id = GuildId::parse("1234567890").expect("identifiant valide");
        let client_id = Uuid::nil();

        assert_eq!(hub.connected_count(), 0);
        assert_eq!(hub.connected_for_guild(&guild_id), 0);
        assert!(!hub.is_connected(&client_id));
        assert!(!hub.close_client(&client_id, 4402));
        assert!(!hub.close_client_with_reason(&client_id, 4404, "guild_deleted"));
        assert_eq!(hub.broadcast_stop(&guild_id).await, 0);
        assert_eq!(hub.broadcast_settings(&guild_id).await, 0);
    }
}
