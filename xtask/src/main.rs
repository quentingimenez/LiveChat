//! Tâches d'outillage du workspace LiveChat : migrateur de données, harnais de parité, release.
//!
//! Sous-commandes disponibles en Phase 2 :
//!
//! - `migrate-legacy --from <legacy.sqlite> --to <nouvelle.sqlite> [--dry-run] [--force]` :
//!   migration one-shot de la base SQLite du serveur legacy (Prisma 5.22) vers le
//!   schéma 3.0 (`docs/rust/schema-v3.sql`). Le déroulé, le format du rapport et
//!   la procédure de rollback sont documentés dans `docs/rust/migrator.md`.
//! - `parity capture|compare` : harnais de parité HTTP entre le serveur legacy
//!   TypeScript et la cible Rust (goldens, normalisation, rapport). Voir
//!   `docs/rust/parity-harness.md`.
//! - `verify [--offline] [--mutants]` : local verification pipeline (fmt,
//!   clippy, tests, doctests, deny, plus mutants with `--mutants`) with one
//!   verdict line per stage; the exit code is the number of failed stages.
//!
//! `--help` affiche l'aide. La CLI est analysée à la main, volontairement :
//! `clap` n'appartient pas aux dépendances du workspace (deux sous-commandes,
//! analyse triviale à maintenir).
#![forbid(unsafe_code)]

mod loadtest;
mod migrate;
mod parity;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;

use migrate::Options;

const USAGE: &str = "\
xtask — outillage du workspace LiveChat 3.0

USAGE :
    cargo run -p xtask -- migrate-legacy --from <legacy.sqlite> --to <nouvelle.sqlite> [--dry-run] [--force]
    cargo run -p xtask -- parity capture --base-url <URL> [OPTIONS]
    cargo run -p xtask -- parity compare --base-url <URL> --golden <DIR> [OPTIONS]
    cargo run -p xtask -- verify [--offline] [--mutants]
    cargo run -p xtask -- loadtest [OPTIONS]

