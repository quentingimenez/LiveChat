//! Requêtes HTTP exécutées par le harnais et forme canonique des réponses.

use std::collections::BTreeMap;
use std::time::Duration;

use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::parity::case::{AuthMode, Case};
use crate::parity::normalize::{
    normalize_text, RATE_LIMIT_REMAINING_PLACEHOLDER, RATE_LIMIT_RESET_PLACEHOLDER,
    REDACTED_PLACEHOLDER, RETRY_AFTER_PLACEHOLDER,
};
use crate::parity::{ParityError, ParityResult};

/// Version du harnais, recopiée dans les goldens et les rapports.
pub const HARNESS_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Délai par requête par défaut.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Jeton volontairement invalide pour les cas `admin_invalid`.
pub const INVALID_TOKEN: &str = "parity-invalid-token";
/// Taille maximale d'un corps texte conservé dans un golden.
pub const MAX_TEXT_BODY_BYTES: usize = 128 * 1024;

/// En-têtes conservés dans les goldens (liste blanche stable).
pub const CAPTURED_HEADERS: &[&str] = &[
    "accept-ranges",
    "cache-control",
    "content-disposition",
    "content-length",
    "content-range",
    "content-security-policy",
    "content-type",
    "cross-origin-resource-policy",
    "etag",
    "last-modified",
    "location",
    "permissions-policy",
    "referrer-policy",
    "retry-after",
    "www-authenticate",
    "x-content-type-options",
    "x-frame-options",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
    "x-request-id",
];

/// Nombre maximal de requêtes envoyées par la sonde de rate limit.
pub const RATE_LIMIT_PROBE_MAX: usize = 120;

// `set-cookie` n'est jamais capturé : il ne figure pas dans [`CAPTURED_HEADERS`].

/// Corps d'une réponse, déjà classé JSON/texte/vide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Body {
    /// Aucun corps.
    Empty,
    /// Corps JSON (la forme brute est conservée, sans normalisation).
    Json {
        /// Valeur JSON du corps.
        value: Value,
    },
    /// Corps non JSON conservé sous forme de texte.
    Text {
        /// Texte du corps (tronqué au-delà de [`MAX_TEXT_BODY_BYTES`]).
        value: String,
    },
}

impl Body {
    /// Classe un corps brut à partir des octets et du `Content-Type`.
    pub fn from_bytes(bytes: &[u8], content_type: Option<&str>) -> Self {
        let trimmed = trim_ascii(bytes);
        if trimmed.is_empty() {
            return Self::Empty;
        }
        let looks_json = content_type
            .map(|value| value.to_ascii_lowercase().contains("json"))
            .unwrap_or(false)
            || matches!(trimmed.first(), Some(b'{') | Some(b'['));
        if looks_json {
            if let Ok(value) = serde_json::from_slice::<Value>(trimmed) {
                return Self::Json { value };
            }
        }
        Self::Text {
            value: truncate_text(&String::from_utf8_lossy(trimmed)),
        }
    }

    /// Copie dont tous les secrets détectables sont remplacés par `<redacted>`.
    pub fn redacted(&self) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Json { value } => {
                let mut redacted = value.clone();
                redact_value(&mut redacted);
                Self::Json { value: redacted }
            }
            Self::Text { value } => Self::Text {
                value: normalize_text(value),
            },
        }
    }
}

/// Réponse capturée : statut, en-têtes filtrés et corps classé.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapturedResponse {
    /// Code HTTP.
    pub status: u16,
    /// En-têtes retenus par la liste blanche, valeurs normalisées.
    pub headers: BTreeMap<String, String>,
    /// Corps de la réponse.
    pub body: Body,
}

impl CapturedResponse {
    /// Copie destinée à la publication : secrets remplacés.
    pub fn redacted(&self) -> Self {
        Self {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.redacted(),
        }
    }
}

/// Jetons fournis par la ligne de commande.
#[derive(Debug, Clone, Default)]
pub struct Tokens {
    /// Jeton administrateur.
    pub admin: Option<String>,
    /// Jeton de client overlay.
    pub overlay: Option<String>,
    /// Jeton de client ingest.
    pub ingest: Option<String>,
}

