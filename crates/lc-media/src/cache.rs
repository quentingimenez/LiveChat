//! Budgets et éviction LRU du cache média (fonctions pures).
//!
//! Ces fonctions ne touchent ni le disque ni la base : l'appelant (Phase 5b)
//! charge les entrées depuis la DB, demande un plan, puis applique les
//! suppressions. Cela garde la politique de quota testable sans I/O.

/// Durée après laquelle un média FAILED/PROCESSING est considéré comme périmé.
pub const STALE_PENDING_TTL_MS: i64 = 2 * 3_600_000;

/// Entrée de cache média, vue par la politique de quota.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheEntry {
    /// Clé content-addressed (`source_hash`).
    pub key: String,
    /// Taille en octets.
    pub size_bytes: u64,
    /// Dernier accès, en millisecondes epoch UTC.
    pub last_accessed_ms: i64,
    /// Date de création, en millisecondes epoch UTC.
    pub created_at_ms: i64,
    /// Média épinglé par un usage persistant (board, job actif).
    pub pinned: bool,
    /// Média associé à un job PENDING/PLAYING (non évictable).
    pub active: bool,
    /// Média FAILED/PROCESSING (purgeable plus vite).
    pub stale: bool,
}

impl CacheEntry {
    /// Instant d'expiration TTL de l'entrée.
    pub fn expiry_ms(&self, ttl_ms: i64) -> i64 {
        self.last_accessed_ms.saturating_add(ttl_ms)
    }

    /// `true` si l'entrée est expirée à `now_ms`.
    pub fn is_expired_at(&self, now_ms: i64, ttl_ms: i64) -> bool {
        if self.expiry_ms(ttl_ms) <= now_ms {
            return true;
        }
        self.stale && self.last_accessed_ms.saturating_add(STALE_PENDING_TTL_MS) <= now_ms
    }

    /// `true` si la politique d'éviction peut supprimer l'entrée.
    pub fn is_evictable(&self) -> bool {
        !self.pinned && !self.active
    }
}

/// Plan d'éviction calculé avant application.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheEvictionPlan {
    /// Le budget tient après application du plan.
    pub has_capacity: bool,
    /// Taille du média à faire entrer.
    pub required_bytes: u64,
    /// Budget total configuré.
    pub max_total_bytes: u64,
    /// Total occupé avant éviction (hors médias épinglés).
    pub total_before_bytes: u64,
    /// Total occupé après éviction (hors médias épinglés).
    pub total_after_bytes: u64,
    /// Clés à supprimer, dans l'ordre d'éviction.
    pub evicted_keys: Vec<String>,
    /// Octets libérés par le plan.
    pub evicted_bytes: u64,
}

impl CacheEvictionPlan {
    /// Nombre de médias évincés.
    pub fn evicted_count(&self) -> usize {
        self.evicted_keys.len()
    }

    /// Octets qui manqueraient si `has_capacity` est faux.
    pub fn missing_bytes(&self) -> u64 {
        self.required_bytes
            .saturating_add(self.total_before_bytes)
            .saturating_sub(self.evicted_bytes)
            .saturating_sub(self.max_total_bytes)
    }
}

/// Total du cache (entrées non épinglées).
pub fn cache_total_bytes(entries: &[CacheEntry]) -> u64 {
    entries
        .iter()
        .filter(|entry| !entry.pinned)
        .map(|entry| entry.size_bytes)
        .sum()
}

/// Total du board (entrées épinglées).
pub fn board_total_bytes(entries: &[CacheEntry]) -> u64 {
    entries
        .iter()
        .filter(|entry| entry.pinned)
        .map(|entry| entry.size_bytes)
        .sum()
}

