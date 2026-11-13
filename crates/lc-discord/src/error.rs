//! Erreurs des handlers et messages localisés associés.
//!
//! Le legacy traduisait les erreurs d'ingestion en codes
//! (`server/src/services/media/mediaErrors.ts`) puis en messages via
//! `getLocalizedMediaErrorMessage`. Ce module reprend les mêmes codes et les
//! mêmes textes (voir [`crate::i18n`]) pour que les réponses Discord restent
//! identiques.

use thiserror::Error;

use crate::i18n::{interpolate, strings, Lang};

/// Limites de stockage utilisées pour rendre les messages d'erreur média.
///
/// Le legacy exposait ces valeurs via `env.MEDIA_MAX_SIZE_MB` (2048),
/// `env.MEDIA_CACHE_MAX_TOTAL_MB` (5120) et `env.MEDIA_BOARD_MAX_TOTAL_MB`
/// (15360). Toute valeur `< 1` est ramenée à `1` dans le message, comme
/// `Math.max(1, …)` côté TypeScript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaLimits {
    /// Taille maximale d'un média (Mo).
    pub max_size_mb: i64,
    /// Taille maximale du cache média non persistant (Mo).
    pub max_cache_total_mb: i64,
    /// Taille maximale du stockage persistant du board média (Mo).
    pub max_board_total_mb: i64,
}

impl Default for MediaLimits {
    fn default() -> Self {
        Self {
            max_size_mb: 2_048,
            max_cache_total_mb: 5_120,
            max_board_total_mb: 15_360,
        }
    }
}

/// Codes d'erreur média du legacy (`mediaErrors.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaErrorCode {
    /// Source non supportée (plateforme inconnue…).
    UnsupportedSource,
    /// Média privé ou nécessitant une authentification.
    PrivateOrAuthRequired,
    /// Média protégé par DRM.
    DrmProtected,
    /// Média introuvable (lien supprimé/expiré).
    MediaNotFound,
    /// Délai de téléchargement dépassé.
    DownloadTimeout,
    /// Fichier trop volumineux.
    FileTooLarge,
    /// Cache média non persistant plein.
    CacheStorageLimitReached,
    /// Stockage persistant du board média plein.
    BoardStorageLimitReached,
    /// Fichier téléchargé invalide/corrompu.
    InvalidMedia,
    /// Échec de transcodage.
    TranscodeFailed,
    /// Échec de téléchargement générique (aussi le défaut legacy).
    DownloadFailed,
}

impl MediaErrorCode {
    /// Toutes les variantes, dans l'ordre du legacy.
    pub const ALL: &'static [Self] = &[
        Self::UnsupportedSource,
        Self::PrivateOrAuthRequired,
        Self::DrmProtected,
        Self::MediaNotFound,
        Self::DownloadTimeout,
        Self::FileTooLarge,
        Self::CacheStorageLimitReached,
        Self::BoardStorageLimitReached,
        Self::InvalidMedia,
        Self::TranscodeFailed,
        Self::DownloadFailed,
    ];

    /// Nom technique du code (journaux, diagnostics).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedSource => "UNSUPPORTED_SOURCE",
            Self::PrivateOrAuthRequired => "PRIVATE_OR_AUTH_REQUIRED",
            Self::DrmProtected => "DRM_PROTECTED",
            Self::MediaNotFound => "MEDIA_NOT_FOUND",
            Self::DownloadTimeout => "DOWNLOAD_TIMEOUT",
            Self::FileTooLarge => "FILE_TOO_LARGE",
            Self::CacheStorageLimitReached => "CACHE_STORAGE_LIMIT_REACHED",
            Self::BoardStorageLimitReached => "BOARD_STORAGE_LIMIT_REACHED",
            Self::InvalidMedia => "INVALID_MEDIA",
            Self::TranscodeFailed => "TRANSCODE_FAILED",
            Self::DownloadFailed => "DOWNLOAD_FAILED",
        }
    }

    /// Message localisé équivalent à `getLocalizedMediaErrorMessage`.
    pub fn message(self, lang: Lang, limits: MediaLimits) -> String {
        let dict = strings(lang);
        match self {
            Self::UnsupportedSource => dict.media_error_unsupported_source.to_owned(),
            Self::PrivateOrAuthRequired => dict.media_error_private.to_owned(),
            Self::DrmProtected => dict.media_error_drm.to_owned(),
            Self::MediaNotFound => dict.media_error_not_found.to_owned(),
            Self::DownloadTimeout => dict.media_error_timeout.to_owned(),
            Self::FileTooLarge => {
                let max_size_mb = limits.max_size_mb.max(1).to_string();
                interpolate(dict.media_error_too_large, &[("maxSizeMb", &max_size_mb)])
            }
            Self::CacheStorageLimitReached => {
                let max_cache_total_mb = limits.max_cache_total_mb.max(1).to_string();
                interpolate(
                    dict.media_error_cache_storage_limit,
                    &[("maxCacheTotalMb", &max_cache_total_mb)],
                )
            }
            Self::BoardStorageLimitReached => {
                let max_board_total_mb = limits.max_board_total_mb.max(1).to_string();
                interpolate(
                    dict.media_error_board_storage_limit,
                    &[("maxBoardTotalMb", &max_board_total_mb)],
                )
            }
            Self::InvalidMedia => dict.media_error_invalid_media.to_owned(),
            Self::TranscodeFailed => dict.media_error_transcode.to_owned(),
            Self::DownloadFailed => dict.media_error_download.to_owned(),
        }
    }
}

