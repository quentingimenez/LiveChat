//! Envoi d'un fichier local vers la bibliothèque média du board.
//!
//! La board lit le fichier par morceaux et les pousse via
//! `media_upload_chunk` ; le shell les diffuse en corps brut (pas de
//! multipart) vers `POST {serverUrl}/overlay/media-board/upload`. Le jeton de
//! la guilde ne quitte jamais le shell Rust.

use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::mpsc;
use url::Url;

use crate::net::api;

/// Uploads allowed at the same time across every guild.
pub(crate) const MAX_CONCURRENT_UPLOADS: usize = 8;
/// Largest chunk accepted from the board (8 MiB).
pub(crate) const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
/// Longest accepted file name, in characters.
const MAX_FILENAME_CHARS: usize = 255;
/// Server-side media conversion can take several minutes.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Connection establishment timeout for an upload.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Route reached by an upload.
pub(crate) const UPLOAD_PATH: &str = "/overlay/media-board/upload";
/// Fallback when the board does not send a usable content type.
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// A chunk forwarded to the streaming request body.
pub(crate) type UploadChunk = Result<Vec<u8>, io::Error>;
/// Sender half kept in the registry while the upload is in flight.
pub(crate) type ChunkSender = mpsc::Sender<UploadChunk>;
/// Background task driving the HTTP request and reading its response.
pub(crate) type UploadTask = tauri::async_runtime::JoinHandle<Result<api::ApiResponse, String>>;

/// In-flight upload, removed when finished or cancelled.
pub(crate) struct UploadEntry {
    pub(crate) guild_id: String,
    pub(crate) sender: ChunkSender,
    pub(crate) task: UploadTask,
}

/// Uploads in flight, managed by Tauri and keyed by `uploadId`.
#[derive(Default)]
pub struct UploadRegistry {
    entries: Mutex<HashMap<String, UploadEntry>>,
}

impl UploadRegistry {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, UploadEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Registers an upload. Fails once the concurrent upload limit is reached.
    ///
    /// At the limit, uploads whose request already ended are dropped first:
    /// a board reloaded mid-upload never calls `finish`, and its entries
    /// would otherwise hold a slot until the application restarts.
    pub(crate) fn insert(&self, upload_id: String, entry: UploadEntry) -> Result<(), String> {
        let mut entries = self.lock();
        if entries.len() >= MAX_CONCURRENT_UPLOADS {
            entries.retain(|_, existing| !existing.task.inner().is_finished());
        }
        if entries.len() >= MAX_CONCURRENT_UPLOADS {
            entry.task.abort();
            return Err("trop d'envois en cours".to_string());
        }
        entries.insert(upload_id, entry);
        Ok(())
    }

    /// Clones the chunk sender of an upload, without holding the lock across
    /// the subsequent `.await`.
    pub(crate) fn sender(&self, upload_id: &str) -> Result<ChunkSender, String> {
        self.lock()
            .get(upload_id)
            .map(|entry| entry.sender.clone())
            .ok_or_else(|| "envoi inconnu".to_string())
    }

    /// Removes and returns an upload (dropping the sender closes the body).
    pub(crate) fn take(&self, upload_id: &str) -> Option<UploadEntry> {
        self.lock().remove(upload_id)
    }

    /// Cancels an upload; `false` when the id is unknown.
    pub(crate) fn cancel(&self, upload_id: &str) -> bool {
        match self.lock().remove(upload_id) {
            Some(entry) => {
                entry.task.abort();
                true
            }
            None => false,
        }
    }
}

/// Validates the board-supplied file name.
///
/// The name is trimmed, must be 1 to 255 characters, and may not contain a
/// path separator or a control character. It only travels as a query
/// parameter, so dots ("wait for it....mp4") are harmless.
pub(crate) fn validate_filename(raw: Option<&str>) -> Result<String, String> {
    let Some(filename) = raw.map(str::trim).filter(|name| !name.is_empty()) else {
        return Err("nom de fichier invalide".to_string());
    };
    if filename.chars().count() > MAX_FILENAME_CHARS
        || filename.contains('/')
        || filename.contains('\\')
        || filename.chars().any(char::is_control)
    {
        return Err("nom de fichier invalide".to_string());
    }
    Ok(filename.to_string())
}

/// Cleans the optional content type.
///
/// Only a `type/subtype` shape made of printable ASCII characters (no space,
/// no `;`) is accepted; anything else falls back to
/// `application/octet-stream`.
pub(crate) fn normalize_content_type(raw: Option<&str>) -> String {
    let Some(content_type) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return DEFAULT_CONTENT_TYPE.to_string();
    };
    if content_type
        .chars()
        .any(|character| !character.is_ascii_graphic() || character == ';')
    {
        return DEFAULT_CONTENT_TYPE.to_string();
    }
    let mut parts = content_type.split('/');
    let Some(main_type) = parts.next() else {
        return DEFAULT_CONTENT_TYPE.to_string();
    };
    let Some(sub_type) = parts.next() else {
        return DEFAULT_CONTENT_TYPE.to_string();
    };
    if main_type.is_empty() || sub_type.is_empty() || parts.next().is_some() {
        return DEFAULT_CONTENT_TYPE.to_string();
    }
    content_type.to_string()
}

