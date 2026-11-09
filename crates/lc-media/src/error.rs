//! Erreurs du pipeline média, alignées sur les codes historiques de l'application Node.
//!
//! L'application historique expose onze codes d'erreur à l'API (`UNSUPPORTED_SOURCE`,
//! `PRIVATE_OR_AUTH_REQUIRED`, …). Ce module en conserve les noms, leurs priorités et
//! leur classification textuelle, afin que les routes Rust de la Phase 3 puissent
//! rester strictement compatibles avec les clients existants.
//!
//! Convention de message : [`MediaError::message`] contient un message court destiné
//! à l'utilisateur final, [`MediaError::raw_message`] conserve la sortie brute de
//! l'outil (stderr de `ffmpeg`, réponse HTTP, …) tronquée à
//! [`MAX_RAW_MESSAGE_LEN`] caractères pour les journaux.
use std::fmt;
use std::sync::OnceLock;

use regex::Regex;

/// Longueur maximale de la sortie brute conservée dans [`MediaError::raw_message`].
pub const MAX_RAW_MESSAGE_LEN: usize = 2000;

/// Longueur maximale du message court exposé à l'utilisateur.
pub const MAX_MESSAGE_LEN: usize = 500;

/// Code d'erreur média historique, exposé tel quel par l'API legacy.
///
/// L'ordre de déclaration est sans importance : c'est [`MediaErrorCode::priority`]
/// qui départage deux erreurs concurrentes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MediaErrorCode {
    UnsupportedSource,
    PrivateOrAuthRequired,
    DrmProtected,
    MediaNotFound,
    BoardStorageLimitReached,
    CacheStorageLimitReached,
    FileTooLarge,
    InvalidMedia,
    TranscodeFailed,
    DownloadTimeout,
    DownloadFailed,
}

impl MediaErrorCode {
    /// Tous les codes, dans l'ordre décroissant de priorité.
    pub const ALL: [Self; 11] = [
        Self::UnsupportedSource,
        Self::PrivateOrAuthRequired,
        Self::DrmProtected,
        Self::MediaNotFound,
        Self::BoardStorageLimitReached,
        Self::CacheStorageLimitReached,
        Self::FileTooLarge,
        Self::InvalidMedia,
        Self::TranscodeFailed,
        Self::DownloadTimeout,
        Self::DownloadFailed,
    ];

