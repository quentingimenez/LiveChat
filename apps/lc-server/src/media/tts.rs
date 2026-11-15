//! Speech synthesis for `/dire`, `/cdire` and `POST /overlay/say`: the local
//! Piper engine.
//!
//! Piper reads the text on stdin and writes a WAV file; the WAV is then
//! ingested like an uploaded file (normalized audio, content-addressed cache:
//! the same sentence in the same voice is synthesized once).
//!
//! | Variable | Default |
//! |---|---|
//! | `PIPER_BIN` | `piper` |
//! | `PIPER_VOICE_FR` | `/opt/piper/voices/fr_FR-siwis-medium.onnx` |
//! | `PIPER_VOICE_EN` | `/opt/piper/voices/en_US-lessac-medium.onnx` |

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use lc_discord::{Lang, SpeechFailure};
use lc_domain::MediaAsset;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::process::Command;

use crate::app::AppState;
use crate::media::service::MediaService;

/// Longest text spoken at once (characters).
pub const MAX_SPEECH_CHARS: usize = 1_000;

/// Longest synthesis allowed.
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(60);

/// Collapses every run of whitespace into one space and trims both ends.
pub fn collapse_speech(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapses whitespace and bounds the length. `None` when nothing is left.
pub fn normalize_speech(text: &str) -> Option<String> {
    let collapsed = collapse_speech(text);
    if collapsed.is_empty() {
        return None;
    }
    Some(collapsed.chars().take(MAX_SPEECH_CHARS).collect())
}

/// Temporary directory removed on drop.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, path = %self.0.display(), "dossier temporaire TTS non supprimé");
            }
        }
    }
}

/// Synthesizes `text` and returns the `READY` audio asset.
pub async fn ingest_speech(
    state: &AppState,
    lang: Lang,
    text: &str,
) -> Result<MediaAsset, SpeechFailure> {
    let Some(text) = normalize_speech(text) else {
        return Err(SpeechFailure::EmptyText);
    };
    let voice = match lang {
        Lang::Fr => &state.config.piper_voice_fr,
        Lang::En => &state.config.piper_voice_en,
    };
    let dir = TempDir(std::env::temp_dir().join(format!("livechat-tts-{}", uuid::Uuid::new_v4())));
    if let Err(error) = tokio::fs::create_dir_all(&dir.0).await {
        tracing::error!(%error, "dossier temporaire TTS impossible");
        return Err(SpeechFailure::GenerationFailed);
    }
    let wav = dir.0.join("speech.wav");
    synthesize(&state.config.piper_bin, voice, &text, &wav).await?;

    let bytes = tokio::fs::read(&wav).await.map_err(|error| {
        tracing::error!(%error, "lecture du WAV Piper impossible");
        SpeechFailure::GenerationFailed
    })?;
    if bytes.is_empty() {
        tracing::error!("Piper a produit un fichier vide");
        return Err(SpeechFailure::GenerationFailed);
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let service = MediaService::for_state(state).map_err(|error| {
        tracing::error!(%error, "service média indisponible pour la synthèse vocale");
        SpeechFailure::GenerationFailed
    })?;
    service
        .ingest_file(&wav, "speech.wav", Some("audio/wav"), &sha256)
        .await
        .map_err(|error| {
            tracing::error!(%error, "ingestion de la synthèse vocale échouée");
            SpeechFailure::GenerationFailed
        })
}

async fn synthesize(
    piper_bin: &str,
    voice: &Path,
    text: &str,
    output: &Path,
) -> Result<(), SpeechFailure> {
    let mut child = Command::new(piper_bin)
        .arg("--model")
        .arg(voice)
        .arg("--output_file")
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            tracing::error!(%error, bin = piper_bin, "Piper introuvable ou non exécutable");
            SpeechFailure::GenerationFailed
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        let written = stdin.write_all(text.as_bytes()).await;
        drop(stdin);
        if let Err(error) = written {
            tracing::error!(%error, "envoi du texte à Piper impossible");
            return Err(SpeechFailure::GenerationFailed);
        }
    }
    let output = match tokio::time::timeout(SYNTHESIS_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            tracing::error!(%error, "Piper interrompu");
            return Err(SpeechFailure::GenerationFailed);
        }
        Err(_) => {
            tracing::error!("Piper n'a pas répondu à temps");
            return Err(SpeechFailure::GenerationFailed);
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr.lines().rev().find(|line| !line.trim().is_empty());
        tracing::error!(status = %output.status, error = last.unwrap_or(""), "synthèse Piper échouée");
        return Err(SpeechFailure::GenerationFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_speech, MAX_SPEECH_CHARS};

    #[test]
    fn speech_text_is_collapsed_and_bounded() {
        assert_eq!(
            normalize_speech("  bonjour \n  tout   le monde "),
            Some("bonjour tout le monde".to_owned())
        );
        assert_eq!(normalize_speech(" \n\t "), None);
        let long = "a".repeat(MAX_SPEECH_CHARS + 50);
        assert_eq!(
            normalize_speech(&long).map(|text| text.chars().count()),
            Some(MAX_SPEECH_CHARS)
        );
    }
}
