//! Erreurs de la couche persistance.

use lc_domain::DomainError;

/// Erreur de la couche persistance.
///
/// Les erreurs métier (« média déjà sur le board », « code déjà utilisé »…)
/// remontent enrubannées dans [`DbError::Domain`] : `lc-db` ne connaît pas la
/// couche service, mais les violations de contraintes SQL identifiables sont
/// traduites au plus près de la requête concernée.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Erreur `sqlx`/SQLite brute (connexion, requête, contrainte).
    #[error("erreur SQLite : {0}")]
    Sqlx(#[from] sqlx::Error),
    /// Échec d'application des migrations.
    #[error("échec des migrations : {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// Erreur du système de fichiers (création du dossier de données…).
    #[error("erreur d'E/S : {0}")]
    Io(#[from] std::io::Error),
    /// Erreur métier (conflit, entité absente…).
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// La base ne contient pas la version de schéma attendue.
    #[error("version de schéma inattendue : attendu {expected}, trouvé {found}")]
    SchemaVersion {
        /// Version attendue par ce build.
        expected: &'static str,
        /// Version réellement lue dans `app_metadata` (ou `<absente>`).
        found: String,
    },
    /// Une ligne relue ne respecte pas le contrat stocké (date hors plage, enum
    /// inconnu, UUID invalide…). Traduit une incohérence base/contrat, pas une
    /// saisie utilisateur.
    #[error("ligne corrompue ({table}) : {detail}")]
    Corrupt {
        /// Table concernée.
        table: &'static str,
        /// Détail lisible (colonne + valeur fautive).
        detail: String,
    },
    /// URL de connexion illisible/non supportée.
    #[error("URL de connexion invalide : {0}")]
    InvalidUrl(String),
}

/// Alias de résultat de la couche persistance.
pub type DbResult<T> = Result<T, DbError>;

impl DbError {
    /// `true` si l'erreur est une violation d'unicité SQLite.
    pub fn is_unique_violation(&self) -> bool {
        match self {
            Self::Sqlx(error) => is_unique_violation(error),
            _ => false,
        }
    }

    /// `true` si l'erreur est une violation de clé étrangère SQLite.
    pub fn is_foreign_key_violation(&self) -> bool {
        match self {
            Self::Sqlx(error) => is_foreign_key_violation(error),
            _ => false,
        }
    }

    /// `true` si l'erreur est une violation de contrainte `CHECK` SQLite.
    pub fn is_check_violation(&self) -> bool {
        match self {
            Self::Sqlx(error) => is_check_violation(error),
            _ => false,
        }
    }

    /// `true` si l'erreur est un verrou SQLite transitoire (`SQLITE_BUSY` ou
    /// `SQLITE_LOCKED`).
    ///
    /// Ces erreurs ne sont pas définitives : une autre connexion (autre
    /// processus, migrateur, sauvegarde) peut tenir le verrou d'écriture plus
    /// longtemps que `busy_timeout`. L'appelant peut alors réessayer avec un
    /// backoff borné, par exemple via [`crate::with_busy_retry`].
    pub fn is_busy(&self) -> bool {
        match self {
            Self::Sqlx(error) => is_busy(error),
            _ => false,
        }
    }
}

fn database_message(error: &sqlx::Error) -> Option<&str> {
    match error {
        sqlx::Error::Database(database_error) => Some(database_error.message()),
        _ => None,
    }
}

/// `true` si `error` est une violation d'unicité SQLite.
pub(crate) fn is_unique_violation(error: &sqlx::Error) -> bool {
    database_message(error).is_some_and(|message| {
        message.contains("UNIQUE constraint failed")
            || message.contains("PRIMARY KEY constraint failed")
    })
}

/// `true` si `error` est une violation de clé étrangère SQLite.
pub(crate) fn is_foreign_key_violation(error: &sqlx::Error) -> bool {
    database_message(error).is_some_and(|message| message.contains("FOREIGN KEY constraint failed"))
}

/// `true` si `error` est une violation de contrainte `CHECK` SQLite.
pub(crate) fn is_check_violation(error: &sqlx::Error) -> bool {
    database_message(error).is_some_and(|message| message.contains("CHECK constraint failed"))
}

/// `true` si `error` est un verrou SQLite transitoire.
///
/// SQLite ne fournit pas de code stable via `sqlx::Error` (`DatabaseError::code`
/// vaut le code étendu, variable selon la version) : on reconnaît les messages
/// documentés par SQLite, qui sont stables et non localisés.
pub(crate) fn is_busy(error: &sqlx::Error) -> bool {
    database_message(error).is_some_and(|message| {
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("database schema is locked")
            || message.contains("database is busy")
    })
}

/// Traduit une violation d'unicité en [`DomainError::Conflict`], laisse passer
/// les autres erreurs.
pub(crate) fn conflict_on_unique(error: sqlx::Error, description: impl Into<String>) -> DbError {
    if is_unique_violation(&error) {
        DbError::Domain(DomainError::conflict(description))
    } else {
        DbError::Sqlx(error)
    }
}
