//! Appairage HTTP v2 de l'overlay.
//!
//! Contrat : `POST {serverUrl}/overlay/pair/consume` avec `{code, deviceName}`.
//! La réponse v2 ne contient plus la configuration complète : seul
//! `clientToken`, `guildId` et `sessionMode` sont garantis (les champs legacy
//! `apiBaseUrl`, `clientId`, `authorName`, `deviceName` restent lus de façon
//! tolérante). Le legacy tentait d'abord une connexion TLS stricte puis
//! rétrogradait silencieusement en `rejectUnauthorized: false` ; ici la
//! rétrogradation est explicite via `LIVECHAT_INSECURE_TLS=1` et documentée
//! dans `docs/rust/lc-overlay-8b.md`.

use std::time::Duration;

use lc_protocol::SessionMode;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use url::Url as ParsedUrl;

use crate::config::OverlayConfig;
use crate::state::AppState;

/// Délai maximal d'une requête d'appairage.
const PAIR_TIMEOUT: Duration = Duration::from_secs(15);
/// Option TLS legacy (certificat non vérifié), explicite et documentée.
const INSECURE_TLS_ENV: &str = "LIVECHAT_INSECURE_TLS";
/// Nom d'appareil de secours si l'appelant n'en fournit pas.
const FALLBACK_DEVICE_NAME: &str = "LiveChat-Overlay";

/// Requête d'appairage validée côté shell.
#[derive(Debug, Clone)]
pub struct PairingRequest {
    pub server_url: String,
    pub code: String,
    pub device_name: Option<String>,
}

/// Résultat de l'appairage : sert à la commande Tauri et aux effets tray.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOutcome {
    pub ok: bool,
    pub server_url: Option<String>,
    pub guild_id: Option<String>,
    pub guest_mode: bool,
    pub message: String,
}

/// Réponse tolérante de `POST /overlay/pair/consume`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairConsumeResponse {
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    client_token: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    guild_id: Option<String>,
    #[serde(default)]
    author_name: Option<String>,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    session_mode: Option<SessionMode>,
}

impl PairingRequest {
    /// Valide la requête comme le legacy (`missing_required_fields`).
    fn validated(&self) -> Result<(String, String), String> {
        let server_url = self.server_url.trim().trim_end_matches('/').to_string();
        if server_url.is_empty() {
            return Err("champs requis manquants : serverUrl".to_string());
        }
        let code = self.code.trim().to_ascii_uppercase();
        if code.is_empty() {
            return Err("champs requis manquants : code".to_string());
        }
        Ok((server_url, code))
    }

    fn device_name(&self) -> String {
        match self.device_name.as_deref() {
            Some(name) if !name.trim().is_empty() => name.trim().to_string(),
            _ => fallback_device_name(),
        }
    }
}

