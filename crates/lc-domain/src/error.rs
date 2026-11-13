//! Erreur métier unique du domaine.
//!
//! Les repositories de `lc-db` réutilisent ces variantes pour signaler
//! « introuvable », « conflit d'unicité » ou « état interdit » ; le serveur
//! les mappe vers des codes HTTP ou des fermetures WebSocket (cf.
//! `docs/rust/protocol-v2.md` §3 pour les codes 4401/4402).

use thiserror::Error;

/// Erreur de domaine, indépendante du stockage et du transport.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    /// Entité absente (ou non visible dans l'état courant).
    #[error("{entity} introuvable : {id}")]
    NotFound {
        /// Nom logique de l'entité (`media_asset`, `playback_job`, `pairing_code`…).
        entity: &'static str,
        /// Identifiant recherché, déjà formaté pour les journaux.
        id: String,
    },

    /// Violation d'unicité (clé primaire ou contrainte `UNIQUE`).
    #[error("conflit : {0}")]
    Conflict(String),

    /// Transition ou opération interdite dans l'état courant.
    #[error("état invalide : {0}")]
    InvalidState(String),

    /// Entrée invalide (format, contrainte `CHECK`, référence absente…).
    #[error("validation : {0}")]
    Validation(String),
}

impl DomainError {
    /// Construit [`DomainError::NotFound`] de façon concise.
    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity,
            id: id.into(),
        }
    }

    /// Construit [`DomainError::Conflict`].
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    /// Construit [`DomainError::InvalidState`].
    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::InvalidState(message.into())
    }

    /// Construit [`DomainError::Validation`].
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::DomainError;

    #[test]
    fn messages_sont_lisibles() {
        let error = DomainError::not_found("media_asset", "abc");
        assert_eq!(error.to_string(), "media_asset introuvable : abc");

        let error = DomainError::conflict("token_hash déjà utilisé");
        assert_eq!(error.to_string(), "conflit : token_hash déjà utilisé");

        let error = DomainError::invalid_state("job déjà terminé");
        assert!(error.to_string().contains("job déjà terminé"));

        let error = DomainError::validation("kind inconnu");
        assert!(error.to_string().contains("kind inconnu"));
    }
}
