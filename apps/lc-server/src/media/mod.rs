//! Service média persistant des routes `/ingest` (Phases 3c/5b).
//!
//! Ce module fait le lien entre le pipeline média sans état de [`lc_media`] et
//! les tables SQLite de [`lc_db`] : il décide si un média peut être réutilisé
//! (même empreinte de source), exécute l'ingestion, persiste son cycle de vie
//! `PROCESSING → READY`/`FAILED`, applique les budgets de cache et de board
//! média, puis crée les `PlaybackJob` correspondants.
//!
//! ## Découpage
//!
//! - [`settings`] : pont entre la configuration du serveur et [`lc_media::config::MediaConfig`] ;
//! - [`tokens`] : génération et hachage SHA-256 des jetons d'ingestion ;
//! - [`errors`] : erreurs du service (`MediaError` ou erreur de base) ;
//! - [`hook`] : point d'extension `on_ingested`, câblé en Phase 4b par le hub
//!   de lecture (`lc-playback`) sans dépendance inverse vers `crate::ws` ;
//! - [`assets`] : persistance et cycle de vie des `MediaAsset` ;
//! - [`confinement`] : canonicalisation fail-closed de la racine de stockage ;
//! - [`purge`] : worker de purge des lignes périmées et fichiers orphelins ;
//! - [`quotas`] : éviction du cache et budget du board média (`lc_media::cache`) ;
//! - [`board`] : ajout d'un média au board média (épinglage, doublons, budget) ;
//! - [`jobs`] : construction des `PlaybackJob` avec la résolution de durée du
//!   legacy (durée demandée → durée du média → réglages de guilde → défaut) ;
//! - [`service`] : orchestration de l'ingestion (`MediaService`).
//!
//! Aucune de ces fonctions ne panique : les erreurs sont typées et remontées
//! aux handlers HTTP, qui les traduisent en réponses legacy.

pub mod assets;
pub mod board;
pub mod confinement;
pub mod errors;
pub mod hook;
pub mod html;
pub mod jobs;
pub mod purge;
pub mod quotas;
pub mod service;
pub mod settings;
pub mod tokens;

pub use confinement::ensure_storage_root;
pub use errors::{MediaServiceError, MediaServiceResult};
pub use hook::{install_ingest_hook, notify_ingested, IngestHook};
pub use purge::{purge_once, spawn_purge_worker, PurgeReport, ORPHAN_GRACE, PURGE_INTERVAL};
pub use service::{shared_pipeline_for_state, MediaService};
