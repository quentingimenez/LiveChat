//! Téléchargement d'une pochette de board par le shell.
//!
//! Le WebView ne peut pas `fetch` un site qui n'envoie pas d'en-tête CORS ; le
//! board demande donc l'image au shell, qui la télécharge ici et la renvoie en
//! base64. Mêmes limites que le chemin WebView (HTTP(S) uniquement, 10 Mio,
//! images seulement), plus les gardes propres à une requête faite pour
//! l'utilisateur : pas d'identifiants dans l'URL, redirections plafonnées.

use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, LOCATION};
use url::Url;

use crate::net::upload::{normalize_content_type, MAX_ARTWORK_BYTES};

/// Durée maximale du téléchargement complet.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Durée maximale pour ouvrir la connexion.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Redirections suivies avant d'abandonner.
const MAX_REDIRECTS: usize = 3;

/// Pourquoi un lien n'a pas donné d'image ; [`LinkError::code`] est ce que le
/// board traduit en message (`core/artwork.js`).
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
    /// L'image dépasse [`MAX_ARTWORK_BYTES`].
    TooLarge,
    /// La réponse est vide.
    Empty,
}

impl LinkError {
    /// Code stable lu par `artworkLinkError` côté web.
    pub(crate) fn code(self) -> String {
        match self {
            LinkError::Invalid => "invalid_link".to_string(),
            LinkError::Unreachable => "unreachable".to_string(),
            LinkError::TooManyRedirects => "too_many_redirects".to_string(),
            LinkError::Http(status) => format!("http_{status}"),
            LinkError::NotImage => "not_image".to_string(),
            LinkError::TooLarge => "too_large".to_string(),
            LinkError::Empty => "empty".to_string(),
        }
    }
}

