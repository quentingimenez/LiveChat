//! Tests offline du harnais de parité (aucun accès réseau réel).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::parity::capture::{run_capture, CaptureArgs, GoldenManifest, GOLDEN_SCHEMA_VERSION};
use crate::parity::case::{parse_suite, Case, CompareRules, SuiteId};
use crate::parity::cli::parse as parse_cli;
use crate::parity::compare::{compare_case, run_compare, CompareArgs};
use crate::parity::normalize::{
    matches_pattern, normalize_body, normalize_text, normalize_value,
    RATE_LIMIT_REMAINING_PLACEHOLDER, RATE_LIMIT_RESET_PLACEHOLDER, REDACTED_PLACEHOLDER,
    RETRY_AFTER_PLACEHOLDER, STORAGE_PATH_PLACEHOLDER, TIMESTAMP_PLACEHOLDER, UUID_PLACEHOLDER,
    VOLATILE_PLACEHOLDER,
};
use crate::parity::report::{CompareReport, Outcome, Summary};
use crate::parity::request::{
    build_client, execute, filtered_headers, normalize_header_value, probe_rate_limit,
    requirement_gap, Body, CapturedResponse, PreparedRequest, Tokens, CAPTURED_HEADERS,
};
use crate::parity::ParityCommand;

const CASE_TIMEOUT: Duration = Duration::from_secs(5);

/// Réponse canned servie par le faux serveur.
struct MockResponse {
    status: u16,
    content_type: &'static str,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
}

impl MockResponse {
    fn json(status: u16, body: &'static str) -> Self {
        Self {
            status,
            content_type: "application/json",
            headers: Vec::new(),
            body,
        }
    }

    fn text(status: u16, body: &'static str) -> Self {
        Self {
            status,
            content_type: "text/plain",
            headers: Vec::new(),
            body,
        }
    }
}

/// Faux serveur HTTP minimal : sert `responses` dans l'ordre, une connexion par requête.
struct MockServer {
    address: SocketAddr,
}

impl MockServer {
    fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    let lowercase = line.to_ascii_lowercase();
                    if let Some(value) = lowercase.strip_prefix("content-length:") {
                        content_length = value.trim().parse::<usize>().unwrap_or(0);
                    }
                }
                if content_length > 0 {
                    let mut body = vec![0u8; content_length];
                    if reader.read_exact(&mut body).is_err() {
                        continue;
                    }
                }
                let mut payload = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    response.status,
                    reason(response.status),
                    response.content_type,
                    response.body.len()
                );
                for (name, value) in &response.headers {
                    payload.push_str(&format!("{name}: {value}\r\n"));
                }
                payload.push_str("\r\n");
                payload.push_str(response.body);
                if stream.write_all(payload.as_bytes()).is_err() {
                    break;
                }
                if stream.flush().is_err() {
                    break;
                }
            }
        });
        Self { address }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        426 => "Upgrade Required",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

fn suite_json() -> String {
    json!({
        "schema_version": 1,
        "label": "test-suite",
        "notes": "suite de test offline",
        "cases": [
            {
                "id": "root",
                "description": "racine",
                "method": "GET",
                "path": "/",
                "compare": {"compare_headers": ["content-type"]}
            },
            {
                "id": "unknown_route",
                "description": "route inconnue",
                "method": "GET",
                "path": "/nope",
                "compare": {"ignore_body": false, "compare_headers": ["content-type"]}
            },
            {
                "id": "admin_overview",
                "description": "carte admin protégée",
                "method": "GET",
                "path": "/admin/api/overview",
                "auth": "admin"
            }
        ]
    })
    .to_string()
}

fn write_suite(directory: &Path) -> PathBuf {
    let path = directory.join("suite.json");
    std::fs::write(&path, suite_json()).unwrap();
    path
}

fn capture_args(base_url: String, golden: &Path, cases: &Path) -> CaptureArgs {
    CaptureArgs {
        base_url,
        out: golden.to_path_buf(),
        suite: SuiteId::V1,
        cases: Some(cases.to_path_buf()),
        admin_token: None,
        overlay_token: None,
        ingest_token: None,
        media_id: None,
        rate_limit_probe: false,
        timeout: CASE_TIMEOUT,
    }
}