COMMANDES :
    migrate-legacy    Migre une base SQLite legacy (serveur Prisma 5.22) vers le schéma 3.0.
    parity            Harnais de parité HTTP legacy TS ↔ cible Rust (`parity help` pour l'aide).
    verify            Local verification pipeline (fmt, clippy, tests, doctests, deny).
    loadtest          WebSocket load harness: boots a real lc-server, drives
                      overlay clients plus an ingest poster, checks budgets.

OPTIONS (migrate-legacy) :
    --from <PATH>     Base legacy à lire (ouverture strictement en lecture seule).
    --to <PATH>       Base cible à créer ; seul le fichier <to>.tmp est écrit puis renommé.
    --dry-run         Valide et affiche le rapport sans rien écrire.
    --force           Autorise le remplacement d'un fichier cible déjà existant.
    -h, --help        Affiche cette aide.

OPTIONS (verify) :
    --offline         Adds --offline to every cargo stage that accepts it.
    --mutants         Adds the mutants stage: cargo-mutants over the Rust
                      diff since origin/main.
    -h, --help        Shows this help.

OPTIONS (loadtest) :
    --server-bin <PATH>     lc-server binary to launch (default: target/debug/lc-server).
    --clients <N>           Overlay WebSocket clients to drive (default: 50).
    --duration-secs <N>     Load window in seconds (default: 60).
    --ingest-rate <N>       POSTs per second spread over /ingest (default: 20).
    --max-hello-p95-ms <N>  Hello->welcome p95 budget in milliseconds (default: 250).
    --report <PATH>         Where to write the JSON report (also printed to stdout).

CODES DE SORTIE :
    0  succès ; 1  migration ou vérification échouée ; 2  arguments invalides.
    verify : 0 when every executed stage passes, otherwise the number of
    failed stages.
    loadtest: 0 when every budget passes, otherwise 1.
";

/// Sous-commande demandée sur la ligne de commande.
#[derive(Debug)]
enum Command {
    Help,
    MigrateLegacy(Options),
    Parity(parity::ParityCommand),
    Verify(verify::Options),
    Loadtest(loadtest::Options),
}

/// Erreurs d'analyse de la ligne de commande (code de sortie 2).
#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("commande inconnue : {0} (voir --help)")]
    UnknownCommand(String),
    #[error("argument inconnu : {0} (voir --help)")]
    UnknownArgument(String),
    #[error("valeur manquante pour {0}")]
    MissingValue(&'static str),
    #[error("valeur fournie plusieurs fois pour {0}")]
    DuplicateValue(&'static str),
    #[error("option --from obligatoire pour migrate-legacy")]
    MissingFrom,
    #[error("option --to obligatoire pour migrate-legacy")]
    MissingTo,
    #[error("arguments parity invalides : {0} (voir parity help)")]
    ParityArguments(String),
    #[error("invalid value for {0}: {1}")]
    InvalidValue(&'static str, String),
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
        Command::Parity(command) => ExitCode::from(parity::execute(command).await),
        Command::Verify(options) => ExitCode::from(verify::execute(options)),
        Command::Loadtest(options) => ExitCode::from(match loadtest::run(&options).await {
            Ok(code) => code,
            Err(err) => {
                eprintln!("xtask loadtest: {err}");
                1
            }
        }),
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
        "parity" => parity::cli::parse(args)
            .map(Command::Parity)
            .map_err(CliError::ParityArguments),
        "verify" => parse_verify(args),
        "loadtest" => parse_loadtest(args),
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

/// Parses the arguments of `verify`.
fn parse_verify(args: impl Iterator<Item = String>) -> Result<Command, CliError> {
    let mut offline = false;
    let mut mutants = false;

    for arg in args {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "--offline" => {
                reject_inline_value("--offline", inline.as_deref())?;
                if offline {
                    return Err(CliError::DuplicateValue("--offline"));
                }
                offline = true;
            }
            "--mutants" => {
                reject_inline_value("--mutants", inline.as_deref())?;
                if mutants {
                    return Err(CliError::DuplicateValue("--mutants"));
                }
                mutants = true;
            }
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }

    Ok(Command::Verify(verify::Options { offline, mutants }))
}

/// Parses the arguments of `loadtest`.
fn parse_loadtest(args: impl Iterator<Item = String>) -> Result<Command, CliError> {
    let mut server_bin: Option<PathBuf> = None;
    let mut clients: Option<usize> = None;
    let mut duration_secs: Option<u64> = None;
    let mut ingest_rate: Option<u64> = None;
    let mut max_hello_p95_ms: Option<u64> = None;
    let mut report: Option<PathBuf> = None;
    let mut args = args;

    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_string())),
            None => (arg.as_str(), None),
        };
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "--server-bin" => {
                if server_bin.is_some() {
                    return Err(CliError::DuplicateValue("--server-bin"));
                }
                server_bin = Some(take_path("--server-bin", inline, &mut args)?);
            }
            "--clients" => {
                if clients.is_some() {
                    return Err(CliError::DuplicateValue("--clients"));
                }
                clients = Some(take_number("--clients", inline, &mut args)?);
            }
            "--duration-secs" => {
                if duration_secs.is_some() {
                    return Err(CliError::DuplicateValue("--duration-secs"));
                }
                duration_secs = Some(take_number("--duration-secs", inline, &mut args)?);
            }
            "--ingest-rate" => {
                if ingest_rate.is_some() {
                    return Err(CliError::DuplicateValue("--ingest-rate"));
                }
                ingest_rate = Some(take_number("--ingest-rate", inline, &mut args)?);
            }
            "--max-hello-p95-ms" => {
                if max_hello_p95_ms.is_some() {
                    return Err(CliError::DuplicateValue("--max-hello-p95-ms"));
                }
                max_hello_p95_ms = Some(take_number("--max-hello-p95-ms", inline, &mut args)?);
            }
            "--report" => {
                if report.is_some() {
                    return Err(CliError::DuplicateValue("--report"));
                }
                report = Some(take_path("--report", inline, &mut args)?);
            }
            other => return Err(CliError::UnknownArgument(other.to_string())),
        }
    }

    if clients == Some(0) {
        return Err(CliError::InvalidValue("--clients", "at least 1".to_owned()));
    }
    if duration_secs == Some(0) {
        return Err(CliError::InvalidValue(
            "--duration-secs",
            "at least 1".to_owned(),
        ));
    }
    if ingest_rate == Some(0) {
        return Err(CliError::InvalidValue(
            "--ingest-rate",
            "at least 1".to_owned(),
        ));
    }

    Ok(Command::Loadtest(loadtest::Options {
        server_bin: server_bin.unwrap_or_else(default_server_bin),
        clients: clients.unwrap_or(50),
        duration_secs: duration_secs.unwrap_or(60),
        ingest_rate: ingest_rate.unwrap_or(20),
        max_hello_p95_ms: max_hello_p95_ms.unwrap_or(250),
        report,
    }))
}

/// Fetches the value of an option that expects a whole number.
fn take_number<T>(
    flag: &'static str,
    inline: Option<String>,
    args: &mut impl Iterator<Item = String>,
) -> Result<T, CliError>
where
    T: std::str::FromStr,
{
    let raw = match inline {
        Some(value) => value,
        None => args.next().ok_or(CliError::MissingValue(flag))?,
    };
    raw.trim()
        .parse::<T>()
        .map_err(|_| CliError::InvalidValue(flag, raw))
}