    /// Code textuel historique (MAJUSCULES, identique au legacy).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedSource => "UNSUPPORTED_SOURCE",
            Self::PrivateOrAuthRequired => "PRIVATE_OR_AUTH_REQUIRED",
            Self::DrmProtected => "DRM_PROTECTED",
            Self::MediaNotFound => "MEDIA_NOT_FOUND",
            Self::BoardStorageLimitReached => "BOARD_STORAGE_LIMIT_REACHED",
            Self::CacheStorageLimitReached => "CACHE_STORAGE_LIMIT_REACHED",
            Self::FileTooLarge => "FILE_TOO_LARGE",
            Self::InvalidMedia => "INVALID_MEDIA",
            Self::TranscodeFailed => "TRANSCODE_FAILED",
            Self::DownloadTimeout => "DOWNLOAD_TIMEOUT",
            Self::DownloadFailed => "DOWNLOAD_FAILED",
        }
    }

    /// Priorité historique : la valeur la plus haute gagne lors d'un arbitrage.
    pub const fn priority(self) -> u8 {
        match self {
            Self::UnsupportedSource => 90,
            Self::PrivateOrAuthRequired => 85,
            Self::DrmProtected => 80,
            Self::MediaNotFound => 75,
            Self::BoardStorageLimitReached => 73,
            Self::CacheStorageLimitReached => 72,
            Self::FileTooLarge => 70,
            Self::InvalidMedia => 65,
            Self::TranscodeFailed => 60,
            Self::DownloadTimeout => 55,
            Self::DownloadFailed => 50,
        }
    }

    /// Statut HTTP cible pour une future API média dédiée.
    ///
    /// Attention : les routes legacy `/ingest` et `/overlay` répondent **toujours**
    /// `422 media_ingestion_failed` pour un échec d'ingestion. Cette méthode décrit
    /// le mapping « propre » (400/401/422/503) ; [`Self::legacy_http_status`] et
    /// [`Self::legacy_api_label`] restent la référence pour la parité REST.
    pub const fn http_status(self) -> u16 {
        match self {
            Self::UnsupportedSource | Self::FileTooLarge => 400,
            Self::PrivateOrAuthRequired => 401,
            Self::DrmProtected
            | Self::MediaNotFound
            | Self::InvalidMedia
            | Self::TranscodeFailed
            | Self::DownloadFailed => 422,
            Self::DownloadTimeout
            | Self::CacheStorageLimitReached
            | Self::BoardStorageLimitReached => 503,
        }
    }

    /// Statut HTTP historique : `422` pour absolument tous les échecs d'ingestion.
    pub const fn legacy_http_status(self) -> u16 {
        422
    }

    /// Libellé de champ `error` pour une future API média dédiée.
    pub const fn api_label(self) -> ApiErrorLabel {
        match self.http_status() {
            400 => ApiErrorLabel::InvalidPayload,
            401 => ApiErrorLabel::Unauthorized,
            503 => ApiErrorLabel::IngestApiDisabled,
            _ => ApiErrorLabel::MediaIngestionFailed,
        }
    }

    /// Libellé historique : `media_ingestion_failed` dans tous les cas.
    pub const fn legacy_api_label(self) -> ApiErrorLabel {
        ApiErrorLabel::MediaIngestionFailed
    }

    /// Retrouve un code à partir de sa représentation textuelle.
    pub fn from_code_str(raw: &str) -> Option<Self> {
        let upper = raw.trim().to_ascii_uppercase();
        Self::ALL.into_iter().find(|code| code.as_str() == upper)
    }
}

impl fmt::Display for MediaErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Libellé du champ `error` renvoyé par les routes média.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApiErrorLabel {
    /// `400` — charge utile invalide.
    InvalidPayload,
    /// `401` — authentification requise.
    Unauthorized,
    /// `422` — l'ingestion du média a échoué.
    MediaIngestionFailed,
    /// `503` — l'API d'ingestion n'est pas configurée.
    IngestApiDisabled,
}

impl ApiErrorLabel {
    /// Représentation JSON historique.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidPayload => "invalid_payload",
            Self::Unauthorized => "unauthorized",
            Self::MediaIngestionFailed => "media_ingestion_failed",
            Self::IngestApiDisabled => "ingest_api_disabled",
        }
    }
}

impl fmt::Display for ApiErrorLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Erreur typée du pipeline média.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaError {
    code: MediaErrorCode,
    message: String,
    raw_message: Option<String>,
}

