//! Rapport de comparaison : structure JSON et rendu console.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::parity::{ParityError, ParityResult};

/// Version de schéma du rapport.
pub const REPORT_SCHEMA_VERSION: u32 = 1;
/// Nombre maximal de différences détaillées par cas.
pub const MAX_DIFFS_PER_CASE: usize = 20;

/// Résultat d'un cas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// La cible correspond au golden.
    Pass,
    /// La cible diverge du golden.
    Fail,
    /// Le cas n'a pas pu être rejoué (jeton ou variable manquants).
    Skipped,
}

impl Outcome {
    /// Libellé console.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Skipped => "SKIP",
        }
    }
}

/// Différence ponctuelle entre golden et réponse observée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    /// Pointeur JSON (ou nom de champ d'en-tête/statut) concerné.
    pub pointer: String,
    /// Valeur attendue (issue du golden).
    pub golden: Value,
    /// Valeur observée sur la cible.
    pub actual: Value,
}

/// Résultat détaillé d'un cas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResult {
    /// Identifiant du cas.
    pub case_id: String,
    /// Description du cas.
    pub description: String,
    /// Verdict.
    pub outcome: Outcome,
    /// Statut attendu par le golden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_status: Option<u16>,
    /// Statut observé sur la cible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_status: Option<u16>,
    /// Statuts alternatifs tolérés.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_statuses: Vec<u16>,
    /// Différences structurelles ou de valeurs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diffs: Vec<Diff>,
    /// Note explicative du cas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Durée de l'exécution en millisecondes.
    pub duration_ms: u64,
    /// Erreur d'exécution éventuelle (réseau, requête refusée…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Compteurs du rapport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Cas exécutés et conformes.
    pub passed: usize,
    /// Cas exécutés et divergents.
    pub failed: usize,
    /// Cas non exécutés.
    pub skipped: usize,
}

impl Summary {
    /// Nombre total de cas de la suite.
    pub fn total(&self) -> usize {
        self.passed + self.failed + self.skipped
    }

    /// Taux de réussite parmi les cas exécutés.
    pub fn pass_rate(&self) -> f64 {
        let executed = self.passed + self.failed;
        if executed == 0 {
            return 0.0;
        }
        (self.passed as f64) * 100.0 / (executed as f64)
    }
}

/// Rapport complet d'une comparaison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareReport {
    /// Version du schéma de rapport.
    pub schema_version: u32,
    /// Version du harnais.
    pub harness_version: String,
    /// Date de génération (RFC 3339, UTC).
    pub generated_at: String,
    /// URL de base de la cible.
    pub base_url: String,
    /// Dossier golden utilisé.
    pub golden_dir: String,
    /// Étiquette de la suite golden.
    pub golden_label: String,
    /// Pointeurs ignorés via la ligne de commande.
    pub ignored_pointers: Vec<String>,
    /// Compteurs.
    pub summary: Summary,
    /// Résultats par cas, dans l'ordre de la suite.
    pub cases: Vec<CaseResult>,
}

impl CompareReport {
    /// Indique si tous les cas exécutés sont conformes.
    pub fn is_success(&self) -> bool {
        self.summary.failed == 0
    }

    /// Recalcule les compteurs depuis les résultats.
    pub fn recount(&mut self) {
        let mut summary = Summary::default();
        for case in &self.cases {
            match case.outcome {
                Outcome::Pass => summary.passed += 1,
                Outcome::Fail => summary.failed += 1,
                Outcome::Skipped => summary.skipped += 1,
            }
        }
        self.summary = summary;
    }
}

/// Écrit le rapport JSON sur disque.
pub fn write_report(path: &Path, report: &CompareReport) -> ParityResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut text = serde_json::to_string_pretty(report)?;
    text.push('\n');
    std::fs::write(path, text).map_err(|error| {
        ParityError::Io(std::io::Error::new(
            error.kind(),
            format!("écriture du rapport {} : {error}", path.display()),
        ))
    })
}

/// Rend la table console d'un rapport.
pub fn render_console(report: &CompareReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Parité {} ↔ {} ({} cas)",
        report.golden_label,
        report.base_url,
        report.summary.total()
    ));
    lines.push(String::new());
    lines.push(format!(
        "{:<40} {:<6} {:<8} {:<8} {}",
        "cas", "résultat", "attendu", "reçu", "détail"
    ));
    lines.push("-".repeat(96));

    for case in &report.cases {
        let expected = case
            .expected_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "-".to_string());
        let actual = case
            .actual_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "-".to_string());
        let detail = match case.outcome {
            Outcome::Pass => case.note.clone().unwrap_or_default(),
            Outcome::Skipped => case
                .error
                .clone()
                .or_else(|| case.note.clone())
                .unwrap_or_default(),
            Outcome::Fail => {
                if let Some(error) = &case.error {
                    error.clone()
                } else {
                    case.diffs.first().map(describe_diff).unwrap_or_default()
                }
            }
        };
        lines.push(format!(
            "{:<40} {:<6} {:<8} {:<8} {}",
            truncate(&case.case_id, 40),
            case.outcome.label(),
            expected,
            actual,
            truncate(&detail, 120)
        ));
        if case.outcome == Outcome::Fail {
            for diff in case.diffs.iter().skip(1) {
                lines.push(format!("    ↳ {}", truncate(&describe_diff(diff), 150)));
            }
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "{} passés, {} échecs, {} ignorés — {:.1} % de réussite",
        report.summary.passed,
        report.summary.failed,
        report.summary.skipped,
        report.summary.pass_rate()
    ));
    lines.join("\n")
}

fn describe_diff(diff: &Diff) -> String {
    format!(
        "{} : attendu {}, reçu {}",
        diff.pointer,
        short_value(&diff.golden),
        short_value(&diff.actual)
    )
}

fn short_value(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    truncate(&text, 80)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}