/// Planifie l'éviction pour faire tenir `required_bytes` dans `max_total_bytes`.
///
/// Les médias épinglés (board) sortent du budget cache ; les médias actifs ne
/// sont jamais évincés. Les candidats sont triés du plus ancien accès au plus
/// récent (LRU), puis par date de création et clé pour un ordre stable.
pub fn plan_cache_eviction(
    entries: &[CacheEntry],
    required_bytes: u64,
    max_total_bytes: u64,
    exclude_key: Option<&str>,
) -> CacheEvictionPlan {
    let total_before = cache_total_bytes(entries);
    let mut plan = CacheEvictionPlan {
        has_capacity: total_before.saturating_add(required_bytes) <= max_total_bytes,
        required_bytes,
        max_total_bytes,
        total_before_bytes: total_before,
        total_after_bytes: total_before,
        evicted_keys: Vec::new(),
        evicted_bytes: 0,
    };
    if plan.has_capacity {
        return plan;
    }
    let mut candidates: Vec<&CacheEntry> = entries
        .iter()
        .filter(|entry| entry.is_evictable())
        .filter(|entry| exclude_key != Some(entry.key.as_str()))
        .collect();
    candidates.sort_by(|left, right| {
        left.last_accessed_ms
            .cmp(&right.last_accessed_ms)
            .then(left.created_at_ms.cmp(&right.created_at_ms))
            .then(left.key.cmp(&right.key))
    });
    let mut total = total_before;
    for candidate in candidates {
        if total.saturating_add(required_bytes) <= max_total_bytes {
            plan.has_capacity = true;
            break;
        }
        total = total.saturating_sub(candidate.size_bytes);
        plan.evicted_bytes = plan.evicted_bytes.saturating_add(candidate.size_bytes);
        plan.evicted_keys.push(candidate.key.clone());
    }
    plan.has_capacity = total.saturating_add(required_bytes) <= max_total_bytes;
    plan.total_after_bytes = total;
    plan
}

/// Planifie l'éviction nécessaire pour respecter le budget du board.
///
/// Les candidats sont les médias épinglés et inactifs, du plus ancien accès au
/// plus récent.
pub fn plan_board_eviction(
    entries: &[CacheEntry],
    required_bytes: u64,
    board_max_total_bytes: u64,
) -> CacheEvictionPlan {
    let total_before = board_total_bytes(entries);
    let mut plan = CacheEvictionPlan {
        has_capacity: total_before.saturating_add(required_bytes) <= board_max_total_bytes,
        required_bytes,
        max_total_bytes: board_max_total_bytes,
        total_before_bytes: total_before,
        total_after_bytes: total_before,
        evicted_keys: Vec::new(),
        evicted_bytes: 0,
    };
    if plan.has_capacity {
        return plan;
    }
    let mut candidates: Vec<&CacheEntry> = entries
        .iter()
        .filter(|entry| entry.pinned && !entry.active)
        .collect();
    candidates.sort_by(|left, right| {
        left.last_accessed_ms
            .cmp(&right.last_accessed_ms)
            .then(left.created_at_ms.cmp(&right.created_at_ms))
            .then(left.key.cmp(&right.key))
    });
    let mut total = total_before;
    for candidate in candidates {
        if total.saturating_add(required_bytes) <= board_max_total_bytes {
            break;
        }
        total = total.saturating_sub(candidate.size_bytes);
        plan.evicted_bytes = plan.evicted_bytes.saturating_add(candidate.size_bytes);
        plan.evicted_keys.push(candidate.key.clone());
    }
    plan.has_capacity = total.saturating_add(required_bytes) <= board_max_total_bytes;
    plan.total_after_bytes = total;
    plan
}