async fn capture_fixture(golden: &Path, server: &MockServer, cases: &Path) {
    let args = capture_args(server.base_url(), golden, cases);
    let report = run_capture(&args).await.unwrap();
    assert_eq!(report.captured, 2);
    assert_eq!(report.skipped, 1);
}

#[test]
fn embedded_suite_is_valid() {
    let suite = SuiteId::V1.embedded().unwrap();
    assert_eq!(suite.label, "livechat-v1");
    assert!(suite.cases.len() >= 30, "{} cas", suite.cases.len());
}

#[test]
fn embedded_v2_suite_is_valid() {
    let suite = SuiteId::V2.embedded().unwrap();
    assert_eq!(suite.label, "livechat-v2");
    assert!(suite.cases.len() >= 10, "{} cas", suite.cases.len());
    let last = suite.cases.last().unwrap();
    assert!(last.requires("rate_limit_probe"), "dernier cas : {last:?}");
    assert_eq!(SuiteId::V2.as_str(), "v2");
    assert_eq!(
        SuiteId::V2.default_golden_dir(),
        "docs/rust/parity/golden-v2"
    );
    assert_eq!(SuiteId::parse("V2"), Some(SuiteId::V2));
    assert_eq!(SuiteId::parse("2"), Some(SuiteId::V2));
    assert_eq!(SuiteId::parse("v3"), None);
}

#[test]
fn suite_validation_rejects_duplicates_and_bad_methods() {
    let duplicate = json!({
        "schema_version": 1,
        "label": "bad",
        "cases": [
            {"id": "a", "description": "a", "method": "GET", "path": "/a"},
            {"id": "a", "description": "b", "method": "GET", "path": "/b"}
        ]
    })
    .to_string();
    assert!(parse_suite(&duplicate).is_err());

    let bad_method = json!({
        "schema_version": 1,
        "label": "bad",
        "cases": [{"id": "a", "description": "a", "method": "TRACE", "path": "/a"}]
    })
    .to_string();
    assert!(parse_suite(&bad_method).is_err());

    let suite = parse_suite(&suite_json()).unwrap();
    assert_eq!(suite.label, "test-suite");
    assert_eq!(suite.cases.len(), 3);
    assert!(suite.cases[2].auth.requires_token());
}

#[test]
fn suite_validation_rejects_unknown_requirements_and_misplaced_probe() {
    let unknown_requirement = json!({
        "schema_version": 1,
        "label": "bad",
        "cases": [{
            "id": "a",
            "description": "a",
            "method": "GET",
            "path": "/a",
            "require": ["media-id"]
        }]
    })
    .to_string();
    assert!(parse_suite(&unknown_requirement).is_err());

    let bad_status = json!({
        "schema_version": 1,
        "label": "bad",
        "cases": [{
            "id": "a",
            "description": "a",
            "method": "GET",
            "path": "/a",
            "status_alternatives": [42]
        }]
    })
    .to_string();
    assert!(parse_suite(&bad_status).is_err());

    let misplaced_probe = json!({
        "schema_version": 1,
        "label": "bad",
        "cases": [
            {
                "id": "probe",
                "description": "probe",
                "method": "GET",
                "path": "/health",
                "require": ["rate_limit_probe"]
            },
            {
                "id": "after",
                "description": "après la sonde",
                "method": "GET",
                "path": "/"
            }
        ]
    })
    .to_string();
    assert!(parse_suite(&misplaced_probe).is_err());
}

