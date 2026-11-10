//! Câblage final des composants du serveur (propriété Lead).
//!
//! Ce module relie les briques qui ne doivent pas se connaître mutuellement :
//!
//! - [`crate::ws::Hub`] implémente le trait [`AdminHub`] de la lane admin
//!   (compteurs de connexions, fermeture ciblée, diffusions `stop`/`settings`) ;
//! - le hook d'ingestion média notifie le moteur de lecture **déjà démarré**
//!   (`PlaybackEngine::notify`), jamais `enqueue` : le service média a déjà
//!   persisté le job `PENDING` (cf. [`crate::media::hook`]).
//!
//! Aucun comportement métier ici : uniquement de la glue, testée unitairement.

#![forbid(unsafe_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use lc_domain::entities::PlaybackJob;
use lc_domain::ids::GuildId;
use uuid::Uuid;

use crate::app::AppState;
use crate::http::admin::hub::{install_admin_hub, AdminHub};
use crate::media::hook::{install_ingest_hook, IngestHook};
use crate::ws::Hub;

/// Implémentation du contrat admin au-dessus du hub WebSocket réel.
///
/// Le hub est créé au démarrage par `serve()` et partagé entre le routeur
/// (`router_with_hub`), l'admin (`install_admin_hub`) et le hook média.
#[async_trait]
impl AdminHub for Hub {
    fn connected_count(&self) -> usize {
        Hub::connected_count(self)
    }

    fn connected_for_guild(&self, guild_id: &GuildId) -> usize {
        Hub::connected_for_guild(self, guild_id)
    }

    fn is_connected(&self, client_id: &Uuid) -> bool {
        Hub::is_connected(self, client_id)
    }

    fn close_client(&self, client_id: &Uuid, code: u16) -> bool {
        Hub::close_client(self, client_id, code)
    }

    async fn broadcast_stop(&self, guild_id: &GuildId) -> usize {
        Hub::broadcast_stop(self, guild_id).await
    }

    async fn broadcast_purge(&self, guild_id: &GuildId) -> usize {
        Hub::broadcast_purge(self, guild_id).await
    }

    async fn broadcast_settings(&self, guild_id: &GuildId) -> usize {
        Hub::broadcast_settings(self, guild_id).await
    }
}

/// Hook média : réveille le moteur de lecture quand un job vient d'être créé.
///
/// Le moteur est créé paresseusement au premier `hello` : tant qu'aucun overlay
/// n'est connecté, le job reste `PENDING` en base et sera lu au `bootstrap`
/// suivant (aucune perte, comportement legacy équivalent).
pub struct IngestNotifier {
    hub: Arc<Hub>,
}

impl IngestNotifier {
    /// Crée le hook pour le hub donné.
    #[must_use]
    pub fn new(hub: Arc<Hub>) -> Self {
        Self { hub }
    }
}

impl IngestHook for IngestNotifier {
    fn on_ingested<'a>(
        &'a self,
        job: &'a PlaybackJob,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            match self.hub.engine() {
                Some(engine) => {
                    if let Err(error) = engine.notify(&job.guild_id).await {
                        tracing::warn!(
                            error = %error,
                            guild_id = %job.guild_id.as_str(),
                            "notification du moteur de lecture impossible"
                        );
                    }
                }
                None => tracing::debug!(
                    guild_id = %job.guild_id.as_str(),
                    "job mis en file ; moteur de lecture pas encore démarré"
                ),
            }
        })
    }
}

/// Installe le hub comme `AdminHub` global et branche le hook d'ingestion.
///
/// Retourne `true` si l'`AdminHub` a bien été installé (un second appel ne
/// remplace pas l'instance existante ; le hook média est, lui, remplaçable).
pub fn install(state: &AppState, hub: &Arc<Hub>) -> bool {
    let _ = state;
    let hub_handle: Arc<Hub> = Arc::clone(hub);
    let admin: Arc<dyn AdminHub> = hub_handle;
    let installed = install_admin_hub(admin);
    if !installed {
        tracing::warn!("AdminHub déjà installé : le hub WebSocket n'est pas celui utilisé");
    }
    install_ingest_hook(Arc::new(IngestNotifier::new(Arc::clone(hub))));
    installed
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn hub() -> Arc<Hub> {
        Arc::new(Hub::new())
    }

    fn as_dyn(hub: &Arc<Hub>) -> Arc<dyn AdminHub> {
        let clone: Arc<Hub> = Arc::clone(hub);
        clone
    }

    #[test]
    fn admin_hub_contract_reflects_an_empty_hub() {
        let hub = hub();
        let admin = as_dyn(&hub);
        assert_eq!(admin.connected_count(), 0);
        let guild = GuildId::parse("123456789012345678").expect("guild id valide");
        assert_eq!(admin.connected_for_guild(&guild), 0);
        assert!(!admin.is_connected(&Uuid::new_v4()));
        assert!(!admin.close_client(&Uuid::new_v4(), 1000));
    }

    #[tokio::test]
    async fn broadcast_on_empty_hub_is_a_noop() {
        let hub = hub();
        let admin = as_dyn(&hub);
        let guild = GuildId::parse("123456789012345678").expect("guild id valide");
        assert_eq!(admin.broadcast_stop(&guild).await, 0);
        assert_eq!(admin.broadcast_settings(&guild).await, 0);
    }

    #[tokio::test]
    async fn ingest_notifier_without_engine_is_a_noop() {
        let notifier = IngestNotifier::new(hub());
        let now = chrono::Utc::now();
        let job = PlaybackJob {
            id: lc_domain::ids::PlaybackJobId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guild id valide"),
            media_asset_id: None,
            text: Some("bonjour".into()),
            show_text: true,
            author_name: None,
            author_image: None,
            duration_sec: 5,
            priority: 0,
            status: lc_domain::enums::JobStatus::Pending,
            resumes_after_job_id: None,
            resume_offset_sec: 0,
            remaining_ms_snapshot: None,
            last_playback_state_at: None,
            submission_date: now,
            execution_date: now,
            scheduled_at: now,
            started_at: None,
            finished_at: None,
        };
        IngestHook::on_ingested(&notifier, &job).await;
    }
}
