//! Analyse des arguments de la sous-commande `parity`.
//!
//! Parseur volontairement manuel et minimal, cohérent avec le reste de `xtask`
//! (pas de dépendance `clap`).

use std::path::PathBuf;
use std::time::Duration;

use crate::parity::capture::CaptureArgs;
use crate::parity::case::SuiteId;
use crate::parity::compare::CompareArgs;
use crate::parity::request::DEFAULT_TIMEOUT;
use crate::parity::ParityCommand;

/// Aide de la sous-commande `parity`.
pub const PARITY_USAGE: &str = "\
Harnais de parité LiveChat (legacy TypeScript ↔ cible Rust)

USAGE :
  xtask parity capture --base-url <URL> [OPTIONS]
  xtask parity compare --base-url <URL> --golden <DIR> [OPTIONS]
  xtask parity help

COMMANDES :
  capture   Rejoue la suite de cas contre un serveur et écrit les goldens.
  compare   Rejoue la suite golden contre une cible et rapporte les différences.
  help      Affiche cette aide.

OPTIONS COMMUNES :
  --base-url <URL>        URL de base (obligatoire).
  --admin-token <TOKEN>   Jeton administrateur (Bearer).
  --overlay-token <TOKEN> Jeton client overlay (Bearer).
  --ingest-token <TOKEN>  Jeton client ingest (Bearer).
  --media-id <ID>         Identifiant média pour les cas Range (sinon ignorés).
  --rate-limit-probe      Exécute les cas marqués `rate_limit_probe` (sinon ignorés).
  --timeout <SECONDES>    Délai par requête (défaut : 10).

OPTIONS CAPTURE :
  --suite <v1|v2>         Suite embarquée (défaut : v1, contrat legacy).
  --out <DIR>             Dossier golden de sortie (défaut : celui de la suite).
  --cases <FILE>          Suite de cas alternative (défaut : suite embarquée).

OPTIONS COMPARE :
  --golden <DIR>          Dossier golden (obligatoire).
  --report <FILE>         Écrit le rapport JSON à cet emplacement.
  --ignore <JSONPATH>     Pointeur ignoré en plus des règles de cas (répétable).

CODES DE SORTIE :
  0  tous les cas exécutés sont conformes
  1  au moins une différence
  2  erreur de configuration ou d'exécution
";

/// Analyse les arguments de `xtask parity …`.
pub fn parse<I>(args: I) -> Result<ParityCommand, String>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter();
    let Some(subcommand) = iter.next() else {
        return Ok(ParityCommand::Help);
    };
    match subcommand.as_str() {
        "help" | "--help" | "-h" => Ok(ParityCommand::Help),
        "capture" => Ok(ParityCommand::Capture(parse_capture(&mut iter)?)),
        "compare" => Ok(ParityCommand::Compare(parse_compare(&mut iter)?)),
        other => Err(format!(
            "sous-commande parity inconnue : {other} (attendu : capture, compare ou help)"
        )),
    }
}

fn parse_capture(iter: &mut dyn Iterator<Item = String>) -> Result<CaptureArgs, String> {
    let mut base_url: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut suite: Option<SuiteId> = None;
    let mut cases: Option<PathBuf> = None;
    let mut admin_token: Option<String> = None;
    let mut overlay_token: Option<String> = None;
    let mut ingest_token: Option<String> = None;
    let mut media_id: Option<String> = None;
    let mut rate_limit_probe = false;
    let mut timeout: Option<u64> = None;

    while let Some(argument) = iter.next() {
        let (flag, inline) = split_flag(&argument)?;
        match flag {
            "--base-url" => set_once(&mut base_url, take_value(flag, inline, iter)?, flag)?,
            "--out" => set_once(
                &mut out,
                PathBuf::from(take_value(flag, inline, iter)?),
                flag,
            )?,
            "--suite" => {
                let value = take_value(flag, inline, iter)?;
                let parsed = SuiteId::parse(&value)
                    .ok_or_else(|| format!("suite inconnue : {value} (attendu : v1 ou v2)"))?;
                set_once(&mut suite, parsed, flag)?
            }
            "--cases" => set_once(
                &mut cases,
                PathBuf::from(take_value(flag, inline, iter)?),
                flag,
            )?,
            "--admin-token" => set_once(&mut admin_token, take_value(flag, inline, iter)?, flag)?,
            "--overlay-token" => {
                set_once(&mut overlay_token, take_value(flag, inline, iter)?, flag)?
            }
            "--ingest-token" => set_once(&mut ingest_token, take_value(flag, inline, iter)?, flag)?,
            "--media-id" => set_once(&mut media_id, take_value(flag, inline, iter)?, flag)?,
            "--rate-limit-probe" => {
                reject_inline_value(flag, inline)?;
                rate_limit_probe = true;
            }
            "--timeout" => {
                let parsed = parse_seconds(take_value(flag, inline, iter)?)?;
                set_once(&mut timeout, parsed, flag)?
            }
            other => return Err(format!("argument inconnu pour parity capture : {other}")),
        }
    }

    let suite = suite.unwrap_or(SuiteId::V1);
    Ok(CaptureArgs {
        base_url: require(base_url, "--base-url")?,
        out: out.unwrap_or_else(|| PathBuf::from(suite.default_golden_dir())),
        suite,
        cases,
        admin_token,
        overlay_token,
        ingest_token,
        media_id,
        rate_limit_probe,
        timeout: resolve_timeout(timeout)?,
    })
}

