//! Définition déclarative des cas de parité.
//!
//! La suite par défaut est embarquée dans le binaire (`include_str!`) pour que
//! `cargo run -p xtask -- parity …` fonctionne sans fichier externe ; `--cases`
//! permet de rejouer une suite alternative depuis le disque.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::parity::{ParityError, ParityResult};

/// Suite de cas v1 (contrat REST legacy, 26 routes).
pub const EMBEDDED_SUITE_V1: &str = include_str!("../../../docs/rust/parity/cases/v1.json");

/// Suite de cas v2 (contrat cible Rust : évolutions voulues de la Phase 3+).
pub const EMBEDDED_SUITE_V2: &str = include_str!("../../../docs/rust/parity/cases/v2.json");

/// Version de schéma des fichiers de cas et des goldens.
pub const CASE_SCHEMA_VERSION: u32 = 1;

/// Dépendances qu'un cas peut déclarer dans `require`.
pub const KNOWN_REQUIREMENTS: &[&str] = &[
    "media_id",
    "admin_token",
    "overlay_token",
    "ingest_token",
    "rate_limit_probe",
];

/// Identifiant de suite embarquée dans le binaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteId {
    /// Contrat REST legacy (comparaison TS ↔ Rust).
    V1,
    /// Contrat cible v2 (évolutions voulues, référence de non-régression Rust).
    V2,
}

impl SuiteId {
    /// Identifiant textuel (`v1`/`v2`) utilisé par la CLI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }

    /// Analyse `v1`/`v2` (insensible à la casse).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "v1" | "1" => Some(Self::V1),
            "v2" | "2" => Some(Self::V2),
            _ => None,
        }
    }

    /// Texte JSON embarqué de la suite.
    pub const fn embedded_text(self) -> &'static str {
        match self {
            Self::V1 => EMBEDDED_SUITE_V1,
            Self::V2 => EMBEDDED_SUITE_V2,
        }
    }

    /// Charge et valide la suite embarquée.
    pub fn embedded(self) -> ParityResult<CaseSuite> {
        parse_suite(self.embedded_text())
    }

    /// Dossier golden par défaut associé à la suite.
    pub const fn default_golden_dir(self) -> &'static str {
        match self {
            Self::V1 => "docs/rust/parity/golden-v1",
            Self::V2 => "docs/rust/parity/golden-v2",
        }
    }
}

/// Ensemble de cas rejoué contre un serveur.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CaseSuite {
    /// Version du schéma de la suite.
    pub schema_version: u32,
    /// Étiquette lisible (ex. `livechat-v1`).
    pub label: String,
    /// Notes libres sur la suite.
    #[serde(default)]
    pub notes: Option<String>,
    /// Cas à rejouer.
    pub cases: Vec<Case>,
}

/// Un cas de parité : une requête HTTP fixe et ses règles de comparaison.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Case {
    /// Identifiant unique, utilisé comme nom de fichier golden.
    pub id: String,
    /// Description lisible du cas.
    pub description: String,
    /// Méthode HTTP (`GET`, `POST`, `PATCH`, `DELETE`, `HEAD`, `PUT`).
    pub method: String,
    /// Chemin relatif à l'URL de base, query comprise ; `{media_id}` est substitué.
    pub path: String,
    /// Corps JSON éventuel (sérialisé tel quel, `Content-Type: application/json`).
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    /// En-têtes supplémentaires envoyés tels quels.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Type de jeton envoyé dans `Authorization: Bearer …`.
    #[serde(default)]
    pub auth: AuthMode,
    /// Variables requises pour exécuter le cas (`media_id`) ; sinon le cas est ignoré.
    #[serde(default)]
    pub require: Vec<String>,
    /// Règles de normalisation/comparaison du corps et des en-têtes.
    #[serde(default)]
    pub compare: CompareRules,
    /// Statuts supplémentaires tolérés par rapport au golden.
    #[serde(default)]
    pub status_alternatives: Vec<u16>,
    /// Note explicative (divergence connue, état de base…) recopiée dans le rapport.
    #[serde(default)]
    pub note: Option<String>,
}

/// Mode d'authentification appliqué au cas.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// Aucun jeton.
    #[default]
    None,
    /// `Authorization: Bearer <admin-token>`.
    Admin,
    /// `Authorization: Bearer` avec un jeton volontairement invalide.
    AdminInvalid,
    /// `Authorization: Bearer <overlay-token>`.
    Overlay,
    /// `Authorization: Bearer <ingest-token>`.
    Ingest,
}

impl AuthMode {
    /// Indique si le mode exige un jeton fourni sur la ligne de commande.
    pub fn requires_token(self) -> bool {
        matches!(self, Self::Admin | Self::Overlay | Self::Ingest)
    }
}

