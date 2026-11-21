//! Appels REST authentifiés vers le serveur d'une guilde.
//!
//! L'interface (fenêtre board) ne manipule jamais le jeton : elle demande au
//! shell d'appeler `{serverUrl}/overlay/...` pour une guilde donnée, et le
//! shell ajoute l'en-tête `Authorization`. Seules les routes `/overlay/` sont
//! joignables, avec les méthodes du contrat REST.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Method;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::config::Membership;
use crate::net::tls;

/// Connexion TCP/TLS : un serveur muet ne doit pas immobiliser l'action.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Lecture : au-delà, la board affiche une erreur au lieu d'attendre.
const GET_TIMEOUT: Duration = Duration::from_secs(15);
/// Écriture : le serveur peut avoir à valider et persister l'action.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Connexions inactives gardées pour réutiliser TCP/TLS entre deux actions.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Sonde TCP des connexions gardées.
const TCP_KEEPALIVE: Duration = Duration::from_secs(60);
/// Longueur maximale d'un chemin (query comprise).
const MAX_PATH_CHARS: usize = 2_048;
/// Préfixe obligatoire des routes joignables.
const ALLOWED_PREFIX: &str = "/overlay/";

/// Réponse relayée à l'interface.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse {
    /// `true` pour un statut 2xx.
    pub ok: bool,
    pub status: u16,
    /// Corps JSON, ou `null` s'il est vide ou illisible.
    pub body: Value,
}

/// Méthode HTTP autorisée (insensible à la casse).
fn parse_method(raw: &str) -> Result<Method, String> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PATCH" => Ok(Method::PATCH),
        "PUT" => Ok(Method::PUT),
        "DELETE" => Ok(Method::DELETE),
        other => Err(format!("méthode non autorisée : {other}")),
    }
}

/// Construit l'URL cible en refusant tout ce qui sortirait de `/overlay/`.
///
/// Le chemin est résolu puis re-vérifié sur l'URL finale : ni `..`, ni URL
/// absolue, ni changement d'hôte ne peuvent contourner le préfixe.
pub(crate) fn build_url(server_url: &str, path: &str, allow_insecure: bool) -> Result<Url, String> {
    if path.chars().count() > MAX_PATH_CHARS {
        return Err("chemin trop long".to_string());
    }
    if !path.starts_with(ALLOWED_PREFIX) || path.contains("..") || path.contains('\\') {
        return Err("chemin non autorisé".to_string());
    }
    let base = Url::parse(server_url.trim().trim_end_matches('/'))
        .map_err(|error| format!("serverUrl invalide : {error}"))?;
    tls::require_secure_scheme(base.scheme(), "serverUrl", allow_insecure)?;
    let url = base
        .join(path)
        .map_err(|error| format!("chemin invalide : {error}"))?;
    if url.origin() != base.origin() || !url.path().starts_with(ALLOWED_PREFIX) {
        return Err("chemin non autorisé".to_string());
    }
    Ok(url)
}

/// One HTTP client per TLS mode, kept for the whole process.
///
/// Building a client per call threw away the connection pool: every action
/// paid a new TCP/TLS handshake before the server even saw the request, which
/// made a slow switch look frozen. The insecure client is separate because
/// `danger_accept_invalid_certs` cannot be changed after construction.
fn shared_client(insecure: bool) -> Result<&'static reqwest::Client, String> {
    static SECURE: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    static INSECURE: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    let cached = if insecure { &INSECURE } else { &SECURE };
    let client = cached.get_or_init(|| {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .tcp_keepalive(TCP_KEEPALIVE)
            .user_agent(concat!("lc-overlay/", env!("CARGO_PKG_VERSION")));
        if insecure {
            builder = builder.danger_accept_invalid_certs(true);
        }
        builder
            .build()
            .map_err(|error| format!("client HTTP impossible : {error}"))
    });
    client.as_ref().map_err(|error| error.clone())
}

/// Deadline of one request: a read must fail fast, a write may take longer.
fn timeout_for(method: &Method) -> Duration {
    if *method == Method::GET {
        GET_TIMEOUT
    } else {
        WRITE_TIMEOUT
    }
}

/// Exécute un appel REST authentifié pour la guilde de `membership`.
pub async fn request(
    membership: &Membership,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<ApiResponse, String> {
    request_with_headers(membership, method, path, body, &[]).await
}

/// [`request`] with extra request headers (e.g. a second guild's token).
pub async fn request_with_headers(
    membership: &Membership,
    method: &str,
    path: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> Result<ApiResponse, String> {
    let method = parse_method(method)?;
    let insecure = tls::insecure_tls_enabled();
    let url = build_url(&membership.server_url, path, insecure)?;
    let client = shared_client(insecure)?;
    let mut request = client
        .request(method.clone(), url)
        .timeout(timeout_for(&method))
        .bearer_auth(&membership.client_token);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("serveur injoignable : {error}"))?;
    Ok(read_response(response).await)
}

/// Reads a response body into the shape exposed to the board.
///
/// Shared by REST calls and streamed uploads: an empty or non-JSON body
/// becomes `Value::Null` and `ok` reflects the HTTP status.
pub(crate) async fn read_response(response: reqwest::Response) -> ApiResponse {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    ApiResponse {
        ok: status.is_success(),
        status: status.as_u16(),
        body: serde_json::from_str(&text).unwrap_or(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_overlay_routes_are_reachable() {
        let base = "https://api.example.test/";
        assert_eq!(
            build_url(base, "/overlay/queue?x=1", false)
                .expect("route overlay")
                .as_str(),
            "https://api.example.test/overlay/queue?x=1"
        );
        for path in [
            "/admin/api/overview",
            "overlay/queue",
            "/overlay/../admin/api/overview",
            "//evil.test/overlay/queue",
            "https://evil.test/overlay/queue",
            "/overlay\\..\\admin",
        ] {
            assert!(
                build_url(base, path, false).is_err(),
                "{path} doit être refusé"
            );
        }
    }

    #[test]
    fn insecure_scheme_needs_derogation() {
        assert!(build_url("http://127.0.0.1:3000", "/overlay/guild", false).is_err());
        assert!(build_url("http://127.0.0.1:3000", "/overlay/guild", true).is_ok());
    }

    #[test]
    fn methods_are_restricted() {
        assert!(parse_method("patch").is_ok());
        // Favorites and pictures of board items are set with PUT.
        assert!(parse_method("PUT").is_ok());
        assert!(parse_method("OPTIONS").is_err());
        assert!(parse_method("CONNECT").is_err());
    }

    #[test]
    fn reads_fail_faster_than_writes() {
        let read = timeout_for(&Method::GET);
        let write = timeout_for(&Method::POST);
        assert_eq!(read, GET_TIMEOUT);
        assert_eq!(write, WRITE_TIMEOUT);
        assert!(read < write);
        assert!(CONNECT_TIMEOUT < read);
    }

    #[test]
    fn the_client_is_reused() {
        let first = shared_client(false).expect("client sécurisé");
        let second = shared_client(false).expect("client sécurisé");
        assert!(std::ptr::eq(first, second));
        let insecure = shared_client(true).expect("client non vérifié");
        assert!(!std::ptr::eq(first, insecure));
    }
}
