//! Abstraction du temps pour les tests et le scheduler.
//!
//! Les repositories `lc-db` **ne lisent jamais l'horloge eux-mêmes** : les
//! horodatages (`now`, `started_at`, `finished_at`…) sont passés explicitement
//! par l'appelant. Cela rend les tests d'expiration/claim déterministes sans
//! mocker le pool. `Clock` sert donc surtout au serveur (Phase 3/4) qui peut
//! injecter un [`FixedClock`] dans ses workers pendant les tests.

use std::sync::RwLock;

use chrono::{DateTime, Duration, Utc};

use crate::time::datetime_to_ms;

/// Source de temps injectable.
pub trait Clock: Send + Sync {
    /// Instant courant.
    fn now(&self) -> DateTime<Utc>;

    /// Instant courant en millisecondes epoch UTC.
    fn now_ms(&self) -> i64 {
        datetime_to_ms(self.now())
    }
}

/// Horloge système réelle (`Utc::now()`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Horloge figée, avancée manuellement (tests unitaires/scheduler).
///
/// Utilise un `RwLock` et récupère le garde empoisonné (`into_inner`) au lieu
/// de propager une panique : une erreur dans un test parallèle ne doit pas
/// casser les autres tests qui partagent l'horloge.
#[derive(Debug)]
pub struct FixedClock {
    now: RwLock<DateTime<Utc>>,
}

impl FixedClock {
    /// Fige l'horloge à `at`.
    pub fn new(at: DateTime<Utc>) -> Self {
        Self {
            now: RwLock::new(at),
        }
    }

    /// Fixe l'instant courant.
    pub fn set(&self, at: DateTime<Utc>) {
        let mut guard = match self.now.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = at;
    }

    /// Avance l'horloge de `delta` (valeurs négatives acceptées).
    ///
    /// Si l'addition déborde de la plage `chrono`, l'horloge reste inchangée.
    pub fn advance(&self, delta: Duration) {
        let mut guard = match self.now.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(next) = guard.checked_add_signed(delta) {
            *guard = next;
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        match self.now.read() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Clock, FixedClock, SystemClock};
    use crate::time::{datetime_to_ms, ms_to_datetime};
    use chrono::Duration;

    #[test]
    fn fixed_clock_reste_stable_puis_avance() {
        let start = ms_to_datetime(1_700_000_000_000).unwrap();
        let clock = FixedClock::new(start);

        assert_eq!(clock.now(), start);
        assert_eq!(clock.now_ms(), 1_700_000_000_000);

        clock.advance(Duration::milliseconds(250));
        assert_eq!(clock.now_ms(), 1_700_000_000_250);
        assert_eq!(datetime_to_ms(clock.now()), clock.now_ms());
    }

    #[test]
    fn fixed_clock_set_remplace_l_instant() {
        let clock = FixedClock::new(ms_to_datetime(0).unwrap());
        let target = ms_to_datetime(42).unwrap();
        clock.set(target);
        assert_eq!(clock.now(), target);
    }

    #[test]
    fn system_clock_est_proche_du_present() {
        let clock = SystemClock;
        let now = clock.now();
        let delta = (chrono::Utc::now() - now).num_seconds().abs();
        assert!(delta < 5, "horloge système incohérente : {delta}s d'écart");
        assert!(clock.now_ms() > 0);
    }
}
