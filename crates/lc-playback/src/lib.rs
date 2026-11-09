//! Ordonnanceur de lecture LiveChat (Phase 4).
//!
//! Ce crate contient la logique de file de lecture extraite du serveur pour être
//! testable sans HTTP ni WebSocket :
//!
//! - sélection du prochain job (priorité, date de soumission, reprises),
//! - invariant « un seul job `PLAYING` par guilde »,
//! - préemption par un job prioritaire (mèmes) et reprise du job interrompu,
//! - machine à états `PENDING → PLAYING → DONE/FAILED` avec checkpoints de
//!   progression (`remaining_ms_snapshot`, `resume_offset_sec`),
//! - `stop.request` manuel (tout ou job ciblé) et purge périodique.
//!
//! L'intégration HTTP/WebSocket (diffusion des messages `play`/`stop`, timers
//! réels) reste dans `apps/lc-server` : ce crate expose des décisions pures et
//! des effets de persistance via `lc-db`, pilotés par une horloge injectée.
//!
//! L'API publique est définie au fil de la Phase 4 (voir `docs/rust/lc-playback.md`).

#![forbid(unsafe_code)]
