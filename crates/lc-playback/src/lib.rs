//! Ordonnanceur de lecture LiveChat (Phase 4).
//!
//! Ce crate extrait la logique de l'ordonnanceur legacy
//! (`server/src/services/playbackScheduler.ts`, 979 lignes) pour la rendre
//! testable sans HTTP ni WebSocket.
//!
//! ## Rôle
//!
//! [`PlaybackEngine`] possède la machine à états des jobs de lecture et décide
//! **quand** un job passe `PENDING → PLAYING → DONE/FAILED`. Les effets
//! extérieurs sont injectés :
//!
//! - [`lc_domain::Clock`] fournit le temps courant (tests déterministes) ;
//! - [`PlaybackSink`] diffuse les messages `play`/`stop` vers les overlays
//!   (l'implémentation réelle vit dans `apps/lc-server`, branchée sur le hub
//!   WebSocket v2) ;
//! - la persistance passe par `lc-db` (jobs, guildes, médias, codes
//!   d'appairage).
//!
//! ## Invariants
//!
//! 1. **Au plus un job `PLAYING` par guilde** : la sélection passe par
//!    `PlaybackJobsRepository::claim_next` (écriture atomique) et les chemins
//!    internes vérifient l'invariant avant de restaurer un job suspendu.
//! 2. **Ordre de la file** : `priority DESC`, `submission_date ASC`, `id ASC`
//!    (parité avec le scheduler legacy).
//! 3. **Préemption** : un job `PENDING` de priorité strictement supérieure au
//!    job actif (typiquement un média du board, [`MEDIA_BOARD_JOB_PRIORITY`] = 100)
//!    suspend le
//!    job actif (`PENDING` + `resumes_after_job_id` + `resume_offset_sec`) et le
//!    reprend automatiquement quand le job prioritaire se termine.
//! 4. **Un seul job à la fois** : toutes les opérations publiques prennent un
//!    verrou interne, les transitions concurrentes sont donc sérialisées.
//!
//! ## Intégration serveur (Phase 4)
//!
//! - Le hub WS appelle [`PlaybackEngine::handle_playback_state`] (message
//!   `playback.state`, débit limité à 1/s côté client et checkpoints côté
//!   moteur), [`PlaybackEngine::handle_playback_ended`] (`playback.ended`),
//!   [`PlaybackEngine::handle_error`] (`error`), [`PlaybackEngine::stop`]
//!   (`stop.request`) et [`PlaybackEngine::current_job`] (reconnexion
//!   `welcome.currentJob`).
//! - Le worker Discord/appelle [`PlaybackEngine::enqueue`] après création d'un
//!   job et [`PlaybackEngine::notify`] après une modification externe.
//! - La boucle [`PlaybackEngine::run`] assure les timers (durée de lecture,
//!   libération des jobs bloqués) et la purge de rétention.
//!
//! ## Limites connues (documentées dans `docs/rust/lc-playback.md`)
//!
//! - Le payload `play` construit ici contient une URL média **non signée**
//!   (`{api_base_url}/overlay/media/{assetId}`) : le hub doit la remplacer par
//!   une URL signée par client (cf. `docs/rust/protocol-v2.md` §8).
//! - L'ancien payload riche encodé dans `job.text` (préfixe
//!   `__LC_OVERLAY_RICH__::`) n'est pas décodé : les cartes tweet et les
//!   légendes doivent être persistées en champs structurés par les workers
//!   (Phase 4b/5) ou traitées par le hub. En attendant, `text.value` restitue
//!   `job.text` tel quel et `tweet_card` reste `null`.
//! - Pas de recomputation en cascade des `execution_date`/`scheduled_at` des
//!   jobs en attente (le legacy le faisait) : l'ordre de sélection n'en dépend
//!   pas (`priority`, `submission_date`), seules les prévisions d'affichage
//!   seraient décalées.

#![forbid(unsafe_code)]

mod config;
mod engine;
mod error;
mod payload;
mod sink;
mod store;

pub use config::PlaybackConfig;
pub use engine::{
    PlaybackEndedEvent, PlaybackEngine, PlaybackStateEvent, PlaybackStateOutcome, PurgeReport,
    ReleaseOutcome, StopOutcome, TickReport,
};
pub use error::{PlaybackError, PlaybackResult};
pub use sink::PlaybackSink;

/// Priorité d'un job déclenché depuis le board média (parité avec le scheduler
/// legacy).
pub const MEDIA_BOARD_JOB_PRIORITY: i64 = 100;

/// Marge ajoutée au verrou `busy_until` d'une guilde (millisecondes).
pub const PLAYBACK_LOCK_PADDING_MS: i64 = 250;

/// Délai de grâce avant de libérer un job `PLAYING` dépassé (millisecondes).
pub const PLAYBACK_STALE_RELEASE_GRACE_MS: i64 = 10_000;

/// Verrou `busy_until` minimal après un `playback.state` (millisecondes).
pub const MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS: i64 = 5_000;

/// Délai maximal de validité d'un snapshot `remaining_ms` (millisecondes).
pub const PLAYBACK_STATE_MAX_AGE_MS: i64 = 15_000;

/// Nombre maximal d'itérations de la boucle d'une guilde (parité legacy).
pub const GUILD_RUN_MAX_ITERATIONS: usize = 25;

/// Durée de lecture par défaut d'un job sans durée ni média (secondes).
pub const DEFAULT_PLAYBACK_DURATION_SEC: i64 = 30;

/// Rétention par défaut des jobs terminés (heures).
pub const DEFAULT_RETENTION_HOURS: i64 = 24;
