//! Téléchargement d'une pochette de board par le shell.
//!
//! Le WebView ne peut pas `fetch` un site qui n'envoie pas d'en-tête CORS ; le
//! board demande donc l'image au shell, qui la télécharge ici et la renvoie en
//! base64. Mêmes limites que le chemin WebView (HTTP(S) uniquement, 10 Mio,
//! images seulement), plus les gardes propres à une requête faite pour
//! l'utilisateur : pas d'identifiants dans l'URL, redirections plafonnées.

use std::time::Duration;

use reqwest::header::CONTENT_TYPE;

use crate::net::fetch::get;
use crate::net::upload::{normalize_content_type, MAX_ARTWORK_BYTES};

pub(crate) use crate::net::fetch::LinkError;

/// Durée maximale du téléchargement complet.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20);

/// Image téléchargée et son type MIME final.
#[derive(Debug)]
pub(crate) struct Downloaded {
    pub(crate) content_type: String,
    pub(crate) bytes: Vec<u8>,
}

/// Télécharge l'image de `raw`, en suivant jusqu'à [`crate::net::fetch::MAX_REDIRECTS`]
/// redirections.
pub(crate) async fn download(raw: &str) -> Result<Downloaded, LinkError> {
    tokio::time::timeout(DOWNLOAD_TIMEOUT, download_inner(raw))
        .await
        .unwrap_or(Err(LinkError::Unreachable))
}

async fn download_inner(raw: &str) -> Result<Downloaded, LinkError> {
    let response = get(raw).await?;
    read_picture(response).await
}

async fn read_picture(response: reqwest::Response) -> Result<Downloaded, LinkError> {
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
    use crate::net::test_support::{http_response, redirect, serve};
    use tokio::net::TcpListener;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
    const WEBP: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";

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
}