/// Default server binary: `target/debug/lc-server` with the platform's
/// executable suffix.
fn default_server_bin() -> PathBuf {
    PathBuf::from("target")
        .join("debug")
        .join(format!("lc-server{}", std::env::consts::EXE_SUFFIX))
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

    use std::path::Path;

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
            _ => panic!("migrate-legacy attendu"),
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
            _ => panic!("migrate-legacy attendu"),
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
    fn parity_dispatch() {
        let command = parse_args(args(&["parity", "help"])).expect("arguments valides");
        match command {
            Command::Parity(parity::ParityCommand::Help) => {}
            other => panic!("commande parity help attendue : {other:?}"),
        }
        assert!(matches!(
            parse_args(args(&["parity", "bogus"])),
            Err(CliError::ParityArguments(_))
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

    #[test]
    fn verify_parses_offline_and_mutants() {
        assert!(matches!(
            parse_args(args(&["verify"])),
            Ok(Command::Verify(verify::Options {
                offline: false,
                mutants: false
            }))
        ));
        assert!(matches!(
            parse_args(args(&["verify", "--offline", "--mutants"])),
            Ok(Command::Verify(verify::Options {
                offline: true,
                mutants: true
            }))
        ));
        assert!(matches!(
            parse_args(args(&["verify", "--mutants", "--offline"])),
            Ok(Command::Verify(verify::Options {
                offline: true,
                mutants: true
            }))
        ));
    }

    #[test]
    fn loadtest_defaults() {
        assert!(matches!(
            parse_args(args(&["loadtest"])),
            Ok(Command::Loadtest(loadtest::Options {
                server_bin,
                clients: 50,
                duration_secs: 60,
                ingest_rate: 20,
                max_hello_p95_ms: 250,
                report: None,
            })) if server_bin == default_server_bin()
        ));
    }

    #[test]
    fn loadtest_parses_every_flag() {
        assert!(matches!(
            parse_args(args(&[
                "loadtest",
                "--server-bin",
                "bin/lc-server.exe",
                "--clients",
                "7",
                "--duration-secs",
                "9",
                "--ingest-rate",
                "3",
                "--max-hello-p95-ms",
                "120",
                "--report",
                "out.json"
            ])),
            Ok(Command::Loadtest(loadtest::Options {
                server_bin,
                clients: 7,
                duration_secs: 9,
                ingest_rate: 3,
                max_hello_p95_ms: 120,
                report: Some(ref path),
            })) if server_bin == Path::new("bin/lc-server.exe")
                && path.as_path() == Path::new("out.json")
        ));
        assert!(matches!(
            parse_args(args(&["loadtest", "--clients=12", "--report=r.json"])),
            Ok(Command::Loadtest(loadtest::Options {
                clients: 12,
                report: Some(ref path),
                ..
            })) if path.as_path() == Path::new("r.json")
        ));
    }

    #[test]
    fn loadtest_rejects_bad_flags() {
        assert!(matches!(
            parse_args(args(&["loadtest", "--full"])),
            Err(CliError::UnknownArgument(_))
        ));
        assert!(matches!(
            parse_args(args(&["loadtest", "--clients"])),
            Err(CliError::MissingValue("--clients"))
        ));
        assert!(matches!(
            parse_args(args(&["loadtest", "--clients", "abc"])),
            Err(CliError::InvalidValue("--clients", _))
        ));
        assert!(matches!(
            parse_args(args(&["loadtest", "--clients", "0"])),
            Err(CliError::InvalidValue("--clients", _))
        ));
        assert!(matches!(
            parse_args(args(&["loadtest", "--clients", "5", "--clients", "6"])),
            Err(CliError::DuplicateValue("--clients"))
        ));
        assert!(matches!(
            parse_args(args(&["loadtest", "--duration-secs", "0"])),
            Err(CliError::InvalidValue("--duration-secs", _))
        ));
        assert!(matches!(
            parse_args(args(&["loadtest", "--ingest-rate", "0"])),
            Err(CliError::InvalidValue("--ingest-rate", _))
        ));
    }

    #[test]
    fn verify_rejects_unknown_flag() {
        assert!(matches!(
            parse_args(args(&["verify", "--full"])),
            Err(CliError::UnknownArgument(_))
        ));
        assert!(matches!(
            parse_args(args(&["verify", "--inconnu"])),
            Err(CliError::UnknownArgument(_))
        ));
        assert!(matches!(
            parse_args(args(&["verify", "--offline=oui"])),
            Err(CliError::UnknownArgument(_))
        ));
        assert!(matches!(
            parse_args(args(&["verify", "--offline", "--offline"])),
            Err(CliError::DuplicateValue("--offline"))
        ));
    }

    #[test]
    fn default_server_bin_pins_the_debug_path() {
        #[cfg(windows)]
        {
            assert_eq!(
                default_server_bin(),
                PathBuf::from(r"target\debug\lc-server.exe")
            );
        }
        #[cfg(unix)]
        {
            assert_eq!(
                default_server_bin(),
                PathBuf::from("target/debug/lc-server")
            );
        }
    }
}
