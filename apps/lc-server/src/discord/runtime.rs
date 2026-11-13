//! Démarrage et arrêt du runtime Discord.
//!
//! Le serveur HTTP et le bot vivent dans le **même** processus mais restent
//! découplés :
//!
//! - [`DiscordRuntime::start`] est **non bloquant après le login** : il construit
//!   le client serenity, branche le handler, puis lance la boucle gateway dans
//!   une tâche Tokio et rend un [`DiscordHandle`] ;
//! - sans `DISCORD_TOKEN`, il renvoie [`DiscordError::Disabled`] (erreur
//!   « nominale », voir [`DiscordError::is_disabled`]) : le serveur continue de
//!   tourner sans bot, comme le legacy dont `loadDiscord` était asynchrone ;
//! - l'arrêt suit [`crate::Shutdown`] : la tâche gateway écoute le signal et
//!   ferme les shards ; [`DiscordHandle::shutdown`] attend la sortie effective
//!   (borne [`SHUTDOWN_TIMEOUT`]).
//!
//! Le contrat de démarrage (401/priv-intents, application id, etc.) reste géré
//! par serenity ; les erreurs sont journalisées et exposées via
//! [`DiscordStatus`], embarqué par la sonde `/health` ([`DiscordStatusProbe`]).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serenity::all::{Client, GatewayIntents, ShardManager};
use tokio::task::JoinHandle;

use lc_db::Db;
use lc_discord::{BotConfig, DbBotContext};
use lc_domain::Clock;

use crate::http::health::{HealthProbe, HealthState};
use crate::shutdown::{Shutdown, DRAIN_TIMEOUT};
use crate::{AppState, Config};

use super::adapters::{bot_config_from, DiscordAdapters};
use super::event_handler::DiscordEventHandler;
use super::DiscordError;

/// Marge d'attente de la sortie du gateway à l'arrêt.
pub const SHUTDOWN_TIMEOUT: Duration = DRAIN_TIMEOUT;

/// État partagé du runtime, alimenté par le handler gateway.
///
/// `ready` passe à `true` au premier `ready` ; les erreurs du gateway et de
/// l'enregistrement des commandes sont conservées pour les journaux et pour un
/// futur endpoint d'administration (non exposé ici).
#[derive(Debug, Default)]
pub struct DiscordStatus {
    ready: AtomicBool,
    gateway_error: Mutex<Option<String>>,
    registration_error: Mutex<Option<String>>,
    registered_commands: AtomicUsize,
}

