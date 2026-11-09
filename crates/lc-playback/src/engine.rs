//! Moteur d'ordonnancement de la lecture.
//!
//! Le moteur ne fait aucune E/S réseau : il persiste les transitions dans
//! SQLite (`lc-db`), consulte l'horloge injectée et remet les payloads à un
//! [`PlaybackSink`]. Il est conçu pour être piloté par le serveur :
//!
//! - [`PlaybackEngine::run`] pour les timers de durée et la boucle de fond ;
//! - [`PlaybackEngine::handle_playback_state`] / `handle_playback_ended` /
//!   `handle_error` / `stop` / `current_job` pour les messages du hub WS ;
//! - [`PlaybackEngine::enqueue`] / `notify` pour les créations de jobs.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lc_db::Db;
use lc_domain::{
    datetime_to_ms, Clock, GuildId, JobStatus, MediaAsset, PlaybackJob, PlaybackJobId,
};
use lc_protocol::{
    CurrentJobPayload, PlaybackEndedReason, PlaybackState, StopPayload, StopReason,
    StopRequestSource, MAX_PLAYBACK_REMAINING_MS,
};
use tokio::sync::{Mutex, Notify};
use tracing::{info, warn};

use crate::config::PlaybackConfig;
use crate::error::PlaybackResult;
use crate::payload;
use crate::sink::PlaybackSink;
use crate::store::PlaybackStore;

/// Événement `playback.state` reçu du hub WS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackStateEvent {
    /// Guilde concernée.
    pub guild_id: GuildId,
    /// Job visé ; `None` = tous les jobs `PLAYING` de la guilde.
    pub job_id: Option<PlaybackJobId>,
    /// État rapporté par l'overlay.
    pub state: PlaybackState,
    /// Temps restant estimé (millisecondes), rampé à 24 h.
    pub remaining_ms: Option<u64>,
}

impl PlaybackStateEvent {
    /// Construit un événement `playback.state`.
    pub fn new(
        guild_id: GuildId,
        job_id: Option<PlaybackJobId>,
        state: PlaybackState,
        remaining_ms: Option<u64>,
    ) -> Self {
        Self {
            guild_id,
            job_id,
            state,
            remaining_ms,
        }
    }
}

/// Événement `playback.ended` reçu du hub WS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackEndedEvent {
    /// Guilde concernée.
    pub guild_id: GuildId,
    /// Job terminé.
    pub job_id: PlaybackJobId,
    /// Raison rapportée par l'overlay.
    pub reason: PlaybackEndedReason,
}

impl PlaybackEndedEvent {
    /// Construit un événement `playback.ended`.
    pub fn new(guild_id: GuildId, job_id: PlaybackJobId, reason: PlaybackEndedReason) -> Self {
        Self {
            guild_id,
            job_id,
            reason,
        }
    }
}

/// Résultat du traitement d'un `playback.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStateOutcome {
    /// Checkpoint persisté.
    Persisted,
    /// Checkpoint ignoré par le débit minimal (1/s par job).
    Throttled,
    /// Aucun job `PLAYING` correspondant.
    Ignored,
}

/// Résultat d'une libération de job (`playback.ended`, `error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseOutcome {
    /// Un job a effectivement été libéré.
    pub released: bool,
    /// Un nouveau job a été dispatché immédiatement après.
    pub next_started: bool,
}

/// Résultat d'une demande d'arrêt.
#[derive(Clone, Debug, PartialEq)]
pub struct StopOutcome {
    /// Identifiants des jobs effectivement arrêtés.
    pub stopped: Vec<String>,
    /// Payload `stop` émis vers la room.
    pub payload: StopPayload,
}

/// Bilan d'un passage de [`PlaybackEngine::tick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickReport {
    /// Nombre de guildes traitées.
    pub processed_guilds: usize,
    /// Nombre de guildes encore actives après le passage.
    pub active_guilds: usize,
}

/// Bilan d'une purge de rétention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PurgeReport {
    /// Jobs terminés supprimés.
    pub playback_jobs: u64,
    /// Codes d'appairage expirés/consommés supprimés.
    pub pairing_codes: u64,
}

