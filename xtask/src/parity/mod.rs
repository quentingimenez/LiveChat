//! Harnais de parité entre le serveur legacy TypeScript et la cible Rust.
//!
//! Deux commandes, volontairement séparées :
//!
//! - `parity capture` rejoue une suite de cas fixe contre un serveur (legacy ou
//!   cible) et écrit un golden par cas dans un dossier versionné ;
//! - `parity compare` rejoue la suite golden contre une cible, normalise les
//!   réponses puis compare statut, en-têtes filtrés et corps, en produisant un
//!   rapport lisible et un code de sortie exploitable en CI.
//!
//! Les cas sont déclarés dans `docs/rust/parity/cases/v1.json` (suite embarquée
//! dans le binaire). La capture stocke les réponses brutes redactées ; la
//! normalisation n'intervient qu'à la comparaison.

pub mod capture;
pub mod case;
pub mod cli;
pub mod compare;
pub mod normalize;
pub mod report;
pub mod request;

#[cfg(test)]
mod tests;

use crate::parity::capture::CaptureArgs;
use crate::parity::compare::CompareArgs;

/// Code de sortie : tous les cas exécutés sont conformes.
pub const EXIT_PASS: u8 = 0;
/// Code de sortie : au moins une différence détectée.
pub const EXIT_DIFFERENCES: u8 = 1;
/// Code de sortie : erreur de configuration ou d'exécution.
pub const EXIT_ERROR: u8 = 2;

/// Sous-commande `parity` analysée.
#[derive(Debug)]
pub enum ParityCommand {
    /// Aide.
    Help,
    /// Capture des goldens.
    Capture(CaptureArgs),
    /// Comparaison aux goldens.
    Compare(CompareArgs),
}

/// Erreur du harnais de parité.
#[derive(Debug, thiserror::Error)]
pub enum ParityError {
    /// Erreur d'entrée/sortie.
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
    /// Erreur JSON.
    #[error("JSON : {0}")]
    Json(#[from] serde_json::Error),
    /// Argument de ligne de commande invalide.
    #[error("argument : {0}")]
    Argument(String),
    /// Golden ou suite invalide.
    #[error("golden : {0}")]
    Golden(String),
    /// Erreur d'exécution HTTP.
    #[error("HTTP : {0}")]
    Http(String),
}

impl ParityError {
    /// Construit une erreur d'argument.
    pub fn argument(message: impl Into<String>) -> Self {
        Self::Argument(message.into())
    }

    /// Construit une erreur de golden.
    pub fn golden(message: impl Into<String>) -> Self {
        Self::Golden(message.into())
    }

    /// Construit une erreur d'exécution HTTP.
    pub fn http(message: impl Into<String>) -> Self {
        Self::Http(message.into())
    }
}

/// Résultat d'une opération de parité.
pub type ParityResult<T> = Result<T, ParityError>;

/// Exécute une sous-commande et renvoie le code de sortie associé.
pub async fn execute(command: ParityCommand) -> u8 {
    match command {
        ParityCommand::Help => {
            println!("{}", cli::PARITY_USAGE);
            EXIT_PASS
        }
        ParityCommand::Capture(args) => match capture::run_capture(&args).await {
            Ok(report) => {
                println!(
                    "parity capture ({}) : {} goldens écrits, {} cas ignorés → {}",
                    args.suite.as_str(),
                    report.captured,
                    report.skipped,
                    args.out.display()
                );
                EXIT_PASS
            }
            Err(error) => {
                eprintln!("parity capture : {error}");
                EXIT_ERROR
            }
        },
        ParityCommand::Compare(args) => match compare::run_compare(&args).await {
            Ok(report) => {
                println!("{}", report::render_console(&report));
                if let Some(path) = &args.report {
                    if let Err(error) = report::write_report(path, &report) {
                        eprintln!("parity compare : {error}");
                        return EXIT_ERROR;
                    }
                    println!("rapport écrit : {}", path.display());
                }
                if report.is_success() {
                    EXIT_PASS
                } else {
                    EXIT_DIFFERENCES
                }
            }
            Err(error) => {
                eprintln!("parity compare : {error}");
                EXIT_ERROR
            }
        },
    }
}
