//! Client HTTP gardé contre les attaques SSRF.
//!
//! Toutes les requêtes sortantes du pipeline média passent par
//! [`send_guarded`] : chaque cible est validée par [`crate::guard`] (schéma,
//! résolution DNS, refus des IP internes), le client est épinglé sur l'IP
//! validée pour empêcher le rebinding, et les redirections sont suivies
//! manuellement (maximum [`crate::guard::MAX_REDIRECTS`] sauts) avec
//! revalidation complète de chaque destination.
//!
//! Les corps texte sont bornés ([`crate::guard::MAX_TEXT_RESPONSE_BYTES`]) pour
//! empêcher une source hostile de faire gonfler la mémoire du serveur.

use std::time::Duration;

use reqwest::header::{HeaderMap, LOCATION};
use reqwest::{Method, Response};
use serde::de::DeserializeOwned;
use url::Url;

use crate::config::DownloadConfig;
use crate::error::{code_from_http_status, MediaError, MediaErrorCode, MediaResult};
use crate::guard::{self, MAX_ERROR_BODY_BYTES, MAX_TEXT_RESPONSE_BYTES};

/// En-têtes transmis à un hôte différent lors d'une redirection.
///
/// Les cookies, jetons et en-têtes d'authentification ne sont jamais
/// transmis à un autre hôte.
const CROSS_HOST_HEADERS: [&str; 4] = ["accept", "accept-language", "range", "referer"];

/// Corps texte d'une réponse HTTP (borné à [`MAX_TEXT_RESPONSE_BYTES`]).
#[derive(Debug)]
pub struct HttpText {
    /// Corps décodé en UTF-8 (caractères invalides remplacés).
    pub body: String,
    /// URL finale après redirections (validée).
    pub final_url: String,
    /// En-têtes de la réponse finale.
    pub headers: HeaderMap,
}

/// Envoie une requête HTTP gardée et suit les redirections en revalidant
/// chaque cible.
///
/// Renvoie la réponse finale (non redirigée) et l'URL validée correspondante.
pub async fn send_guarded(
    config: &DownloadConfig,
    method: Method,
    raw_url: &str,
    timeout: Duration,
    headers: &[(&str, &str)],
) -> MediaResult<(Response, Url)> {
    let initial = guard::validate_target(raw_url).await?;
    let original_host = initial.host.clone();
    let mut next = initial.url.clone();
    for hop in 0..=guard::MAX_REDIRECTS {
        let target = if hop == 0 {
            initial.clone()
        } else {
            guard::validate_target(next.as_str()).await?
        };
        let client = guard::build_pinned_client(config, &target)?;
        let same_host = target.host == original_host;
        let mut request = client
            .request(method.clone(), target.url.clone())
            .timeout(timeout);
        for (name, value) in headers {
            if same_host
                || CROSS_HOST_HEADERS
                    .iter()
                    .any(|allowed| name.eq_ignore_ascii_case(allowed))
            {
                request = request.header(*name, *value);
            }
        }
        let response = request.send().await.map_err(request_error)?;
        if response.status().is_redirection() {
            if hop == guard::MAX_REDIRECTS {
                return Err(MediaError::new(
                    MediaErrorCode::DownloadFailed,
                    format!(
                        "trop de redirections HTTP (maximum {})",
                        guard::MAX_REDIRECTS
                    ),
                ));
            }
            let Some(location) = response.headers().get(LOCATION) else {
                return Err(MediaError::new(
                    MediaErrorCode::DownloadFailed,
                    "redirection HTTP sans en-tête Location",
                ));
            };
            let location = location.to_str().map_err(|error| {
                MediaError::with_raw(
                    MediaErrorCode::DownloadFailed,
                    "redirection HTTP illisible",
                    error.to_string(),
                )
            })?;
            next = guard::resolve_redirect(&target.url, location)?;
            continue;
        }
        return Ok((response, target.url));
    }
    Err(MediaError::new(
        MediaErrorCode::DownloadFailed,
        "boucle de redirections HTTP",
    ))
}

