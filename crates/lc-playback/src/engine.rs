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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration as StdDuration;

use lc_db::Db;
use lc_domain::{
    datetime_to_ms, Clock, GuildId, JobStatus, MediaAsset, PlaybackJob, PlaybackJobId,
};
use lc_protocol::{
    CurrentJobPayload, PlaybackEndedReason, PlaybackState, StopPayload, StopReason,
    StopRequestSource, MAX_PLAYBACK_REMAINING_MS,
};
use time::{Duration as TimeDuration, UtcDateTime};
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

/// État du superviseur de la boucle de lecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackEngineState {
    /// Boucle créée, premier `bootstrap`/`tick` pas encore réussi.
    Starting,
    /// Dernier passage réussi.
    Running,
    /// Dernier passage en erreur ; la boucle retente avec backoff.
    Degraded,
    /// Boucle arrêtée (arrêt demandé ou shutdown).
    Stopped,
}

impl PlaybackEngineState {
    const fn as_u8(self) -> u8 {
        match self {
            Self::Starting => 0,
            Self::Running => 1,
            Self::Degraded => 2,
            Self::Stopped => 3,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Running,
            2 => Self::Degraded,
            3 => Self::Stopped,
            _ => Self::Starting,
        }
    }
}

/// Vue publique de la santé du moteur, exposée à `/health`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackHealth {
    /// État courant.
    pub state: PlaybackEngineState,
    /// Nombre d'échecs consécutifs depuis le dernier passage réussi.
    pub consecutive_failures: u32,
    /// Dernière erreur, tronquée pour rester exploitable en logs.
    pub last_error: Option<String>,
}

/// Compteur de santé partagé entre la boucle et `GET /health`.
#[derive(Debug)]
pub struct EngineHealth {
    state: AtomicU8,
    consecutive_failures: AtomicU32,
    last_error: RwLock<Option<String>>,
}

impl EngineHealth {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(PlaybackEngineState::Starting.as_u8()),
            consecutive_failures: AtomicU32::new(0),
            last_error: RwLock::new(None),
        }
    }

    fn mark_starting(&self) {
        self.state
            .store(PlaybackEngineState::Starting.as_u8(), Ordering::SeqCst);
    }

    fn mark_running(&self) {
        self.state
            .store(PlaybackEngineState::Running.as_u8(), Ordering::SeqCst);
        self.consecutive_failures.store(0, Ordering::SeqCst);
        if let Ok(mut error) = self.last_error.write() {
            *error = None;
        }
    }

    fn mark_failure(&self, error: &str) {
        self.state
            .store(PlaybackEngineState::Degraded.as_u8(), Ordering::SeqCst);
        self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut last) = self.last_error.write() {
            *last = Some(truncate_health_error(error));
        }
    }

    fn mark_stopped(&self) {
        self.state
            .store(PlaybackEngineState::Stopped.as_u8(), Ordering::SeqCst);
    }

    /// Instantané cohérent pour une réponse HTTP ou un log.
    #[must_use]
    pub fn snapshot(&self) -> PlaybackHealth {
        PlaybackHealth {
            state: PlaybackEngineState::from_u8(self.state.load(Ordering::SeqCst)),
            consecutive_failures: self.consecutive_failures.load(Ordering::SeqCst),
            last_error: self.last_error.read().ok().and_then(|error| error.clone()),
        }
    }
}

/// Tronque un message d'erreur à une taille raisonnable pour `/health`.
fn truncate_health_error(error: &str) -> String {
    const MAX: usize = 512;
    let mut truncated: String = error.chars().take(MAX).collect();
    if error.chars().count() > MAX {
        truncated.push('…');
    }
    truncated
}

/// Backoff initial après une erreur du moteur.
const ENGINE_INITIAL_BACKOFF: StdDuration = StdDuration::from_secs(1);
/// Backoff maximal après une erreur persistante.
const ENGINE_MAX_BACKOFF: StdDuration = StdDuration::from_secs(60);

