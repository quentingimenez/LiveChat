//! Enregistrement d'un média de la board à l'emplacement choisi par l'utilisateur.
//!
//! Le WebView ne peut pas écrire de fichier : le board demande au shell
//! d'ouvrir une boîte de dialogue « Enregistrer sous », puis de télécharger
//! l'URL signée dans le fichier choisi. Le nom proposé vient du titre du
//! média, l'extension de l'URL (sinon du type MIME, sinon `.bin` : jamais une
//! extension exécutable) ; quand le nom choisi n'a pas d'extension, celle du
//! média lui est ajoutée.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::net::fetch::{get, parse_link, LinkError};
use crate::net::upload::normalize_content_type;

/// Durée maximale du téléchargement complet.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// Taille maximale d'un média enregistré (500 Mio).
const MAX_MEDIA_BYTES: usize = 500 * 1024 * 1024;
/// Longueur maximale du nom, avant l'extension.
const MAX_STEM_CHARS: usize = 80;
/// Extensions qu'un `download` de média peut produire.
const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "webm", "mov", "mkv", "ogg", "ogv", "mp3", "m4a", "wav", "flac", "opus", "aac",
    "png", "jpg", "jpeg", "gif", "webp", "avif",
];
/// Noms de périphériques réservés par Windows, refusés même avec une extension.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Fichier écrit et nom retenu.
#[derive(Debug)]
pub(crate) struct Saved {
    pub(crate) path: PathBuf,
    pub(crate) file_name: String,
}

/// Nom proposé dans la boîte de dialogue : le titre nettoyé, complété par
/// l'extension lisible dans l'URL quand elle y est.
pub(crate) fn suggested_name(raw: &str, label: &str) -> String {
    let stem = safe_stem(label)
        .map(|stem| strip_media_extension(&stem))
        .unwrap_or_else(|| "media".to_string());
    match Url::parse(raw).ok().as_ref().and_then(extension_of_path) {
        Some(extension) => format!("{stem}.{extension}"),
        None => stem,
    }
}

/// Télécharge `raw` dans `target`, le fichier que l'utilisateur vient de
/// choisir. Une extension manquante est complétée avec celle du média.
pub(crate) async fn save(raw: &str, target: &Path) -> Result<Saved, LinkError> {
    tokio::time::timeout(DOWNLOAD_TIMEOUT, save_inner(raw, target))
        .await
        .unwrap_or(Err(LinkError::Unreachable))
}

async fn save_inner(raw: &str, target: &Path) -> Result<Saved, LinkError> {
    let parsed = parse_link(raw)?;
    let mut response = get(raw).await?;
    let declared = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| normalize_content_type(Some(value)));
    if let Some(kind) = declared.as_deref() {
        if is_page(kind) {
            return Err(LinkError::NotMedia);
        }
    }
    if let Some(length) = response.content_length() {
        if length == 0 {
            return Err(LinkError::Empty);
        }
        if length > MAX_MEDIA_BYTES as u64 {
            return Err(LinkError::TooLarge);
        }
    }
    let extension = extension_of_path(&parsed)
        .or_else(|| declared.as_deref().and_then(extension_of_type))
        .unwrap_or_else(|| "bin".to_string());
    let path = with_media_extension(target, &extension);
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|_| LinkError::Io)?;
    if let Err(error) = write_body(&mut file, &mut response).await {
        drop(file);
        drop(tokio::fs::remove_file(&path).await);
        return Err(error);
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Saved { path, file_name })
}

/// Le chemin choisi, complété de `extension` seulement s'il n'en a pas déjà
/// une : le nom et l'extension décidés par l'utilisateur sont conservés.
fn with_media_extension(target: &Path, extension: &str) -> PathBuf {
    match target.extension().and_then(|value| value.to_str()) {
        Some(existing) if !existing.trim().is_empty() => target.to_path_buf(),
        _ => target.with_extension(extension),
    }
}