/// Clés à purger (TTL dépassé ou média FAILED/PROCESSING trop ancien).
pub fn purge_candidates(entries: &[CacheEntry], now_ms: i64, ttl_ms: i64) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.is_expired_at(now_ms, ttl_ms))
        .map(|entry| entry.key.clone())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn entry(key: &str, size: u64, last_accessed_ms: i64) -> CacheEntry {
        CacheEntry {
            key: key.to_string(),
            size_bytes: size,
            last_accessed_ms,
            created_at_ms: last_accessed_ms,
            ..CacheEntry::default()
        }
    }

    #[test]
    fn no_eviction_when_budget_fits() {
        let entries = vec![entry("a", 100, 1), entry("b", 200, 2)];
        let plan = plan_cache_eviction(&entries, 300, 1_000, None);
        assert!(plan.has_capacity);
        assert_eq!(plan.total_before_bytes, 300);
        assert_eq!(plan.total_after_bytes, 300);
        assert!(plan.evicted_keys.is_empty());
        assert_eq!(plan.missing_bytes(), 0);
    }

    #[test]
    fn evicts_lru_until_budget_fits() {
        let entries = vec![
            entry("old", 400, 10),
            entry("mid", 400, 20),
            entry("new", 400, 30),
        ];
        let plan = plan_cache_eviction(&entries, 400, 1_000, None);
        assert!(plan.has_capacity);
        assert_eq!(plan.total_before_bytes, 1_200);
        assert_eq!(
            plan.evicted_keys,
            vec!["old".to_string(), "mid".to_string()]
        );
        assert_eq!(plan.evicted_bytes, 800);
        assert_eq!(plan.total_after_bytes, 400);
    }

    #[test]
    fn skips_pinned_and_active_and_excluded() {
        let mut pinned = entry("pinned", 500, 1);
        pinned.pinned = true;
        let mut active = entry("active", 500, 2);
        active.active = true;
        let entries = vec![pinned, active, entry("a", 500, 3), entry("b", 500, 4)];
        let plan = plan_cache_eviction(&entries, 600, 2_000, Some("a"));
        assert_eq!(plan.total_before_bytes, 1_500);
        assert_eq!(plan.evicted_keys, vec!["b".to_string()]);
        assert!(plan.has_capacity);
        let without_b = plan_cache_eviction(&entries, 1_600, 1_500, Some("a"));
        assert!(!without_b.has_capacity);
        assert_eq!(without_b.evicted_keys, vec!["b".to_string()]);
    }

    #[test]
    fn reports_missing_bytes_when_insufficient() {
        let entries = vec![entry("a", 100, 1), entry("b", 100, 2)];
        let plan = plan_cache_eviction(&entries, 950, 900, None);
        assert!(!plan.has_capacity);
        assert_eq!(plan.evicted_keys.len(), 2);
        assert_eq!(plan.total_after_bytes, 0);
        assert_eq!(plan.evicted_count(), 2);
        assert_eq!(plan.missing_bytes(), 50);
    }

    #[test]
    fn board_budget_evicts_pinned_lru_only() {
        let mut first = entry("b1", 400, 10);
        first.pinned = true;
        let mut second = entry("b2", 400, 20);
        second.pinned = true;
        let mut active = entry("b3", 400, 5);
        active.pinned = true;
        active.active = true;
        let entries = vec![first, second, active, entry("cache", 100, 1)];
        assert_eq!(board_total_bytes(&entries), 1_200);
        assert_eq!(cache_total_bytes(&entries), 100);
        let plan = plan_board_eviction(&entries, 100, 1_000);
        assert!(plan.has_capacity);
        assert_eq!(plan.evicted_keys, vec!["b1".to_string()]);
        assert_eq!(plan.total_after_bytes, 800);
    }

    #[test]
    fn expiry_and_purge_candidates() {
        let mut failed = entry("failed", 10, 0);
        failed.stale = true;
        let entries = vec![entry("fresh", 10, 1_000), entry("old", 10, 0), failed];
        let ttl_ms = 1_000;
        assert!(!entries[0].is_expired_at(1_500, ttl_ms));
        assert!(entries[1].is_expired_at(1_500, ttl_ms));
        assert!(entries[2].is_expired_at(1_500, ttl_ms));
        assert_eq!(entries[2].expiry_ms(ttl_ms), 1_000);
        assert_eq!(
            purge_candidates(&entries, 1_500, ttl_ms),
            vec!["old".to_string(), "failed".to_string()]
        );
        // Un média FAILED/PROCESSING est purgeable après 2 h même si le TTL
        // du cache (12 h ici) n'est pas encore atteint.
        let stale = CacheEntry {
            key: "stale".to_string(),
            last_accessed_ms: 0,
            created_at_ms: 0,
            stale: true,
            ..CacheEntry::default()
        };
        assert!(stale.is_expired_at(STALE_PENDING_TTL_MS + 1, 43_200_000));
        let mut fresh_stale = stale;
        fresh_stale.stale = false;
        assert!(!fresh_stale.is_expired_at(STALE_PENDING_TTL_MS + 1, 43_200_000));
    }
}
