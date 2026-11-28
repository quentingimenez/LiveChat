//! A tiny HTTP server for the `net` tests: one connection per request, one
//! response computed from the path. Shared so the artwork and media download
//! suites exercise the same server instead of two copies that drift apart.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub(crate) fn http_response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
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

pub(crate) fn redirect(location: &str) -> Vec<u8> {
    http_response("302 Found", &[("location", location)], b"")
}

/// Serves each request on its own connection with the response `reply`
/// computes from the path.
pub(crate) async fn serve(
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