impl DiscordStatus {
    /// `true` quand le gateway a envoyé `READY` au moins une fois.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Dernière erreur fatale du gateway, si le runtime s'est arrêté en erreur.
    pub fn gateway_error(&self) -> Option<String> {
        self.gateway_error
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Dernière erreur d'enregistrement des commandes globales.
    pub fn registration_error(&self) -> Option<String> {
        self.registration_error
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// Nombre de commandes globales enregistrées au dernier `ready`.
    pub fn registered_commands(&self) -> usize {
        self.registered_commands.load(Ordering::Acquire)
    }

    pub(crate) fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub(crate) fn mark_registered(&self, count: usize) {
        self.registered_commands.store(count, Ordering::Release);
        if let Ok(mut guard) = self.registration_error.lock() {
            *guard = None;
        }
    }

    pub(crate) fn set_registration_error(&self, message: impl Into<String>) {
        if let Ok(mut guard) = self.registration_error.lock() {
            *guard = Some(message.into());
        }
    }

    pub(crate) fn set_gateway_error(&self, message: impl Into<String>) {
        self.ready.store(false, Ordering::Release);
        if let Ok(mut guard) = self.gateway_error.lock() {
            *guard = Some(message.into());
        }
    }

    pub(crate) fn mark_stopped(&self) {
        self.ready.store(false, Ordering::Release);
    }
}

/// Sonde `/health` branchée sur l'état réel du gateway.
///
/// Avant le premier `ready` (ou après un arrêt), la sonde est `Down` ; à partir
/// du `ready`, elle est `Ok`.
#[derive(Debug, Clone)]
pub struct DiscordStatusProbe {
    status: Arc<DiscordStatus>,
}

impl DiscordStatusProbe {
    /// Construit la sonde depuis l'état partagé.
    pub fn new(status: Arc<DiscordStatus>) -> Self {
        Self { status }
    }
}

impl HealthProbe for DiscordStatusProbe {
    fn state(&self) -> HealthState {
        if self.status.is_ready() {
            HealthState::Ok
        } else {
            HealthState::Down
        }
    }
}

/// Poignée du runtime Discord rendue au serveur.
///
/// Le serveur la conserve jusqu'à l'arrêt, puis appelle
/// [`DiscordHandle::shutdown`] (voir l'exemple de câblage dans
/// `crate::discord`).
pub struct DiscordHandle {
    status: Arc<DiscordStatus>,
    shard_manager: Arc<ShardManager>,
    shutdown: Shutdown,
    task: JoinHandle<()>,
}

impl DiscordHandle {
    /// État partagé alimenté par le gateway.
    pub fn status(&self) -> Arc<DiscordStatus> {
        Arc::clone(&self.status)
    }

    /// Sonde `/health` à passer à `AppState::with_discord_probe`.
    pub fn probe(&self) -> Arc<dyn HealthProbe> {
        Arc::new(DiscordStatusProbe::new(Arc::clone(&self.status)))
    }

    /// Ferme les shards et attend la sortie de la boucle gateway.
    ///
    /// Déclenche [`crate::Shutdown`] (idempotent) : le serveur appelant est déjà
    /// en train de s'arrêter. L'attente est bornée par [`SHUTDOWN_TIMEOUT`].
    pub async fn shutdown(self) -> Result<(), DiscordError> {
        self.shutdown.trigger();
        self.shard_manager.shutdown_all().await;

        match tokio::time::timeout(SHUTDOWN_TIMEOUT, self.task).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                tracing::error!(%error, "boucle gateway Discord interrompue");
                Err(DiscordError::ShutdownTimeout)
            }
            Err(_) => {
                tracing::error!("gateway Discord toujours actif après le délai d'arrêt");
                Err(DiscordError::ShutdownTimeout)
            }
        }
    }
}

/// Point d'entrée du runtime Discord.
pub struct DiscordRuntime;

/// Horloge partagée — implémente [`Clock`] pour un `Arc<dyn Clock>`.
///
/// `lc_domain` ne fournit pas d'implémentation de `Clock` pour `Arc<dyn Clock>`,
/// et [`DbBotContext`] est paramétré par la **type concret** de l'horloge : ce
/// newtype permet de réutiliser l'horloge de [`AppState`] sans la dupliquer.
#[derive(Clone)]
pub struct SharedClock(Arc<dyn Clock>);

impl SharedClock {
    /// Construit l'horloge partagée.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self(clock)
    }
}

impl std::fmt::Debug for SharedClock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SharedClock")
    }
}

impl Clock for SharedClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        self.0.now()
    }
}

impl DiscordRuntime {
    /// Démarre le bot Discord et rend une poignée de contrôle.
    ///
    /// # Erreurs
    ///
    /// - [`DiscordError::Disabled`] si `config.discord_token` est absent/vide ;
    /// - [`DiscordError::DatabaseUnavailable`] si `state.db` est `None` (les
    ///   commandes écrivent dans `lc-db`) ;
    /// - [`DiscordError::Client`] si la construction du client serenity échoue.
    ///
    /// La connexion gateway, l'enregistrement des commandes et le traitement des
    /// interactions se font ensuite dans la tâche de fond ; leurs erreurs sont
    /// journalisées et stockées dans [`DiscordStatus`].
    pub async fn start(
        config: Arc<Config>,
        state: AppState,
    ) -> Result<DiscordHandle, DiscordError> {
        let token = config
            .discord_token
            .as_ref()
            .filter(|secret| !secret.is_empty())
            .map(|secret| secret.expose_secret().to_owned())
            .ok_or(DiscordError::Disabled)?;

        let db: Db = state.db.clone().ok_or(DiscordError::DatabaseUnavailable)?;

        let status = Arc::new(DiscordStatus::default());
        let bot_config: BotConfig = bot_config_from(&config);
        let adapters = DiscordAdapters::with_clock(db.clone(), Arc::clone(&state.clock));
        let context = build_context(&db, &state, &bot_config, &adapters);

        let handler = DiscordEventHandler::new(db, context, bot_config, Arc::clone(&status));

        let mut client = Client::builder(token, GatewayIntents::GUILDS)
            .event_handler(handler)
            .await
            .map_err(|error| DiscordError::Client(Box::new(error)))?;

        let shard_manager: Arc<ShardManager> = Arc::clone(&client.shard_manager);
        let task_shard_manager = Arc::clone(&shard_manager);
        let shutdown = state.shutdown.clone();
        let signal = shutdown.subscribe();
        let task_status = Arc::clone(&status);

        let task = tokio::spawn(async move {
            tokio::select! {
                result = client.start() => {
                    match result {
                        Ok(()) => {
                            tracing::info!("boucle gateway Discord terminée");
                        }
                        Err(error) => {
                            tracing::error!(%error, "gateway Discord en erreur");
                            task_status.set_gateway_error(error.to_string());
                        }
                    }
                    task_status.mark_stopped();
                }
                _ = signal.wait() => {
                    tracing::info!("arrêt du serveur : fermeture du gateway Discord");
                    task_shard_manager.shutdown_all().await;
                    task_status.mark_stopped();
                }
            }
        });

        Ok(DiscordHandle {
            status,
            shard_manager,
            shutdown,
            task,
        })
    }
}

