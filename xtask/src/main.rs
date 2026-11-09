//! Tâches d'outillage du workspace LiveChat : migrateur de données, harnais de parité, release.
//!
//! Sous-commandes disponibles en Phase 2 :
//!
//! - `migrate-legacy --from <legacy.sqlite> --to <nouvelle.sqlite> [--dry-run] [--force]` :
//!   migration one-shot de la base SQLite du serveur legacy (Prisma 5.22) vers le
//!   schéma 3.0 (`docs/rust/schema-v3.sql`). Le déroulé, le format du rapport et
//!   la procédure de rollback sont documentés dans `docs/rust/migrator.md`.
//!
//! `--help` affiche l'aide. La CLI est analysée à la main, volontairement :
//! `clap` n'appartient pas aux dépendances du workspace (une seule sous-commande,
//! analyse triviale à maintenir).
#![forbid(unsafe_code)]

mod migrate;

use std::path::PathBuf;
use std::process::ExitCode;

use migrate::Options;

const USAGE: &str = "\
xtask — outillage du workspace LiveChat 3.0

USAGE :
    cargo run -p xtask -- migrate-legacy --from <legacy.sqlite> --to <nouvelle.sqlite> [--dry-run] [--force]

COMMANDES :
    migrate-legacy    Migre une base SQLite legacy (serveur Prisma 5.22) vers le schéma 3.0.

OPTIONS (migrate-legacy) :
    --from <PATH>     Base legacy à lire (ouverture strictement en lecture seule).
    --to <PATH>       Base cible à créer ; seul le fichier <to>.tmp est écrit puis renommé.
    --dry-run         Valide et affiche le rapport sans rien écrire.
    --force           Autorise le remplacement d'un fichier cible déjà existant.
    -h, --help        Affiche cette aide.

CODES DE SORTIE :
    0  succès ; 1  migration ou vérification échouée ; 2  arguments invalides.
";

/// Sous-commande demandée sur la ligne de commande.
#[derive(Debug)]
enum Command {
    Help,
    MigrateLegacy(Options),
}

/// Erreurs d'analyse de la ligne de commande (code de sortie 2).
#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("commande inconnue : {0} (voir --help)")]
    UnknownCommand(String),
    #[error("argument inconnu pour migrate-legacy : {0} (voir --help)")]
    UnknownArgument(String),
    #[error("valeur manquante pour {0}")]
    MissingValue(&'static str),
    #[error("valeur fournie plusieurs fois pour {0}")]
    DuplicateValue(&'static str),
    #[error("option --from obligatoire pour migrate-legacy")]
    MissingFrom,
    #[error("option --to obligatoire pour migrate-legacy")]
    MissingTo,
}

#[tokio::main]
async fn main() -> ExitCode {
    let command = match parse_args(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("xtask : {err}");
            eprintln!();
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match command {
        Command::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::MigrateLegacy(options) => match migrate::execute(&options).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("xtask migrate-legacy : échec de la migration");
                eprintln!("  {err}");
                ExitCode::from(1)
            }
        },
    }
}

/// Analyse `std::env::args().skip(1)`.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, CliError> {
    let mut args = args.into_iter();
    let Some(subcommand) = args.next() else {
        return Ok(Command::Help);
    };
    match subcommand.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "migrate-legacy" => parse_migrate_legacy(args),
        other => Err(CliError::UnknownCommand(other.to_string())),
    }
}

/// Analyse les arguments de `migrate-legacy`.
fn parse_migrate_legacy(args: impl Iterator<Item = String>) -> Result<Command, CliError> {
    let mut from: Option<PathBuf> = None;
    let mut to: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut force = false;
    let mut args = args;

    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "--from" => {
                if from.is_some() {
                    return Err(CliError::DuplicateValue("--from"));
                }
                from = Some(take_path("--from", inline, &mut args)?);
            }
            "--to" => {
                if to.is_some() {
                    return Err(CliError::DuplicateValue("--to"));
                }
                to = Some(take_path("--to", inline, &mut args)?);
            }
            "--dry-run" => {
                reject_inline_value("--dry-run", inline.as_deref())?;
                dry_run = true;
            }
            "--force" => {
                reject_inline_value("--force", inline.as_deref())?;
                force = true;
            }
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }

    let from = from.ok_or(CliError::MissingFrom)?;
    let to = to.ok_or(CliError::MissingTo)?;
    Ok(Command::MigrateLegacy(Options {
        from,
        to,
        dry_run,
        force,
    }))
}