/// Image téléchargée et son type MIME final.
#[derive(Debug)]
pub(crate) struct Downloaded {
    pub(crate) content_type: String,
    pub(crate) bytes: Vec<u8>,
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

/// Télécharge l'image de `raw`, en suivant jusqu'à [`MAX_REDIRECTS`]
/// redirections.
pub(crate) async fn download(raw: &str) -> Result<Downloaded, LinkError> {
    tokio::time::timeout(DOWNLOAD_TIMEOUT, download_inner(raw))
        .await
        .unwrap_or(Err(LinkError::Unreachable))
}

async fn download_inner(raw: &str) -> Result<Downloaded, LinkError> {
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
        let Some(location) = redirect_of(&response) else {
            return read_picture(response).await;
        };
        target = target.join(&location).map_err(|_| LinkError::Invalid)?;
        check_link(&target)?;
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

async fn read_picture(response: reqwest::Response) -> Result<Downloaded, LinkError> {
    let status = response.status();
    if !status.is_success() {
        return Err(LinkError::Http(status.as_u16()));
    }
    let declared = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| normalize_content_type(Some(value)));
    if let Some(length) = response.content_length() {
        if length == 0 {
            return Err(LinkError::Empty);
        }
        if length > MAX_ARTWORK_BYTES as u64 {
            return Err(LinkError::TooLarge);
        }
    }
    let mut body = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = body.chunk().await.map_err(|_| LinkError::Unreachable)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_ARTWORK_BYTES {
            return Err(LinkError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err(LinkError::Empty);
    }
    let content_type = picture_type(declared.as_deref(), &bytes).ok_or(LinkError::NotImage)?;
    Ok(Downloaded {
        content_type,
        bytes,
    })
}

/// Type MIME de l'image : le type déclaré s'il en est déjà un, sinon celui que
/// disent les octets.
fn picture_type(declared: Option<&str>, bytes: &[u8]) -> Option<String> {
    if let Some(kind) =
        declared.filter(|kind| kind.len() > 6 && kind[..6].eq_ignore_ascii_case("image/"))
    {
        return Some(kind.to_string());
    }
    sniff_type(bytes).map(str::to_string)
}

/// Reconnaît les formats que le serveur accepte comme image.
fn sniff_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    const WEBP: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";

    fn http_response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {status}\r\n");
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str(&format!(
            "content-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        ));
        let mut response = head.into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn redirect(location: &str) -> Vec<u8> {
        http_response("302 Found", &[("location", location)], b"")
    }

    /// Sert chaque requête par une connexion dédiée, avec la réponse que
    /// calcule `reply` à partir du chemin.
    async fn serve(
        mut reply: impl FnMut(&str) -> Vec<u8> + Send + 'static,
    ) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("écoute");
        let address = listener.local_addr().expect("adresse");
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let read = match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&request);
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .map(str::to_string)
                    .unwrap_or_else(|| "/".to_string());
                if socket.write_all(&reply(&path)).await.is_err() {
                    return;
                }
            }
        });
        (format!("http://{address}"), task)
    }

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
    fn sniff_type_knows_the_formats_the_server_accepts() {
        assert_eq!(sniff_type(PNG), Some("image/png"));
        assert_eq!(sniff_type(b"\xFF\xD8\xFF\xE0rest"), Some("image/jpeg"));
        assert_eq!(sniff_type(b"GIF89a\x01\x00"), Some("image/gif"));
        assert_eq!(sniff_type(WEBP), Some("image/webp"));
        assert_eq!(sniff_type(b"<html>404</html>"), None);
        assert_eq!(sniff_type(b""), None);
    }

    #[test]
    fn picture_type_prefers_a_declared_image_type() {
        assert_eq!(
            picture_type(Some("image/png"), b""),
            Some("image/png".to_string())
        );
        assert_eq!(
            picture_type(Some("image/webp"), PNG),
            Some("image/webp".to_string())
        );
        // Un type non-image ne fait pas foi : les octets décident.
        assert_eq!(
            picture_type(Some("text/html"), PNG),
            Some("image/png".to_string())
        );
        assert_eq!(picture_type(None, WEBP), Some("image/webp".to_string()));
        assert_eq!(picture_type(Some("text/html"), b"<html></html>"), None);
    }

    #[tokio::test]
    async fn download_returns_the_picture_and_its_type() {
        let (server, task) =
            serve(|_| http_response("200 OK", &[("content-type", "image/png")], PNG)).await;
        let picture = download(&format!("{server}/chamber.webp"))
            .await
            .expect("téléchargé");
        assert_eq!(picture.content_type, "image/png");
        assert_eq!(picture.bytes, PNG);
        task.abort();
    }

    #[tokio::test]
    async fn download_sniffs_a_declared_octet_stream() {
        let (server, task) = serve(|_| {
            http_response(
                "200 OK",
                &[("content-type", "application/octet-stream")],
                WEBP,
            )
        })
        .await;
        let picture = download(&format!("{server}/image"))
            .await
            .expect("téléchargé");
        assert_eq!(picture.content_type, "image/webp");
        task.abort();
    }

    #[tokio::test]
    async fn download_reports_http_errors() {
        let (server, task) = serve(|_| http_response("404 Not Found", &[], b"gone")).await;
        let error = download(&format!("{server}/missing.png"))
            .await
            .expect_err("404");
        assert_eq!(error, LinkError::Http(404));
        assert_eq!(error.code(), "http_404");
        task.abort();
    }

    #[tokio::test]
    async fn download_refuses_a_page_that_is_not_a_picture() {
        let (server, task) =
            serve(|_| http_response("200 OK", &[("content-type", "text/html")], b"<html></html>"))
                .await;
        let error = download(&format!("{server}/page")).await.expect_err("page");
        assert_eq!(error, LinkError::NotImage);
        task.abort();
    }

    #[tokio::test]
    async fn download_refuses_a_picture_over_the_limit() {
        let (server, task) = serve(|_| {
            let mut response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                MAX_ARTWORK_BYTES + 1
            )
            .into_bytes();
            response.extend_from_slice(PNG);
            response
        })
        .await;
        let error = download(&format!("{server}/huge.png"))
            .await
            .expect_err("trop grande");
        assert_eq!(error, LinkError::TooLarge);
        task.abort();
    }

    #[tokio::test]
    async fn download_refuses_an_oversized_stream() {
        let (server, task) = serve(|_| {
            let mut head = String::from(
                "HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
            );
            let body = vec![0x89u8; MAX_ARTWORK_BYTES + 1];
            head.push_str(&format!("{:x}\r\n", body.len()));
            let mut response = head.into_bytes();
            response.extend_from_slice(&body);
            response.extend_from_slice(b"\r\n");
            response
        })
        .await;
        let error = download(&format!("{server}/stream.png"))
            .await
            .expect_err("flux trop grand");
        assert_eq!(error, LinkError::TooLarge);
        task.abort();
    }

    #[tokio::test]
    async fn download_refuses_an_empty_answer() {
        let (server, task) =
            serve(|_| http_response("200 OK", &[("content-type", "image/png")], b"")).await;
        let error = download(&format!("{server}/empty.png"))
            .await
            .expect_err("vide");
        assert_eq!(error, LinkError::Empty);
        task.abort();
    }

    #[tokio::test]
    async fn download_follows_redirects_and_keeps_the_final_type() {
        let (server, task) = serve(|path| match path {
            "/start" => redirect("/hop"),
            "/hop" => redirect("/image"),
            _ => http_response("200 OK", &[("content-type", "image/png")], PNG),
        })
        .await;
        let picture = download(&format!("{server}/start")).await.expect("suivi");
        assert_eq!(picture.bytes, PNG);
        task.abort();
    }

    #[tokio::test]
    async fn download_stops_after_too_many_redirects() {
        let (server, task) = serve(|path| match path {
            "/r0" => redirect("/r1"),
            "/r1" => redirect("/r2"),
            "/r2" => redirect("/r3"),
            "/r3" => redirect("/image"),
            _ => http_response("200 OK", &[("content-type", "image/png")], PNG),
        })
        .await;
        let error = download(&format!("{server}/r0"))
            .await
            .expect_err("trop de redirections");
        assert_eq!(error, LinkError::TooManyRedirects);
        task.abort();
    }

    #[tokio::test]
    async fn download_refuses_a_redirect_to_another_scheme() {
        let (server, task) = serve(|_| redirect("ftp://example.com/image.png")).await;
        let error = download(&format!("{server}/start")).await.expect_err("ftp");
        assert_eq!(error, LinkError::Invalid);
        task.abort();
    }

    #[tokio::test]
    async fn download_reports_an_unreachable_site() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("écoute");
        let address = listener.local_addr().expect("adresse");
        drop(listener);
        let error = download(&format!("http://{address}/image.png"))
            .await
            .expect_err("injoignable");
        assert_eq!(error, LinkError::Unreachable);
    }

    #[test]
    fn codes_are_stable_for_the_board() {
        assert_eq!(LinkError::Invalid.code(), "invalid_link");
        assert_eq!(LinkError::Unreachable.code(), "unreachable");
        assert_eq!(LinkError::TooManyRedirects.code(), "too_many_redirects");
        assert_eq!(LinkError::Http(500).code(), "http_500");
        assert_eq!(LinkError::NotImage.code(), "not_image");
        assert_eq!(LinkError::TooLarge.code(), "too_large");
        assert_eq!(LinkError::Empty.code(), "empty");
    }
}
