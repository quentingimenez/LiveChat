//! Erreurs de l'ordonnanceur.

use lc_db::DbError;
use lc_domain::DomainError;

/// Erreur de l'ordonnanceur de lecture.
///
/// Toutes les erreurs de persistance (`lc-db`) et de domaine (`lc-domain`)
/// remontent telles quelles : l'appelant (serveur, worker) décide de la
/// politique de retry et de journalisation.
#[derive(Debug, thiserror::Error)]
pub enum PlaybackError {
    /// Erreur de la base de données.
    #[error(transparent)]
    Db(#[from] DbError),
    /// Erreur de validation du domaine (identifiants, statuts…).
    #[error(transparent)]
    Domain(#[from] DomainError),
}

/// Alias de résultat pour l'ordonnanceur.
pub type PlaybackResult<T> = Result<T, PlaybackError>;
