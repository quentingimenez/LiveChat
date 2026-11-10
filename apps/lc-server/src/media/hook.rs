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

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use lc_domain::entities::PlaybackJob;

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
pub async fn notify_ingested(job: &PlaybackJob) {
    let hook = match HOOK.read() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };

    if let Some(hook) = hook {
        hook.on_ingested(job).await;
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
        let now = chrono::Utc::now();
        PlaybackJob {
            id: PlaybackJobId::new_v4(),
            guild_id: GuildId::parse("123456789012345678").expect("guild id valide"),
            media_asset_id: None,
            text: Some("bonjour".into()),
            show_text: true,
            author_name: None,
            author_image: None,
            duration_sec: 5,
            priority: 0,
            status: JobStatus::Pending,
            resumes_after_job_id: None,
            resume_offset_sec: 0,
            remaining_ms_snapshot: None,
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
    }
}