impl MediaError {
    /// Construit une erreur à partir d'un code et d'un message court.
    pub fn new(code: MediaErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: truncate(message.into(), MAX_MESSAGE_LEN),
            raw_message: None,
        }
    }

    /// Construit une erreur en conservant la sortie brute de l'outil.
    pub fn with_raw(
        code: MediaErrorCode,
        message: impl Into<String>,
        raw_message: impl Into<String>,
    ) -> Self {
        let raw = truncate(raw_message.into(), MAX_RAW_MESSAGE_LEN);
        Self {
            code,
            message: truncate(message.into(), MAX_MESSAGE_LEN),
            raw_message: if raw.trim().is_empty() {
                None
            } else {
                Some(raw)
            },
        }
    }

    /// Construit une erreur à partir d'une erreur système (fichier, process, …).
    pub fn from_io(code: MediaErrorCode, context: &str, error: &std::io::Error) -> Self {
        Self::new(code, format!("{context} : {error}"))
    }

    /// Code d'erreur.
    pub fn code(&self) -> MediaErrorCode {
        self.code
    }

    /// Message court destiné à l'utilisateur.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Sortie brute de l'outil ou de la réponse HTTP, si disponible.
    pub fn raw_message(&self) -> Option<&str> {
        self.raw_message.as_deref()
    }

    /// Statut HTTP cible.
    pub fn http_status(&self) -> u16 {
        self.code.http_status()
    }

    /// Statut HTTP historique (`422`).
    pub fn legacy_http_status(&self) -> u16 {
        self.code.legacy_http_status()
    }

    /// Libellé `error` cible.
    pub fn api_label(&self) -> ApiErrorLabel {
        self.code.api_label()
    }

    /// Libellé `error` historique.
    pub fn legacy_api_label(&self) -> ApiErrorLabel {
        self.code.legacy_api_label()
    }

    /// `true` si l'erreur porte ce code.
    pub fn is(&self, code: MediaErrorCode) -> bool {
        self.code == code
    }

    /// Construit une erreur depuis un statut HTTP reçu pendant le téléchargement.
    pub fn from_http_status(status: u16) -> Self {
        let code = code_from_http_status(status);
        Self::new(code, format!("la source a répondu HTTP {status}"))
    }

    /// Compose un résumé journalisable (`CODE: message`).
    pub fn summary(&self) -> String {
        format!("{}: {}", self.code.as_str(), self.message)
    }

    /// Classe un texte brut (stderr d'un outil, corps HTTP, message d'erreur) et
    /// construit l'erreur correspondante.
    pub fn classify(raw: &str, fallback: MediaErrorCode) -> Self {
        let code = classify_code(raw, fallback);
        let message = first_line(raw)
            .map(str::to_string)
            .unwrap_or_else(|| format!("échec de l'ingestion du média ({})", code.as_str()));
        Self::with_raw(code, message, raw)
    }

    /// Construit l'erreur associée à l'échec d'un processus externe.
    pub fn from_command_failure(
        program: &str,
        stdout: &str,
        stderr: &str,
        fallback: MediaErrorCode,
    ) -> Self {
        let raw = join_outputs(stdout, stderr);
        let code = classify_code(&raw, fallback);
        let message = first_line(&raw)
            .map(str::to_string)
            .unwrap_or_else(|| format!("échec du processus {program}"));
        Self::with_raw(code, message, raw)
    }

    /// Conserve l'erreur la plus pertinente des deux (priorité legacy décroissante).
    pub fn most_relevant(self, other: Self) -> Self {
        if other.code.priority() > self.code.priority() {
            other
        } else {
            self
        }
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MediaError {}

/// Résultat du pipeline média.
pub type MediaResult<T> = Result<T, MediaError>;

/// Associe un statut HTTP à un code legacy.
pub fn code_from_http_status(status: u16) -> MediaErrorCode {
    match status {
        401 | 403 => MediaErrorCode::PrivateOrAuthRequired,
        404 | 410 => MediaErrorCode::MediaNotFound,
        408 | 504 => MediaErrorCode::DownloadTimeout,
        413 => MediaErrorCode::FileTooLarge,
        _ => MediaErrorCode::DownloadFailed,
    }
}

/// Classe un texte brut en code legacy, avec un code de repli.
///
/// L'ordre de test suit la priorité historique (DRM, auth, introuvable, timeout,
/// taille, quotas, média invalide, source non supportée, transcodage, téléchargement).
pub fn classify_code(raw: &str, fallback: MediaErrorCode) -> MediaErrorCode {
    let text = raw.to_ascii_lowercase();
    if let Some(status) = extract_http_status(&text) {
        // Les erreurs d'authentification explicites sont prioritaires sur un statut
        // HTTP générique (une page de connexion répond souvent 200/500).
        if contains_auth_signal(&text) {
            return MediaErrorCode::PrivateOrAuthRequired;
        }
        let mapped = code_from_http_status(status);
        if mapped != MediaErrorCode::DownloadFailed {
            return mapped;
        }
    }

    for (signal, code) in SIGNALS {
        if text.contains(signal) {
            return *code;
        }
    }

    // Un statut HTTP non typé (500, 502, …) reste un échec de téléchargement.
    extract_http_status(&text)
        .map(code_from_http_status)
        .unwrap_or(fallback)
}

/// Extrait le premier statut HTTP 4xx/5xx mentionné dans un texte.
pub fn extract_http_status(raw: &str) -> Option<u16> {
    for pattern in http_status_patterns() {
        if let Some(captures) = pattern.captures(raw) {
            if let Some(found) = captures
                .get(1)
                .and_then(|group| group.as_str().parse::<u16>().ok())
            {
                return Some(found);
            }
        }
    }
    None
}

fn http_status_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            "(?i)\\b(?:http(?:/\\d(?:\\.\\d)?)?|status(?:\\s+code)?|error|response)\\s*[:=]?\\s*([45]\\d{2})\\b",
            "(?i)\\b([45]\\d{2})\\s*(?:status|error|forbidden|unauthorized|not found)\\b",
            "(?i)\\b(?:e|err|code)\\s*[:=]\\s*([45]\\d{2})\\b",
        ]
        .into_iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .collect()
    })
}