/// État interne sérialisé par le verrou du moteur.
#[derive(Debug, Default)]
struct Inner {
    /// `bootstrap` a déjà été exécuté.
    bootstrapped: bool,
    /// Guildes avec travail en cours/en attente.
    active_guilds: BTreeSet<GuildId>,
    /// Dernier job terminé par guilde (pour la reprise après préemption).
    just_finished: BTreeMap<GuildId, PlaybackJobId>,
    /// Dernier checkpoint `playback.state` persisté par job (débit 1/s).
    state_persisted_at: BTreeMap<PlaybackJobId, DateTime<Utc>>,
}

/// Bilan interne d'un passage sur une guilde.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GuildRun {
    /// Au moins un job a été dispatché pendant ce passage.
    started: bool,
    /// La guilde a encore du travail (job actif ou jobs bloqués).
    active: bool,
}

/// Ordonnanceur de lecture testable.
///
/// Voir [`PlaybackEngine::new`] pour la construction et
/// `docs/rust/lc-playback.md` pour la chronologie et les invariants.
pub struct PlaybackEngine<S: PlaybackSink, C: Clock> {
    store: PlaybackStore,
    db: Db,
    config: PlaybackConfig,
    sink: S,
    clock: C,
    inner: Mutex<Inner>,
    shutting_down: AtomicBool,
    shutdown_notify: Notify,
}

impl<S: PlaybackSink, C: Clock> PlaybackEngine<S, C> {
    /// Crée un moteur.
    ///
    /// `db` est cloné pour le [`PlaybackStore`] interne ; `sink` et `clock`
    /// sont injectés (production : hub WS + `SystemClock` ; tests : `FakeSink`
    /// + `FixedClock`).
    pub fn new(db: Db, config: PlaybackConfig, sink: S, clock: C) -> Self {
        Self {
            store: PlaybackStore::new(db.clone()),
            db,
            config,
            sink,
            clock,
            inner: Mutex::new(Inner::default()),
            shutting_down: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
        }
    }

    /// Configuration courante.
    pub fn config(&self) -> &PlaybackConfig {
        &self.config
    }

    /// Sink de sortie injecté.
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// Horloge injectée.
    pub fn clock(&self) -> &C {
        &self.clock
    }

    /// Initialise le moteur : récupère les guildes actives et les traite.
    ///
    /// À appeler une fois au démarrage du serveur (avant [`Self::run`]).
    pub async fn bootstrap(&self) -> PlaybackResult<usize> {
        let guilds = self.store.active_guild_ids().await?;
        self.inner.lock().await.bootstrapped = true;
        for guild_id in &guilds {
            self.process_guild(guild_id).await?;
        }
        Ok(guilds.len())
    }

    /// Un passage de la boucle : traite toutes les guildes ayant du travail.
    pub async fn tick(&self) -> PlaybackResult<TickReport> {
        let bootstrapped = self.inner.lock().await.bootstrapped;
        if !bootstrapped {
            self.bootstrap().await?;
        }
        let guilds = self.store.active_guild_ids().await?;
        {
            let mut inner = self.inner.lock().await;
            for guild_id in &guilds {
                inner.active_guilds.insert(guild_id.clone());
            }
        }
        let mut processed_guilds = 0;
        for guild_id in &guilds {
            self.process_guild(guild_id).await?;
            processed_guilds += 1;
        }
        let active_guilds = self.inner.lock().await.active_guilds.len();
        Ok(TickReport {
            processed_guilds,
            active_guilds,
        })
    }

    /// Crée un job puis tente immédiatement de le dispatcher.
    ///
    /// Utilisé par les workers (Discord, admin, mèmes) après construction du
    /// `PlaybackJob` ; le job peut préempter la lecture en cours s'il est de
    /// priorité supérieure.
    pub async fn enqueue(&self, job: &PlaybackJob) -> PlaybackResult<()> {
        self.db.playback_jobs().create(job).await?;
        self.process_guild(&job.guild_id).await?;
        Ok(())
    }

    /// Re-traite une guilde après une modification externe de ses jobs.
    pub async fn notify(&self, guild_id: &GuildId) -> PlaybackResult<()> {
        self.process_guild(guild_id).await?;
        Ok(())
    }