/// Double le backoff sans dépasser le plafond.
fn next_engine_backoff(current: StdDuration) -> StdDuration {
    current.saturating_mul(2).min(ENGINE_MAX_BACKOFF)
}

/// Pause partagée d'une guilde, conservée par le moteur.
///
/// Le job reste `PLAYING` : les overlays figent leur média et le watchdog est
/// suspendu tant que la pause est active. Le temps restant est figé au moment
/// de la demande pour que la reprise puisse repositionner le verrou de guilde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseState {
    /// Job suspendu, si la guilde lisait un média.
    pub job_id: Option<PlaybackJobId>,
    /// Temps restant du job au moment de la pause, en millisecondes.
    pub remaining_ms: u64,
    /// Instant de la demande de pause, pour exclure la pause du temps restant.
    pub paused_at: UtcDateTime,
}

/// État interne sérialisé par le verrou du moteur.
#[derive(Debug, Default)]
struct Inner {
    /// `bootstrap` a déjà été exécuté.
    bootstrapped: bool,
    /// Guildes avec travail en cours/en attente.
    active_guilds: BTreeSet<GuildId>,
    /// Dernier checkpoint `playback.state` persisté par job (débit 1/s).
    state_persisted_at: BTreeMap<PlaybackJobId, UtcDateTime>,
    /// Pauses partagées par guilde (job figé, pas encore terminé).
    paused: BTreeMap<GuildId, PauseState>,
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
    health: Arc<EngineHealth>,
    shutting_down: AtomicBool,
    shutdown_notify: Notify,
    #[cfg(test)]
    fail_next_tick: AtomicBool,
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
            health: Arc::new(EngineHealth::new()),
            shutting_down: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            #[cfg(test)]
            fail_next_tick: AtomicBool::new(false),
        }
    }

    /// État courant du superviseur, pour `/health`.
    #[must_use]
    pub fn health(&self) -> PlaybackHealth {
        self.health.snapshot()
    }

    /// Force le prochain `tick` à échouer (tests de reprise uniquement).
    #[cfg(test)]
    fn fail_next_tick_for_test(&self) {
        self.fail_next_tick.store(true, Ordering::SeqCst);
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
        #[cfg(test)]
        if self.fail_next_tick.swap(false, Ordering::SeqCst) {
            return Err(crate::error::PlaybackError::Db(lc_db::DbError::InvalidUrl(
                "échec de tick injecté par le test".to_owned(),
            )));
        }
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
    /// Utilisé par les workers (Discord, admin, médias du board) après
    /// construction du `PlaybackJob`. Le job est mis en file : si une lecture
    /// est déjà en cours dans la guilde, il attend son tour (aucune
    /// interruption).
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
        let busy_until = now + TimeDuration::milliseconds(busy_for_ms);
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
        let min_interval = TimeDuration::try_from(self.config.state_persist_min_interval)
            .unwrap_or_else(|_| TimeDuration::milliseconds(0));
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
            self.db
                .playback_jobs()
                .set_playback_state(job.id, now)
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
        // Une trame ne peut terminer qu'un job de sa propre guilde. Un overlay
        // qui change de guilde peut router un `playback.ended` tardif vers la
        // session suivante ; l'accepter permettrait à n'importe quel client
        // d'arrêter la lecture d'une autre guilde avec un UUID de job.
        if event.guild_id != job.guild_id {
            warn!(
                job = %event.job_id,
                job_guild = %job.guild_id,
                event_guild = %event.guild_id,
                "playback.ended ignoré : le job appartient à une autre guilde"
            );
            return Ok(ReleaseOutcome {
                released: false,
                next_started: false,
            });
        }
        let guild_id = job.guild_id.clone();
        self.db
            .playback_jobs()
            .finish(event.job_id, JobStatus::Done, now)
            .await?;
        self.clear_busy(&guild_id).await?;
        info!(
            guild = %guild_id,
            job = %event.job_id,
            source = job.source.as_db_str(),
            reason = ?event.reason,
            "job de lecture terminé"
        );
        {
            let mut inner = self.inner.lock().await;
            inner.state_persisted_at.remove(&event.job_id);
            if inner
                .paused
                .get(&guild_id)
                .and_then(|pause| pause.job_id.as_ref())
                == Some(&event.job_id)
            {
                inner.paused.remove(&guild_id);
            }
        }
        let run = self.process_guild(&guild_id).await?;
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
        // Une erreur ne vaut que pour la guilde propriétaire : une trame
        // mal routée ne doit pas faire échouer le média d'une autre guilde.
        if job.guild_id != *guild_id {
            warn!(
                job = %job_id,
                job_guild = %job.guild_id,
                event_guild = %guild_id,
                "erreur de lecture ignorée : le job appartient à une autre guilde"
            );
            return Ok(ReleaseOutcome {
                released: false,
                next_started: false,
            });
        }
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
        {
            let mut inner = self.inner.lock().await;
            inner.state_persisted_at.remove(&job_id);
            if inner
                .paused
                .get(guild_id)
                .and_then(|pause| pause.job_id.as_ref())
                == Some(&job_id)
            {
                inner.paused.remove(guild_id);
            }
        }
        let run = self.process_guild(guild_id).await?;
        Ok(ReleaseOutcome {
            released: true,
            next_started: run.started,
        })
    }

    /// Met la lecture de la guilde en pause partagée.
    ///
    /// Le job `PLAYING` reste actif : [`Self::run_guild`] cesse de le surveiller
    /// et ne dispatche rien tant que la pause est posée. Sans job en cours, la
    /// demande est sans effet (`None`) ; une pause déjà posée est renvoyée telle
    /// quelle (idempotence).
    pub async fn pause(
        &self,
        guild_id: &GuildId,
        remaining_ms: Option<u64>,
    ) -> PlaybackResult<Option<PauseState>> {
        let now = self.clock.now();
        let mut playing = self.store.playing_jobs(guild_id).await?;
        // Même sélection que `current_job` : le job le plus récent.
        let Some(job) = playing.pop() else {
            return Ok(None);
        };
        let mut inner = self.inner.lock().await;
        if let Some(existing) = inner.paused.get(guild_id) {
            return Ok(Some(existing.clone()));
        }
        let estimated = self.estimated_remaining_ms(&job, now).max(0);
        let estimated_ms = u64::try_from(estimated).unwrap_or_default();
        let pause = PauseState {
            job_id: Some(job.id),
            remaining_ms: remaining_ms.unwrap_or(estimated_ms).min(estimated_ms),
            paused_at: now,
        };
        inner.paused.insert(guild_id.clone(), pause.clone());
        Ok(Some(pause))
    }

    /// Reprend la lecture de la guilde après une pause partagée.
    ///
    /// Retourne la pause retirée (utile pour le payload diffusé). Le verrou de
    /// guilde est repoussé du temps restant figé et le job est considéré comme
    /// récemment confirmé, afin que le watchdog ne le libère pas pendant que
    /// les overlays redémarrent leur média.
    ///
    /// `started_at` est aussi décalé du temps passé en pause : le temps de
    /// lecture actif doit rester identique pour un overlay qui rejoint après la
    /// reprise (`current_job`) comme pour le watchdog.
    pub async fn resume(&self, guild_id: &GuildId) -> PlaybackResult<Option<PauseState>> {
        let paused = self.inner.lock().await.paused.remove(guild_id);
        let Some(pause) = paused else {
            self.process_guild(guild_id).await?;
            return Ok(None);
        };
        let now = self.clock.now();
        let remaining_ms = pause.remaining_ms.min(MAX_PLAYBACK_REMAINING_MS);
        let busy_for_ms = i64::try_from(remaining_ms)
            .unwrap_or(i64::MAX)
            .saturating_add(self.config.lock_padding_ms);
        if let Err(error) = self
            .db
            .guilds()
            .set_busy_until(
                guild_id,
                Some(now + TimeDuration::milliseconds(busy_for_ms)),
            )
            .await
        {
            self.inner
                .lock()
                .await
                .paused
                .insert(guild_id.clone(), pause);
            return Err(error.into());
        }
        if let Some(job_id) = pause.job_id {
            let paused_for_ms = datetime_to_ms(now).saturating_sub(datetime_to_ms(pause.paused_at));
            if paused_for_ms > 0 {
                if let Err(error) = self.store.shift_started_at(job_id, paused_for_ms).await {
                    self.inner
                        .lock()
                        .await
                        .paused
                        .insert(guild_id.clone(), pause);
                    return Err(error.into());
                }
            }
            self.inner
                .lock()
                .await
                .state_persisted_at
                .insert(job_id, now);
        }
        self.process_guild(guild_id).await?;
        Ok(Some(pause))
    }

    /// Pause active d'une guilde, pour le `welcome` et les messages de session.
    pub async fn paused_state(&self, guild_id: &GuildId) -> Option<PauseState> {
        self.inner.lock().await.paused.get(guild_id).cloned()
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
        let released = self.release_playing_jobs(guild_id, job_id).await?;
        let payload = StopPayload {
            reason,
            job_id: stop_payload_job_id(job_id, &released),
        };
        self.sink.on_stop(guild_id, payload.clone()).await;
        self.process_guild(guild_id).await?;
        Ok(StopOutcome {
            stopped: released.iter().map(|id| id.to_canonical()).collect(),
            payload,
        })
    }

    /// Arrête **tous** les jobs `PLAYING` d'une guilde avec `jobId: null`.
    ///
    /// Variante de [`Self::stop_with_reason`] utilisée par la purge
    /// administrateur : le protocole exige `stop {reason:"purge",jobId:null}`
    /// même lorsqu'un seul job est libéré (cf. `protocol-v2.md` §6.3).
    pub async fn stop_all_with_reason(
        &self,
        guild_id: &GuildId,
        reason: StopReason,
    ) -> PlaybackResult<StopOutcome> {
        let released = self.release_playing_jobs(guild_id, None).await?;
        let payload = StopPayload {
            reason,
            job_id: None,
        };
        self.sink.on_stop(guild_id, payload.clone()).await;
        self.process_guild(guild_id).await?;
        Ok(StopOutcome {
            stopped: released.iter().map(|id| id.to_canonical()).collect(),
            payload,
        })
    }

    /// Libère les jobs `PLAYING` **sans émettre de `stop`**.
    ///
    /// Utilisé par le hub lorsque le payload protocolaire a déjà été diffusé :
    /// révocation d'un client (`stop {reason:"revoked",jobId:null}` diffusé
    /// avant le close `4402`, cf. `protocol-v2.md` §6.3/§10). Le `StopOutcome`
    /// retourné documente la raison et le `jobId` que l'appelant a diffusés.
    pub async fn release_playing_without_stop(
        &self,
        guild_id: &GuildId,
        job_id: Option<PlaybackJobId>,
        reason: StopReason,
    ) -> PlaybackResult<StopOutcome> {
        let released = self.release_playing_jobs(guild_id, job_id).await?;
        let payload = StopPayload {
            reason,
            job_id: stop_payload_job_id(job_id, &released),
        };
        self.process_guild(guild_id).await?;
        Ok(StopOutcome {
            stopped: released.iter().map(|id| id.to_canonical()).collect(),
            payload,
        })
    }

    /// Libère les jobs `PLAYING` ciblés, efface la réservation `busyUntil` et
    /// oublie les checkpoints de diffusion associés.
    async fn release_playing_jobs(
        &self,
        guild_id: &GuildId,
        job_id: Option<PlaybackJobId>,
    ) -> PlaybackResult<Vec<PlaybackJobId>> {
        let now = self.clock.now();
        let released = self.store.release_playing(guild_id, job_id, now).await?;
        self.clear_busy(guild_id).await?;
        {
            let mut inner = self.inner.lock().await;
            for released_id in &released {
                inner.state_persisted_at.remove(released_id);
            }
            let clears_pause = inner
                .paused
                .get(guild_id)
                .is_some_and(|pause| job_id.is_none() || pause.job_id.as_ref() == job_id.as_ref());
            if clears_pause {
                inner.paused.remove(guild_id);
            }
        }
        Ok(released)
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
        let artwork = self.load_artwork(&job, media.as_ref()).await?;
        let display_scale = self.load_display_scale(&job, media.as_ref()).await?;
        let timing = payload::persisted_timing(&job);
        let play = payload::build_play_payload(
            &job,
            media.as_ref(),
            artwork.as_ref(),
            display_scale,
            timing,
            &self.config,
        );
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
    pub async fn purge(&self, now: UtcDateTime) -> PlaybackResult<PurgeReport> {
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
    /// Une erreur de persistance ne termine plus la tâche : le moteur passe en
    /// état `Degraded`, attend un backoff borné, puis retente. Un `tick` qui
    /// réussit le ramène en `Running` ; `/health` voit la transition.
    pub async fn run<F>(&self, shutdown: F) -> PlaybackResult<()>
    where
        F: Future<Output = ()> + Send,
    {
        tokio::pin!(shutdown);
        self.health.mark_starting();
        let mut backoff = ENGINE_INITIAL_BACKOFF;
        let mut bootstrapped = false;
        loop {
            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            if !bootstrapped {
                match self.bootstrap().await {
                    Ok(_) => {
                        bootstrapped = true;
                        self.health.mark_running();
                        backoff = ENGINE_INITIAL_BACKOFF;
                    }
                    Err(error) => {
                        self.health.mark_failure(&error.to_string());
                        warn!(
                            error = %error,
                            retry_ms = u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
                            "échec de bootstrap du moteur de lecture ; reprise prévue"
                        );
                        tokio::select! {
                            _ = &mut shutdown => break,
                            () = tokio::time::sleep(backoff) => {}
                        }
                        backoff = next_engine_backoff(backoff);
                        continue;
                    }
                }
            }
            match self.tick().await {
                Ok(_) => {
                    self.health.mark_running();
                    backoff = ENGINE_INITIAL_BACKOFF;
                }
                Err(error) => {
                    self.health.mark_failure(&error.to_string());
                    warn!(
                        error = %error,
                        retry_ms = u64::try_from(backoff.as_millis()).unwrap_or(u64::MAX),
                        "échec de tick du moteur de lecture ; reprise prévue"
                    );
                    tokio::select! {
                        _ = &mut shutdown => break,
                        () = tokio::time::sleep(backoff) => {}
                    }
                    backoff = next_engine_backoff(backoff);
                    continue;
                }
            }
            if self.shutting_down.load(Ordering::SeqCst) {
                break;
            }
            tokio::select! {
                _ = &mut shutdown => break,
                () = tokio::time::sleep(self.config.tick_interval) => {}
            }
        }
        self.health.mark_stopped();
        info!("ordonnanceur de lecture arrêté");
        Ok(())
    }

    /// Variante de [`Self::run`] pilotée par [`Self::request_shutdown`].
    pub async fn run_until_shutdown(&self) -> PlaybackResult<()> {
        self.run(self.shutdown_notify.notified()).await
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
        let mut started = false;
        for _ in 0..self.config.max_guild_iterations {
            let now = self.clock.now();
            let playing = self.store.playing_jobs(guild_id).await?;
            // Une pause partagée fige le job PLAYING : ni watchdog, ni
            // dispatch du job suivant tant que la reprise n'est pas demandée.
            if let Some(pause) = inner.paused.get(guild_id) {
                let still_playing = pause
                    .job_id
                    .as_ref()
                    .is_some_and(|job_id| playing.iter().any(|job| &job.id == job_id));
                if still_playing {
                    return Ok(GuildRun {
                        started,
                        active: true,
                    });
                }
                // Le job pausé a disparu (arrêt ou fin concurrente) : la pause
                // est obsolète, on reprend l'ordonnancement normal.
                inner.paused.remove(guild_id);
            }
            if let Some(active) = playing.first().cloned() {
                self.handle_multi_playing(inner, guild_id, &playing, now)
                    .await?;
                let expected_end_ms = active
                    .started_at
                    .map(datetime_to_ms)
                    .unwrap_or_else(|| datetime_to_ms(now))
                    + active.duration_sec.max(1) * 1_000
                    + self.config.lock_padding_ms;
                if datetime_to_ms(now) >= expected_end_ms + self.config.stale_release_grace_ms {
                    // Un overlay multi-guilde peut garder ce `play` en file
                    // locale : ses keep-alives `playback.state` récents prouvent
                    // qu'il attend encore d'être affiché, donc le watchdog ne
                    // doit pas le libérer (sinon la file serveur dérive).
                    let recently_confirmed = inner
                        .state_persisted_at
                        .get(&active.id)
                        .map(|persisted_at| datetime_to_ms(*persisted_at))
                        .is_some_and(|persisted_at_ms| {
                            datetime_to_ms(now) - persisted_at_ms
                                < self.config.state_snapshot_max_age_ms
                        });
                    if !recently_confirmed {
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
                        inner.state_persisted_at.remove(&active.id);
                        continue;
                    }
                    tracing::debug!(
                        guild = %guild_id,
                        job = %active.id,
                        "watchdog suspendu : keep-alive playback.state récent (file locale)"
                    );
                }
                return Ok(GuildRun {
                    started,
                    active: true,
                });
            }

            self.clear_stale_busy(guild_id, now).await?;

            let Some(job) = self.store.claim_next_for_guild(guild_id, now).await? else {
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
                inner.state_persisted_at.remove(&job.id);
                continue;
            }
            let guild = self.db.guilds().get(guild_id).await?;
            let timing =
                payload::dispatch_timing(&job, media.as_ref(), guild.as_ref(), &self.config);
            let artwork = self.load_artwork(&job, media.as_ref()).await?;
            let display_scale = self.load_display_scale(&job, media.as_ref()).await?;
            let play = payload::build_play_payload(
                &job,
                media.as_ref(),
                artwork.as_ref(),
                display_scale,
                timing,
                &self.config,
            );
            self.store
                .mark_dispatched(
                    job.id,
                    timing.play_duration_sec,
                    timing.start_offset_sec,
                    now,
                )
                .await?;
            let busy_until = now
                + TimeDuration::milliseconds(
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
                inner.state_persisted_at.remove(&job.id);
                continue;
            }
            info!(
                guild = %guild_id,
                job = %job.id,
                source = job.source.as_db_str(),
                duration_sec = timing.play_duration_sec,
                start_offset_sec = timing.start_offset_sec,
                "job de lecture dispatché"
            );
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
        inner: &mut Inner,
        guild_id: &GuildId,
        playing: &[PlaybackJob],
        now: UtcDateTime,
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
            inner.state_persisted_at.remove(&extra.id);
        }
        Ok(())
    }

    /// Charge le média associé à un job, si présent.
    async fn load_media(&self, job: &PlaybackJob) -> PlaybackResult<Option<MediaAsset>> {
        match job.media_asset_id {
            Some(media_asset_id) => Ok(self.db.media_assets().get(media_asset_id).await?),
            None => Ok(None),
        }
    }

    /// Picture shown with an audio: the one of the guild's board item holding
    /// this audio, when « show on overlays » is ticked and the picture is
    /// ready. A lookup failure only drops the picture.
    async fn load_artwork(
        &self,
        job: &PlaybackJob,
        media: Option<&MediaAsset>,
    ) -> PlaybackResult<Option<MediaAsset>> {
        let Some(media) = media.filter(|asset| asset.kind == lc_domain::MediaKind::Audio) else {
            return Ok(None);
        };
        let Some(artwork_id) = self
            .db
            .media_board()
            .overlay_artwork(&job.guild_id, media.id)
            .await?
        else {
            return Ok(None);
        };
        Ok(self
            .db
            .media_assets()
            .get(artwork_id)
            .await?
            .filter(|picture| picture.is_ready() && picture.kind == lc_domain::MediaKind::Image))
    }

    /// Scale chosen for this media on the guild's board, in percent. A missing
    /// board item (a media played outside the board) or a lookup failure only
    /// means natural size.
    async fn load_display_scale(
        &self,
        job: &PlaybackJob,
        media: Option<&MediaAsset>,
    ) -> PlaybackResult<Option<i64>> {
        let Some(media) = media else {
            return Ok(None);
        };
        let item = match self
            .db
            .media_board()
            .get_by_guild_and_asset(&job.guild_id, media.id)
            .await
        {
            Ok(item) => item,
            Err(error) => {
                warn!(%error, "lecture de l'échelle d'affichage impossible : taille naturelle");
                return Ok(None);
            }
        };
        Ok(item.and_then(|item| item.display_scale_percent))
    }

    /// Estime le temps restant d'un job à partir de son horloge de départ.
    fn estimated_remaining_ms(&self, job: &PlaybackJob, now: UtcDateTime) -> i64 {
        let now_ms = datetime_to_ms(now);
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
    async fn clear_stale_busy(&self, guild_id: &GuildId, now: UtcDateTime) -> PlaybackResult<()> {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use lc_db::DbConfig;
    use lc_domain::FixedClock;
    use lc_protocol::{PlayPayload, StopPayload};
    use time::UtcDateTime;
    use tokio::sync::Notify;

    use super::*;

    struct NoopSink;

    impl PlaybackSink for NoopSink {
        async fn on_play(&self, _guild_id: &GuildId, _payload: PlayPayload) -> usize {
            0
        }

        async fn on_stop(&self, _guild_id: &GuildId, _payload: StopPayload) {}
    }

    #[tokio::test]
    async fn run_degrades_on_tick_error_then_recovers_and_stops() {
        let db = Db::open_in_memory(&DbConfig::default())
            .await
            .expect("base mémoire");
        let engine = Arc::new(PlaybackEngine::new(
            db,
            PlaybackConfig::default(),
            NoopSink,
            FixedClock::new(UtcDateTime::now()),
        ));
        engine.fail_next_tick_for_test();

        let shutdown = Arc::new(Notify::new());
        let run = tokio::spawn({
            let engine = Arc::clone(&engine);
            let shutdown = Arc::clone(&shutdown);
            async move { engine.run(async move { shutdown.notified().await }).await }
        });

        // Le tick injecté échoue : le moteur doit être visible en dégradé.
        tokio::time::timeout(StdDuration::from_secs(2), async {
            loop {
                if engine.health().state == PlaybackEngineState::Degraded {
                    break;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
        })
        .await
        .expect("le moteur doit passer en dégradé");
        assert!(engine.health().consecutive_failures >= 1);

        // Le backoff de 1 s laisse le tick suivant réussir : retour en running.
        tokio::time::timeout(StdDuration::from_secs(3), async {
            loop {
                let health = engine.health();
                if health.state == PlaybackEngineState::Running && health.consecutive_failures == 0
                {
                    break;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
        })
        .await
        .expect("le moteur doit récupérer après le backoff");

        engine.request_shutdown();
        shutdown.notify_waiters();
        tokio::time::timeout(StdDuration::from_secs(2), run)
            .await
            .expect("la boucle doit s'arrêter")
            .expect("la tâche de fond ne doit pas paniquer")
            .expect("l'arrêt demandé est un succès");
        assert_eq!(engine.health().state, PlaybackEngineState::Stopped);
    }
}
