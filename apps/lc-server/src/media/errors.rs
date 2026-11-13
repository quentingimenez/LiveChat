//! Erreurs du service média.
//!
//! Toutes les erreurs d'ingestion sont des erreurs `MediaError` de
//! [`lc_media`] : elles portent un code legacy (`UNSUPPORTED_SOURCE`,
//! `CACHE_STORAGE_LIMIT_REACHED`…) que la route `/ingest` renvoie tel quel en
//! 422. Les erreurs de base de données sont considérées comme internes :
//! la route répond 500 `internal_error`, comme le legacy pour les erreurs non
//! média.

use lc_db::DbError;
use lc_media::error::{MediaError, MediaErrorCode};

/// Erreur du service média.
#[derive(Debug, thiserror::Error)]
pub enum MediaServiceError {
    /// Erreur d'ingestion/stockage avec un code legacy.
    #[error("{0}")]
    Media(#[from] MediaError),
    /// Erreur d'accès à SQLite (considérée comme interne).
    #[error("base de données : {0}")]
    Database(#[from] DbError),
    /// Configuration média invalide ou pipeline indisponible.
    #[error("service média indisponible : {0}")]
    Unavailable(String),
}

/// Résultat du service média.
pub type MediaServiceResult<T> = Result<T, MediaServiceError>;

impl MediaServiceError {
    /// Construit une erreur interne de service média.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Erreur média associée, si l'erreur doit être renvoyée en 422 legacy.
    ///
    /// `None` signifie erreur interne (la route répond 500 `internal_error`).
    pub fn media_error(&self) -> Option<&MediaError> {
        match self {
            Self::Media(error) => Some(error),
            Self::Database(_) | Self::Unavailable(_) => None,
        }
    }

    /// Code d'erreur legacy, si disponible.
    pub fn media_code(&self) -> Option<MediaErrorCode> {
        self.media_error().map(MediaError::code)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn media_errors_expose_code_and_message() {
        let error = MediaServiceError::Media(MediaError::new(
            MediaErrorCode::InvalidMedia,
            "média invalide",
        ));

        assert_eq!(error.media_code(), Some(MediaErrorCode::InvalidMedia));
        assert_eq!(
            error.media_error().map(MediaError::message),
            Some("média invalide")
        );
        assert!(error.to_string().contains("média invalide"));
    }

    #[test]
    fn internal_errors_have_no_media_code() {
        let unavailable = MediaServiceError::unavailable("pas de pipeline");
        assert_eq!(unavailable.media_code(), None);
        assert!(unavailable.media_error().is_none());

        let database = MediaServiceError::Database(DbError::InvalidUrl("file:".into()));
        assert_eq!(database.media_code(), None);
        assert!(database.to_string().contains("base de données"));
    }
}