impl Tokens {
    /// Jeton correspondant à un mode d'authentification, s'il est disponible.
    pub fn for_mode(&self, mode: AuthMode) -> Option<&str> {
        match mode {
            AuthMode::None => None,
            AuthMode::AdminInvalid => Some(INVALID_TOKEN),
            AuthMode::Admin => self.admin.as_deref(),
            AuthMode::Overlay => self.overlay.as_deref(),
            AuthMode::Ingest => self.ingest.as_deref(),
        }
    }

    /// Vrai si tous les jetons nécessaires au cas sont disponibles.
    pub fn satisfies(&self, mode: AuthMode) -> bool {
        !mode.requires_token() || self.for_mode(mode).is_some()
    }
}

/// Vérifie les dépendances d'un cas avant capture ou comparaison.
///
/// Renvoie la raison du saut (variable, jeton ou sonde manquants) ou `None`
/// lorsque le cas peut être exécuté.
pub fn requirement_gap(
    case: &Case,
    media_id: Option<&str>,
    tokens: &Tokens,
    rate_limit_probe: bool,
) -> Option<String> {
    if case.resolved_path(media_id).is_none() {
        return Some("variable requise non fournie (media_id)".to_string());
    }
    if case.requires("rate_limit_probe") && !rate_limit_probe {
        return Some("sonde de rate limit désactivée (--rate-limit-probe)".to_string());
    }
    if !tokens.satisfies(case.auth) {
        return Some(format!(
            "jeton manquant pour l'authentification {:?}",
            case.auth
        ));
    }
    for (requirement, token) in [
        ("admin_token", tokens.admin.as_ref()),
        ("overlay_token", tokens.overlay.as_ref()),
        ("ingest_token", tokens.ingest.as_ref()),
    ] {
        if case.requires(requirement) && token.is_none() {
            return Some(format!("jeton manquant ({requirement})"));
        }
    }
    None
}

/// Requête prête à être envoyée.
#[derive(Debug, Clone)]
pub struct PreparedRequest {
    /// Méthode HTTP en majuscules.
    pub method: String,
    /// Chemin résolu (query comprise).
    pub path: String,
    /// URL absolue construite à partir de l'URL de base.
    pub url: String,
    /// Jeton effectivement envoyé (jamais journalisé).
    pub token: Option<String>,
    /// Corps JSON sérialisé.
    pub body: Option<String>,
    /// En-têtes supplémentaires.
    pub headers: Vec<(String, String)>,
    /// Délai de la requête.
    pub timeout: Duration,
}

impl PreparedRequest {
    /// Prépare l'exécution d'un cas ; `None` si un `{media_id}` demeure non résolu.
    pub fn build(
        case: &Case,
        base_url: &str,
        media_id: Option<&str>,
        tokens: &Tokens,
        timeout: Duration,
    ) -> ParityResult<Option<Self>> {
        let Some(path) = case.resolved_path(media_id) else {
            return Ok(None);
        };
        let base = reqwest::Url::parse(base_url)
            .map_err(|error| ParityError::argument(format!("URL de base invalide : {error}")))?;
        let url = format!("{}{}", base.as_str().trim_end_matches('/'), path);
        let body = match &case.body {
            Some(value) => Some(serde_json::to_string(value)?),
            None => None,
        };
        let headers = case
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();

        Ok(Some(Self {
            method: case.method_upper(),
            path,
            url,
            token: tokens.for_mode(case.auth).map(str::to_string),
            body,
            headers,
            timeout,
        }))
    }
}

/// Construit le client HTTP du harnais (pas de suivi de redirection).
pub fn build_client(timeout: Duration) -> ParityResult<Client> {
    Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| {
            ParityError::http(format!("impossible de construire le client HTTP : {error}"))
        })
}

