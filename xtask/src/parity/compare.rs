//! Rejeu d'une suite de cas contre une cible et comparaison aux goldens.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::parity::capture::{
    load_golden, load_manifest, normalize_base_url, timestamp, GoldenRecord,
};
use crate::parity::case::{Case, CompareRules};
use crate::parity::normalize::{normalize_body, normalize_text};
use crate::parity::report::{
    CaseResult, CompareReport, Diff, Outcome, Summary, MAX_DIFFS_PER_CASE, REPORT_SCHEMA_VERSION,
};
use crate::parity::request::{
    build_client, execute, probe_rate_limit, requirement_gap, Body, CapturedResponse,
    PreparedRequest, Tokens,
};
use crate::parity::{ParityError, ParityResult};

/// Nom de la copie autoportante de la suite dans un dossier golden.
const CASES_FILE: &str = "cases.json";

/// Paramètres de la sous-commande `parity compare`.
#[derive(Debug, Clone)]
pub struct CompareArgs {
    /// URL de base de la cible.
    pub base_url: String,
    /// Dossier golden contenant `manifest.json` et `cases.json`.
    pub golden: PathBuf,
    /// Emplacement du rapport JSON (optionnel).
    pub report: Option<PathBuf>,
    /// Pointeurs JSON ignorés en plus des règles de cas.
    pub ignore: Vec<String>,
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

impl CompareArgs {
    /// Jetons regroupés pour la préparation des requêtes.
    pub fn tokens(&self) -> Tokens {
        Tokens {
            admin: self.admin_token.clone(),
            overlay: self.overlay_token.clone(),
            ingest: self.ingest_token.clone(),
        }
    }
}

/// Rejoue la suite d'un dossier golden et produit un rapport.
pub async fn run_compare(args: &CompareArgs) -> ParityResult<CompareReport> {
    let manifest = load_manifest(&args.golden)?;
    if manifest.schema_version != crate::parity::capture::GOLDEN_SCHEMA_VERSION {
        return Err(ParityError::golden(format!(
            "manifeste de version {} non supportée (attendu {})",
            manifest.schema_version,
            crate::parity::capture::GOLDEN_SCHEMA_VERSION
        )));
    }

    let suite = load_golden_suite(&args.golden)?;
    let client = build_client(args.timeout)?;
    let tokens = args.tokens();
    let cli_ignore = args.ignore.clone();

    let mut cases = Vec::with_capacity(suite.cases.len());
    for case in &suite.cases {
        cases.push(compare_one(case, args, &client, &tokens, &cli_ignore).await);
    }

    let mut report = CompareReport {
        schema_version: REPORT_SCHEMA_VERSION,
        harness_version: crate::parity::request::HARNESS_VERSION.to_string(),
        generated_at: timestamp(),
        base_url: normalize_base_url(&args.base_url),
        golden_dir: args.golden.display().to_string(),
        golden_label: manifest.suite_label.clone(),
        ignored_pointers: cli_ignore,
        summary: Summary::default(),
        cases,
    };
    report.recount();
    Ok(report)
}

async fn compare_one(
    case: &Case,
    args: &CompareArgs,
    client: &reqwest::Client,
    tokens: &Tokens,
    cli_ignore: &[String],
) -> CaseResult {
    let started = Instant::now();
    let base_result = || CaseResult {
        case_id: case.id.clone(),
        description: case.description.clone(),
        outcome: Outcome::Skipped,
        expected_status: None,
        actual_status: None,
        expected_statuses: case.status_alternatives.clone(),
        diffs: Vec::new(),
        note: case.note.clone(),
        duration_ms: 0,
        error: None,
    };

    let golden = match load_golden(&args.golden, &case.id) {
        Ok(golden) => golden,
        Err(error) => {
            let mut result = base_result();
            result.error = Some(format!("golden absent ou illisible : {error}"));
            result.duration_ms = elapsed_ms(started);
            return result;
        }
    };

    if let Some(reason) = requirement_gap(
        case,
        args.media_id.as_deref(),
        tokens,
        args.rate_limit_probe,
    ) {
        let mut result = base_result();
        result.expected_status = Some(golden.response.status);
        result.error = Some(reason);
        result.duration_ms = elapsed_ms(started);
        return result;
    }

    let rules = case.compare.with_cli_ignore(cli_ignore);
    let request = match PreparedRequest::build(
        case,
        &args.base_url,
        args.media_id.as_deref(),
        tokens,
        args.timeout,
    ) {
        Ok(Some(request)) => request,
        Ok(None) => {
            let mut result = base_result();
            result.expected_status = Some(golden.response.status);
            result.error = Some("variable requise non fournie (media_id)".to_string());
            result.duration_ms = elapsed_ms(started);
            return result;
        }
        Err(error) => {
            let mut result = base_result();
            result.expected_status = Some(golden.response.status);
            result.error = Some(format!("préparation de la requête impossible : {error}"));
            result.duration_ms = elapsed_ms(started);
            return result;
        }
    };

    if case.requires("rate_limit_probe") {
        if let Err(error) = probe_rate_limit(client, &args.base_url, args.timeout).await {
            let mut result = base_result();
            result.expected_status = Some(golden.response.status);
            result.error = Some(format!("sonde de rate limit impossible : {error}"));
            result.duration_ms = elapsed_ms(started);
            return result;
        }
    }

    let actual = match execute(client, &request).await {
        Ok(response) => response,
        Err(error) => {
            let mut result = base_result();
            result.expected_status = Some(golden.response.status);
            result.error = Some(format!("exécution impossible : {error}"));
            result.duration_ms = elapsed_ms(started);
            return result;
        }
    };

    let mut result = compare_case(case, &golden, &actual, &rules);
    result.duration_ms = elapsed_ms(started);
    result
}

/// Compare un golden et une réponse observée et produit le résultat du cas.
pub fn compare_case(
    case: &Case,
    golden: &GoldenRecord,
    actual: &CapturedResponse,
    rules: &CompareRules,
) -> CaseResult {
    let mut diffs = Vec::new();

    let status_matches = actual.status == golden.response.status
        || case.status_alternatives.contains(&actual.status);
    if !status_matches {
        diffs.push(Diff {
            pointer: "/status".to_string(),
            golden: Value::from(golden.response.status),
            actual: Value::from(actual.status),
        });
    }

    for name in &rules.compare_headers {
        let expected = golden
            .response
            .headers
            .get(name)
            .map(|value| Value::String(normalize_text(value)));
        let observed = actual
            .headers
            .get(name)
            .map(|value| Value::String(normalize_text(value)));
        if expected != observed {
            diffs.push(Diff {
                pointer: format!("/headers/{name}"),
                golden: expected.unwrap_or(Value::Null),
                actual: observed.unwrap_or(Value::Null),
            });
        }
    }

    if !rules.ignore_body {
        let expected = normalize_body(&golden.response.body, rules);
        let observed = normalize_body(&actual.body, rules);
        compare_bodies(&expected, &observed, rules, &mut diffs);
    }

    let outcome = if diffs.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail
    };