#[test]
fn captured_headers_normalize_volatile_values_and_skip_cookies() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-ratelimit-limit", "100".parse().unwrap());
    headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
    headers.insert("x-ratelimit-reset", "1762919280".parse().unwrap());
    headers.insert("retry-after", "42".parse().unwrap());
    headers.insert("x-request-id", "parity-correlation-id".parse().unwrap());
    headers.insert(
        "content-security-policy",
        "default-src 'self'".parse().unwrap(),
    );
    headers.insert("set-cookie", "session=secret".parse().unwrap());

    let captured = filtered_headers(&headers);
    assert_eq!(
        captured.get("x-ratelimit-limit").map(String::as_str),
        Some("100")
    );
    assert_eq!(
        captured.get("x-ratelimit-remaining").map(String::as_str),
        Some(RATE_LIMIT_REMAINING_PLACEHOLDER)
    );
    assert_eq!(
        captured.get("x-ratelimit-reset").map(String::as_str),
        Some(RATE_LIMIT_RESET_PLACEHOLDER)
    );
    assert_eq!(
        captured.get("retry-after").map(String::as_str),
        Some(RETRY_AFTER_PLACEHOLDER)
    );
    assert_eq!(
        captured.get("x-request-id").map(String::as_str),
        Some("parity-correlation-id")
    );
    assert_eq!(
        captured.get("content-security-policy").map(String::as_str),
        Some("default-src 'self'")
    );
    assert!(!captured.contains_key("set-cookie"), "{captured:?}");

    assert_eq!(
        normalize_header_value("x-request-id", "550e8400-e29b-41d4-a716-446655440000"),
        UUID_PLACEHOLDER
    );
    assert_eq!(
        normalize_header_value("x-request-id", "parity-correlation-id"),
        "parity-correlation-id"
    );
    assert!(CAPTURED_HEADERS.contains(&"content-security-policy"));
    assert!(CAPTURED_HEADERS.contains(&"x-request-id"));
}

#[test]
fn requirement_gap_reports_missing_inputs() {
    let mut media_case = sample_case("media", "/overlay/media/{media_id}");
    media_case.require = vec!["media_id".to_string()];
    media_case.auth = crate::parity::case::AuthMode::Overlay;
    let tokens = Tokens::default();

    let gap = requirement_gap(&media_case, None, &tokens, false).unwrap();
    assert!(gap.contains("media_id"), "{gap}");
    let gap = requirement_gap(&media_case, Some("asset"), &tokens, false).unwrap();
    assert!(gap.contains("jeton"), "{gap}");

    let mut probe_case = sample_case("probe", "/health");
    probe_case.require = vec!["rate_limit_probe".to_string()];
    let gap = requirement_gap(&probe_case, None, &Tokens::default(), false).unwrap();
    assert!(gap.contains("rate-limit-probe"), "{gap}");
    assert!(requirement_gap(&probe_case, None, &Tokens::default(), true).is_none());
}

