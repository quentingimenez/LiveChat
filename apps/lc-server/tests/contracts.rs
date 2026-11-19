//! HTTP contract tests of the public server surface.
//!
//! These integration tests boot the real `lc_server` router on an ephemeral
//! loopback port and pin the externally observable wire contracts:
//!
//! - `GET /health` always answers HTTP 200 with an exact set of seven JSON
//!   keys, whatever the state of the optional dependencies;
//! - an anonymous `GET /admin/api/overview` (no session cookie) is rejected
//!   with HTTP 401 and the exact `unauthorized` JSON body.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use lc_server::{router, AppState, Config, Shutdown};

/// Secret admin token that configures the fallback login method.
///
/// Without at least one configured login method, `/admin/api/**` answers
/// 503 `admin_ui_disabled` before any session check runs (see
/// `auth_503_when_no_method_is_configured` in the admin unit tests), so the
/// anonymous-401 contract needs a configured method to be reachable.
const ADMIN_UI_TOKEN: &str = "contract-test-admin-token";

/// Exact, sorted key set of the `GET /health` JSON body.
const HEALTH_KEYS: [&str; 7] = [
    "activeJobs",
    "db",
    "discord",
    "protocolVersion",
    "status",
    "uptimeSec",
    "version",
];

/// Owns the temporary assets of one contract server.
///
/// Dropping the guard aborts the serve task and releases the temporary
/// directory; the bound address is handed to the caller instead.
struct ServerGuard {
    serve: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.serve.abort();
    }
}

/// Boots the full server router on an ephemeral loopback port.
///
/// `db` selects whether a SQLite database is attached to the state; the
/// config is otherwise the minimal documented pair set (`API_URL`,
/// `NODE_ENV`) plus the admin token that makes the login-rejection contract
/// reachable. Returns the bound address and a guard whose `Drop` aborts the
/// serve task.
async fn spin_server(db: Option<lc_db::Db>) -> (SocketAddr, ServerGuard) {
    let dir = tempfile::tempdir().expect("temporary directory");
    let pairs = [
        ("API_URL".to_owned(), "http://127.0.0.1:3000".to_owned()),
        ("ADMIN_UI_TOKEN".to_owned(), ADMIN_UI_TOKEN.to_owned()),
        ("NODE_ENV".to_owned(), "test".to_owned()),
    ];
    let config = Arc::new(Config::from_pairs(pairs).expect("valid test configuration"));
    let shutdown = Shutdown::new().0;
    let state = AppState::new(Arc::clone(&config), db, shutdown);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind of the contract server");
    let addr = listener.local_addr().expect("local address");
    let serve = tokio::spawn(async move {
        drop(
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await,
        );
    });

    // `bind` returned, but the spawned serve task still gets a bounded
    // window to prove that it actually answers before the caller proceeds.
    let client = reqwest::Client::new();
    let health_url = format!("http://{addr}/health");
    let mut answered = false;
    for _ in 0..100 {
        if client.get(&health_url).send().await.is_ok() {
            answered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(answered, "contract server never answered /health at {addr}");

    (addr, ServerGuard { serve, _dir: dir })
}

/// Asserts the exact documented key set of a `GET /health` body.
fn assert_health_key_set(body: &Value) {
    let mut keys: Vec<String> = body
        .as_object()
        .expect("health body must be a JSON object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    let expected: Vec<String> = HEALTH_KEYS.iter().map(|key| (*key).to_owned()).collect();
    assert_eq!(
        keys, expected,
        "GET /health must expose exactly the documented keys"
    );
}

#[tokio::test]
async fn health_contract_with_a_database() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let db_path = dir.path().join("contract.sqlite");
    let db = lc_db::Db::open(db_path, &lc_db::DbConfig::default())
        .await
        .expect("contract database");
    let (addr, _guard) = spin_server(Some(db)).await;

    let response = reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("GET /health");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.expect("JSON health body");

    assert_health_key_set(&body);

    let status = body["status"].as_str().expect("status must be a string");
    // Documented-value guard: `degraded` is a legal value of the contract,
    // but this harness does not inject a down probe, so only `ok` is reachable.
    assert!(
        status == "ok" || status == "degraded",
        "status must be one of the documented values, got {status}"
    );
    assert_eq!(status, "ok");

    assert_eq!(
        body["version"],
        json!(env!("CARGO_PKG_VERSION")),
        "version must be the package version"
    );
    assert_eq!(
        body["protocolVersion"].as_i64(),
        Some(i64::from(lc_protocol::PROTOCOL_VERSION)),
        "protocolVersion must be the protocol constant"
    );
    assert_eq!(body["protocolVersion"].as_i64(), Some(2));
    assert_eq!(body["db"], json!("ok"));
    assert_eq!(body["discord"], json!("disabled"));
    let uptime_sec = body["uptimeSec"]
        .as_i64()
        .expect("uptimeSec must be an integer");
    assert!(uptime_sec >= 0, "uptimeSec must be >= 0, got {uptime_sec}");
    assert_eq!(
        body["activeJobs"].as_i64(),
        Some(0),
        "activeJobs must count the PLAYING jobs"
    );
}

#[tokio::test]
async fn health_contract_without_a_database() {
    let (addr, _guard) = spin_server(None).await;

    let response = reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("GET /health");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.expect("JSON health body");

    assert_health_key_set(&body);

    assert_eq!(body["status"], json!("ok"));
    assert_eq!(body["db"], json!("disabled"));
    assert!(body["activeJobs"].is_null(), "activeJobs must be JSON null");
}

#[tokio::test]
async fn anonymous_admin_overview_is_rejected() {
    let (addr, _guard) = spin_server(None).await;

    let response = reqwest::Client::new()
        .get(format!("http://{addr}/admin/api/overview"))
        .send()
        .await
        .expect("GET /admin/api/overview");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    let text = response.text().await.expect("raw 401 body");
    assert_eq!(
        text, r#"{"error":"unauthorized"}"#,
        "the anonymous 401 body must stay byte-exact"
    );
    let body: Value = serde_json::from_str(&text).expect("JSON 401 body");
    assert_eq!(body, json!({ "error": "unauthorized" }));
}