/// Écrit le corps de la réponse dans le fichier déjà réservé.
async fn write_body(
    file: &mut tokio::fs::File,
    response: &mut reqwest::Response,
) -> Result<(), LinkError> {
    let mut written = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(|_| LinkError::Unreachable)? {
        written = written.saturating_add(chunk.len());
        if written > MAX_MEDIA_BYTES {
            return Err(LinkError::TooLarge);
        }
        file.write_all(&chunk).await.map_err(|_| LinkError::Io)?;
    }
    if written == 0 {
        return Err(LinkError::Empty);
    }
    file.flush().await.map_err(|_| LinkError::Io)?;
    Ok(())
}

/// Un nom de fichier sûr pour Windows : pas de séparateur de chemin, pas de
/// caractère réservé, pas de point final, [`MAX_STEM_CHARS`] caractères au plus.
/// `None` quand rien d'exploitable ne reste.
pub(crate) fn safe_stem(raw: &str) -> Option<String> {
    let replaced: String = raw
        .trim()
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                ' '
            } else {
                character
            }
        })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches([' ', '.']).to_string();
    let cap = trimmed
        .char_indices()
        .nth(MAX_STEM_CHARS)
        .map(|(index, _)| index)
        .unwrap_or(trimmed.len());
    let stem = trimmed[..cap].trim_end_matches([' ', '.']).to_string();
    if stem.is_empty() {
        return None;
    }
    let device = stem.split('.').next().unwrap_or(&stem).to_ascii_uppercase();
    if RESERVED_NAMES.contains(&device.as_str()) {
        return Some(format!("_{stem}"));
    }
    Some(stem)
}

/// Retire l'extension média qu'un titre peut déjà porter (`clip.mp4`) avant
/// d'ajouter la vraie : le fichier ne finit pas en `clip.mp4.mp4`.
pub(crate) fn strip_media_extension(stem: &str) -> String {
    match stem.rsplit_once('.') {
        Some((base, extension))
            if !base.is_empty()
                && MEDIA_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()) =>
        {
            base.to_string()
        }
        _ => stem.to_string(),
    }
}

/// Extension lue dans le chemin de l'URL, si c'est une extension média connue.
fn extension_of_path(url: &Url) -> Option<String> {
    let last = url.path_segments()?.next_back()?;
    let (_, extension) = last.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    MEDIA_EXTENSIONS
        .contains(&extension.as_str())
        .then_some(extension)
}

/// Extension déduite du type MIME, si c'est un type média connu.
fn extension_of_type(content_type: &str) -> Option<String> {
    let (kind, subtype) = content_type.split_once('/')?;
    if !matches!(
        kind.to_ascii_lowercase().as_str(),
        "image" | "audio" | "video"
    ) {
        return None;
    }
    let extension = match subtype.to_ascii_lowercase().as_str() {
        "jpeg" => "jpg".to_string(),
        "mpeg" => "mp3".to_string(),
        "quicktime" => "mov".to_string(),
        other => other.to_string(),
    };
    MEDIA_EXTENSIONS
        .contains(&extension.as_str())
        .then_some(extension)
}