/// Envoie une rafale de `GET /health` jusqu'à observer un `429`.
///
/// Renvoie `true` si le rate limit s'est déclenché, `false` si le serveur n'a
/// jamais limité la rafale (serveur exempté, proxy, …). N'échoue que sur erreur
/// réseau : la suite décide ensuite quoi faire du résultat.
pub async fn probe_rate_limit(
    client: &Client,
    base_url: &str,
    timeout: Duration,
) -> ParityResult<bool> {
    let url = format!("{}/health", base_url.trim_end_matches('/'));
    for _ in 0..RATE_LIMIT_PROBE_MAX {
        let response = client
            .get(&url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|error| ParityError::http(format!("sonde de rate limit {url} : {error}")))?;
        let status = response.status().as_u16();
        response.bytes().await.map_err(|error| {
            ParityError::http(format!(
                "sonde de rate limit {url} : corps illisible : {error}"
            ))
        })?;
        if status == 429 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Exécute une requête et capture la réponse.
pub async fn execute(client: &Client, request: &PreparedRequest) -> ParityResult<CapturedResponse> {
    let method = Method::from_bytes(request.method.as_bytes()).map_err(|error| {
        ParityError::argument(format!(
            "méthode HTTP invalide {} : {error}",
            request.method
        ))
    })?;

    let mut builder = client
        .request(method, &request.url)
        .timeout(request.timeout);
    if let Some(token) = &request.token {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    if let Some(body) = &request.body {
        builder = builder
            .header(CONTENT_TYPE, "application/json")
            .body(body.clone());
    }

    let response = builder.send().await.map_err(|error| {
        ParityError::http(format!("{} {} : {error}", request.method, request.url))
    })?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response.bytes().await.map_err(|error| {
        ParityError::http(format!("lecture du corps {} : {error}", request.url))
    })?;

    Ok(capture_response(status, &headers, &bytes))
}

/// Construit une réponse capturée à partir du statut, des en-têtes et du corps.
pub fn capture_response(status: u16, headers: &HeaderMap, bytes: &[u8]) -> CapturedResponse {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    CapturedResponse {
        status,
        headers: filtered_headers(headers),
        body: Body::from_bytes(bytes, content_type.as_deref()),
    }
}

/// Filtre et normalise les en-têtes conservés dans un golden.
pub fn filtered_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut captured = BTreeMap::new();
    for name in CAPTURED_HEADERS {
        if let Some(value) = headers.get(*name) {
            if let Ok(text) = value.to_str() {
                captured.insert((*name).to_string(), normalize_header_value(name, text));
            }
        }
    }
    captured
}

/// Normalise la valeur d'un en-tête capturé.
///
/// Les valeurs qui varient d'une exécution à l'autre (`x-ratelimit-remaining`,
/// `x-ratelimit-reset`, `retry-after`) sont remplacées par un placeholder
/// stable ; les autres passent par [`normalize_text`] (UUID, chemins, …).
pub fn normalize_header_value(name: &str, value: &str) -> String {
    match name {
        "x-ratelimit-remaining" => RATE_LIMIT_REMAINING_PLACEHOLDER.to_string(),
        "x-ratelimit-reset" => RATE_LIMIT_RESET_PLACEHOLDER.to_string(),
        "retry-after" => RETRY_AFTER_PLACEHOLDER.to_string(),
        _ => normalize_text(value),
    }
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let is_space = |byte: u8| byte.is_ascii_whitespace();
    let start = bytes.iter().position(|byte| !is_space(*byte));
    let end = bytes.iter().rposition(|byte| !is_space(*byte));
    match (start, end) {
        (Some(start), Some(end)) => &bytes[start..=end],
        _ => &[],
    }
}

fn truncate_text(text: &str) -> String {
    if text.len() <= MAX_TEXT_BODY_BYTES {
        return text.to_string();
    }
    let mut end = MAX_TEXT_BODY_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…<tronqué>", &text[..end])
}

/// Remplace les valeurs ressemblant à des secrets dans une valeur JSON.
pub fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let lowercase = key.to_ascii_lowercase();
                let sensitive = lowercase.contains("token")
                    || lowercase.contains("secret")
                    || lowercase.contains("password")
                    || lowercase.contains("cookie")
                    || lowercase.contains("authorization");
                if sensitive && !child.is_null() {
                    *child = Value::String(REDACTED_PLACEHOLDER.to_string());
                } else {
                    redact_value(child);
                }
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                redact_value(child);
            }
        }
        Value::String(text) => {
            let normalized = normalize_text(text);
            if normalized != *text {
                *text = normalized;
            }
        }
        _ => {}
    }
}