/// Récupère la valeur d'une option qui attend un chemin.
fn take_path(
    flag: &'static str,
    inline: Option<String>,
    args: &mut impl Iterator<Item = String>,
) -> Result<PathBuf, CliError> {
    match inline {
        Some(value) => Ok(PathBuf::from(value)),
        None => args
            .next()
            .map(PathBuf::from)
            .ok_or(CliError::MissingValue(flag)),
    }
}

/// Refuse une valeur inline (`--dry-run=...`) pour une option qui n'en prend pas.
fn reject_inline_value(flag: &str, inline: Option<&str>) -> Result<(), CliError> {
    match inline {
        Some(_) => Err(CliError::UnknownArgument(format!("{flag}=..."))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn sans_argument_affiche_aide() {
        assert!(matches!(parse_args(args(&[])), Ok(Command::Help)));
        assert!(matches!(parse_args(args(&["--help"])), Ok(Command::Help)));
        assert!(matches!(parse_args(args(&["-h"])), Ok(Command::Help)));
    }

    #[test]
    fn migrate_legacy_complet() {
        let command = parse_args(args(&[
            "migrate-legacy",
            "--from",
            "legacy.sqlite",
            "--to",
            "v3.sqlite",
            "--dry-run",
            "--force",
        ]))
        .expect("arguments valides");
        match command {
            Command::MigrateLegacy(options) => {
                assert_eq!(options.from, PathBuf::from("legacy.sqlite"));
                assert_eq!(options.to, PathBuf::from("v3.sqlite"));
                assert!(options.dry_run);
                assert!(options.force);
            }
            Command::Help => panic!("migrate-legacy attendu"),
        }
    }

    #[test]
    fn migrate_legacy_valeurs_inline() {
        let command = parse_args(args(&[
            "migrate-legacy",
            "--from=legacy.sqlite",
            "--to=v3.sqlite",
        ]))
        .expect("arguments valides");
        match command {
            Command::MigrateLegacy(options) => {
                assert_eq!(options.from, PathBuf::from("legacy.sqlite"));
                assert_eq!(options.to, PathBuf::from("v3.sqlite"));
                assert!(!options.dry_run);
                assert!(!options.force);
            }
            Command::Help => panic!("migrate-legacy attendu"),
        }
    }

    #[test]
    fn migrate_legacy_arguments_manquants() {
        assert!(matches!(
            parse_args(args(&["migrate-legacy", "--to", "v3.sqlite"])),
            Err(CliError::MissingFrom)
        ));
        assert!(matches!(
            parse_args(args(&["migrate-legacy", "--from", "legacy.sqlite"])),
            Err(CliError::MissingTo)
        ));
        assert!(matches!(
            parse_args(args(&["migrate-legacy", "--from"])),
            Err(CliError::MissingValue("--from"))
        ));
    }

    #[test]
    fn migrate_legacy_arguments_invalides() {
        assert!(matches!(
            parse_args(args(&["migrate-legacy", "--inconnu"])),
            Err(CliError::UnknownArgument(_))
        ));
        assert!(matches!(
            parse_args(args(&["migrate-legacy", "--dry-run=oui"])),
            Err(CliError::UnknownArgument(_))
        ));
        assert!(matches!(
            parse_args(args(&["autre-commande"])),
            Err(CliError::UnknownCommand(_))
        ));
        assert!(matches!(
            parse_args(args(&[
                "migrate-legacy",
                "--from",
                "a",
                "--from",
                "b",
                "--to",
                "c"
            ])),
            Err(CliError::DuplicateValue("--from"))
        ));
    }
}
