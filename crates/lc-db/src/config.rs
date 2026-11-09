//! Configuration d'ouverture de la base.

use std::time::Duration;

/// Options d'ouverture d'un [`Db`](crate::Db).
///
/// Concentre les réglages qui touchent au pool SQLite : taille, timeouts,
/// création du fichier/dossier et exécution des migrations. Les pragmas
/// (`journal_mode`, `foreign_keys`, `synchronous`) ne sont **pas** exposés :
/// ils font partie du contrat de persistance (cf. `docs/rust/lc-db.md`).
#[derive(Clone, Debug)]
pub struct DbConfig {
    /// Nombre maximal de connexions du pool (défaut : 5, cf. contrainte du
    /// board Phase 2).
    pub max_connections: u32,
    /// Nombre minimal de connexions maintenues ouvertes (défaut : 1).
    pub min_connections: u32,
    /// Délai d'attente d'une connexion libre du pool (défaut : 30 s).
    pub acquire_timeout: Duration,
    /// `PRAGMA busy_timeout` en millisecondes (défaut : 5 s).
    pub busy_timeout: Duration,
    /// Créer le fichier SQLite s'il n'existe pas (défaut : `true`).
    pub create_if_missing: bool,
    /// Créer le dossier parent du fichier au besoin (défaut : `true`).
    pub create_parent_dirs: bool,
    /// Exécuter les migrations embarquées à l'ouverture (défaut : `true`).
    pub run_migrations: bool,
    /// Vérifier `app_metadata.schema_version` à l'ouverture (défaut : `true`).
    pub check_schema_version: bool,
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            max_connections: 5,
            min_connections: 1,
            acquire_timeout: Duration::from_secs(30),
            busy_timeout: Duration::from_secs(5),
            create_if_missing: true,
            create_parent_dirs: true,
            run_migrations: true,
            check_schema_version: true,
        }
    }
}

impl DbConfig {
    /// Configuration par défaut.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fixe la taille maximale du pool.
    ///
    /// Une valeur `0` est ramenée à `1` au moment de l'ouverture (SQLite exige
    /// au moins une connexion).
    pub fn with_max_connections(mut self, max_connections: u32) -> Self {
        self.max_connections = max_connections;
        self
    }

    /// Fixe le `PRAGMA busy_timeout`.
    pub fn with_busy_timeout(mut self, busy_timeout: Duration) -> Self {
        self.busy_timeout = busy_timeout;
        self
    }

    /// Désactive l'application des migrations à l'ouverture (utile pour tester
    /// une base existante ou piloter les migrations manuellement).
    pub fn without_migrations(mut self) -> Self {
        self.run_migrations = false;
        self.check_schema_version = false;
        self
    }

    /// Désactive la vérification de `schema_version` à l'ouverture.
    pub fn without_schema_version_check(mut self) -> Self {
        self.check_schema_version = false;
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::DbConfig;
    use std::time::Duration;

    #[test]
    fn defaut_phase_2() {
        let config = DbConfig::default();
        assert_eq!(config.max_connections, 5);
        assert_eq!(config.busy_timeout, Duration::from_millis(5_000));
        assert!(config.run_migrations);
        assert!(config.check_schema_version);
        assert!(config.create_parent_dirs);
    }

    #[test]
    fn constructeurs_chaines() {
        let config = DbConfig::new()
            .with_max_connections(2)
            .with_busy_timeout(Duration::from_millis(250))
            .without_migrations();
        assert_eq!(config.max_connections, 2);
        assert_eq!(config.busy_timeout, Duration::from_millis(250));
        assert!(!config.run_migrations);
        assert!(!config.check_schema_version);
    }
}