    /// Traite un checkpoint `playback.state` (message client).
    ///
    /// Met à jour le verrou `busy_until` de la guilde
    /// (`max(remaining_ms, 5 s) + 250 ms`) et persiste `remaining_ms` au plus
    /// une fois par seconde et par job.
    pub async fn handle_playback_state(
        &self,
        event: PlaybackStateEvent,
    ) -> PlaybackResult<PlaybackStateOutcome> {
        let now = self.clock.now();
        let remaining_ms = event
            .remaining_ms
            .map(|remaining| remaining.min(MAX_PLAYBACK_REMAINING_MS));
        let busy_for_ms = remaining_ms
            .and_then(|remaining| i64::try_from(remaining).ok())
            .unwrap_or(0)
            .max(self.config.min_busy_lock_ms)
            + self.config.lock_padding_ms;
        let busy_until = now + ChronoDuration::milliseconds(busy_for_ms);
        self.db
            .guilds()
            .set_busy_until(&event.guild_id, Some(busy_until))
            .await?;

        let job_ids: Vec<PlaybackJobId> = match event.job_id {
            Some(job_id) => vec![job_id],
            None => self
                .store
                .playing_jobs(&event.guild_id)
                .await?
                .into_iter()
                .map(|job| job.id)
                .collect(),
        };
        let min_interval = ChronoDuration::from_std(self.config.state_persist_min_interval)
            .unwrap_or_else(|_| ChronoDuration::milliseconds(0));
        let mut outcome = PlaybackStateOutcome::Ignored;
        for job_id in job_ids {
            let last_persisted = self
                .inner
                .lock()
                .await
                .state_persisted_at
                .get(&job_id)
                .copied();
            if let Some(last_persisted) = last_persisted {
                if now - last_persisted < min_interval {
                    outcome = PlaybackStateOutcome::Throttled;
                    continue;
                }
            }
            let Some(job) = self.db.playback_jobs().get(job_id).await? else {
                continue;
            };
            if job.status != JobStatus::Playing {
                continue;
            }
            let remaining_snapshot =
                remaining_ms.and_then(|remaining| i64::try_from(remaining).ok());
            self.db
                .playback_jobs()
                .set_playback_state(job.id, job.resume_offset_sec, remaining_snapshot, now)
                .await?;
            self.inner
                .lock()
                .await
                .state_persisted_at
                .insert(job.id, now);
            outcome = PlaybackStateOutcome::Persisted;
        }
        Ok(outcome)
    }

    /// Traite un `playback.ended` : le job visé est terminé (`DONE`) et libéré.
    ///
    /// Idempotent : si le job n'est plus `PLAYING`, aucun changement n'est
    /// effectué (pas d'ack ni d'erreur, cf. protocol-v2 §5.4).
    pub async fn handle_playback_ended(
        &self,
        event: PlaybackEndedEvent,
    ) -> PlaybackResult<ReleaseOutcome> {
        let now = self.clock.now();
        let Some(job) = self.db.playback_jobs().get(event.job_id).await? else {
            return Ok(ReleaseOutcome {
                released: false,
                next_started: false,
            });
        };
        if job.status != JobStatus::Playing {
            return Ok(ReleaseOutcome {
                released: false,
                next_started: false,
            });
        }
        self.db
            .playback_jobs()
            .finish(event.job_id, JobStatus::Done, now)
            .await?;
        self.clear_busy(&event.guild_id).await?;
        self.inner
            .lock()
            .await
            .just_finished
            .insert(event.guild_id.clone(), event.job_id);
        let run = self.process_guild(&event.guild_id).await?;
        Ok(ReleaseOutcome {
            released: true,
            next_started: run.started,
        })
    }