fn contains_auth_signal(text: &str) -> bool {
    AUTH_SIGNALS.iter().any(|signal| text.contains(signal))
}

fn first_line(raw: &str) -> Option<&str> {
    raw.lines().map(str::trim).find(|line| !line.is_empty())
}

fn join_outputs(stdout: &str, stderr: &str) -> String {
    let mut pieces: Vec<&str> = Vec::new();
    if !stderr.trim().is_empty() {
        pieces.push(stderr.trim());
    }
    if !stdout.trim().is_empty() {
        pieces.push(stdout.trim());
    }
    pieces.join("\n")
}

fn truncate(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value
    } else {
        let mut truncated: String = value.chars().take(max_chars).collect();
        truncated.push('…');
        truncated
    }
}

/// Signaux textuels d'une page d'authentification (même liste que l'ancien serveur).
const AUTH_SIGNALS: &[&str] = &[
    "private video",
    "private media",
    "login required",
    "login is required",
    "log in to tiktok",
    "sign in to tiktok",
    "connecte-toi à tiktok",
    "please login",
    "please log in",
    "audience control",
    "contrôle du public",
    "age-restricted",
    "age restricted",
    "members only",
    "this video is private",
    "forbidden",
];

/// Signaux textuels classés par code, testés dans l'ordre de priorité historique.
const SIGNALS: &[(&str, MediaErrorCode)] = &[
    // DRM (ne jamais tester « libdrm » : les journaux ffmpeg en parlent sans DRM).
    ("drm protected", MediaErrorCode::DrmProtected),
    ("drm-protected", MediaErrorCode::DrmProtected),
    ("widevine", MediaErrorCode::DrmProtected),
    ("playready", MediaErrorCode::DrmProtected),
    ("fairplay", MediaErrorCode::DrmProtected),
    ("encrypted media", MediaErrorCode::DrmProtected),
    ("content is encrypted", MediaErrorCode::DrmProtected),
    // Authentification / contenu privé.
    ("login required", MediaErrorCode::PrivateOrAuthRequired),
    ("sign in to confirm", MediaErrorCode::PrivateOrAuthRequired),
    (
        "confirm you're not a bot",
        MediaErrorCode::PrivateOrAuthRequired,
    ),
    (
        "confirm you are not a bot",
        MediaErrorCode::PrivateOrAuthRequired,
    ),
    ("audience control", MediaErrorCode::PrivateOrAuthRequired),
    ("contrôle du public", MediaErrorCode::PrivateOrAuthRequired),
    (
        "connecte-toi à tiktok",
        MediaErrorCode::PrivateOrAuthRequired,
    ),
    ("log in to tiktok", MediaErrorCode::PrivateOrAuthRequired),
    ("sign in to tiktok", MediaErrorCode::PrivateOrAuthRequired),
    ("members only", MediaErrorCode::PrivateOrAuthRequired),
    ("age-restricted", MediaErrorCode::PrivateOrAuthRequired),
    ("age restricted", MediaErrorCode::PrivateOrAuthRequired),
    ("video is private", MediaErrorCode::PrivateOrAuthRequired),
    ("private video", MediaErrorCode::PrivateOrAuthRequired),
    ("forbidden", MediaErrorCode::PrivateOrAuthRequired),
    // Média introuvable.
    ("video unavailable", MediaErrorCode::MediaNotFound),
    ("media not found", MediaErrorCode::MediaNotFound),
    ("not found", MediaErrorCode::MediaNotFound),
    ("404", MediaErrorCode::MediaNotFound),
    ("410", MediaErrorCode::MediaNotFound),
    // Timeouts.
    ("timed out", MediaErrorCode::DownloadTimeout),
    ("timeout", MediaErrorCode::DownloadTimeout),
    ("etimedout", MediaErrorCode::DownloadTimeout),
    ("econnreset", MediaErrorCode::DownloadTimeout),
    ("network is unreachable", MediaErrorCode::DownloadTimeout),
    // Taille.
    ("payload too large", MediaErrorCode::FileTooLarge),
    ("content too large", MediaErrorCode::FileTooLarge),
    ("file too large", MediaErrorCode::FileTooLarge),
    ("too large", MediaErrorCode::FileTooLarge),
    // Quotas (board avant cache, comme l'ancien serveur).
    (
        "board storage limit",
        MediaErrorCode::BoardStorageLimitReached,
    ),
    (
        "board storage quota",
        MediaErrorCode::BoardStorageLimitReached,
    ),
    (
        "cache storage limit",
        MediaErrorCode::CacheStorageLimitReached,
    ),
    (
        "cache storage quota",
        MediaErrorCode::CacheStorageLimitReached,
    ),
    (
        "storage limit reached",
        MediaErrorCode::CacheStorageLimitReached,
    ),
    // Média invalide ou corrompu.
    (
        "invalid data found when processing input",
        MediaErrorCode::InvalidMedia,
    ),
    ("moov atom not found", MediaErrorCode::InvalidMedia),
    ("invalid or corrupted", MediaErrorCode::InvalidMedia),
    (
        "could not find codec parameters",
        MediaErrorCode::InvalidMedia,
    ),
    ("invalid media", MediaErrorCode::InvalidMedia),
    // Source non supportée.
    ("unsupported url", MediaErrorCode::UnsupportedSource),
    ("unsupported webpage", MediaErrorCode::UnsupportedSource),
    ("no suitable extractor", MediaErrorCode::UnsupportedSource),
    (
        "no suitable info extractor",
        MediaErrorCode::UnsupportedSource,
    ),
    ("unsupported source", MediaErrorCode::UnsupportedSource),
    ("is not a valid url", MediaErrorCode::UnsupportedSource),
    // Transcodage.
    ("conversion failed", MediaErrorCode::TranscodeFailed),
    ("ffprobe", MediaErrorCode::TranscodeFailed),
    ("ffmpeg", MediaErrorCode::TranscodeFailed),
    ("transcod", MediaErrorCode::TranscodeFailed),
    // Téléchargement (le plus générique en dernier).
    ("unable to download", MediaErrorCode::DownloadFailed),
    ("http error", MediaErrorCode::DownloadFailed),
    ("download failed", MediaErrorCode::DownloadFailed),
    ("network error", MediaErrorCode::DownloadFailed),
    ("connection refused", MediaErrorCode::DownloadFailed),
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn legacy_codes_are_complete() {
        assert_eq!(MediaErrorCode::ALL.len(), 11);
        let mut codes: Vec<&'static str> = MediaErrorCode::ALL
            .iter()
            .map(|code| code.as_str())
            .collect();
        codes.sort_unstable();
        assert_eq!(
            codes,
            vec![
                "BOARD_STORAGE_LIMIT_REACHED",
                "CACHE_STORAGE_LIMIT_REACHED",
                "DOWNLOAD_FAILED",
                "DOWNLOAD_TIMEOUT",
                "DRM_PROTECTED",
                "FILE_TOO_LARGE",
                "INVALID_MEDIA",
                "MEDIA_NOT_FOUND",
                "PRIVATE_OR_AUTH_REQUIRED",
                "TRANSCODE_FAILED",
                "UNSUPPORTED_SOURCE",
            ]
        );
    }

    #[test]
    fn priorities_are_strictly_decreasing() {
        let priorities: Vec<u8> = MediaErrorCode::ALL
            .iter()
            .map(|code| code.priority())
            .collect();
        assert_eq!(priorities, vec![90, 85, 80, 75, 73, 72, 70, 65, 60, 55, 50]);
        let mut sorted = priorities.clone();
        sorted.sort_unstable_by(|left, right| right.cmp(left));
        assert_eq!(priorities, sorted);
    }

    #[test]
    fn http_statuses_use_the_four_expected_values() {
        let statuses: Vec<u16> = MediaErrorCode::ALL
            .iter()
            .map(|code| code.http_status())
            .collect();
        assert!(statuses
            .iter()
            .all(|status| matches!(status, 400 | 401 | 422 | 503)));
        assert_eq!(MediaErrorCode::PrivateOrAuthRequired.http_status(), 401);
        assert_eq!(MediaErrorCode::MediaNotFound.http_status(), 422);
        assert_eq!(MediaErrorCode::DownloadTimeout.http_status(), 503);
        assert_eq!(MediaErrorCode::UnsupportedSource.http_status(), 400);
        assert_eq!(MediaErrorCode::FileTooLarge.http_status(), 400);
        assert_eq!(MediaErrorCode::ALL[0].legacy_http_status(), 422);
    }

    #[test]
    fn api_labels_match_statuses() {
        assert_eq!(
            MediaErrorCode::UnsupportedSource.api_label(),
            ApiErrorLabel::InvalidPayload
        );
        assert_eq!(
            MediaErrorCode::PrivateOrAuthRequired.api_label(),
            ApiErrorLabel::Unauthorized
        );
        assert_eq!(
            MediaErrorCode::TranscodeFailed.api_label(),
            ApiErrorLabel::MediaIngestionFailed
        );
        assert_eq!(
            MediaErrorCode::DownloadTimeout.api_label(),
            ApiErrorLabel::IngestApiDisabled
        );
        assert_eq!(
            MediaErrorCode::ALL[0].legacy_api_label(),
            ApiErrorLabel::MediaIngestionFailed
        );
        assert_eq!(
            ApiErrorLabel::MediaIngestionFailed.as_str(),
            "media_ingestion_failed"
        );
    }

    #[test]
    fn classifies_legacy_signals() {
        let cases = [
            (
                "ERROR: This video is DRM protected",
                MediaErrorCode::DrmProtected,
            ),
            (
                "ERROR: [youtube] abc: Private video. Sign in if you've been granted access",
                MediaErrorCode::PrivateOrAuthRequired,
            ),
            (
                "ERROR: [youtube] abc: Video unavailable",
                MediaErrorCode::MediaNotFound,
            ),
            (
                "ERROR: unable to download video data: HTTP Error 404: Not Found",
                MediaErrorCode::MediaNotFound,
            ),
            (
                "ERROR: unable to download webpage: HTTP Error 500: Internal Server Error",
                MediaErrorCode::DownloadFailed,
            ),
            (
                "Read timed out after 180 seconds",
                MediaErrorCode::DownloadTimeout,
            ),
            ("Request entity too large", MediaErrorCode::FileTooLarge),
            (
                "Board storage limit reached for guild",
                MediaErrorCode::BoardStorageLimitReached,
            ),
            (
                "Cache storage limit reached",
                MediaErrorCode::CacheStorageLimitReached,
            ),
            (
                "Invalid data found when processing input",
                MediaErrorCode::InvalidMedia,
            ),
            (
                "ERROR: Unsupported URL: https://example.com",
                MediaErrorCode::UnsupportedSource,
            ),
            ("Conversion failed!", MediaErrorCode::TranscodeFailed),
            (
                "ERROR: unable to download video data",
                MediaErrorCode::DownloadFailed,
            ),
            (
                "something completely different",
                MediaErrorCode::DownloadFailed,
            ),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                classify_code(raw, MediaErrorCode::DownloadFailed),
                expected,
                "raw={raw}"
            );
        }
    }

    #[test]
    fn classifies_http_statuses() {
        assert_eq!(
            classify_code("HTTP 403 Forbidden", MediaErrorCode::DownloadFailed),
            MediaErrorCode::PrivateOrAuthRequired
        );
        assert_eq!(
            classify_code("status: 413", MediaErrorCode::DownloadFailed),
            MediaErrorCode::FileTooLarge
        );
        assert_eq!(
            classify_code("HTTP/1.1 404 Not Found", MediaErrorCode::DownloadFailed),
            MediaErrorCode::MediaNotFound
        );
        assert_eq!(
            classify_code("HTTP 504 Gateway Timeout", MediaErrorCode::DownloadFailed),
            MediaErrorCode::DownloadTimeout
        );
        assert_eq!(
            classify_code("HTTP 502", MediaErrorCode::DownloadFailed),
            MediaErrorCode::DownloadFailed
        );
        assert_eq!(
            classify_code("HTTP/2 429", MediaErrorCode::DownloadFailed),
            MediaErrorCode::DownloadFailed
        );
    }

    #[test]
    fn extract_http_status_only_matches_client_or_server_errors() {
        assert_eq!(extract_http_status("http error 404"), Some(404));
        assert_eq!(
            extract_http_status("HTTP/1.1 503 Service Unavailable"),
            Some(503)
        );
        assert_eq!(extract_http_status("HTTP 200 OK"), None);
        assert_eq!(extract_http_status("aucun statut"), None);
    }

    #[test]
    fn most_relevant_keeps_highest_priority() {
        let low = MediaError::new(MediaErrorCode::DownloadFailed, "down");
        let high = MediaError::new(MediaErrorCode::MediaNotFound, "gone");
        assert_eq!(
            low.clone().most_relevant(high.clone()).code(),
            MediaErrorCode::MediaNotFound
        );
        assert_eq!(
            high.most_relevant(low).code(),
            MediaErrorCode::MediaNotFound
        );
        let same_a = MediaError::new(MediaErrorCode::InvalidMedia, "a");
        let same_b = MediaError::new(MediaErrorCode::InvalidMedia, "b");
        assert_eq!(same_a.most_relevant(same_b).message(), "a");
    }

    #[test]
    fn from_command_failure_keeps_raw_output() {
        let error = MediaError::from_command_failure(
            "ffmpeg",
            "frame=1\n",
            "Error: moov atom not found\nConversion failed!\n",
            MediaErrorCode::TranscodeFailed,
        );
        // « moov atom not found » contient « not found » : comme dans l'ancien
        // serveur, le signal média introuvable prime sur le signal média invalide.
        assert_eq!(error.code(), MediaErrorCode::MediaNotFound);
        assert_eq!(error.message(), "Error: moov atom not found");
        let raw = error.raw_message().unwrap_or_default();
        assert!(raw.contains("Conversion failed!"));
        assert!(raw.contains("frame=1"));
    }

    #[test]
    fn raw_message_is_capped() {
        let long = "x".repeat(MAX_RAW_MESSAGE_LEN + 50);
        let error = MediaError::with_raw(MediaErrorCode::InvalidMedia, "boom", long);
        let raw = error.raw_message().unwrap_or_default();
        assert_eq!(raw.chars().count(), MAX_RAW_MESSAGE_LEN + 1);
        assert!(raw.ends_with('…'));
    }

    #[test]
    fn message_is_capped_and_displayed() {
        let long = "y".repeat(MAX_MESSAGE_LEN + 10);
        let error = MediaError::new(MediaErrorCode::InvalidMedia, long);
        assert_eq!(error.message().chars().count(), MAX_MESSAGE_LEN + 1);
        assert_eq!(error.to_string(), error.message());
        assert!(error.summary().starts_with("INVALID_MEDIA: "));
    }

    #[test]
    fn codes_round_trip_through_text() {
        for code in MediaErrorCode::ALL {
            assert_eq!(MediaErrorCode::from_code_str(code.as_str()), Some(code));
            assert_eq!(
                MediaErrorCode::from_code_str(&code.as_str().to_ascii_lowercase()),
                Some(code)
            );
        }
        assert_eq!(MediaErrorCode::from_code_str("NOPE"), None);
    }
}