fn build_context(
    db: &Db,
    state: &AppState,
    bot_config: &BotConfig,
    adapters: &DiscordAdapters,
) -> Arc<DbBotContext<SharedClock>> {
    Arc::new(DbBotContext::with_clock(
        db.clone(),
        SharedClock::new(Arc::clone(&state.clock)),
        bot_config.clone(),
        adapters.media(),
        adapters.playback(),
        adapters.media_board(),
        adapters.overlays(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use chrono::{TimeZone, Utc};
    use lc_domain::FixedClock;

    use crate::shutdown::Shutdown;
    use crate::{AppState, Config};

    use super::{DiscordError, DiscordRuntime, SharedClock};
    use lc_domain::Clock;

    fn config_with(pairs: &[(&str, &str)]) -> Arc<Config> {
        let mut entries = vec![("API_URL".to_owned(), "http://localhost:3333".to_owned())];
        entries.extend(
            pairs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
        );
        Arc::new(Config::from_pairs(entries).unwrap())
    }

    fn state(config: Arc<Config>) -> AppState {
        AppState::new(config, None, Shutdown::detached())
    }

    #[tokio::test]
    async fn sans_token_le_runtime_est_desactive_sans_reseau() {
        let config = config_with(&[]);
        let error = match DiscordRuntime::start(Arc::clone(&config), state(config)).await {
            Ok(_) => panic!("runtime inattendu sans DISCORD_TOKEN"),
            Err(error) => error,
        };
        assert!(matches!(error, DiscordError::Disabled));
        assert!(error.is_disabled());
    }

    #[tokio::test]
    async fn token_present_mais_sans_base_est_une_erreur_explicite() {
        let config = config_with(&[
            ("DISCORD_TOKEN", "jeton-de-test"),
            ("DISCORD_CLIENT_ID", "123456789012345678"),
        ]);
        let error = match DiscordRuntime::start(Arc::clone(&config), state(config)).await {
            Ok(_) => panic!("runtime inattendu sans base SQLite"),
            Err(error) => error,
        };
        assert!(matches!(error, DiscordError::DatabaseUnavailable));
        assert!(!error.is_disabled());
    }

    #[tokio::test]
    #[ignore = "gateway réel : exige DISCORD_TOKEN/DISCORD_CLIENT_ID et un réseau"]
    async fn gateway_reel_configure_par_environnement() {
        let Ok(token) = std::env::var("DISCORD_TOKEN") else {
            return;
        };
        assert!(!token.is_empty(), "DISCORD_TOKEN fourni mais vide");
    }

    #[test]
    fn shared_clock_delegue_a_l_horloge_du_serveur() {
        let at = Utc.with_ymd_and_hms(2024, 1, 2, 3, 4, 5).unwrap();
        let clock = SharedClock::new(Arc::new(FixedClock::new(at)));
        assert_eq!(clock.now(), at);
    }
}