/// Builds the upload URL with the optional query parameters.
pub(crate) fn upload_url(
    server_url: &str,
    filename: &str,
    title: Option<&str>,
    message: Option<&str>,
    allow_insecure: bool,
) -> Result<Url, String> {
    let mut url = api::build_url(server_url, UPLOAD_PATH, allow_insecure)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("filename", filename);
        for (key, value) in [("title", title), ("message", message)] {
            if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
                query.append_pair(key, value);
            }
        }
    }
    Ok(url)
}

/// Builds the HTTP client used for uploads (long timeout, dev TLS override).
pub(crate) fn build_client(insecure: bool) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(UPLOAD_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("lc-overlay/", env!("CARGO_PKG_VERSION")));
    if insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder
        .build()
        .map_err(|error| format!("client HTTP impossible : {error}"))
}

/// Runs the streamed request until the server responds.
pub(crate) async fn run_upload(
    client: reqwest::Client,
    url: Url,
    token: &str,
    content_type: &str,
    receiver: mpsc::Receiver<UploadChunk>,
) -> Result<api::ApiResponse, String> {
    let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(
        receiver,
        |mut receiver| async move { receiver.recv().await.map(|chunk| (chunk, receiver)) },
    ));
    let response = client
        .post(url)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(body)
        .send()
        .await
        .map_err(|error| format!("serveur injoignable : {error}"))?;
    Ok(api::read_response(response).await)
}

/// Response of `media_upload_start`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaUploadStartResponse {
    pub upload_id: String,
}

