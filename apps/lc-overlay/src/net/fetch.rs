//! Requête HTTP du shell vers un lien fourni par l'utilisateur : URL validée
//! (HTTP(S), sans identifiants), redirections plafonnées et revalidées, réponse
//! finale rendue à l'appelant. Le délai total reste à la charge de l'appelant :
//! une pochette de 10 Mio et une vidéo de plusieurs centaines de Mio n'ont pas
//! le même budget.

use std::time::Duration;

use reqwest::header::LOCATION;
use url::Url;

/// Durée maximale pour ouvrir la connexion.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Redirections suivies avant d'abandonner.
pub(crate) const MAX_REDIRECTS: usize = 3;

/// Pourquoi un lien n'a pas donné le fichier attendu ; [`LinkError::code`] est
/// ce que le board traduit en message (`core/artwork.js`,
/// `core/media-download.js`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkError {
    /// Pas un lien HTTP(S), ou redirection vers autre chose.
    Invalid,
    /// Échec DNS, connexion, TLS ou délai dépassé.
    Unreachable,
    /// Plus de [`MAX_REDIRECTS`] redirections.
    TooManyRedirects,
    /// Le site a répondu autre chose qu'un succès.
    Http(u16),
    /// La réponse n'est pas une image.
    NotImage,
    /// La réponse n'est pas un média.
    NotMedia,
    /// Le fichier dépasse la limite de son chemin.
    TooLarge,
    /// La réponse est vide.
    Empty,
    /// Le fichier n'a pas pu être écrit sur le disque.
    Io,
}

impl LinkError {
    /// Code stable lu par `artworkLinkError` et `mediaDownloadError` côté web.
    pub(crate) fn code(self) -> String {
        match self {
            LinkError::Invalid => "invalid_link".to_string(),
            LinkError::Unreachable => "unreachable".to_string(),
            LinkError::TooManyRedirects => "too_many_redirects".to_string(),
            LinkError::Http(status) => format!("http_{status}"),
            LinkError::NotImage => "not_image".to_string(),
            LinkError::NotMedia => "not_media".to_string(),
            LinkError::TooLarge => "too_large".to_string(),
            LinkError::Empty => "empty".to_string(),
            LinkError::Io => "io".to_string(),
        }
    }
}

/// Analyse le lien : URL absolue HTTP(S), sans identifiants.
pub(crate) fn parse_link(raw: &str) -> Result<Url, LinkError> {
    let url = Url::parse(raw.trim()).map_err(|_| LinkError::Invalid)?;
    check_link(&url)?;
    Ok(url)
}

fn check_link(url: &Url) -> Result<(), LinkError> {
    if url.host_str().is_none() || !matches!(url.scheme(), "http" | "https") {
        return Err(LinkError::Invalid);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(LinkError::Invalid);
    }
    Ok(())
}

/// Ouvre `raw` et rend la réponse finale, succès seulement : une redirection
/// est suivie jusqu'à [`MAX_REDIRECTS`] et revalidée, une réponse non 2xx
/// devient [`LinkError::Http`].
pub(crate) async fn get(raw: &str) -> Result<reqwest::Response, LinkError> {
    let mut target = parse_link(raw)?;
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("lc-overlay/", env!("CARGO_PKG_VERSION")))
        // Redirections suivies à la main pour les plafonner et revalider
        // chaque cible (même règle que le chemin WebView).
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| LinkError::Unreachable)?;
    for _ in 0..=MAX_REDIRECTS {
        let response = client
            .get(target.clone())
            .send()
            .await
            .map_err(|_| LinkError::Unreachable)?;
        if let Some(location) = redirect_of(&response) {
            target = target.join(&location).map_err(|_| LinkError::Invalid)?;
            check_link(&target)?;
            continue;
        }
        if !response.status().is_success() {
            return Err(LinkError::Http(response.status().as_u16()));
        }
        return Ok(response);
    }
    Err(LinkError::TooManyRedirects)
}

fn redirect_of(response: &reqwest::Response) -> Option<String> {
    if !response.status().is_redirection() {
        return None;
    }
    response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_link_keeps_http_links_and_refuses_the_rest() {
        assert!(parse_link("https://valorantinfo.com/images/us/chamber.webp").is_ok());
        assert!(parse_link("  http://example.com/a.png  ").is_ok());
        assert_eq!(parse_link("a.png"), Err(LinkError::Invalid));
        assert_eq!(parse_link("/a.png"), Err(LinkError::Invalid));
        assert_eq!(
            parse_link("ftp://example.com/a.png"),
            Err(LinkError::Invalid)
        );
        assert_eq!(parse_link("javascript:alert(1)"), Err(LinkError::Invalid));
        assert_eq!(
            parse_link("https://user:pass@example.com/a.png"),
            Err(LinkError::Invalid)
        );
        assert_eq!(
            parse_link("https://user@example.com/a.png"),
            Err(LinkError::Invalid)
        );
        assert_eq!(parse_link(""), Err(LinkError::Invalid));
    }

    #[test]
    fn every_error_has_a_stable_code() {
        assert_eq!(LinkError::Invalid.code(), "invalid_link");
        assert_eq!(LinkError::Unreachable.code(), "unreachable");
        assert_eq!(LinkError::TooManyRedirects.code(), "too_many_redirects");
        assert_eq!(LinkError::Http(500).code(), "http_500");
        assert_eq!(LinkError::NotImage.code(), "not_image");
        assert_eq!(LinkError::NotMedia.code(), "not_media");
        assert_eq!(LinkError::TooLarge.code(), "too_large");
        assert_eq!(LinkError::Empty.code(), "empty");
        assert_eq!(LinkError::Io.code(), "io");
    }
}