/// Erreur média enrichie (code + détail technique pour les journaux).
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("erreur média {code:?}")]
pub struct MediaError {
    /// Code métier (utilisé pour choisir le message localisé).
    pub code: MediaErrorCode,
    /// Détail technique non exposé à Discord.
    pub detail: Option<String>,
}

impl MediaError {
    /// Construit une erreur depuis un code.
    pub fn new(code: MediaErrorCode) -> Self {
        Self { code, detail: None }
    }

    /// Construit une erreur depuis un code et un détail de diagnostic.
    pub fn with_detail(code: MediaErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }

    /// Message localisé à afficher.
    pub fn message(&self, lang: Lang, limits: MediaLimits) -> String {
        self.code.message(lang, limits)
    }
}

impl From<MediaErrorCode> for MediaError {
    fn from(code: MediaErrorCode) -> Self {
        Self::new(code)
    }
}

/// Échec de la synthèse vocale (`gtts` legacy).
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SpeechFailure {
    /// Aucun audio n'a pu être généré (réseau, voix inconnue…).
    #[error("la génération vocale a échoué")]
    GenerationFailed,
    /// Le texte à dire est vide après normalisation.
    #[error("le texte à dire est vide")]
    EmptyText,
}

/// Erreur générique d'un handler (persistance, guilde absente, backend…).
#[derive(Debug, Error)]
pub enum BotError {
    /// Erreur de persistance `lc-db`.
    #[error("erreur de base de données : {0}")]
    Db(#[from] lc_db::DbError),
    /// Commande nécessitant une guilde exécutée en message privé.
    #[error("commande sans guilde : {command}")]
    MissingGuild {
        /// Nom logique de la commande (voir [`crate::CommandKey::id`]).
        command: &'static str,
    },
    /// Erreur média non gérée par le handler (repli).
    #[error("erreur média : {0}")]
    Media(#[from] MediaError),
    /// Échec de la synthèse vocale.
    #[error("synthèse vocale : {0}")]
    Speech(#[from] SpeechFailure),
    /// Échec du moteur de lecture (repli).
    #[error("moteur de lecture : {0}")]
    Playback(String),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{MediaError, MediaErrorCode, MediaLimits};
    use crate::i18n::Lang;

    #[test]
    fn messages_localises_avec_limites() {
        let limits = MediaLimits {
            max_size_mb: 0,
            max_cache_total_mb: 10,
            max_board_total_mb: 20,
        };

        assert_eq!(
            MediaErrorCode::FileTooLarge.message(Lang::Fr, limits),
            "Le média est trop volumineux (limite actuelle: 1 MB)."
        );
        assert_eq!(
            MediaErrorCode::CacheStorageLimitReached.message(Lang::En, limits),
            "The non-persistent media cache is full (limit: 10 MB). Retry in a moment."
        );
        assert_eq!(
            MediaErrorCode::BoardStorageLimitReached.message(Lang::En, limits),
            "Persistent media board storage is full (limit: 20 MB). Remove older board items first."
        );
    }

    #[test]
    fn codes_et_details() {
        assert_eq!(MediaErrorCode::DrmProtected.as_str(), "DRM_PROTECTED");
        assert_eq!(MediaErrorCode::ALL.len(), 11);

        let error = MediaError::with_detail(MediaErrorCode::DownloadTimeout, "timeout 180s");
        assert_eq!(error.code, MediaErrorCode::DownloadTimeout);
        assert_eq!(error.detail.as_deref(), Some("timeout 180s"));
        assert!(matches!(
            MediaError::from(MediaErrorCode::InvalidMedia).code,
            MediaErrorCode::InvalidMedia
        ));
    }
}