impl MediaUploadStartResponse {
    pub fn new(upload_id: String) -> Self {
        Self { upload_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn filename_validation_accepts_display_names_and_rejects_paths() {
        assert_eq!(
            validate_filename(Some("  L’Escale 🔱 & co.mp4  ")).expect("nom valide"),
            "L’Escale 🔱 & co.mp4"
        );
        assert_eq!(
            validate_filename(Some("clip.mp4")).expect("nom valide"),
            "clip.mp4"
        );
        assert_eq!(
            validate_filename(Some("wait for it....mp4")).expect("nom valide"),
            "wait for it....mp4"
        );
        for invalid in [
            None,
            Some(""),
            Some("   "),
            Some("dossier/clip.mp4"),
            Some("dossier\\clip.mp4"),
            Some("..\\clip.mp4"),
            Some("clip\n.mp4"),
            Some("clip\u{0}.mp4"),
        ] {
            assert!(
                validate_filename(invalid).is_err(),
                "nom refusé attendu : {invalid:?}"
            );
        }
        let too_long = "a".repeat(MAX_FILENAME_CHARS + 1);
        assert!(validate_filename(Some(&too_long)).is_err());
        let longest = "a".repeat(MAX_FILENAME_CHARS);
        assert!(validate_filename(Some(&longest)).is_ok());
    }

    #[test]
    fn content_type_is_cleaned_or_falls_back_to_octet_stream() {
        assert_eq!(normalize_content_type(Some(" video/mp4 ")), "video/mp4");
        assert_eq!(
            normalize_content_type(Some("application/vnd.livechat+json")),
            "application/vnd.livechat+json"
        );
        for invalid in [
            None,
            Some(""),
            Some("video"),
            Some("video/"),
            Some("/mp4"),
            Some("video/mp4; charset=utf-8"),
            Some("video/mp 4"),
            Some("vidéo/mp4"),
        ] {
            assert_eq!(
                normalize_content_type(invalid),
                DEFAULT_CONTENT_TYPE,
                "content type refusé attendu : {invalid:?}"
            );
        }
    }

    #[test]
    fn upload_url_encodes_the_filename_and_omits_empty_texts() {
        let url = upload_url(
            "https://api.example.test/",
            "L’Escale 🔱 & co.mp4",
            Some("  Titre  "),
            Some("   "),
            false,
        )
        .expect("url");
        assert_eq!(url.path(), UPLOAD_PATH);
        let pairs = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            pairs,
            vec![
                ("filename".to_string(), "L’Escale 🔱 & co.mp4".to_string()),
                ("title".to_string(), "Titre".to_string()),
            ]
        );
    }

    /// An upload still in flight: its request never ends.
    fn dummy_entry() -> UploadEntry {
        let (sender, _receiver) = mpsc::channel(4);
        let task = tauri::async_runtime::spawn(std::future::pending());
        UploadEntry {
            guild_id: "111".to_string(),
            sender,
            task,
        }
    }

    /// An upload whose request already ended (answered or timed out).
    fn finished_entry() -> UploadEntry {
        let (sender, _receiver) = mpsc::channel(4);
        let task = tauri::async_runtime::spawn(async {
            Ok::<api::ApiResponse, String>(api::ApiResponse {
                ok: true,
                status: 200,
                body: serde_json::Value::Null,
            })
        });
        UploadEntry {
            guild_id: "111".to_string(),
            sender,
            task,
        }
    }

    #[test]
    fn registry_limits_concurrent_uploads_and_rejects_unknown_ids() {
        let registry = UploadRegistry::default();
        for index in 0..MAX_CONCURRENT_UPLOADS {
            registry
                .insert(format!("upload-{index}"), dummy_entry())
                .expect("envoi accepté");
        }
        assert_eq!(
            registry
                .insert("too-many".to_string(), dummy_entry())
                .expect_err("limite atteinte"),
            "trop d'envois en cours"
        );
        assert!(registry.sender("unknown").is_err());
        assert!(!registry.cancel("unknown"));
        assert!(registry.cancel("upload-0"));
        assert!(registry.take("upload-0").is_none());
        // A slot was freed: the limit no longer rejects.
        registry
            .insert("after-free".to_string(), dummy_entry())
            .expect("envoi accepté après libération");
    }

    #[tokio::test]
    async fn registry_frees_slots_of_uploads_that_already_ended() {
        let registry = UploadRegistry::default();
        for index in 0..MAX_CONCURRENT_UPLOADS {
            registry
                .insert(format!("upload-{index}"), finished_entry())
                .expect("envoi accepté");
        }
        // These tasks end at once, as an abandoned upload does once its
        // request times out.
        for index in 0..MAX_CONCURRENT_UPLOADS {
            while !registry
                .lock()
                .get(&format!("upload-{index}"))
                .is_some_and(|entry| entry.task.inner().is_finished())
            {
                tokio::task::yield_now().await;
            }
        }
        registry
            .insert("after-timeout".to_string(), dummy_entry())
            .expect("les envois terminés libèrent leur place");
        assert!(registry.sender("after-timeout").is_ok());
        assert!(registry.sender("upload-0").is_err());
    }

    /// Reads a chunked HTTP/1.1 request until the terminating chunk, then
    /// replies with a minimal JSON response.
    async fn read_chunked_request(stream: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            let read = stream.read(&mut chunk).await.expect("lecture des en-têtes");
            assert!(read > 0, "connexion fermée avant les en-têtes");
            buffer.extend_from_slice(&chunk[..read]);
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();

        let mut body = Vec::new();
        let mut position = header_end;
        loop {
            let line_end = loop {
                if let Some(offset) = buffer[position..]
                    .windows(2)
                    .position(|window| window == b"\r\n")
                {
                    break offset;
                }
                let read = stream.read(&mut chunk).await.expect("lecture d'un chunk");
                assert!(read > 0, "connexion fermée avant la fin du corps");
                buffer.extend_from_slice(&chunk[..read]);
            };
            let line = String::from_utf8_lossy(&buffer[position..position + line_end]).to_string();
            position += line_end + 2;
            let size = usize::from_str_radix(line.trim(), 16).expect("taille de chunk");
            if size == 0 {
                break;
            }
            while buffer.len() < position + size + 2 {
                let read = stream.read(&mut chunk).await.expect("lecture du corps");
                assert!(read > 0, "connexion fermée avant la fin du chunk");
                buffer.extend_from_slice(&chunk[..read]);
            }
            body.extend_from_slice(&buffer[position..position + size]);
            assert_eq!(&buffer[position + size..position + size + 2], b"\r\n");
            position += size + 2;
        }

        let response_body = r#"{"created":true}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("réponse serveur");
        (headers, body)
    }

    #[tokio::test]
    async fn streamed_upload_forwards_chunks_in_order_to_the_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let server = tokio::spawn(async move {
            let (mut stream, _peer) = listener.accept().await.expect("accept");
            read_chunked_request(&mut stream).await
        });

        let url = upload_url(
            &format!("http://{address}"),
            "L’Escale 🔱 & co.mp4",
            None,
            None,
            true,
        )
        .expect("url");
        let client = build_client(true).expect("client");
        let (sender, receiver) = mpsc::channel(4);
        let upload = tokio::spawn(async move {
            run_upload(client, url, "test-token", "video/mp4", receiver).await
        });
        sender
            .send(Ok(b"premier".to_vec()))
            .await
            .expect("premier morceau");
        sender
            .send(Ok(b"second".to_vec()))
            .await
            .expect("second morceau");
        drop(sender);

        let response = upload.await.expect("tâche d'envoi").expect("réponse");
        assert!(response.ok);
        assert_eq!(response.status, 200);
        assert_eq!(response.body["created"], serde_json::json!(true));

        let (headers, body) = server.await.expect("serveur mock");
        let headers = headers.to_ascii_lowercase();
        assert!(
            headers.contains("authorization: bearer test-token"),
            "{headers}"
        );
        assert!(headers.contains("content-type: video/mp4"), "{headers}");
        assert!(headers.contains("transfer-encoding: chunked"), "{headers}");
        assert_eq!(body, b"premiersecond");
    }
}