    /// Traite une erreur de lecture rapportée par un overlay.
    ///
    /// Le job visé est marqué `FAILED` (s'il était `PLAYING` ou `PENDING`), le
    /// verrou de guilde est libéré et la suite de la file est traitée.
    pub async fn handle_error(
        &self,
        guild_id: &GuildId,
        job_id: PlaybackJobId,
    ) -> PlaybackResult<ReleaseOutcome> {
        let now = self.clock.now();
        let Some(job) = self.db.playback_jobs().get(job_id).await? else {
            return Ok(ReleaseOutcome {
                released: false,
                next_started: false,
            });
        };
        match job.status {
            JobStatus::Playing => {
                self.db
                    .playback_jobs()
                    .finish(job_id, JobStatus::Failed, now)
                    .await?;
                self.clear_busy(guild_id).await?;
            }
            JobStatus::Pending => {
                self.db
                    .playback_jobs()
                    .set_status(job_id, JobStatus::Failed)
                    .await?;
            }
            JobStatus::Done | JobStatus::Failed => {
                return Ok(ReleaseOutcome {
                    released: false,
                    next_started: false,
                });
            }
        }
        let run = self.process_guild(guild_id).await?;
        Ok(ReleaseOutcome {
            released: true,
            next_started: run.started,
        })
    }

    /// Traite une demande d'arrêt utilisateur (`stop.request`, source
    /// informative). Raison émise : [`StopReason::Manual`].
    pub async fn stop(
        &self,
        guild_id: &GuildId,
        job_id: Option<PlaybackJobId>,
        source: StopRequestSource,
    ) -> PlaybackResult<StopOutcome> {
        // `source` est purement informatif côté protocole (cf. §5.6) : le
        // serveur peut le journaliser, le moteur n'en dépend pas.
        let _ = source;
        self.stop_with_reason(guild_id, job_id, StopReason::Manual)
            .await
    }

    /// Arrête des jobs avec une raison explicite (`manual`, `preempt`,
    /// `revoked`, `purge`), émet le `stop` et traite la file.
    pub async fn stop_with_reason(
        &self,
        guild_id: &GuildId,
        job_id: Option<PlaybackJobId>,
        reason: StopReason,
    ) -> PlaybackResult<StopOutcome> {
        let now = self.clock.now();
        let released = self.store.release_playing(guild_id, job_id, now).await?;
        self.clear_busy(guild_id).await?;
        {
            let mut inner = self.inner.lock().await;
            for released_id in &released {
                inner.state_persisted_at.remove(released_id);
            }
        }
        let stopped: Vec<String> = released.iter().map(|id| id.to_canonical()).collect();
        let payload = StopPayload {
            reason,
            job_id: stop_payload_job_id(job_id, &released),
        };
        self.sink.on_stop(guild_id, payload.clone()).await;
        self.process_guild(guild_id).await?;
        Ok(StopOutcome { stopped, payload })
    }

    /// Payload `welcome.currentJob` : job `PLAYING` le plus récent de la
    /// guilde, avec l'offset de reprise et le temps restant estimé.
    ///
    /// Si plusieurs jobs `PLAYING` sont détectés (anomalie, l'invariant est
    /// garanti par le moteur), le plus récent par `started_at` est retenu et
    /// un avertissement est journalisé (décision Lead §15.3).
    pub async fn current_job(
        &self,
        guild_id: &GuildId,
    ) -> PlaybackResult<Option<CurrentJobPayload>> {
        let now = self.clock.now();
        let mut playing = self.store.playing_jobs(guild_id).await?;
        let Some(job) = playing.pop() else {
            return Ok(None);
        };
        if !playing.is_empty() {
            warn!(
                guild = %guild_id,
                count = playing.len() + 1,
                "plusieurs jobs PLAYING dans la même guilde (anomalie) : le plus récent est exposé"
            );
        }
        let media = self.load_media(&job).await?;
        let timing = payload::persisted_timing(&job);
        let play = payload::build_play_payload(&job, media.as_ref(), timing, &self.config);
        let started_at_ms = job
            .started_at
            .map(datetime_to_ms)
            .unwrap_or_else(|| datetime_to_ms(now));
        let remaining_ms = self.estimated_remaining_ms(&job, now).max(0);
        Ok(Some(CurrentJobPayload {
            play,
            started_at: started_at_ms,
            remaining_ms: u64::try_from(remaining_ms).unwrap_or_default(),
        }))
    }