/// Récupère une page texte via le client gardé.
pub async fn get_text(
    config: &DownloadConfig,
    raw_url: &str,
    timeout: Duration,
    headers: &[(&str, &str)],
) -> MediaResult<HttpText> {
    let (response, final_url) =
        send_guarded(config, Method::GET, raw_url, timeout, headers).await?;
    let status = response.status();
    let captured_headers = response.headers().clone();
    if !status.is_success() {
        let body = read_body_bounded(response, MAX_ERROR_BODY_BYTES, false).await?;
        return Err(MediaError::with_raw(
            code_from_http_status(status.as_u16()),
            format!("la source a répondu HTTP {} ({final_url})", status.as_u16()),
            String::from_utf8_lossy(&body).into_owned(),
        ));
    }
    let body = read_body_bounded(response, MAX_TEXT_RESPONSE_BYTES, true).await?;
    Ok(HttpText {
        body: String::from_utf8_lossy(&body).into_owned(),
        final_url: final_url.to_string(),
        headers: captured_headers,
    })
}

/// Récupère et désérialise une réponse JSON via le client gardé.
pub async fn get_json<T: DeserializeOwned>(
    config: &DownloadConfig,
    raw_url: &str,
    timeout: Duration,
) -> MediaResult<T> {
    let text = get_text(config, raw_url, timeout, &[]).await?;
    serde_json::from_str(&text.body).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::InvalidMedia,
            format!("réponse JSON invalide ({})", text.final_url),
            error.to_string(),
        )
    })
}

/// Résout une URL courte en suivant les redirections gardées.
///
/// Best-effort : renvoie l'URL d'entrée si la résolution échoue.
pub async fn resolve_short_url(
    config: &DownloadConfig,
    raw_url: &str,
    timeout: Duration,
) -> String {
    let trimmed = raw_url.trim();
    for method in [Method::HEAD, Method::GET] {
        if let Ok((response, final_url)) = send_guarded(config, method, trimmed, timeout, &[]).await
        {
            if response.status().is_success() {
                return final_url.to_string();
            }
        }
    }
    trimmed.to_owned()
}

/// Convertit une erreur `reqwest` en [`MediaError`] typée.
pub fn request_error(error: reqwest::Error) -> MediaError {
    if error.is_timeout() {
        return MediaError::with_raw(
            MediaErrorCode::DownloadTimeout,
            "le téléchargement de la source a dépassé le délai autorisé",
            error.to_string(),
        );
    }
    if let Some(status) = error.status() {
        return MediaError::from_http_status(status.as_u16());
    }
    MediaError::with_raw(
        MediaErrorCode::DownloadFailed,
        format!("requête HTTP impossible : {error}"),
        error.to_string(),
    )
}

/// Construit une erreur pour un statut HTTP non 2xx.
pub fn request_error_for_status(status: u16, url: &str) -> MediaError {
    MediaError::with_raw(
        code_from_http_status(status),
        format!("la source a répondu HTTP {status} ({url})"),
        String::new(),
    )
}

/// Lit un corps de réponse en bornant le nombre d'octets.
async fn read_body_bounded(
    mut response: Response,
    limit: u64,
    enforce: bool,
) -> MediaResult<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        let next_len = body.len() as u64 + chunk.len() as u64;
        if next_len > limit {
            if enforce {
                return Err(MediaError::new(
                    MediaErrorCode::InvalidMedia,
                    format!("réponse HTTP supérieure à la limite de {limit} octets"),
                ));
            }
            let remaining = limit.saturating_sub(body.len() as u64) as usize;
            body.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::config::MediaConfig;

    fn download_config() -> DownloadConfig {
        MediaConfig::default().download
    }

    #[tokio::test]
    async fn refuses_loopback_before_sending() {
        let config = download_config();
        let error = send_guarded(
            &config,
            Method::GET,
            "http://127.0.0.1:9/video.mp4",
            Duration::from_secs(1),
            &[],
        )
        .await
        .expect_err("loopback refusé");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
    }

    #[tokio::test]
    async fn get_text_refuses_file_scheme() {
        let config = download_config();
        let error = get_text(&config, "file:/etc/passwd", Duration::from_secs(1), &[])
            .await
            .expect_err("file: refusé");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
    }
}
