//! Envoi d'un fichier local vers la bibliothèque média du board.
//!
//! La board lit le fichier par morceaux et les pousse via
//! `media_upload_chunk`. Le shell ouvre un envoi sur le serveur
//! (`POST {serverUrl}/overlay/media-board/uploads`), diffuse les morceaux en
//! parties brutes de [`PART_TARGET_BYTES`] environ
//! (`PUT …/uploads/{uploadId}/parts?offset=`), puis le termine
//! (`POST …/uploads/{uploadId}/complete`). Les parties restent sous la limite
//! de corps de requête d'un proxy placé devant le serveur (Cloudflare refuse
//! plus de 100 Mo). Le jeton de la guilde ne quitte jamais le shell Rust.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use url::Url;

use crate::net::api;

/// Uploads allowed at the same time across every guild.
pub(crate) const MAX_CONCURRENT_UPLOADS: usize = 8;
/// Largest chunk accepted from the board (8 MiB).
pub(crate) const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
/// A part ends with the chunk that reaches this size (64 MiB), so a part
/// never exceeds 72 MiB: under the server's 90 MiB and Cloudflare's 100 MB.
pub(crate) const PART_TARGET_BYTES: u64 = 64 * 1024 * 1024;
const _: () = assert!(PART_TARGET_BYTES + MAX_CHUNK_BYTES as u64 <= 90 * 1024 * 1024);
/// Longest accepted file name, in characters.
const MAX_FILENAME_CHARS: usize = 255;
/// A part can take long on a slow link; server-side checks are quick.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Connection establishment timeout for an upload.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Route opening an upload; parts, completion and cancellation live below it.
pub(crate) const UPLOADS_PATH: &str = "/overlay/media-board/uploads";

/// A chunk of the file, forwarded to the current part.
pub(crate) type UploadChunk = Vec<u8>;
/// Sender half kept in the registry while the upload is in flight.
pub(crate) type ChunkSender = mpsc::Sender<UploadChunk>;
/// Background task driving the upload and returning the server's answer.
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

    /// Removes and returns an upload (dropping the sender ends the file).
    pub(crate) fn take(&self, upload_id: &str) -> Option<UploadEntry> {
        self.lock().remove(upload_id)
    }

    /// Cancels an upload; `false` when the id is unknown. The aborted task
    /// abandons the server-side upload.
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
/// path separator or a control character. It only travels in a JSON body, so
/// dots ("wait for it....mp4") are harmless.
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
/// no `;`) is kept; anything else is dropped.
pub(crate) fn normalize_content_type(raw: Option<&str>) -> Option<String> {
    let content_type = raw.map(str::trim).filter(|value| !value.is_empty())?;
    if content_type
        .chars()
        .any(|character| !character.is_ascii_graphic() || character == ';')
    {
        return None;
    }
    let mut parts = content_type.split('/');
    let main_type = parts.next()?;
    let sub_type = parts.next()?;
    if main_type.is_empty() || sub_type.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(content_type.to_string())
}

/// What the board says about the file to send.
#[derive(Debug, Default)]
pub(crate) struct UploadFile<'a> {
    pub(crate) filename: &'a str,
    pub(crate) size: u64,
    pub(crate) title: Option<&'a str>,
    pub(crate) message: Option<&'a str>,
    pub(crate) content_type: Option<&'a str>,
    /// Board item whose media the uploaded file replaces; it must be a board
    /// item id (UUID).
    pub(crate) replace_item_id: Option<&'a str>,
}