/// Règles de normalisation et de comparaison appliquées à une réponse.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompareRules {
    /// Pointeurs JSON ignorés (supprimés des deux côtés), `*` et `**` acceptés.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Pointeurs JSON dont les tableaux sont triés avant comparaison.
    #[serde(default)]
    pub sort_arrays: Vec<String>,
    /// Pointeurs JSON dont la valeur est remplacée par `"<volatile>"`.
    #[serde(default)]
    pub volatile: Vec<String>,
    /// Ne compare pas le corps de la réponse (statut seul).
    #[serde(default)]
    pub ignore_body: bool,
    /// Tolère les clés présentes côté cible mais absentes du golden.
    #[serde(default)]
    pub allow_extra_keys: bool,
    /// En-têtes comparés (valeurs normalisées).
    #[serde(default = "default_compare_headers")]
    pub compare_headers: Vec<String>,
}

impl Default for CompareRules {
    fn default() -> Self {
        Self {
            ignore: Vec::new(),
            sort_arrays: Vec::new(),
            volatile: Vec::new(),
            ignore_body: false,
            allow_extra_keys: false,
            compare_headers: default_compare_headers(),
        }
    }
}

fn default_compare_headers() -> Vec<String> {
    vec!["content-type".to_string()]
}

impl CompareRules {
    /// Règles effectives, enrichies des `--ignore` de la ligne de commande.
    pub fn with_cli_ignore(&self, cli_ignore: &[String]) -> Self {
        let mut rules = self.clone();
        rules.ignore.extend(cli_ignore.iter().cloned());
        rules
    }
}

impl Case {
    /// Méthode HTTP en majuscules.
    pub fn method_upper(&self) -> String {
        self.method.to_ascii_uppercase()
    }

    /// Vrai si le cas déclare la dépendance `requirement` (`require`).
    pub fn requires(&self, requirement: &str) -> bool {
        self.require.iter().any(|value| value == requirement)
    }

    /// Chemin avec `{media_id}` substitué par la valeur fournie, s'il y en a une.
    pub fn resolved_path(&self, media_id: Option<&str>) -> Option<String> {
        if !self.path.contains("{media_id}") {
            return Some(self.path.clone());
        }
        let media_id = media_id?;
        Some(self.path.replace("{media_id}", media_id))
    }
}

/// Charge une suite depuis un fichier JSON.
pub fn load_suite(path: &Path) -> ParityResult<CaseSuite> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        ParityError::Io(std::io::Error::new(
            error.kind(),
            format!("impossible de lire la suite {} : {error}", path.display()),
        ))
    })?;
    parse_suite(&text)
}

/// Analyse une suite JSON et la valide.
pub fn parse_suite(text: &str) -> ParityResult<CaseSuite> {
    let suite: CaseSuite = serde_json::from_str(text)?;
    validate_suite(&suite)?;
    Ok(suite)
}

/// Valide une suite : schéma, identifiants uniques, méthodes, chemins,
/// dépendances (`require`), statuts alternatifs et position des sondes.
pub fn validate_suite(suite: &CaseSuite) -> ParityResult<()> {
    if suite.schema_version != CASE_SCHEMA_VERSION {
        return Err(ParityError::golden(format!(
            "suite {} : version de schéma {} non supportée (attendu {})",
            suite.label, suite.schema_version, CASE_SCHEMA_VERSION
        )));
    }

    let mut seen = std::collections::BTreeSet::new();
    for case in &suite.cases {
        if case.id.trim().is_empty() {
            return Err(ParityError::golden("un cas a un identifiant vide"));
        }
        if !case
            .id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            return Err(ParityError::golden(format!(
                "identifiant de cas invalide (alphanumérique, '-' ou '_') : {}",
                case.id
            )));
        }
        if !seen.insert(case.id.clone()) {
            return Err(ParityError::golden(format!(
                "identifiant de cas dupliqué : {}",
                case.id
            )));
        }
        if !case.path.starts_with('/') {
            return Err(ParityError::golden(format!(
                "le chemin du cas {} doit commencer par '/' : {}",
                case.id, case.path
            )));
        }
        if !matches!(
            case.method_upper().as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
        ) {
            return Err(ParityError::golden(format!(
                "méthode HTTP invalide pour le cas {} : {}",
                case.id, case.method
            )));
        }
        for requirement in &case.require {
            if !KNOWN_REQUIREMENTS.contains(&requirement.as_str()) {
                return Err(ParityError::golden(format!(
                    "dépendance inconnue pour le cas {} : {requirement}",
                    case.id
                )));
            }
        }
        for status in &case.status_alternatives {
            if !(100..=599).contains(status) {
                return Err(ParityError::golden(format!(
                    "statut alternatif invalide pour le cas {} : {status}",
                    case.id
                )));
            }
        }
    }

    let probe_index = suite
        .cases
        .iter()
        .position(|case| case.requires("rate_limit_probe"));
    if let Some(index) = probe_index {
        if index + 1 != suite.cases.len() {
            return Err(ParityError::golden(
                "un cas `rate_limit_probe` doit être le dernier de la suite : sa rafale épuise la fenêtre de rate limit",
            ));
        }
    }

    Ok(())
}