    /// Purge la rétention : jobs terminés plus vieux que `retention` et codes
    /// d'appairage expirés/consommés.
    pub async fn purge(&self, now: DateTime<Utc>) -> PlaybackResult<PurgeReport> {
        let cutoff = now - self.config.retention;
        let playback_jobs = self.db.playback_jobs().purge_done_before(cutoff).await?;
        let pairing_codes = self.db.pairing_codes().delete_expired(now).await?;
        Ok(PurgeReport {
            playback_jobs,
            pairing_codes,
        })
    }

    /// Boucle de fond : `bootstrap`, premier `tick`, puis un `tick` par période
    /// jusqu'à résolution de `shutdown` ou appel à [`Self::request_shutdown`].
    ///
    /// Les erreurs de persistance interrompent la boucle et remontent à
    /// l'appelant (le serveur décide de la politique de redémarrage).
    pub async fn run<F>(&self, shutdown: F) -> PlaybackResult<()>
    where
        F: Future<Output = ()> + Send,
    {
        self.bootstrap().await?;
        tokio::pin!(shutdown);
        self.tick().await?;
        loop {
            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {
                _ = &mut shutdown => break,
                () = tokio::time::sleep(self.config.tick_interval) => {
                    self.tick().await?;
                }
            }
        }
        info!("ordonnanceur de lecture arrêté");
        Ok(())
    }

    /// Variante de [`Self::run`] pilotée par [`Self::request_shutdown`].
    pub async fn run_until_shutdown(&self) -> PlaybackResult<()> {
        self.bootstrap().await?;
        self.tick().await?;
        while !self.shutting_down.load(Ordering::SeqCst) {
            tokio::select! {
                () = tokio::time::sleep(self.config.tick_interval) => {
                    self.tick().await?;
                }
                () = self.shutdown_notify.notified() => break,
            }
        }
        info!("ordonnanceur de lecture arrêté");
        Ok(())
    }