fn parse_compare(iter: &mut dyn Iterator<Item = String>) -> Result<CompareArgs, String> {
    let mut base_url: Option<String> = None;
    let mut golden: Option<PathBuf> = None;
    let mut report: Option<PathBuf> = None;
    let mut ignore: Vec<String> = Vec::new();
    let mut admin_token: Option<String> = None;
    let mut overlay_token: Option<String> = None;
    let mut ingest_token: Option<String> = None;
    let mut media_id: Option<String> = None;
    let mut rate_limit_probe = false;
    let mut timeout: Option<u64> = None;

    while let Some(argument) = iter.next() {
        let (flag, inline) = split_flag(&argument)?;
        match flag {
            "--base-url" => set_once(&mut base_url, take_value(flag, inline, iter)?, flag)?,
            "--golden" => set_once(
                &mut golden,
                PathBuf::from(take_value(flag, inline, iter)?),
                flag,
            )?,
            "--report" => set_once(
                &mut report,
                PathBuf::from(take_value(flag, inline, iter)?),
                flag,
            )?,
            "--ignore" => ignore.push(take_value(flag, inline, iter)?),
            "--admin-token" => set_once(&mut admin_token, take_value(flag, inline, iter)?, flag)?,
            "--overlay-token" => {
                set_once(&mut overlay_token, take_value(flag, inline, iter)?, flag)?
            }
            "--ingest-token" => set_once(&mut ingest_token, take_value(flag, inline, iter)?, flag)?,
            "--media-id" => set_once(&mut media_id, take_value(flag, inline, iter)?, flag)?,
            "--rate-limit-probe" => {
                reject_inline_value(flag, inline)?;
                rate_limit_probe = true;
            }
            "--timeout" => {
                let parsed = parse_seconds(take_value(flag, inline, iter)?)?;
                set_once(&mut timeout, parsed, flag)?
            }
            other => return Err(format!("argument inconnu pour parity compare : {other}")),
        }
    }

    Ok(CompareArgs {
        base_url: require(base_url, "--base-url")?,
        golden: require(golden, "--golden")?,
        report,
        ignore,
        admin_token,
        overlay_token,
        ingest_token,
        media_id,
        rate_limit_probe,
        timeout: resolve_timeout(timeout)?,
    })
}

fn split_flag(argument: &str) -> Result<(&str, Option<String>), String> {
    if !argument.starts_with("--") {
        return Err(format!("argument inattendu : {argument}"));
    }
    match argument.split_once('=') {
        Some((flag, value)) => Ok((flag, Some(value.to_string()))),
        None => Ok((argument, None)),
    }
}

fn take_value(
    flag: &str,
    inline: Option<String>,
    iter: &mut dyn Iterator<Item = String>,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value);
    }
    iter.next()
        .ok_or_else(|| format!("valeur manquante pour l'argument {flag}"))
}

fn reject_inline_value(flag: &str, inline: Option<String>) -> Result<(), String> {
    match inline {
        None => Ok(()),
        Some(_) => Err(format!("l'argument {flag} n'accepte pas de valeur")),
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("argument dupliqué : {flag}"));
    }
    *slot = Some(value);
    Ok(())
}

fn require<T>(slot: Option<T>, flag: &'static str) -> Result<T, String> {
    slot.ok_or_else(|| format!("argument obligatoire manquant : {flag}"))
}

fn parse_seconds(value: String) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("durée invalide (entier de secondes attendu) : {value}"))
}

fn resolve_timeout(timeout: Option<u64>) -> Result<Duration, String> {
    match timeout {
        None => Ok(DEFAULT_TIMEOUT),
        Some(seconds) if seconds > 0 => Ok(Duration::from_secs(seconds)),
        Some(_) => Err("--timeout doit être un entier strictement positif".to_string()),
    }
}