/// Builds the JSON body opening the upload; blank texts are left out.
pub(crate) fn upload_description(file: &UploadFile<'_>) -> Result<Value, String> {
    if file.size == 0 {
        return Err("fichier vide".to_string());
    }
    let mut description = serde_json::Map::new();
    description.insert("filename".to_string(), file.filename.into());
    description.insert("size".to_string(), file.size.into());
    for (key, value) in [("title", file.title), ("message", file.message)] {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            description.insert(key.to_string(), value.into());
        }
    }
    if let Some(content_type) = normalize_content_type(file.content_type) {
        description.insert("contentType".to_string(), content_type.into());
    }
    if let Some(item_id) = file
        .replace_item_id
        .map(str::trim)
        .filter(|item_id| !item_id.is_empty())
    {
        let item_id = uuid::Uuid::parse_str(item_id).map_err(|_| "média inconnu".to_string())?;
        description.insert("replaceItemId".to_string(), item_id.to_string().into());
    }
    Ok(Value::Object(description))
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

/// Where and how an upload is sent.
pub(crate) struct UploadTarget {
    pub(crate) client: reqwest::Client,
    pub(crate) server_url: String,
    pub(crate) allow_insecure: bool,
    pub(crate) token: String,
    /// Size at which a part ends ([`PART_TARGET_BYTES`] outside tests).
    pub(crate) part_target: u64,
}

impl UploadTarget {
    fn url(&self, path: &str) -> Result<Url, String> {
        api::build_url(&self.server_url, path, self.allow_insecure)
    }
}

/// Server-side upload abandoned when dropped before its completion: a
/// cancelled task, a failed part or a lost board all release it at once.
struct OpenUpload {
    client: reqwest::Client,
    url: Url,
    token: String,
    completed: bool,
}

impl Drop for OpenUpload {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let request = self
            .client
            .delete(self.url.clone())
            .bearer_auth(&self.token)
            .send();
        tauri::async_runtime::spawn(async move {
            // Best effort: the server also discards idle uploads.
            drop(request.await);
        });
    }
}

/// Runs the upload until the server answers its completion.
///
/// Returns the first refused answer (opening or part), or the completion
/// answer (`202 {ingestionId}`). The receiver ends when the board finishes.
pub(crate) async fn run_upload(
    target: UploadTarget,
    description: Value,
    mut receiver: mpsc::Receiver<UploadChunk>,
) -> Result<api::ApiResponse, String> {
    let response = target
        .client
        .post(target.url(UPLOADS_PATH)?)
        .bearer_auth(&target.token)
        .json(&description)
        .send()
        .await
        .map_err(unreachable)?;
    let opened = api::read_response(response).await;
    let Some(upload_id) = opened
        .ok
        .then(|| opened.body.get("uploadId").and_then(Value::as_str))
        .flatten()
        .and_then(|upload_id| uuid::Uuid::parse_str(upload_id).ok())
    else {
        return Ok(opened);
    };
    let upload_path = format!("{UPLOADS_PATH}/{upload_id}");
    let mut open = OpenUpload {
        client: target.client.clone(),
        url: target.url(&upload_path)?,
        token: target.token.clone(),
        completed: false,
    };

    let mut offset = 0_u64;
    while let Some(first) = receiver.recv().await {
        let mut url = target.url(&format!("{upload_path}/parts"))?;
        url.query_pairs_mut()
            .append_pair("offset", &offset.to_string());
        let (response, sent, ended) = send_part(&target, url, first, &mut receiver).await;
        let response = response?;
        if !response.ok {
            return Ok(response);
        }
        offset += sent;
        if ended {
            break;
        }
    }

    let response = target
        .client
        .post(target.url(&format!("{upload_path}/complete"))?)
        .bearer_auth(&target.token)
        .send()
        .await
        .map_err(unreachable)?;
    // Whatever the answer, the server no longer holds the upload.
    open.completed = true;
    Ok(api::read_response(response).await)
}

