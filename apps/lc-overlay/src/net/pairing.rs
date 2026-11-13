//! Appairage HTTP v2 de l'overlay.
//!
//! Contrat : `POST {serverUrl}/overlay/pair/consume` avec `{code, deviceName}`.
//! La réponse v2 ne contient plus la configuration complète : seul
//! `clientToken`, `guildId` et `sessionMode` sont garantis (les champs legacy
//! `apiBaseUrl`, `clientId`, `authorName`, `deviceName` restent lus de façon
//! tolérante). Le legacy tentait d'abord une connexion TLS stricte puis
//! rétrogradait silencieusement en `rejectUnauthorized: false` ; ici :
//!
//! - `serverUrl` doit être `https://` par défaut. Une URL `http://` n'est
//!   acceptée qu'en build debug avec `LIVECHAT_INSECURE_TLS=1` (cf.
//!   [`super::tls`]) ;
//! - un `apiBaseUrl` renvoyé par le serveur ne peut **jamais** rétrograder une
//!   configuration HTTPS vers HTTP (AUD-A6-003) ;
//! - les erreurs sont explicites (schéma refusé, downgrade refusé).

use std::time::Duration;

use lc_protocol::SessionMode;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use url::Url as ParsedUrl;

use crate::config::{Membership, OverlayConfig};
use crate::net::tls;
use crate::state::AppState;

/// Délai maximal d'une requête d'appairage.
const PAIR_TIMEOUT: Duration = Duration::from_secs(15);
/// Longueurs maximales acceptées avant tout appel réseau.
const MAX_SERVER_URL_CHARS: usize = 2048;
const MAX_CODE_CHARS: usize = 64;
const MAX_DEVICE_NAME_CHARS: usize = 128;
/// Nom d'appareil de secours si l'appelant n'en fournit pas.
const FALLBACK_DEVICE_NAME: &str = "LiveChat-Overlay";

/// Requête d'appairage validée côté shell.
#[derive(Debug, Clone)]
pub struct PairingRequest {
    pub server_url: String,
    pub code: String,
    pub device_name: Option<String>,
    /// Pseudo du profil local (nouveau membre).
    pub display_name: Option<String>,
    /// Avatar du profil local (URL ou image embarquée).
    pub avatar: Option<String>,
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
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    member_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    guild_name: Option<String>,
}

/// Chaîne non vide après `trim`, sinon `None`.
fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

impl PairingRequest {
    /// Valide la requête comme le legacy (`missing_required_fields`), en
    /// ajoutant des bornes de longueur (défense en profondeur IPC).
    fn validated(&self) -> Result<(String, String), String> {
        let server_url = self.server_url.trim().trim_end_matches('/').to_string();
        if server_url.is_empty() {
            return Err("champs requis manquants : serverUrl".to_string());
        }
        if server_url.chars().count() > MAX_SERVER_URL_CHARS {
            return Err("serverUrl trop long".to_string());
        }
        let code = self.code.trim().to_ascii_uppercase();
        if code.is_empty() {
            return Err("champs requis manquants : code".to_string());
        }
        if code.chars().count() > MAX_CODE_CHARS {
            return Err("code d'appairage invalide (trop long)".to_string());
        }
        Ok((server_url, code))
    }