#[tokio::test]
async fn rate_limit_probe_detects_429() {
    let server = MockServer::start(vec![
        MockResponse::json(200, r#"{"status":"ok"}"#),
        MockResponse::json(200, r#"{"status":"ok"}"#),
        MockResponse {
            status: 429,
            content_type: "application/json",
            headers: vec![
                ("x-ratelimit-limit", "100"),
                ("x-ratelimit-remaining", "0"),
                ("retry-after", "12"),
            ],
            body: r#"{"error":"request_error"}"#,
        },
    ]);
    let client = build_client(CASE_TIMEOUT).unwrap();
    let throttled = probe_rate_limit(&client, &server.base_url(), CASE_TIMEOUT)
        .await
        .unwrap();
    assert!(throttled);
}

#[test]
fn normalize_text_replaces_volatile_patterns() {
    let text = "created 2026-11-09T23:12:44.123Z id 550e8400-e29b-41d4-a716-446655440000 \
                auth Bearer abc.def.ghi url https://x.test/media?token=s3cr3t&exp=42";
    let normalized = normalize_text(text);
    assert!(normalized.contains(TIMESTAMP_PLACEHOLDER), "{normalized}");
    assert!(normalized.contains(UUID_PLACEHOLDER), "{normalized}");
    assert!(
        normalized.contains(&format!("Bearer {REDACTED_PLACEHOLDER}")),
        "{normalized}"
    );
    assert!(
        normalized.contains(&format!("token={REDACTED_PLACEHOLDER}")),
        "{normalized}"
    );
    assert!(normalized.contains("exp=<redacted>"), "{normalized}");
    assert!(!normalized.contains("s3cr3t"));

    let storage = normalize_text("C:\\Users\\x\\data\\media\\abc\\video.mp4");
    assert!(storage.contains(STORAGE_PATH_PLACEHOLDER), "{storage}");
}

#[test]
fn normalize_value_applies_rules() {
    let value = json!({
        "a": 1,
        "when": "2026-11-09T23:12:44Z",
        "meta": {"generatedAt": "2026-11-09T23:12:44Z", "version": "3.0.0"},
        "items": [{"n": 2}, {"n": 1}]
    });
    let rules = CompareRules {
        ignore: vec!["/a".to_string()],
        sort_arrays: vec!["/items".to_string()],
        volatile: vec!["/meta/*".to_string()],
        ..CompareRules::default()
    };
    let normalized = normalize_value(&value, "", &rules).unwrap();
    assert_eq!(normalized.get("a"), None);
    assert_eq!(
        normalized["when"],
        Value::String(TIMESTAMP_PLACEHOLDER.to_string())
    );
    assert_eq!(normalized["meta"]["version"], VOLATILE_PLACEHOLDER);
    assert_eq!(normalized["items"][0]["n"], 1);
    assert_eq!(normalized["items"][1]["n"], 2);

    let body = Body::Json { value };
    let normalized_body = normalize_body(&body, &rules);
    assert!(matches!(normalized_body, Body::Json { .. }));
}

#[test]
fn patterns_support_wildcards() {
    assert!(matches_pattern("**/status", "/a/status"));
    assert!(matches_pattern("**/status", "/status"));
    assert!(matches_pattern("/a/*", "/a/b"));
    assert!(!matches_pattern("/a/*", "/a/b/c"));
    assert!(!matches_pattern("/a", "/b"));
}

#[test]
fn body_classification_handles_json_text_and_empty() {
    assert_eq!(Body::from_bytes(b"", Some("application/json")), Body::Empty);
    assert_eq!(
        Body::from_bytes(b" {\"ok\":true} ", Some("application/json")),
        Body::Json {
            value: json!({"ok": true})
        }
    );
    assert_eq!(
        Body::from_bytes(b"hello", Some("text/plain")),
        Body::Text {
            value: "hello".to_string()
        }
    );
}

#[test]
fn compare_case_accepts_status_alternatives() {
    let case = sample_case("alt", "/alt");
    let mut golden = sample_golden(&case);
    golden.response = CapturedResponse {
        status: 404,
        headers: BTreeMap::new(),
        body: Body::Json {
            value: json!({"error": "pairing_code_invalid_or_expired"}),
        },
    };
    let actual = CapturedResponse {
        status: 400,
        headers: BTreeMap::new(),
        body: Body::Json {
            value: json!({"error": "pairing_code_invalid_or_expired"}),
        },
    };
    let mut case = case;
    case.status_alternatives = vec![400];
    let result = compare_case(&case, &golden, &actual, &CompareRules::default());
    assert_eq!(result.outcome, Outcome::Pass, "{:?}", result.diffs);
}

#[test]
fn compare_case_reports_nested_and_extra_key_diffs() {
    let case = sample_case("nested", "/nested");
    let mut golden = sample_golden(&case);
    golden.response = CapturedResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: Body::Json {
            value: json!({"a": {"b": 1}, "list": [1, 2]}),
        },
    };
    let actual = CapturedResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: Body::Json {
            value: json!({"a": {"b": 2}, "list": [1, 2], "extra": true}),
        },
    };
    let result = compare_case(&case, &golden, &actual, &CompareRules::default());
    assert_eq!(result.outcome, Outcome::Fail);
    let pointers: Vec<&str> = result
        .diffs
        .iter()
        .map(|diff| diff.pointer.as_str())
        .collect();
    assert!(pointers.contains(&"/a/b"), "{pointers:?}");
    assert!(pointers.contains(&"/extra"), "{pointers:?}");
}