/// Une page ou une erreur d'API, jamais un média.
fn is_page(content_type: &str) -> bool {
    let kind = content_type.to_ascii_lowercase();
    kind == "text/html" || kind == "application/json"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::{http_response, serve};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("URL de test")
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("lecture du dossier")
            .map(|entry| {
                entry
                    .expect("entrée")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn suggested_name_adds_the_extension_read_in_the_url() {
        assert_eq!(
            suggested_name("https://srv.test/media/a1.MP4?exp=1&sig=x", "Clip Discord"),
            "Clip Discord.mp4"
        );
        assert_eq!(
            suggested_name("https://srv.test/media/a1?exp=1&sig=x", "Clip Discord"),
            "Clip Discord"
        );
        assert_eq!(
            suggested_name("https://srv.test/media/clip.webm", "clip.mp4"),
            "clip.webm"
        );
        assert_eq!(suggested_name("https://srv.test/media/a1", "   "), "media");
        assert_eq!(suggested_name("pas une url", "Clip"), "Clip");
    }

    #[test]
    fn safe_stem_keeps_a_readable_title() {
        assert_eq!(safe_stem("Clip Discord"), Some("Clip Discord".to_string()));
        assert_eq!(safe_stem("Été à Paris"), Some("Été à Paris".to_string()));
        assert_eq!(safe_stem("  clip  "), Some("clip".to_string()));
    }

    #[test]
    fn safe_stem_replaces_what_windows_refuses() {
        assert_eq!(safe_stem("a/b\\c"), Some("a b c".to_string()));
        assert_eq!(safe_stem("a<b>c:d"), Some("a b c d".to_string()));
        assert_eq!(safe_stem("clip..."), Some("clip".to_string()));
        assert_eq!(safe_stem("clip."), Some("clip".to_string()));
    }

    #[test]
    fn safe_stem_falls_back_when_nothing_remains() {
        assert_eq!(safe_stem(""), None);
        assert_eq!(safe_stem("   "), None);
        assert_eq!(safe_stem("..."), None);
        assert_eq!(safe_stem(" . "), None);
    }

    #[test]
    fn safe_stem_prefixes_reserved_device_names() {
        assert_eq!(safe_stem("CON"), Some("_CON".to_string()));
        assert_eq!(safe_stem("con"), Some("_con".to_string()));
        assert_eq!(safe_stem("LPT1"), Some("_LPT1".to_string()));
        assert_eq!(safe_stem("CONSOLE"), Some("CONSOLE".to_string()));
        assert_eq!(safe_stem("CON.ete"), Some("_CON.ete".to_string()));
    }

    #[test]
    fn safe_stem_caps_the_length_on_character_boundaries() {
        let long = "é".repeat(200);
        let stem = safe_stem(&long).expect("nom");
        assert_eq!(stem.chars().count(), MAX_STEM_CHARS);
    }

    #[test]
    fn strip_media_extension_removes_only_a_real_media_extension() {
        assert_eq!(strip_media_extension("clip.mp4"), "clip");
        assert_eq!(strip_media_extension("clip.MP4"), "clip");
        assert_eq!(strip_media_extension("clip.final"), "clip.final");
        assert_eq!(strip_media_extension("clip"), "clip");
    }

    #[test]
    fn with_media_extension_keeps_the_name_chosen_by_the_user() {
        assert_eq!(
            with_media_extension(Path::new("C:\\Téléchargements\\clip.webm"), "png"),
            Path::new("C:\\Téléchargements\\clip.webm")
        );
        assert_eq!(
            with_media_extension(Path::new("C:\\Téléchargements\\clip"), "png"),
            Path::new("C:\\Téléchargements\\clip.png")
        );
        assert_eq!(
            with_media_extension(Path::new("C:\\Téléchargements\\clip."), "png"),
            Path::new("C:\\Téléchargements\\clip.png")
        );
    }

    #[test]
    fn the_extension_comes_from_the_url_then_the_type() {
        assert_eq!(
            extension_of_path(&url("https://srv.test/media/a1.MP4?exp=1&sig=x")).as_deref(),
            Some("mp4")
        );
        assert_eq!(extension_of_path(&url("https://srv.test/media/a1")), None);
        assert_eq!(
            extension_of_path(&url("https://srv.test/media/a1.pdf")),
            None
        );
        assert_eq!(extension_of_type("video/mp4").as_deref(), Some("mp4"));
        assert_eq!(extension_of_type("image/jpeg").as_deref(), Some("jpg"));
        assert_eq!(extension_of_type("audio/mpeg").as_deref(), Some("mp3"));
        assert_eq!(extension_of_type("application/octet-stream"), None);
    }

    #[test]
    fn a_page_is_never_saved_as_a_media() {
        assert!(is_page("text/html"));
        assert!(is_page("APPLICATION/JSON"));
        assert!(!is_page("image/png"));
        assert!(!is_page("video/mp4"));
    }

    #[tokio::test]
    async fn save_writes_the_media_under_the_chosen_name() {
        let (server, task) =
            serve(|_| http_response("200 OK", &[("content-type", "image/png")], PNG)).await;
        let dir = tempfile::tempdir().expect("dossier");
        let target = dir.path().join("Clip Discord");
        let saved = save(&format!("{server}/media/a1?exp=100&sig=x"), &target)
            .await
            .expect("enregistré");
        assert_eq!(saved.file_name, "Clip Discord.png");
        assert_eq!(saved.path, dir.path().join("Clip Discord.png"));
        assert_eq!(std::fs::read(&saved.path).expect("contenu"), PNG);
        task.abort();
    }

    #[tokio::test]
    async fn save_keeps_the_extension_chosen_by_the_user() {
        let (server, task) =
            serve(|_| http_response("200 OK", &[("content-type", "image/png")], PNG)).await;
        let dir = tempfile::tempdir().expect("dossier");
        let target = dir.path().join("clip.webm");
        let saved = save(&format!("{server}/media/a1"), &target)
            .await
            .expect("enregistré");
        assert_eq!(saved.path, target);
        assert_eq!(std::fs::read(&saved.path).expect("contenu"), PNG);
        assert_eq!(files(dir.path()), ["clip.webm"]);
        task.abort();
    }

    #[tokio::test]
    async fn save_refuses_a_page_instead_of_a_media() {
        let (server, task) = serve(|_| {
            http_response(
                "200 OK",
                &[("content-type", "text/html")],
                b"<html>connectez-vous</html>",
            )
        })
        .await;
        let dir = tempfile::tempdir().expect("dossier");
        let error = save(&format!("{server}/a1"), &dir.path().join("Clip"))
            .await
            .expect_err("page refusée");
        assert_eq!(error, LinkError::NotMedia);
        assert!(files(dir.path()).is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn save_never_writes_an_executable_extension() {
        let (server, task) = serve(|_| {
            http_response(
                "200 OK",
                &[("content-type", "application/x-msdownload")],
                b"MZ",
            )
        })
        .await;
        let dir = tempfile::tempdir().expect("dossier");
        let saved = save(&format!("{server}/a1"), &dir.path().join("setup"))
            .await
            .expect("enregistré");
        assert_eq!(saved.file_name, "setup.bin");
        task.abort();
    }

    #[tokio::test]
    async fn save_reports_an_empty_answer() {
        let (server, task) =
            serve(|_| http_response("200 OK", &[("content-type", "video/mp4")], b"")).await;
        let dir = tempfile::tempdir().expect("dossier");
        let error = save(&format!("{server}/a1.mp4"), &dir.path().join("Clip"))
            .await
            .expect_err("vide");
        assert_eq!(error, LinkError::Empty);
        assert!(files(dir.path()).is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn save_reports_a_too_large_media() {
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: video/mp4\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            MAX_MEDIA_BYTES + 1
        );
        let (server, task) = serve(move |_| reply.clone().into_bytes()).await;
        let dir = tempfile::tempdir().expect("dossier");
        let error = save(&format!("{server}/a1.mp4"), &dir.path().join("Clip"))
            .await
            .expect_err("trop gros");
        assert_eq!(error, LinkError::TooLarge);
        assert!(files(dir.path()).is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn save_removes_the_partial_file_when_the_transfer_breaks() {
        let reply = b"HTTP/1.1 200 OK\r\ncontent-type: video/mp4\r\ncontent-length: 100\r\nconnection: close\r\n\r\nabcd".to_vec();
        let (server, task) = serve(move |_| reply.clone()).await;
        let dir = tempfile::tempdir().expect("dossier");
        let error = save(&format!("{server}/a1.mp4"), &dir.path().join("Clip"))
            .await
            .expect_err("coupé");
        assert_eq!(error, LinkError::Unreachable);
        assert!(files(dir.path()).is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn save_reports_the_status_the_site_answered_with() {
        let (server, task) = serve(|_| http_response("404 Not Found", &[], b"rien")).await;
        let dir = tempfile::tempdir().expect("dossier");
        let error = save(&format!("{server}/a1.mp4"), &dir.path().join("Clip"))
            .await
            .expect_err("404");
        assert_eq!(error, LinkError::Http(404));
        assert!(files(dir.path()).is_empty());
        task.abort();
    }
}