    fn device_name(&self) -> String {
        match self.device_name.as_deref() {
            Some(name) if !name.trim().is_empty() => name
                .trim()
                .chars()
                .take(MAX_DEVICE_NAME_CHARS)
                .collect::<String>(),
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
    tls::insecure_tls_enabled()
}

/// Construit l'URL de l'endpoint d'appairage à partir de `serverUrl`.
fn pairing_endpoint(server_url: &str) -> Result<Url, String> {
    pairing_endpoint_with(server_url, insecure_tls_enabled())
}

/// Variante testable : `allow_insecure` remplace la lecture de l'environnement.
fn pairing_endpoint_with(server_url: &str, allow_insecure: bool) -> Result<Url, String> {
    let base = server_url.trim().trim_end_matches('/');
    let mut url =
        ParsedUrl::parse(base).map_err(|error| format!("serverUrl invalide : {error}"))?;
    tls::require_secure_scheme(url.scheme(), "serverUrl", allow_insecure)?;
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
    let requested = ParsedUrl::parse(request.server_url.trim())
        .map_err(|error| format!("serverUrl invalide : {error}"))?;
    let base_url = match response
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        Some(raw) => {
            let parsed =
                ParsedUrl::parse(raw).map_err(|error| format!("apiBaseUrl invalide : {error}"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(format!(
                    "apiBaseUrl de schéma non supporté : {}",
                    parsed.scheme()
                ));
            }
            if requested.scheme() == "https" && parsed.scheme() == "http" {
                return Err(
                    "le serveur a renvoyé un apiBaseUrl http incompatible avec un appairage HTTPS \
                     (mise à jour du serveur requise)"
                        .to_string(),
                );
            }
            parsed.to_string()
        }
        None => requested.to_string(),
    };
    let guild_id = clean(response.guild_id.as_deref())
        .ok_or_else(|| "réponse d'appairage invalide : guildId absent".to_string())?;
    let guest_mode = response.session_mode == Some(SessionMode::InviteReadOnly);
    // La guilde rejointe devient la guilde active ; les autres adhésions sont
    // conservées (multi-guilde).
    config.join(Membership {
        server_url: base_url.trim_end_matches('/').to_string(),
        guild_id,
        guild_name: clean(response.guild_name.as_deref()),
        guild_icon_url: None,
        client_token: token.to_string(),
        client_id: clean(response.client_id.as_deref()),
        member_id: clean(response.member_id.as_deref()),
        display_name: clean(response.display_name.as_deref())
            .or_else(|| clean(response.author_name.as_deref())),
        role: clean(response.role.as_deref()),
        guest_mode,
        muted: false,
        volume: 1.0,
        media_bindings: Default::default(),
    });
    config.author_name = clean(response.author_name.as_deref());
    config.device_name =
        Some(clean(response.device_name.as_deref()).unwrap_or_else(|| device_name.to_string()));
    if let Some(display_name) = clean(request.display_name.as_deref()) {
        config.profile_name = Some(display_name);
    }
    if let Some(avatar) = clean(request.avatar.as_deref()) {
        config.profile_avatar = Some(avatar);
    }
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
            "displayName": clean(request.display_name.as_deref())
                .or_else(|| state.config_snapshot().profile_name),
            "avatar": clean(request.avatar.as_deref())
                .or_else(|| state.config_snapshot().profile_avatar),
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
            display_name: None,
            avatar: None,
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
        assert!(pairing_endpoint("ftp://example.test").is_err());
    }

    #[test]
    fn http_server_url_is_refused_without_derogation() {
        let error = pairing_endpoint("http://127.0.0.1:3000").expect_err("http refusé");
        assert!(
            error.contains("HTTPS"),
            "message explicite attendu: {error}"
        );
        assert_eq!(
            pairing_endpoint_with("http://127.0.0.1:3000", true)
                .expect("dérogation dev")
                .as_str(),
            "http://127.0.0.1:3000/overlay/pair/consume"
        );
    }

    #[test]
    fn api_base_url_cannot_downgrade_https_to_http() {
        let mut config = OverlayConfig::default();
        let response = PairConsumeResponse {
            api_base_url: Some("http://api.example.test/".to_string()),
            client_token: Some("abc123".to_string()),
            client_id: None,
            guild_id: Some("guild-42".to_string()),
            author_name: None,
            device_name: None,
            session_mode: None,
            role: None,
            member_id: None,
            display_name: None,
            guild_name: None,
        };
        let error = apply_consume_response(&mut config, &request(), &response, "PC-TEST")
            .expect_err("downgrade refusé");
        assert!(error.contains("apiBaseUrl"), "message attendu: {error}");
        assert_eq!(config.server_url, None);
        assert_eq!(config.client_token, None);
    }

    #[test]
    fn api_base_url_https_upgrade_is_accepted() {
        let mut config = OverlayConfig::default();
        let mut request = request();
        request.server_url = "http://api.example.test".to_string();
        let response = PairConsumeResponse {
            api_base_url: Some("https://api.example.test/".to_string()),
            client_token: Some("abc123".to_string()),
            client_id: None,
            guild_id: Some("guild-42".to_string()),
            author_name: None,
            device_name: None,
            session_mode: None,
            role: None,
            member_id: None,
            display_name: None,
            guild_name: None,
        };
        apply_consume_response(&mut config, &request, &response, "PC-TEST")
            .expect("upgrade accepté");
        assert_eq!(
            config.server_url.as_deref(),
            Some("https://api.example.test")
        );
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
            display_name: None,
            avatar: None,
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
            role: None,
            member_id: None,
            display_name: None,
            guild_name: None,
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
    fn second_guild_is_added_and_becomes_active() {
        let mut config = OverlayConfig::default();
        let first = PairConsumeResponse {
            api_base_url: None,
            client_token: Some("token-a".to_string()),
            client_id: Some("client-a".to_string()),
            guild_id: Some("111".to_string()),
            author_name: None,
            device_name: None,
            session_mode: Some(SessionMode::Normal),
            role: Some("admin".to_string()),
            member_id: Some("member-a".to_string()),
            display_name: Some("Quentin".to_string()),
            guild_name: Some("Les Sardines".to_string()),
        };
        apply_consume_response(&mut config, &request(), &first, "PC-TEST").expect("guilde 1");
        config
            .media_bindings
            .insert("Ctrl+1".to_string(), "media-a".to_string());

        let mut second_request = request();
        second_request.display_name = Some("  Quentin B. ".to_string());
        let second = PairConsumeResponse {
            api_base_url: None,
            client_token: Some("token-b".to_string()),
            client_id: None,
            guild_id: Some("222".to_string()),
            author_name: None,
            device_name: None,
            session_mode: Some(SessionMode::InviteReadOnly),
            role: Some("guest".to_string()),
            member_id: None,
            display_name: None,
            guild_name: None,
        };
        apply_consume_response(&mut config, &second_request, &second, "PC-TEST").expect("guilde 2");

        assert_eq!(config.memberships.len(), 2);
        assert_eq!(config.guild_id.as_deref(), Some("222"));
        assert_eq!(config.client_token.as_deref(), Some("token-b"));
        assert!(config.guest_mode);
        // Les raccourcis de la première guilde restent rangés avec elle.
        assert!(config.media_bindings.is_empty());
        let kept = config.membership("111").expect("guilde 1 conservée");
        assert_eq!(kept.client_token, "token-a");
        assert_eq!(kept.role.as_deref(), Some("admin"));
        assert_eq!(kept.guild_name.as_deref(), Some("Les Sardines"));
        assert_eq!(
            kept.media_bindings.get("Ctrl+1").map(String::as_str),
            Some("media-a")
        );
        assert_eq!(config.profile_name.as_deref(), Some("Quentin B."));

        // Rejoindre à nouveau la même guilde remplace son adhésion.
        apply_consume_response(&mut config, &request(), &first, "PC-TEST").expect("re-join");
        assert_eq!(config.memberships.len(), 2);
        assert_eq!(config.guild_id.as_deref(), Some("111"));

        // Quitter la guilde active rend l'autre active.
        assert!(config.leave("111"));
        assert_eq!(config.guild_id.as_deref(), Some("222"));
        assert!(config.leave("222"));
        assert!(!config.has_pairing_config());
        assert!(config.memberships.is_empty());
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
            role: None,
            member_id: None,
            display_name: None,
            guild_name: None,
        };
        let error = apply_consume_response(&mut config, &request(), &response, "PC-TEST")
            .expect_err("clientToken requis");
        assert!(error.contains("clientToken"));
    }
}