#[test]
fn prepared_request_resolves_media_id_and_tokens() {
    let suite = parse_suite(
        &json!({
            "schema_version": 1,
            "label": "media",
            "cases": [{
                "id": "media_range",
                "description": "range",
                "method": "GET",
                "path": "/overlay/media/{media_id}",
                "auth": "overlay",
                "require": ["media_id"]
            }]
        })
        .to_string(),
    )
    .unwrap();
    let case = &suite.cases[0];
    let tokens = Tokens {
        overlay: Some("overlay-token".to_string()),
        ..Tokens::default()
    };

    let request = PreparedRequest::build(
        case,
        "http://127.0.0.1:1",
        Some("asset-1"),
        &tokens,
        CASE_TIMEOUT,
    )
    .unwrap()
    .unwrap();
    assert_eq!(request.url, "http://127.0.0.1:1/overlay/media/asset-1");
    assert_eq!(request.token.as_deref(), Some("overlay-token"));

    let skipped =
        PreparedRequest::build(case, "http://127.0.0.1:1", None, &tokens, CASE_TIMEOUT).unwrap();
    assert!(skipped.is_none());
}

#[tokio::test]
async fn execute_does_not_follow_redirects() {
    let server = MockServer::start(vec![MockResponse {
        status: 302,
        content_type: "text/plain",
        headers: vec![("Location", "http://example.test/elsewhere")],
        body: "",
    }]);
    let suite = parse_suite(
        &json!({
            "schema_version": 1,
            "label": "redirect",
            "cases": [{"id": "redir", "description": "redir", "method": "GET", "path": "/x"}]
        })
        .to_string(),
    )
    .unwrap();
    let tokens = Tokens::default();
    let request = PreparedRequest::build(
        &suite.cases[0],
        &server.base_url(),
        None,
        &tokens,
        CASE_TIMEOUT,
    )
    .unwrap()
    .unwrap();
    let client = build_client(CASE_TIMEOUT).unwrap();
    let response = execute(&client, &request).await.unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(
        response.headers.get("location").map(String::as_str),
        Some("http://example.test/elsewhere")
    );
}