fn fallback_device_name() -> String {
    match std::env::var("COMPUTERNAME") {
        Ok(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => FALLBACK_DEVICE_NAME.to_string(),
    }
}

fn insecure_tls_enabled() -> bool {
    match std::env::var(INSECURE_TLS_ENV) {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

/// Construit l'URL de l'endpoint d'appairage à partir de `serverUrl`.
fn pairing_endpoint(server_url: &str) -> Result<Url, String> {
    let base = server_url.trim().trim_end_matches('/');
    let mut url =
        ParsedUrl::parse(base).map_err(|error| format!("serverUrl invalide : {error}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("schéma serverUrl non supporté : {other}")),
    }
    url.set_path("/overlay/pair/consume");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn build_client(insecure_tls: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(PAIR_TIMEOUT)
        .user_agent(concat!("lc-overlay/", env!("CARGO_PKG_VERSION")));
    if insecure_tls {
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder
        .build()
        .map_err(|error| format!("client HTTP impossible : {error}"))
}

/// Applique une réponse d'appairage à la configuration (pur, testable).
fn apply_consume_response(
    config: &mut OverlayConfig,
    request: &PairingRequest,
    response: &PairConsumeResponse,
    device_name: &str,
) -> Result<(), String> {
    let token = response
        .client_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "réponse d'appairage invalide : clientToken absent".to_string())?;
    let base_url = response
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(request.server_url.trim());
    config.server_url = Some(base_url.trim_end_matches('/').to_string());
    config.client_token = Some(token.to_string());
    config.client_id = response
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string);
    config.guild_id = response
        .guild_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string);
    config.author_name = response
        .author_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string);
    config.device_name = Some(
        response
            .device_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| device_name.to_string()),
    );
    config.guest_mode = response.session_mode == Some(SessionMode::InviteReadOnly);
    Ok(())
}

/// Exécute l'appairage HTTP. La persistance et les effets Tauri sont faits par
/// l'appelant (commande `pairing_consume`) une fois le résultat connu.
pub async fn consume(state: &AppState, request: PairingRequest) -> PairingOutcome {
    let failure = |message: String| PairingOutcome {
        ok: false,
        server_url: None,
        guild_id: None,
        guest_mode: false,
        message,
    };
    let (server_url, code) = match request.validated() {
        Ok(values) => values,
        Err(message) => return failure(message),
    };
    let endpoint = match pairing_endpoint(&server_url) {
        Ok(endpoint) => endpoint,
        Err(message) => return failure(message),
    };
    let device_name = request.device_name();
    let client = match build_client(insecure_tls_enabled()) {
        Ok(client) => client,
        Err(message) => return failure(message),
    };
    let response = match client
        .post(endpoint)
        .json(&serde_json::json!({
            "code": code,
            "deviceName": device_name,
        }))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => return failure(format!("appairage impossible : {error}")),
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = body.trim();
        let detail = if detail.is_empty() {
            "sans détail".to_string()
        } else {
            detail.chars().take(300).collect::<String>()
        };
        return failure(format!(
            "appairage refusé (HTTP {} - {detail})",
            status.as_u16()
        ));
    }
    let response = match response.json::<PairConsumeResponse>().await {
        Ok(response) => response,
        Err(error) => {
            return failure(format!("réponse d'appairage illisible : {error}"));
        }
    };
    let mut config = state.config_snapshot();
    if let Err(message) = apply_consume_response(&mut config, &request, &response, &device_name) {
        return failure(message);
    }
    if let Err(error) = state.update_config(|current| *current = config.clone()) {
        return failure(format!(
            "sauvegarde de la configuration impossible : {error}"
        ));
    }
    let updated = state.config_snapshot();
    PairingOutcome {
        ok: true,
        server_url: updated.server_url,
        guild_id: updated.guild_id,
        guest_mode: updated.guest_mode,
        message: "appairage réussi".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PairingRequest {
        PairingRequest {
            server_url: "https://api.example.test/".to_string(),
            code: " ab12 ".to_string(),
            device_name: None,
        }
    }

    #[test]
    fn endpoint_is_built_from_server_url() {
        assert_eq!(
            pairing_endpoint("https://api.example.test/")
                .expect("endpoint")
                .as_str(),
            "https://api.example.test/overlay/pair/consume"
        );
        assert_eq!(
            pairing_endpoint("http://127.0.0.1:3000")
                .expect("endpoint")
                .as_str(),
            "http://127.0.0.1:3000/overlay/pair/consume"
        );
        assert!(pairing_endpoint("ftp://example.test").is_err());
    }

    #[test]
    fn request_is_normalized_like_legacy() {
        let (server_url, code) = request().validated().expect("requête valide");
        assert_eq!(server_url, "https://api.example.test");
        assert_eq!(code, "AB12");
        assert!(PairingRequest {
            server_url: String::new(),
            code: "X".to_string(),
            device_name: None,
        }
        .validated()
        .is_err());
    }

    #[test]
    fn consume_response_updates_overlay_config() {
        let mut config = OverlayConfig::default();
        let response = PairConsumeResponse {
            api_base_url: Some("https://api.example.test/".to_string()),
            client_token: Some("abc123".to_string()),
            client_id: Some("client-42".to_string()),
            guild_id: Some("guild-42".to_string()),
            author_name: Some("Babz".to_string()),
            device_name: None,
            session_mode: Some(SessionMode::InviteReadOnly),
        };
        apply_consume_response(&mut config, &request(), &response, "PC-TEST")
            .expect("application de la réponse");
        assert_eq!(
            config.server_url.as_deref(),
            Some("https://api.example.test")
        );
        assert_eq!(config.client_token.as_deref(), Some("abc123"));
        assert_eq!(config.client_id.as_deref(), Some("client-42"));
        assert_eq!(config.guild_id.as_deref(), Some("guild-42"));
        assert_eq!(config.author_name.as_deref(), Some("Babz"));
        assert_eq!(config.device_name.as_deref(), Some("PC-TEST"));
        assert!(config.guest_mode);
    }

    #[test]
    fn consume_response_requires_token() {
        let mut config = OverlayConfig::default();
        let response = PairConsumeResponse {
            api_base_url: None,
            client_token: None,
            client_id: None,
            guild_id: None,
            author_name: None,
            device_name: None,
            session_mode: None,
        };
        let error = apply_consume_response(&mut config, &request(), &response, "PC-TEST")
            .expect_err("clientToken requis");
        assert!(error.contains("clientToken"));
    }
}
