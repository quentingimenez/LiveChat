//! Capture des réponses d'un serveur (legacy TS ou cible Rust) dans des goldens.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::parity::case::{load_suite, CaseSuite, SuiteId};
use crate::parity::request::{
    build_client, execute, probe_rate_limit, requirement_gap, CapturedResponse, PreparedRequest,
    Tokens, HARNESS_VERSION,
};
use crate::parity::{ParityError, ParityResult};

/// Version de schéma des goldens et des manifestes.
pub const GOLDEN_SCHEMA_VERSION: u32 = 1;
/// Nom du fichier de manifeste.
pub const MANIFEST_FILE: &str = "manifest.json";
/// Nom de la copie autoportante de la suite.
pub const CASES_FILE: &str = "cases.json";

/// Golden d'un cas : requête, réponse brute redactée et métadonnées.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenRecord {
    /// Version du schéma golden.
    pub schema_version: u32,
    /// Identifiant du cas.
    pub case_id: String,
    /// Description du cas.
    pub description: String,
    /// Requête rejouée.
    pub request: RequestRecord,
    /// Date de capture (RFC 3339, UTC).
    pub captured_at: String,
    /// Réponse capturée.
    pub response: CapturedResponse,
    /// Note du cas (divergence connue…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Requête effectivement envoyée (sans jeton ni en-tête sensible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    /// Méthode HTTP.
    pub method: String,
    /// Chemin et query.
    pub path: String,
}

/// Manifeste du dossier golden.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenManifest {
    /// Version du schéma de manifeste.
    pub schema_version: u32,
    /// Version du binaire `xtask` ayant produit la capture.
    pub harness_version: String,
    /// Étiquette de la suite de cas.
    pub suite_label: String,
    /// Date de capture (RFC 3339, UTC).
    pub captured_at: String,
    /// URL de base utilisée pour la capture (sans secret).
    pub base_url: String,
    /// Nombre de cas capturés.
    pub case_count: usize,
    /// Détail des goldens écrits.
    pub captured: Vec<ManifestEntry>,
    /// Cas non exécutés et raison.
    pub skipped: Vec<ManifestSkip>,
}

/// Entrée du manifeste pour un golden.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Identifiant du cas.
    pub case_id: String,
    /// Nom du fichier golden.
    pub file: String,
    /// SHA-256 hexadécimal du fichier.
    pub sha256: String,
}

/// Cas volontairement non capturé.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestSkip {
    /// Identifiant du cas.
    pub case_id: String,
    /// Raison du saut (jeton manquant, variable absente…).
    pub reason: String,
}

/// Paramètres de la sous-commande `parity capture`.
#[derive(Debug, Clone)]
pub struct CaptureArgs {
    /// URL de base du serveur cible.
    pub base_url: String,
    /// Dossier de sortie des goldens.
    pub out: PathBuf,
    /// Suite embarquée utilisée quand `cases` est absent.
    pub suite: SuiteId,
    /// Suite alternative de cas (défaut : suite embarquée choisie).
    pub cases: Option<PathBuf>,
    /// Jeton administrateur.
    pub admin_token: Option<String>,
    /// Jeton overlay.
    pub overlay_token: Option<String>,
    /// Jeton ingest.
    pub ingest_token: Option<String>,
    /// Identifiant média utilisé pour les cas Range.
    pub media_id: Option<String>,
    /// Active la sonde de rate limit pour les cas qui la déclarent.
    pub rate_limit_probe: bool,
    /// Délai par requête.
    pub timeout: Duration,
}

impl CaptureArgs {
    /// Jetons regroupés pour la préparation des requêtes.
    pub fn tokens(&self) -> Tokens {
        Tokens {
            admin: self.admin_token.clone(),
            overlay: self.overlay_token.clone(),
            ingest: self.ingest_token.clone(),
        }
    }
}

/// Compte rendu d'une capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureReport {
    /// Nombre de goldens écrits.
    pub captured: usize,
    /// Nombre de cas sautés.
    pub skipped: usize,
}