#[tokio::test]
async fn capture_writes_goldens_manifest_and_skips_missing_tokens() {
    let temp = TempDir::new().unwrap();
    let golden_dir = temp.path().join("golden");
    let cases = write_suite(temp.path());
    let server = MockServer::start(vec![
        MockResponse::json(200, r#"{"service":"livechat-server","status":"ok"}"#),
        MockResponse::json(404, r#"{"error":"request_error"}"#),
    ]);
    capture_fixture(&golden_dir, &server, &cases).await;

    assert!(golden_dir.join("root.json").is_file());
    assert!(golden_dir.join("unknown_route.json").is_file());
    assert!(golden_dir.join("cases.json").is_file());
    assert!(golden_dir.join("manifest.json").is_file());

    let manifest: GoldenManifest =
        serde_json::from_str(&std::fs::read_to_string(golden_dir.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest.schema_version, GOLDEN_SCHEMA_VERSION);
    assert_eq!(manifest.case_count, 2);
    assert_eq!(manifest.captured.len(), 2);
    assert_eq!(manifest.skipped.len(), 1);
    assert_eq!(manifest.skipped[0].case_id, "admin_overview");
    for entry in &manifest.captured {
        assert_eq!(entry.sha256.len(), 64);
        let bytes = std::fs::read(golden_dir.join(&entry.file)).unwrap();
        assert_eq!(entry.sha256, sha256_hex(&bytes));
    }

    let root_golden: Value =
        serde_json::from_str(&std::fs::read_to_string(golden_dir.join("root.json")).unwrap())
            .unwrap();
    assert_eq!(root_golden["response"]["status"], 200);
    assert_eq!(
        root_golden["response"]["body"]["value"]["service"],
        "livechat-server"
    );
}

#[tokio::test]
async fn compare_roundtrip_passes_then_detects_differences() {
    let temp = TempDir::new().unwrap();
    let golden_dir = temp.path().join("golden");
    let cases = write_suite(temp.path());

    let capture_server = MockServer::start(vec![
        MockResponse::json(200, r#"{"service":"livechat-server","status":"ok"}"#),
        MockResponse::json(404, r#"{"error":"request_error"}"#),
    ]);
    capture_fixture(&golden_dir, &capture_server, &cases).await;

    let identical = MockServer::start(vec![
        MockResponse::json(200, r#"{"service":"livechat-server","status":"ok"}"#),
        MockResponse::json(404, r#"{"error":"request_error"}"#),
    ]);
    let report = compare_against(&identical, &golden_dir).await;
    assert_eq!(report.summary.passed, 2);
    assert_eq!(report.summary.failed, 0);
    assert_eq!(report.summary.skipped, 1);
    assert!(report.is_success());
    assert_eq!(report.summary.total(), 3);

    let divergent = MockServer::start(vec![
        MockResponse::json(200, r#"{"service":"livechat-server","status":"degraded"}"#),
        MockResponse::json(404, r#"{"error":"request_error"}"#),
    ]);
    let report = compare_against(&divergent, &golden_dir).await;
    assert_eq!(report.summary.failed, 1);
    assert!(!report.is_success());
    let failing = report
        .cases
        .iter()
        .find(|case| case.outcome == Outcome::Fail)
        .unwrap();
    assert_eq!(failing.case_id, "root");
    assert!(failing
        .diffs
        .iter()
        .any(|diff| diff.pointer == "/status" || diff.pointer == "/body/status"));
}

#[tokio::test]
async fn compare_report_written_to_disk_and_console_rendered() {
    let temp = TempDir::new().unwrap();
    let golden_dir = temp.path().join("golden");
    let cases = write_suite(temp.path());

    let capture_server = MockServer::start(vec![
        MockResponse::json(200, r#"{"service":"livechat-server","status":"ok"}"#),
        MockResponse::json(404, r#"{"error":"request_error"}"#),
    ]);
    capture_fixture(&golden_dir, &capture_server, &cases).await;

    let server = MockServer::start(vec![
        MockResponse::json(200, r#"{"service":"livechat-server","status":"ok"}"#),
        MockResponse::text(500, "boom"),
    ]);
    let report_path = temp.path().join("report.json");
    let args = CompareArgs {
        base_url: server.base_url(),
        golden: golden_dir.clone(),
        report: Some(report_path.clone()),
        ignore: Vec::new(),
        admin_token: None,
        overlay_token: None,
        ingest_token: None,
        media_id: None,
        rate_limit_probe: false,
        timeout: CASE_TIMEOUT,
    };
    let report = run_compare(&args).await.unwrap();
    crate::parity::report::write_report(&report_path, &report).unwrap();
    assert!(report_path.is_file());
    let rendered = crate::parity::report::render_console(&report);
    assert!(rendered.contains("root"));
    assert!(rendered.contains("PASS"));
    assert!(rendered.contains("FAIL"));

    let reloaded: CompareReport =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(reloaded.summary.failed, report.summary.failed);
    assert_eq!(reloaded.schema_version, 1);
}

async fn compare_against(server: &MockServer, golden_dir: &Path) -> CompareReport {
    let args = CompareArgs {
        base_url: server.base_url(),
        golden: golden_dir.to_path_buf(),
        report: None,
        ignore: Vec::new(),
        admin_token: None,
        overlay_token: None,
        ingest_token: None,
        media_id: None,
        rate_limit_probe: false,
        timeout: CASE_TIMEOUT,
    };
    run_compare(&args).await.unwrap()
}

#[test]
fn cli_parses_capture_and_compare() {
    let command = parse_cli(vec![
        "capture".to_string(),
        "--base-url=http://127.0.0.1:3000".to_string(),
    ])
    .unwrap();
    match command {
        ParityCommand::Capture(args) => {
            assert_eq!(args.base_url, "http://127.0.0.1:3000");
            assert_eq!(args.out, PathBuf::from(SuiteId::V1.default_golden_dir()));
            assert_eq!(args.timeout, Duration::from_secs(10));
        }
        other => panic!("commande inattendue : {other:?}"),
    }

    let command = parse_cli(vec![
        "compare".to_string(),
        "--base-url".to_string(),
        "http://127.0.0.1:3000".to_string(),
        "--golden".to_string(),
        "docs/rust/parity/golden-v1".to_string(),
        "--ignore=/a".to_string(),
        "--ignore".to_string(),
        "/b".to_string(),
    ])
    .unwrap();
    match command {
        ParityCommand::Compare(args) => {
            assert_eq!(args.ignore, vec!["/a".to_string(), "/b".to_string()]);
        }
        other => panic!("commande inattendue : {other:?}"),
    }

    assert!(parse_cli(vec!["capture".to_string()]).is_err());
    assert!(parse_cli(vec!["compare".to_string(), "--base-url=x".to_string()]).is_err());
    assert!(parse_cli(vec!["bogus".to_string()]).is_err());
    assert!(parse_cli(vec![
        "capture".to_string(),
        "--base-url=x".to_string(),
        "--timeout=0".to_string()
    ])
    .is_err());
    assert!(matches!(
        parse_cli(Vec::<String>::new()).unwrap(),
        ParityCommand::Help
    ));

    let command = parse_cli(vec![
        "capture".to_string(),
        "--base-url=x".to_string(),
        "--suite=v2".to_string(),
        "--rate-limit-probe".to_string(),
    ])
    .unwrap();
    match command {
        ParityCommand::Capture(args) => {
            assert_eq!(args.suite, SuiteId::V2);
            assert!(args.rate_limit_probe);
            assert_eq!(args.out, PathBuf::from(SuiteId::V2.default_golden_dir()));
        }
        other => panic!("commande inattendue : {other:?}"),
    }

    let command = parse_cli(vec![
        "compare".to_string(),
        "--base-url=x".to_string(),
        "--golden=docs/rust/parity/golden-v2".to_string(),
        "--rate-limit-probe".to_string(),
    ])
    .unwrap();
    match command {
        ParityCommand::Compare(args) => assert!(args.rate_limit_probe),
        other => panic!("commande inattendue : {other:?}"),
    }

    assert!(parse_cli(vec![
        "capture".to_string(),
        "--base-url=x".to_string(),
        "--suite=v3".to_string()
    ])
    .is_err());
    assert!(parse_cli(vec![
        "capture".to_string(),
        "--base-url=x".to_string(),
        "--rate-limit-probe=true".to_string()
    ])
    .is_err());
}

#[test]
fn summary_pass_rate_and_total() {
    let summary = Summary {
        passed: 3,
        failed: 1,
        skipped: 2,
    };
    assert_eq!(summary.total(), 6);
    assert!((summary.pass_rate() - 75.0).abs() < f64::EPSILON);
    let empty = Summary::default();
    assert!((empty.pass_rate() - 0.0).abs() < f64::EPSILON);
}

fn sample_case(id: &str, path: &str) -> Case {
    Case {
        id: id.to_string(),
        description: id.to_string(),
        method: "GET".to_string(),
        path: path.to_string(),
        body: None,
        headers: BTreeMap::new(),
        auth: crate::parity::case::AuthMode::None,
        require: Vec::new(),
        compare: CompareRules::default(),
        status_alternatives: Vec::new(),
        note: None,
    }
}

fn sample_golden(case: &Case) -> crate::parity::capture::GoldenRecord {
    crate::parity::capture::GoldenRecord {
        schema_version: GOLDEN_SCHEMA_VERSION,
        case_id: case.id.clone(),
        description: case.description.clone(),
        request: crate::parity::capture::RequestRecord {
            method: "GET".to_string(),
            path: case.path.clone(),
        },
        captured_at: "2026-11-09T00:00:00.000Z".to_string(),
        response: CapturedResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: Body::Empty,
        },
        note: None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}