/// Streams one part: `first`, then chunks until the part reaches
/// `part_target` or the board ends the file. Returns the server's answer, the
/// bytes sent and whether the file ended.
async fn send_part(
    target: &UploadTarget,
    url: Url,
    first: UploadChunk,
    receiver: &mut mpsc::Receiver<UploadChunk>,
) -> (Result<api::ApiResponse, String>, u64, bool) {
    let (part_sender, part_receiver) = mpsc::channel::<UploadChunk>(2);
    let body = reqwest::Body::wrap_stream(futures_util::stream::unfold(
        part_receiver,
        |mut part_receiver| async move {
            part_receiver
                .recv()
                .await
                .map(|chunk| (Ok::<_, std::io::Error>(chunk), part_receiver))
        },
    ));
    let request = async {
        let response = target
            .client
            .put(url)
            .bearer_auth(&target.token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(unreachable)?;
        Ok(api::read_response(response).await)
    };
    let part_target = target.part_target;
    let forward = async move {
        let mut sent = first.len() as u64;
        if part_sender.send(first).await.is_err() {
            return (sent, false);
        }
        while sent < part_target {
            let Some(chunk) = receiver.recv().await else {
                return (sent, true);
            };
            sent += chunk.len() as u64;
            // The server answered early (refused part): stop forwarding.
            if part_sender.send(chunk).await.is_err() {
                break;
            }
        }
        (sent, false)
    };
    let (response, (sent, ended)) = tokio::join!(request, forward);
    (response, sent, ended)
}

fn unreachable(error: reqwest::Error) -> String {
    format!("serveur injoignable : {error}")
}

/// Largest picture sent for a board item (the server's limit).
pub(crate) const MAX_ARTWORK_BYTES: usize = 10 * 1024 * 1024;

/// Sends the picture of a board item (`PUT …/items/{id}/artwork`).
pub(crate) async fn put_artwork(
    membership: &crate::config::Membership,
    item_id: &str,
    filename: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Result<api::ApiResponse, String> {
    let item_id = uuid::Uuid::parse_str(item_id.trim()).map_err(|_| "média inconnu".to_string())?;
    let insecure = crate::net::tls::insecure_tls_enabled();
    let mut url = api::build_url(
        &membership.server_url,
        &format!("/overlay/media-board/items/{item_id}/artwork"),
        insecure,
    )?;
    url.query_pairs_mut().append_pair("filename", filename);
    let response = build_client(insecure)?
        .put(url)
        .bearer_auth(&membership.client_token)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(bytes)
        .send()
        .await
        .map_err(unreachable)?;
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
    use serde_json::json;
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
    fn content_type_is_cleaned_or_dropped() {
        assert_eq!(
            normalize_content_type(Some(" video/mp4 ")).as_deref(),
            Some("video/mp4")
        );
        assert_eq!(
            normalize_content_type(Some("application/vnd.livechat+json")).as_deref(),
            Some("application/vnd.livechat+json")
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
                None,
                "content type refusé attendu : {invalid:?}"
            );
        }
    }

    #[test]
    fn description_carries_the_file_and_omits_empty_texts() {
        let description = upload_description(&UploadFile {
            filename: "L’Escale 🔱 & co.mp4",
            size: 123,
            title: Some("  Titre  "),
            message: Some("   "),
            content_type: Some("video/mp4"),
            replace_item_id: Some("  "),
        })
        .expect("description");
        assert_eq!(
            description,
            json!({
                "filename": "L’Escale 🔱 & co.mp4",
                "size": 123,
                "title": "Titre",
                "contentType": "video/mp4",
            })
        );
        assert!(upload_description(&UploadFile {
            filename: "a.png",
            size: 0,
            ..UploadFile::default()
        })
        .is_err());
    }

    #[test]
    fn description_carries_the_item_to_replace() {
        let item_id = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let description = upload_description(&UploadFile {
            filename: "photo.png",
            size: 10,
            replace_item_id: Some(&format!("  {item_id} ")),
            ..UploadFile::default()
        })
        .expect("description");
        assert_eq!(
            description,
            json!({"filename": "photo.png", "size": 10, "replaceItemId": item_id})
        );
        for invalid in ["item-3", "../items", "0f8fad5b&title=x"] {
            assert!(
                upload_description(&UploadFile {
                    filename: "a.png",
                    size: 10,
                    replace_item_id: Some(invalid),
                    ..UploadFile::default()
                })
                .is_err(),
                "identifiant refusé attendu : {invalid}"
            );
        }
    }

    /// An upload still in flight: its task never ends.
    fn dummy_entry() -> UploadEntry {
        let (sender, _receiver) = mpsc::channel(4);
        let task = tauri::async_runtime::spawn(std::future::pending());
        UploadEntry {
            guild_id: "111".to_string(),
            sender,
            task,
        }
    }

    /// An upload whose task already ended (answered or timed out).
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

    const UPLOAD_ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    /// A request received by the mock server.
    #[derive(Debug)]
    struct Received {
        method: String,
        target: String,
        headers: String,
        body: Vec<u8>,
    }

    /// Reads more bytes into `buffer`, failing if the connection closes.
    async fn fill(stream: &mut tokio::net::TcpStream, buffer: &mut Vec<u8>) {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.expect("lecture");
        assert!(read > 0, "connexion fermée avant la fin de la requête");
        buffer.extend_from_slice(&chunk[..read]);
    }

    /// Reads one HTTP/1.1 request (`content-length` or chunked body).
    async fn read_request(stream: &mut tokio::net::TcpStream) -> Received {
        let mut buffer = Vec::new();
        let header_end = loop {
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            fill(stream, &mut buffer).await;
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let mut request_line = headers.lines().next().unwrap_or_default().split(' ');
        let method = request_line.next().unwrap_or_default().to_string();
        let target = request_line.next().unwrap_or_default().to_string();
        let lower = headers.to_ascii_lowercase();

        let mut body = Vec::new();
        let mut position = header_end;
        if lower.contains("transfer-encoding: chunked") {
            loop {
                let line_end = loop {
                    if let Some(offset) = buffer[position..]
                        .windows(2)
                        .position(|window| window == b"\r\n")
                    {
                        break offset;
                    }
                    fill(stream, &mut buffer).await;
                };
                let line =
                    String::from_utf8_lossy(&buffer[position..position + line_end]).to_string();
                position += line_end + 2;
                let size = usize::from_str_radix(line.trim(), 16).expect("taille de chunk");
                while buffer.len() < position + size + 2 {
                    fill(stream, &mut buffer).await;
                }
                if size == 0 {
                    break;
                }
                body.extend_from_slice(&buffer[position..position + size]);
                position += size + 2;
            }
        } else {
            let length = lower
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .map_or(0, |value| value.trim().parse::<usize>().expect("longueur"));
            while buffer.len() < header_end + length {
                fill(stream, &mut buffer).await;
            }
            body.extend_from_slice(&buffer[header_end..header_end + length]);
        }
        Received {
            method,
            target,
            headers: lower,
            body,
        }
    }

    /// Mock upload server: answers each request from `reply` and reports it.
    async fn mock_server(
        reply: fn(&Received) -> (u16, &'static str),
    ) -> (String, mpsc::UnboundedReceiver<Received>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("adresse locale");
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _peer)) = listener.accept().await else {
                    return;
                };
                let sender = sender.clone();
                tokio::spawn(async move {
                    let received = read_request(&mut stream).await;
                    let (status, body) = reply(&received);
                    let response = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    drop(stream.write_all(response.as_bytes()).await);
                    drop(sender.send(received));
                });
            }
        });
        (format!("http://{address}"), receiver)
    }

    fn accepting(received: &Received) -> (u16, &'static str) {
        match (received.method.as_str(), received.target.as_str()) {
            ("POST", UPLOADS_PATH) => (
                201,
                r#"{"uploadId":"0f8fad5b-d9cb-469f-a165-70867728950e"}"#,
            ),
            ("PUT", _) => (200, r#"{"received":0}"#),
            ("POST", _) => (202, r#"{"ingestionId":"ing-1"}"#),
            _ => (204, ""),
        }
    }

    fn target(server_url: String, part_target: u64) -> UploadTarget {
        UploadTarget {
            client: build_client(true).expect("client"),
            server_url,
            allow_insecure: true,
            token: "test-token".to_string(),
            part_target,
        }
    }

    #[tokio::test]
    async fn upload_opens_sends_parts_in_order_then_completes() {
        let (server_url, mut requests) = mock_server(accepting).await;
        let description = json!({"filename": "clip.mp4", "size": 21});
        let (sender, receiver) = mpsc::channel(4);
        let upload = tokio::spawn(run_upload(
            target(server_url, 10),
            description.clone(),
            receiver,
        ));
        for chunk in [b"abcdefg", b"hijklmn", b"opqrstu"] {
            sender.send(chunk.to_vec()).await.expect("morceau");
        }
        drop(sender);

        let response = upload.await.expect("tâche d'envoi").expect("réponse");
        assert_eq!(response.status, 202);
        assert_eq!(response.body["ingestionId"], "ing-1");

        let mut received = Vec::new();
        for _ in 0..4 {
            received.push(requests.recv().await.expect("requête"));
        }
        assert_eq!(received[0].method, "POST");
        assert_eq!(received[0].target, UPLOADS_PATH);
        assert!(received[0]
            .headers
            .contains("authorization: bearer test-token"));
        assert_eq!(
            serde_json::from_slice::<Value>(&received[0].body).expect("json"),
            description
        );
        // A part ends with the chunk reaching the target: 14 bytes, then 7.
        let parts_path = format!("{UPLOADS_PATH}/{UPLOAD_ID}/parts");
        assert_eq!(received[1].method, "PUT");
        assert_eq!(received[1].target, format!("{parts_path}?offset=0"));
        assert!(received[1]
            .headers
            .contains("authorization: bearer test-token"));
        assert_eq!(received[1].body, b"abcdefghijklmn");
        assert_eq!(received[2].target, format!("{parts_path}?offset=14"));
        assert_eq!(received[2].body, b"opqrstu");
        assert_eq!(received[3].method, "POST");
        assert_eq!(
            received[3].target,
            format!("{UPLOADS_PATH}/{UPLOAD_ID}/complete")
        );
        // Completed: nothing is cancelled afterwards.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_refused_opening_is_returned_without_sending_parts() {
        fn too_large(_: &Received) -> (u16, &'static str) {
            (413, r#"{"error":"payload_too_large","maxBytes":1048576}"#)
        }
        let (server_url, mut requests) = mock_server(too_large).await;
        let (sender, receiver) = mpsc::channel(4);
        let upload = tokio::spawn(run_upload(
            target(server_url, 10),
            json!({"filename": "big.mp4", "size": 5_000_000}),
            receiver,
        ));
        let response = upload.await.expect("tâche d'envoi").expect("réponse");
        assert_eq!(response.status, 413);
        assert_eq!(response.body["maxBytes"], 1_048_576);
        // The board's next chunk finds the upload closed.
        assert!(sender.send(b"abc".to_vec()).await.is_err());
        assert_eq!(requests.recv().await.expect("requête").method, "POST");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_cancelled_upload_is_abandoned_on_the_server() {
        let (server_url, mut requests) = mock_server(accepting).await;
        let (sender, receiver) = mpsc::channel(4);
        let upload = tauri::async_runtime::spawn(run_upload(
            target(server_url, 10),
            json!({"filename": "clip.mp4", "size": 100}),
            receiver,
        ));
        sender
            .send(b"abcdefghijkl".to_vec())
            .await
            .expect("morceau");
        let opened = requests.recv().await.expect("ouverture");
        assert_eq!(opened.method, "POST");
        let part = requests.recv().await.expect("partie");
        assert_eq!(part.method, "PUT");

        upload.abort();
        let cancelled = requests.recv().await.expect("abandon");
        assert_eq!(cancelled.method, "DELETE");
        assert_eq!(cancelled.target, format!("{UPLOADS_PATH}/{UPLOAD_ID}"));
        assert!(cancelled
            .headers
            .contains("authorization: bearer test-token"));
    }
}
