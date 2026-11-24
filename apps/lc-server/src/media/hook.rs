//! Point d'extension `on_ingested` du service média.
//!
//! Le legacy notifiait le scheduler dès qu'un job était créé
//! (`notifyPlaybackSchedulerJobEnqueued`). En Rust, la lecture appartient à
//! `lc-playback` (Phase 4b) et le hub WebSocket (Phase 4) : le serveur ne peut
//! donc pas les référencer directement depuis ce module.
//!
//! Le contrat est donc un *hook* installé au démarrage :
//!
//! ```text
//! install_ingest_hook(Arc::new(MyPlaybackNotifier))  // Phase 4b, dans run()
//! ...
//! notify_ingested(&job).await                        // service média après INSERT
//! ```
//!
//! L'implémentation Phase 4b appellera `PlaybackEngine::notify(&job.guild_id)` :
//! **pas** `enqueue`, qui insérerait une seconde fois le job en base (le service
//! média l'a déjà persisté via `playback_jobs().create`).
//!
//! Sans hook installé (Phase 3c seule), le job reste en file `PENDING` et sera
//! joué au prochain `bootstrap`/`tick` du moteur : aucune perte de donnée.
//!
//! ## Borne de la notification
//!
//! `notify_ingested` est attendu par la requête `/ingest` **avant** sa réponse
//! `201`. Un moteur de lecture bloqué (verrou interne, sink muet) laissait donc
//! le client sans réponse alors que le job était déjà persisté. L'appel est
//! borné par [`NOTIFY_TIMEOUT`] : à l'échéance, la requête répond et le job
//! reste en file `PENDING`, joué au prochain tick — rien n'est perdu.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use lc_domain::entities::PlaybackJob;

/// Budget accordé au hook de lecture pour réagir à un job créé.
pub const NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// Réaction du serveur à la création d'un job issu d'une ingestion.
pub trait IngestHook: Send + Sync {
    /// Appelé après la persistance du job ; ne doit pas paniquer.
    fn on_ingested<'a>(
        &'a self,
        job: &'a PlaybackJob,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

static HOOK: RwLock<Option<Arc<dyn IngestHook>>> = RwLock::new(None);

/// Installe (ou remplace) le hook de lecture.
///
/// Idempotent : un second appel remplace le précédent, ce qui facilite les
/// tests et une future reconfiguration.
pub fn install_ingest_hook(hook: Arc<dyn IngestHook>) {
    match HOOK.write() {
        Ok(mut slot) => *slot = Some(hook),
        // Un thread ayant paniqué en tenant le verrou ne doit pas empêcher la
        // suite ; on récupère le garde empoisonné sans paniquer.
        Err(poisoned) => *poisoned.into_inner() = Some(hook),
    }
}

/// Notifie le hook installé du job créé, s'il existe.
///
/// Borné par [`NOTIFY_TIMEOUT`] : un hook muet ne doit pas retenir la réponse
/// d'ingestion.
pub async fn notify_ingested(job: &PlaybackJob) {
    notify_within(job, NOTIFY_TIMEOUT).await;
}

/// Variante de [`notify_ingested`] avec un budget explicite (tests).
///
/// Le budget est une durée d'observation, pas une garantie que le hook a
/// terminé : à l'échéance, son futur est annulé et le job reste en file
/// `PENDING`.
async fn notify_within(job: &PlaybackJob, budget: Duration) {
    let hook = match HOOK.read() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };

    let Some(hook) = hook else {
        return;
    };

    if tokio::time::timeout(budget, hook.on_ingested(job))
        .await
        .is_err()
    {
        tracing::warn!(
            guild_id = %job.guild_id.as_str(),
            job_id = %job.id.to_canonical(),
            timeout_ms = budget.as_millis(),
            "hook d'ingestion sans réponse : le job reste en file et sera joué au prochain tick"
        );
    }
}

/// Retire le hook (utilisé par les tests pour ne pas fuiter entre cas).
#[cfg(test)]
pub(crate) fn clear_ingest_hook() {
    match HOOK.write() {
        Ok(mut slot) => *slot = None,
        Err(poisoned) => *poisoned.into_inner() = None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use lc_domain::enums::JobStatus;
    use lc_domain::ids::{GuildId, PlaybackJobId};

    use super::*;

    struct RecordingHook {
        calls: AtomicUsize,
        guilds: Mutex<Vec<String>>,
    }

    impl RecordingHook {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                guilds: Mutex::new(Vec::new()),
            }
        }
    }

    impl IngestHook for RecordingHook {
        fn on_ingested<'a>(
            &'a self,
            job: &'a PlaybackJob,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.guilds
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(job.guild_id.as_str().to_owned());
            Box::pin(async {})
        }
    }

    fn job() -> PlaybackJob {
        let now = time::UtcDateTime::now();
        PlaybackJob {
            id: PlaybackJobId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guild id valide"),
            media_asset_id: None,
            text: Some("bonjour".into()),
            show_text: true,
            author_name: None,
            author_image: None,
            duration_sec: 5,
            source: lc_domain::enums::JobSource::Board,
            status: JobStatus::Pending,
            resume_offset_sec: 0,
            last_playback_state_at: None,
            submission_date: now,
            execution_date: now,
            scheduled_at: now,
            started_at: None,
            finished_at: None,
        }
    }

    // Un seul test pour éviter que les tests parallèles se disputent le hook
    // global.
    #[tokio::test]
    async fn notify_calls_installed_hook() {
        clear_ingest_hook();
        // Sans hook : no-op silencieux.
        notify_ingested(&job()).await;

        let hook = Arc::new(RecordingHook::new());
        install_ingest_hook(hook.clone());

        let created = job();
        notify_ingested(&created).await;
        notify_ingested(&created).await;

        assert_eq!(hook.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            hook.guilds
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            ["123456789012345678", "123456789012345678"]
        );

        // Remplacement : le remplaçant reçoit les appels suivants.
        let replacement = Arc::new(RecordingHook::new());
        install_ingest_hook(replacement.clone());
        notify_ingested(&created).await;

        assert_eq!(replacement.calls.load(Ordering::SeqCst), 1);
        assert_eq!(hook.calls.load(Ordering::SeqCst), 2);

        clear_ingest_hook();
        notify_ingested(&created).await;

        // Un hook qui ne rend pas la main est coupé par le budget : la
        // requête d'ingestion ne doit pas attendre un moteur bloqué.
        struct StuckHook;

        impl IngestHook for StuckHook {
            fn on_ingested<'a>(
                &'a self,
                _job: &'a PlaybackJob,
            ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
                Box::pin(std::future::pending::<()>())
            }
        }

        install_ingest_hook(Arc::new(StuckHook));
        let started = std::time::Instant::now();
        notify_within(&created, Duration::from_millis(20)).await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "la notification doit être coupée (mesuré {:?})",
            started.elapsed()
        );
        clear_ingest_hook();
    }
}
