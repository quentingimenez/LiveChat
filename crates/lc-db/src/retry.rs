//! Réessais bornés des verrous SQLite transitoires.
//!
//! SQLite est mono-écrivain : sous contention (migrateur, sauvegarde, autre
//! processus, checkpoint WAL), une requête d'écriture peut échouer avec
//! `SQLITE_BUSY`/`SQLITE_LOCKED` même après le `busy_timeout` configuré sur la
//! connexion. Ces erreurs ne traduisent pas un échec définitif : l'opération
//! peut réussir dès que l'autre écrivain relâche son verrou.
//!
//! Ce module fournit le petit retry borné utilisé par les primitives dont
//! l'échec ne doit pas arrêter un worker (claim de job de lecture). Le backoff
//! reste volontairement court : mieux vaut rendre la main au scheduler (qui
//! retentera au tick suivant) que bloquer une tâche pendant des secondes.

use std::future::Future;
use std::time::Duration;

use crate::error::DbResult;

/// Nombre total de tentatives (la première comprise).
const MAX_ATTEMPTS: u32 = 4;

/// Backoff entre deux tentatives, en millisecondes.
///
/// Le dernier délai est réutilisé si le nombre de tentatives augmente.
const BACKOFFS_MS: [u64; 3] = [25, 50, 100];

/// Exécute `operation` en réessayant les verrous SQLite transitoires.
///
/// L'opération est rejouée jusqu'à [`MAX_ATTEMPTS`] fois avec un backoff
/// exponentiel borné (25 ms, 50 ms, 100 ms). Les erreurs non transitoires (donc
/// les erreurs métier) sont renvoyées immédiatement, sans retry.
///
/// `F` doit pouvoir créer une **nouvelle** future à chaque appel : passer une
/// closure `|| async { … }` qui rejoue la requête, jamais une future déjà
/// empruntée.
pub async fn with_busy_retry<T, F, Fut>(mut operation: F) -> DbResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = DbResult<T>>,
{
    let mut attempt: u32 = 0;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                let last_attempt = attempt + 1 >= MAX_ATTEMPTS;
                if !error.is_busy() || last_attempt {
                    return Err(error);
                }
                let delay_index = attempt.min(BACKOFFS_MS.len() as u32 - 1);
                let delay_ms = BACKOFFS_MS[delay_index as usize];
                tracing::debug!(
                    tentative = attempt + 1,
                    delai_ms = delay_ms,
                    "verrou SQLite transitoire : nouvelle tentative"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                attempt += 1;
            }
        }
    }
}