    /// Demande l'arrêt de la boucle (idempotent).
    pub fn request_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        self.shutdown_notify.notify_waiters();
    }

    /// Indique si un arrêt a été demandé.
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    // ------------------------------------------------------------------
    // Internes
    // ------------------------------------------------------------------

    /// Traite une guilde sous verrou et met à jour l'ensemble des guildes
    /// actives.
    async fn process_guild(&self, guild_id: &GuildId) -> PlaybackResult<GuildRun> {
        let mut inner = self.inner.lock().await;
        let run = self.run_guild(&mut inner, guild_id).await?;
        if run.active {
            inner.active_guilds.insert(guild_id.clone());
        } else {
            inner.active_guilds.remove(guild_id);
        }
        Ok(run)
    }

    /// Boucle d'une guilde (parité `runGuild` legacy, bornée).
    async fn run_guild(&self, inner: &mut Inner, guild_id: &GuildId) -> PlaybackResult<GuildRun> {
        let mut preferred = inner.just_finished.remove(guild_id);
        let mut started = false;
        for _ in 0..self.config.max_guild_iterations {
            let now = self.clock.now();
            let playing = self.store.playing_jobs(guild_id).await?;
            if let Some(active) = playing.first().cloned() {
                self.handle_multi_playing(guild_id, &playing, now).await?;
                let expected_end_ms = active
                    .started_at
                    .map(datetime_to_ms)
                    .unwrap_or_else(|| datetime_to_ms(now))
                    + active.duration_sec.max(1) * 1_000
                    + self.config.lock_padding_ms;
                if datetime_to_ms(now) >= expected_end_ms + self.config.stale_release_grace_ms {
                    warn!(
                        guild = %guild_id,
                        job = %active.id,
                        "job PLAYING au-delà de sa durée attendue : libération (watchdog)"
                    );
                    self.db
                        .playback_jobs()
                        .finish(active.id, JobStatus::Done, now)
                        .await?;
                    self.clear_busy(guild_id).await?;
                    preferred = Some(active.id);
                    continue;
                }
                if let Some(preemptor) = self.find_preemption(guild_id, &active, now).await? {
                    self.suspend_for_preemption(guild_id, &active, &preemptor, now)
                        .await?;
                    preferred = Some(preemptor.id);
                    continue;
                }
                return Ok(GuildRun {
                    started,
                    active: true,
                });
            }

            self.clear_stale_busy(guild_id, now).await?;

            let Some(job) = self
                .claim_preferred(guild_id, preferred.take(), now)
                .await?
            else {
                return Ok(GuildRun {
                    started,
                    active: false,
                });
            };
            let media = self.load_media(&job).await?;
            if job.media_asset_id.is_some() && !media.as_ref().is_some_and(|asset| asset.is_ready())
            {
                warn!(
                    guild = %guild_id,
                    job = %job.id,
                    "média absent ou non prêt : job marqué FAILED"
                );
                self.db
                    .playback_jobs()
                    .finish(job.id, JobStatus::Failed, now)
                    .await?;
                preferred = None;
                continue;
            }
            let guild = self.db.guilds().get(guild_id).await?;
            let timing =
                payload::dispatch_timing(&job, media.as_ref(), guild.as_ref(), &self.config);
            let play = payload::build_play_payload(&job, media.as_ref(), timing, &self.config);
            self.store
                .mark_dispatched(
                    job.id,
                    timing.play_duration_sec,
                    timing.start_offset_sec,
                    now,
                )
                .await?;
            let busy_until = now
                + ChronoDuration::milliseconds(
                    timing.play_duration_sec * 1_000 + self.config.lock_padding_ms,
                );
            self.db
                .guilds()
                .set_busy_until(guild_id, Some(busy_until))
                .await?;
            let recipients = self.sink.on_play(guild_id, play).await;
            if recipients == 0 {
                warn!(
                    guild = %guild_id,
                    job = %job.id,
                    "aucun overlay connecté : job marqué FAILED"
                );
                self.db
                    .playback_jobs()
                    .finish(job.id, JobStatus::Failed, now)
                    .await?;
                self.clear_busy(guild_id).await?;
                preferred = None;
                continue;
            }
            started = true;
            return Ok(GuildRun {
                started,
                active: true,
            });
        }
        warn!(
            guild = %guild_id,
            max_iterations = self.config.max_guild_iterations,
            "boucle d'ordonnancement interrompue après le nombre maximal d'itérations"
        );
        Ok(GuildRun {
            started,
            active: true,
        })
    }

    /// Libère les jobs `PLAYING` excédentaires (anomalie d'invariant).
    async fn handle_multi_playing(
        &self,
        guild_id: &GuildId,
        playing: &[PlaybackJob],
        now: DateTime<Utc>,
    ) -> PlaybackResult<()> {
        if playing.len() <= 1 {
            return Ok(());
        }
        warn!(
            guild = %guild_id,
            count = playing.len(),
            "plusieurs jobs PLAYING dans la même guilde : libération des plus anciens"
        );
        for extra in playing.iter().skip(1) {
            let _ = self
                .db
                .playback_jobs()
                .finish(extra.id, JobStatus::Done, now)
                .await?;
        }
        Ok(())
    }

    /// Suspend le job actif pour un job plus prioritaire et émet le `stop`.
    async fn suspend_for_preemption(
        &self,
        guild_id: &GuildId,
        active: &PlaybackJob,
        preemptor: &PlaybackJob,
        now: DateTime<Utc>,
    ) -> PlaybackResult<()> {
        let timing = payload::persisted_timing(active);
        let remaining_ms = self.estimated_remaining_ms(active, now).max(0);
        let remaining_sec = (remaining_ms + 999) / 1_000;
        let elapsed_sec = (timing.play_duration_sec - remaining_sec).max(0);
        let next_offset_sec = timing.start_offset_sec.saturating_add(elapsed_sec).max(0);
        self.store
            .suspend_playing(
                active.id,
                guild_id,
                Some(preemptor.id),
                next_offset_sec,
                remaining_ms,
                now,
            )
            .await?;
        self.clear_busy(guild_id).await?;
        self.sink
            .on_stop(
                guild_id,
                StopPayload {
                    reason: StopReason::Preempt,
                    job_id: Some(active.id.to_canonical()),
                },
            )
            .await;
        Ok(())
    }

    /// Cherche un job `PENDING` de priorité strictement supérieure au job
    /// actif (préemption mème).
    async fn find_preemption(
        &self,
        guild_id: &GuildId,
        active: &PlaybackJob,
        now: DateTime<Utc>,
    ) -> PlaybackResult<Option<PlaybackJob>> {
        let pending = self
            .db
            .playback_jobs()
            .list_pending_by_guild(guild_id)
            .await?;
        Ok(pending.into_iter().find(|job| {
            job.priority > active.priority
                && job.scheduled_at <= now
                && job.is_claimable_at(now)
                && job.resumes_after_job_id != Some(active.id)
        }))
    }

    /// Claim : job préféré (reprise après préemption), puis job de reprise
    /// suspendu après lui, puis prochain job selon l'ordre de priorité.
    async fn claim_preferred(
        &self,
        guild_id: &GuildId,
        preferred: Option<PlaybackJobId>,
        now: DateTime<Utc>,
    ) -> PlaybackResult<Option<PlaybackJob>> {
        if let Some(preferred_id) = preferred {
            if let Some(job) = self
                .store
                .claim_specific(preferred_id, guild_id, now)
                .await?
            {
                return Ok(Some(job));
            }
            let pending = self
                .db
                .playback_jobs()
                .list_pending_by_guild(guild_id)
                .await?;
            if let Some(resume) = pending
                .into_iter()
                .find(|job| job.resumes_after_job_id == Some(preferred_id))
            {
                if let Some(job) = self.store.claim_specific(resume.id, guild_id, now).await? {
                    return Ok(Some(job));
                }
            }
        }
        Ok(self.store.claim_next_for_guild(guild_id, now).await?)
    }

    /// Charge le média associé à un job, si présent.
    async fn load_media(&self, job: &PlaybackJob) -> PlaybackResult<Option<MediaAsset>> {
        match job.media_asset_id {
            Some(media_asset_id) => Ok(self.db.media_assets().get(media_asset_id).await?),
            None => Ok(None),
        }
    }

    /// Estime le temps restant d'un job (`playback.state` frais, sinon horloge).
    fn estimated_remaining_ms(&self, job: &PlaybackJob, now: DateTime<Utc>) -> i64 {
        let now_ms = datetime_to_ms(now);
        if let (Some(snapshot), Some(at)) = (job.remaining_ms_snapshot, job.last_playback_state_at)
        {
            let age_ms = (now_ms - datetime_to_ms(at)).max(0);
            if age_ms <= self.config.state_snapshot_max_age_ms {
                return snapshot - age_ms;
            }
        }
        let started_at_ms = job.started_at.map(datetime_to_ms).unwrap_or(now_ms);
        started_at_ms + job.duration_sec.max(1) * 1_000 - now_ms
    }

    /// Libère inconditionnellement le verrou de guilde (le moteur le possède).
    async fn clear_busy(&self, guild_id: &GuildId) -> PlaybackResult<()> {
        self.db.guilds().set_busy_until(guild_id, None).await?;
        Ok(())
    }

    /// Purge un verrou `busy_until` déjà expiré, sans toucher aux verrous
    /// futurs posés par un autre composant ou entre deux jobs.
    async fn clear_stale_busy(&self, guild_id: &GuildId, now: DateTime<Utc>) -> PlaybackResult<()> {
        let Some(guild) = self.db.guilds().get(guild_id).await? else {
            return Ok(());
        };
        let Some(busy_until) = guild.busy_until else {
            return Ok(());
        };
        if busy_until <= now {
            self.db.guilds().set_busy_until(guild_id, None).await?;
        }
        Ok(())
    }
}

/// Détermine le `jobId` du payload `stop` : ciblé s'il est explicite, sinon
/// l'unique job libéré (les arrêts globaux multiples sont émis avec `null`).
fn stop_payload_job_id(
    target: Option<PlaybackJobId>,
    released: &[PlaybackJobId],
) -> Option<String> {
    match target {
        Some(job_id) => Some(job_id.to_canonical()),
        None => match released {
            [single] => Some(single.to_canonical()),
            _ => None,
        },
    }
}
