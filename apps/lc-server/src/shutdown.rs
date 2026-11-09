//! Signal d'arrêt partagé entre le serveur HTTP, les workers et les tâches de fond.
//!
//! ## Pourquoi un `watch` ?
//!
//! L'arrêt gracieux doit être observable par plusieurs composants à la fois :
//! la boucle `axum::serve`, les (futurs) workers et la fermeture du pool SQLite.
//! Un [`tokio::sync::watch`] fournit exactement ce contrat : un état booléen
//! « arrêt demandé » que tous les abonnés peuvent attendre, sans course ni
//! consommation unique.
//!
//! - [`Shutdown`] est clonable et peut déclencher l'arrêt ([`Shutdown::trigger`]).
//! - [`ShutdownSignal`] est un abonné : [`ShutdownSignal::wait`] se résout dès
//!   que l'arrêt est demandé (ou si le déclencheur a disparu, pour ne jamais
//!   bloquer un drain).
//!
//! L'installation des handlers de signaux vit dans [`install_signal_handlers`] :
//! `Ctrl+C` partout, plus `SIGTERM` sur Unix. Aucun endpoint HTTP de *shutdown*
//! n'est exposé (recommandation du POC serveur : pas de surface de debug/arrêt).

use std::time::Duration;

use tokio::sync::watch;

/// État partagé « arrêt demandé ».
#[derive(Debug, Clone)]
pub struct Shutdown {
    sender: watch::Sender<bool>,
}

/// Abonné à l'état d'arrêt.
#[derive(Debug, Clone)]
pub struct ShutdownSignal {
    receiver: watch::Receiver<bool>,
}

impl Shutdown {
    /// Crée l'état d'arrêt et son premier abonné.
    pub fn new() -> (Self, ShutdownSignal) {
        let (sender, receiver) = watch::channel(false);
        (Self { sender }, ShutdownSignal { receiver })
    }

    /// Crée un état d'arrêt sans conserver le premier abonné.
    pub fn detached() -> Self {
        Self::new().0
    }

    /// Demande l'arrêt. Idempotent : un second appel ne change rien.
    pub fn trigger(&self) {
        self.sender.send_replace(true);
    }

    /// `true` si l'arrêt a déjà été demandé.
    pub fn is_triggered(&self) -> bool {
        *self.sender.borrow()
    }

    /// Nouvel abonné à l'état d'arrêt.
    pub fn subscribe(&self) -> ShutdownSignal {
        ShutdownSignal {
            receiver: self.sender.subscribe(),
        }
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::detached()
    }
}

impl ShutdownSignal {
    /// `true` si l'arrêt a déjà été demandé.
    pub fn is_triggered(&self) -> bool {
        *self.receiver.borrow()
    }

    /// Attend que l'arrêt soit demandé.
    ///
    /// Se résout aussi si le [`Shutdown`] a été détruit : mieux vaut un drain
    /// immédiat qu'un serveur impossible à arrêter.
    pub async fn wait(mut self) {
        loop {
            if *self.receiver.borrow_and_update() {
                return;
            }
            if self.receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Démarre la tâche d'écoute des signaux d'arrêt du processus.
///
/// La tâche tourne jusqu'à réception de `Ctrl+C` (toutes plateformes) ou
/// `SIGTERM` (Unix), puis déclenche `shutdown`.
pub fn install_signal_handlers(shutdown: Shutdown) {
    tokio::spawn(async move {
        wait_for_stop_signal().await;
        tracing::info!("signal d'arrêt reçu ; arrêt gracieux demandé");
        shutdown.trigger();
    });
}

#[cfg(unix)]
async fn wait_for_stop_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(sigterm) => sigterm,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "écoute de SIGTERM indisponible ; Ctrl+C reste actif"
            );
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::warn!(error = %error, "écoute de Ctrl+C indisponible");
            }
            return;
        }
    };

    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::warn!(error = %error, "erreur pendant l'écoute de Ctrl+C");
            }
        }
        _ = sigterm.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_stop_signal() {
    // Windows : `Ctrl+C` (console) est le seul signal portable pour un binaire.
    // L'arrêt de service (SIGTERM-like) sera traité avec l'installateur Windows
    // en Phase 7 ; d'ici là, le manager peut fermer le processus.
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(error = %error, "écoute de Ctrl+C indisponible");
    }
}

/// Délai maximal accordé au drainage des connexions après un signal d'arrêt.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn new_shutdown_is_not_triggered() {
        let (shutdown, signal) = Shutdown::new();
        assert!(!shutdown.is_triggered());
        assert!(!signal.is_triggered());
    }

    #[tokio::test]
    async fn trigger_wakes_subscribers() {
        let (shutdown, signal) = Shutdown::new();
        let waiter = tokio::spawn(async move { signal.wait().await });

        shutdown.trigger();

        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("le signal doit se résoudre après trigger")
            .expect("la tâche d'attente ne doit pas paniquer");
        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn double_trigger_is_idempotent_and_wakes_everyone() {
        let (shutdown, first) = Shutdown::new();
        let second = shutdown.subscribe();

        shutdown.trigger();
        shutdown.trigger();

        for signal in [first, second] {
            tokio::time::timeout(Duration::from_secs(1), signal.wait())
                .await
                .expect("chaque abonné doit être réveillé");
        }
        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn late_subscriber_sees_already_triggered_state() {
        let (shutdown, _) = Shutdown::new();
        shutdown.trigger();
        let signal = shutdown.subscribe();
        assert!(signal.is_triggered());
        tokio::time::timeout(Duration::from_secs(1), signal.wait())
            .await
            .expect("un abonné tardif ne doit pas bloquer");
    }

    #[tokio::test]
    async fn dropped_trigger_does_not_block_waiters() {
        let (shutdown, signal) = Shutdown::new();
        drop(shutdown);
        tokio::time::timeout(Duration::from_secs(1), signal.wait())
            .await
            .expect("la disparition du déclencheur doit libérer l'attente");
    }
}
