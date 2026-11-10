//! Pont entre le panneau d'administration et le hub WebSocket.
//!
//! Le panneau ne dépend pas du type concret du hub (`crate::ws`) : il ne parle
//! qu'à ce trait, installé une seule fois au démarrage par le câblage
//! applicatif (voir [`install_admin_hub`]). Tant qu'aucun hub réel n'est
//! installé, [`NoopAdminHub`] répond « rien de connecté » et ne diffuse rien :
//! le panneau reste testable sans serveur WebSocket, et les handlers ne
//! peuvent pas échouer faute de hub.
//!
//! Les méthodes synchrones (`connected_count`, `connected_for_guild`,
//! `is_connected`, `close_client`) sont volontairement non bloquantes : elles
//! sont appelées pour construire l'aperçu (`/admin/api/overview`) et pour
//! fermer une session révoquée, où un `await` n'apporterait rien. Les deux
//! diffusions (`broadcast_stop`, `broadcast_settings`) sont asynchrones car
//! elles peuvent attendre l'envoi sur des sockets.
//!
//! # Exemple de câblage (côté hub réel, lane task-17)
//!
//! ```ignore
//! admin::install_admin_hub(Arc::new(mon_hub_websocket));
//! ```
//!
//! [`install_admin_hub`] n'accepte qu'une installation : le second appel
//! renvoie `false` (le premier hub gagne). C'est volontaire, pour que deux
//! chemins de démarrage ne se contredisent pas silencieusement.

use std::sync::{Arc, OnceLock};

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

    /// Diffuse un arrêt manuel à la guilde ; retourne le nombre de sockets
    /// touchées (0 si le hub n'expose pas cette information).
    async fn broadcast_stop(&self, guild_id: &GuildId) -> usize;

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

    async fn broadcast_stop(&self, _guild_id: &GuildId) -> usize {
        0
    }

    async fn broadcast_settings(&self, _guild_id: &GuildId) -> usize {
        0
    }
}

static ADMIN_HUB: OnceLock<Arc<dyn AdminHub>> = OnceLock::new();

/// Installe le hub réel pour toute la durée du processus.
///
/// Retourne `false` si un hub a déjà été installé (le premier gagne).
pub fn install_admin_hub(hub: Arc<dyn AdminHub>) -> bool {
    ADMIN_HUB.set(hub).is_ok()
}

/// Hub actif : le hub installé, sinon [`NoopAdminHub`].
///
/// `_state` est accepté pour que les appelants n'aient pas à distinguer
/// l'origine du hub ; le hub est global car [`AppState`] ne porte pas encore
/// de champ dédié (le câblage final appartient à la lane app/hub).
pub fn admin_hub(_state: &AppState) -> Arc<dyn AdminHub> {
    ADMIN_HUB
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(NoopAdminHub))
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
        assert_eq!(hub.broadcast_stop(&guild_id).await, 0);
        assert_eq!(hub.broadcast_settings(&guild_id).await, 0);
    }
}
