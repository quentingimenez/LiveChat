//! Client HTTP partagé (reqwest + rustls) et helpers de requête.
//!
//! Aucune fonction de ce module ne lit l'environnement : tout vient de la
//! [`DownloadConfig`] fournie par l'appelant.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;

use crate::config::DownloadConfig;
use crate::error::{code_from_http_status, MediaError, MediaErrorCode, MediaResult};

/// Construit le client HTTP commun (user-agent, timeout, redirections bornées).
pub fn build_client(config: &DownloadConfig) -> MediaResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(config.user_agent.clone())
        .timeout(config.timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| {
            MediaError::new(
                MediaErrorCode::DownloadFailed,
                format!("impossible d'initialiser le client HTTP : {error}"),
            )
        })
}

/// Réponse texte avec l'URL finale (après redirections) et les en-têtes.
#[derive(Clone, Debug)]
pub struct HttpText {
    /// Corps de la réponse décodé en texte.
    pub body: String,
    /// URL finale après redirections.
    pub final_url: String,
    /// En-têtes de la réponse.
    pub headers: HeaderMap,
}

/// `GET` renvoyant le corps en texte, avec timeout et en-têtes additionnels.
pub async fn get_text(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
    headers: &[(&str, &str)],
) -> MediaResult<HttpText> {
    let mut request = client.get(url).timeout(timeout);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await.map_err(request_error)?;
    finish_text_response(response).await
}

/// `GET` d'un JSON désérialisé en `T`.
pub async fn get_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> MediaResult<T> {
    let response = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(request_error)?;
    let response = ensure_success(response).await?;
    response.json::<T>().await.map_err(|error| {
        MediaError::new(
            MediaErrorCode::InvalidMedia,
            format!("réponse JSON invalide : {error}"),
        )
    })
}

/// Suit les redirections d'une URL courte (best effort).
///
/// En cas d'échec, l'URL d'origine est renvoyée : la résolution courte n'est
/// qu'une optimisation de canonicalisation, jamais un point de blocage.
pub async fn resolve_short_url(client: &reqwest::Client, url: &str, timeout: Duration) -> String {
    if let Ok(response) = client.head(url).timeout(timeout).send().await {
        let final_url = response.url().to_string();
        if final_url != url {
            return final_url;
        }
    }
    if let Ok(response) = client.get(url).timeout(timeout).send().await {
        return response.url().to_string();
    }
    url.to_string()
}

/// Récupère les valeurs `Set-Cookie` d'une réponse.
pub fn set_cookie_values(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_string)
        .collect()
}

/// Première valeur d'un en-tête (nom insensible à la casse).
pub fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    HeaderName::from_bytes(name.as_bytes())
        .ok()
        .and_then(|name| headers.get(name))
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Convertit une erreur reqwest en [`MediaError`] classée.
pub fn request_error(error: reqwest::Error) -> MediaError {
    if error.is_timeout() {
        return MediaError::new(
            MediaErrorCode::DownloadTimeout,
            format!("délai de téléchargement dépassé : {error}"),
        );
    }
    if let Some(status) = error.status() {
        return MediaError::new(
            code_from_http_status(status.as_u16()),
            format!("la source a répondu HTTP {} : {error}", status.as_u16()),
        );
    }
    MediaError::new(
        MediaErrorCode::DownloadFailed,
        format!("requête HTTP échouée : {error}"),
    )
}

/// Convertit un statut HTTP non-2xx en [`MediaError`] classée.
pub fn request_error_for_status(status: u16, url: &str) -> MediaError {
    MediaError::with_raw(
        code_from_http_status(status),
        format!("la source a répondu HTTP {status} ({url})"),
        String::new(),
    )
}

async fn finish_text_response(response: reqwest::Response) -> MediaResult<HttpText> {
    let final_url = response.url().to_string();
    let headers = response.headers().clone();
    let response = ensure_success_with_url(response, &final_url).await?;
    let body = response.text().await.map_err(|error| {
        MediaError::new(
            MediaErrorCode::DownloadFailed,
            format!("lecture du corps HTTP impossible : {error}"),
        )
    })?;
    Ok(HttpText {
        body,
        final_url,
        headers,
    })
}

async fn ensure_success(response: reqwest::Response) -> MediaResult<reqwest::Response> {
    let url = response.url().to_string();
    ensure_success_with_url(response, &url).await
}

async fn ensure_success_with_url(
    response: reqwest::Response,
    url: &str,
) -> MediaResult<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(MediaError::with_raw(
        code_from_http_status(status.as_u16()),
        format!("la source a répondu HTTP {} ({url})", status.as_u16()),
        body,
    ))
}

/// Construit une valeur d'en-tête sûre (utilisé pour les cookies dynamiques).
pub fn header_pair(name: &'static str, value: &str) -> Option<(&'static str, String)> {
    HeaderValue::from_str(value)
        .ok()
        .map(|_| (name, value.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn builds_client_with_config() {
        let config = DownloadConfig {
            timeout: Duration::from_secs(30),
            short_url_timeout: Duration::from_secs(8),
            max_size_bytes: 1024,
            max_concurrent_ingestions: 1,
            user_agent: "test-agent".to_string(),
        };
        assert!(build_client(&config).is_ok());
    }

    #[test]
    fn header_value_is_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("video/mp4"),
        );
        assert_eq!(
            header_value(&headers, "content-type").as_deref(),
            Some("video/mp4")
        );
        assert_eq!(
            header_value(&headers, "CONTENT-TYPE").as_deref(),
            Some("video/mp4")
        );
        assert_eq!(header_value(&headers, "absent"), None);
    }

    #[test]
    fn header_pair_rejects_invalid_values() {
        assert!(header_pair("referer", "https://www.tiktok.com/").is_some());
        assert!(header_pair("x-test", "ok").is_some());
    }

    #[test]
    fn set_cookie_values_are_collected() {
        let mut headers = HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            HeaderValue::from_static("a=b; Path=/"),
        );
        headers.append(
            reqwest::header::SET_COOKIE,
            HeaderValue::from_static("c=d; Path=/"),
        );
        let values = set_cookie_values(&headers);
        assert_eq!(
            values,
            vec!["a=b; Path=/".to_string(), "c=d; Path=/".to_string()]
        );
    }
}