    CaseResult {
        case_id: case.id.clone(),
        description: case.description.clone(),
        outcome,
        expected_status: Some(golden.response.status),
        actual_status: Some(actual.status),
        expected_statuses: case.status_alternatives.clone(),
        diffs,
        note: case.note.clone(),
        duration_ms: 0,
        error: None,
    }
}

fn compare_bodies(expected: &Body, observed: &Body, rules: &CompareRules, diffs: &mut Vec<Diff>) {
    match (expected, observed) {
        (Body::Empty, Body::Empty) => {}
        (Body::Json { value: expected }, Body::Json { value: observed }) => {
            diff_json("", expected, observed, rules, diffs);
        }
        (Body::Text { value: expected }, Body::Text { value: observed }) => {
            if expected != observed {
                diffs.push(Diff {
                    pointer: "/body".to_string(),
                    golden: Value::String(expected.clone()),
                    actual: Value::String(observed.clone()),
                });
            }
        }
        (expected, observed) => {
            diffs.push(Diff {
                pointer: "/body".to_string(),
                golden: body_to_value(expected),
                actual: body_to_value(observed),
            });
        }
    }
}

fn body_to_value(body: &Body) -> Value {
    match body {
        Body::Empty => Value::Null,
        Body::Json { value } => value.clone(),
        Body::Text { value } => Value::String(value.clone()),
    }
}

fn diff_json(
    pointer: &str,
    expected: &Value,
    observed: &Value,
    rules: &CompareRules,
    diffs: &mut Vec<Diff>,
) {
    if expected == observed || diffs.len() >= MAX_DIFFS_PER_CASE {
        return;
    }
    match (expected, observed) {
        (Value::Object(expected_map), Value::Object(observed_map)) => {
            for (key, expected_value) in expected_map {
                let child_pointer = format!("{pointer}/{}", escape_token(key));
                match observed_map.get(key) {
                    Some(observed_value) => {
                        diff_json(&child_pointer, expected_value, observed_value, rules, diffs)
                    }
                    None => push_diff(diffs, child_pointer, expected_value.clone(), Value::Null),
                }
            }
            if !rules.allow_extra_keys {
                for (key, observed_value) in observed_map {
                    if !expected_map.contains_key(key) {
                        push_diff(
                            diffs,
                            format!("{pointer}/{}", escape_token(key)),
                            Value::Null,
                            observed_value.clone(),
                        );
                    }
                }
            }
        }
        (Value::Array(expected_items), Value::Array(observed_items)) => {
            if expected_items.len() != observed_items.len() {
                push_diff(
                    diffs,
                    format!("{pointer}/length"),
                    Value::from(expected_items.len()),
                    Value::from(observed_items.len()),
                );
            }
            for (index, (expected_item, observed_item)) in
                expected_items.iter().zip(observed_items.iter()).enumerate()
            {
                diff_json(
                    &format!("{pointer}/{index}"),
                    expected_item,
                    observed_item,
                    rules,
                    diffs,
                );
            }
        }
        (expected, observed) => {
            push_diff(
                diffs,
                pointer.to_string(),
                expected.clone(),
                observed.clone(),
            );
        }
    }
}

fn push_diff(diffs: &mut Vec<Diff>, pointer: String, golden: Value, actual: Value) {
    if diffs.len() >= MAX_DIFFS_PER_CASE {
        return;
    }
    diffs.push(Diff {
        pointer,
        golden,
        actual,
    });
}

fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn elapsed_ms(started: Instant) -> u64 {
    let elapsed = started.elapsed();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

fn load_golden_suite(directory: &std::path::Path) -> ParityResult<crate::parity::case::CaseSuite> {
    let path = directory.join(CASES_FILE);
    if path.is_file() {
        return crate::parity::case::load_suite(&path);
    }
    Err(ParityError::golden(format!(
        "suite de cas absente du dossier golden ({}) : recapturez avec `parity capture` en version récente",
        path.display()
    )))
}