/// Exécute la capture d'une suite complète dans un dossier golden.
pub async fn run_capture(args: &CaptureArgs) -> ParityResult<CaptureReport> {
    let suite = load_capture_suite(args)?;
    let client = build_client(args.timeout)?;
    let tokens = args.tokens();
    let captured_at = timestamp();

    std::fs::create_dir_all(&args.out)?;
    write_suite_copy(&args.out, &suite)?;

    let mut report = CaptureReport::default();
    let mut entries = Vec::new();
    let mut skipped = Vec::new();

    for case in &suite.cases {
        if let Some(reason) = requirement_gap(
            case,
            args.media_id.as_deref(),
            &tokens,
            args.rate_limit_probe,
        ) {
            skipped.push(ManifestSkip {
                case_id: case.id.clone(),
                reason,
            });
            continue;
        }
        let request = match PreparedRequest::build(
            case,
            &args.base_url,
            args.media_id.as_deref(),
            &tokens,
            args.timeout,
        )? {
            Some(request) => request,
            None => {
                skipped.push(ManifestSkip {
                    case_id: case.id.clone(),
                    reason: "variable requise non fournie (media_id)".to_string(),
                });
                continue;
            }
        };

        if case.requires("rate_limit_probe") {
            probe_rate_limit(&client, &args.base_url, args.timeout).await?;
        }

        let response = execute(&client, &request).await?;
        let record = GoldenRecord {
            schema_version: GOLDEN_SCHEMA_VERSION,
            case_id: case.id.clone(),
            description: case.description.clone(),
            request: RequestRecord {
                method: request.method.clone(),
                path: request.path.clone(),
            },
            captured_at: captured_at.clone(),
            response: response.redacted(),
            note: case.note.clone(),
        };
        let file_name = golden_file_name(&case.id);
        let bytes = write_json(&args.out.join(&file_name), &record)?;
        entries.push(ManifestEntry {
            case_id: case.id.clone(),
            file: file_name,
            sha256: sha256_hex(&bytes),
        });
        report.captured += 1;
    }

    report.skipped = skipped.len();
    let manifest = GoldenManifest {
        schema_version: GOLDEN_SCHEMA_VERSION,
        harness_version: HARNESS_VERSION.to_string(),
        suite_label: suite.label.clone(),
        captured_at,
        base_url: normalize_base_url(&args.base_url),
        case_count: report.captured,
        captured: entries,
        skipped,
    };
    write_json(&args.out.join(MANIFEST_FILE), &manifest)?;

    Ok(report)
}

/// Charge la suite de capture depuis `args.cases` ou la suite embarquée choisie.
pub fn load_capture_suite(args: &CaptureArgs) -> ParityResult<CaseSuite> {
    match &args.cases {
        Some(path) => load_suite(path),
        None => args.suite.embedded(),
    }
}

/// Nom de fichier golden d'un cas (`<id>.json`).
pub fn golden_file_name(case_id: &str) -> String {
    format!("{case_id}.json")
}

fn write_suite_copy(directory: &Path, suite: &CaseSuite) -> ParityResult<()> {
    let path = directory.join(CASES_FILE);
    write_json(&path, suite)?;
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> ParityResult<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    let bytes = text.into_bytes();
    std::fs::write(path, &bytes).map_err(|error| {
        ParityError::Io(std::io::Error::new(
            error.kind(),
            format!("écriture de {} : {error}", path.display()),
        ))
    })?;
    Ok(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Horodatage RFC 3339 en millisecondes, UTC.
pub fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// URL de base sans slash final, sans partie sensible.
pub fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
}

/// Charge un manifeste golden.
pub fn load_manifest(directory: &Path) -> ParityResult<GoldenManifest> {
    let path = directory.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        ParityError::Io(std::io::Error::new(
            error.kind(),
            format!(
                "impossible de lire le manifeste {} : {error}",
                path.display()
            ),
        ))
    })?;
    let manifest: GoldenManifest = serde_json::from_str(&text)?;
    Ok(manifest)
}

/// Charge un golden de cas.
pub fn load_golden(directory: &Path, case_id: &str) -> ParityResult<GoldenRecord> {
    let path = directory.join(golden_file_name(case_id));
    let text = std::fs::read_to_string(&path).map_err(|error| {
        ParityError::Io(std::io::Error::new(
            error.kind(),
            format!("impossible de lire le golden {} : {error}", path.display()),
        ))
    })?;
    let record: GoldenRecord = serde_json::from_str(&text)?;
    Ok(record)
}
