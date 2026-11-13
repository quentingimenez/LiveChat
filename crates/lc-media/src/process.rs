//! Exécution contrôlée des binaires externes (`ffmpeg`, `ffprobe`, `yt-dlp`).
//!
//! Chaque processus est lancé avec un timeout, un `kill_on_drop`, une borne
//! stricte de la taille de sortie capturée (AUD-A4-005) et une classification
//! d'erreur cohérente avec le reste du pipeline.

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::error::{MediaError, MediaErrorCode, MediaResult};

/// Taille maximale de sortie capturée par flux (`stdout`/`stderr`), en octets.
///
/// Les binaires externes (`yt-dlp --dump-json`, `ffprobe`, `ffmpeg`) peuvent
/// produire des sorties arbitrairement grandes ; on refuse au-delà de cette
/// borne plutôt que de gonfler la mémoire du serveur (AUD-A4-005).
pub const MAX_COMMAND_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Sortie standard et d'erreur d'un processus terminé avec succès.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    /// Sortie standard (UTF-8 permissif).
    pub stdout: String,
    /// Sortie d'erreur (UTF-8 permissif).
    pub stderr: String,
}

impl CommandOutput {
    /// Concatène stdout et stderr (diagnostics).
    pub fn combined(&self) -> String {
        match (self.stdout.trim().is_empty(), self.stderr.trim().is_empty()) {
            (true, true) => String::new(),
            (false, true) => self.stdout.trim().to_string(),
            (true, false) => self.stderr.trim().to_string(),
            (false, false) => format!("{}\n{}", self.stderr.trim(), self.stdout.trim()),
        }
    }
}

/// Exécute `binary args…` et renvoie la sortie si le processus termine avec succès.
///
/// * erreur de lancement → code `fallback` ;
/// * timeout → [`MediaErrorCode::DownloadTimeout`] (le processus est tué) ;
/// * code de sortie non nul → classification textuelle via
///   [`MediaError::from_command_failure`] ;
/// * sortie capturée au-delà de [`MAX_COMMAND_OUTPUT_BYTES`] → code
///   [`MediaErrorCode::InvalidMedia`].
pub async fn run_command(
    binary: &Path,
    args: &[String],
    timeout: Duration,
    fallback: MediaErrorCode,
) -> MediaResult<CommandOutput> {
    run_command_with_limit(binary, args, timeout, fallback, MAX_COMMAND_OUTPUT_BYTES).await
}

/// Variante de [`run_command`] avec une borne de sortie explicite (tests).
async fn run_command_with_limit(
    binary: &Path,
    args: &[String],
    timeout: Duration,
    fallback: MediaErrorCode,
    output_limit: usize,
) -> MediaResult<CommandOutput> {
    let program = binary.display().to_string();
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            MediaError::new(
                fallback,
                format!("impossible de lancer {program} : {error}"),
            )
        })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        MediaError::new(
            fallback,
            format!("sortie standard de {program} indisponible"),
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        MediaError::new(
            fallback,
            format!("sortie d'erreur de {program} indisponible"),
        )
    })?;

    let stdout_task = tokio::spawn(read_stream_bounded(stdout, output_limit));
    let stderr_task = tokio::spawn(read_stream_bounded(stderr, output_limit));

    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            let _killed = child.kill().await;
            return Err(MediaError::new(
                fallback,
                format!("échec de l'exécution de {program} : {error}"),
            ));
        }
        Err(_) => {
            let _killed = child.kill().await;
            return Err(MediaError::new(
                MediaErrorCode::DownloadTimeout,
                format!(
                    "le processus {program} a dépassé le délai de {:.1} s",
                    timeout.as_secs_f64()
                ),
            ));
        }
    };

    let (stdout_bytes, stderr_bytes) = match tokio::join!(stdout_task, stderr_task) {
        (Ok(Ok(stdout)), Ok(Ok(stderr))) => (stdout, stderr),
        _ => {
            return Err(MediaError::new(
                fallback,
                format!("lecture incomplète de la sortie de {program}"),
            ));
        }
    };

    if stdout_bytes.1 || stderr_bytes.1 {
        return Err(MediaError::new(
            MediaErrorCode::InvalidMedia,
            format!(
                "sortie de {program} trop volumineuse (> {output_limit} octets) : média ou réponse suspecte"
            ),
        ));
    }

    let stdout = String::from_utf8_lossy(&stdout_bytes.0).into_owned();
    let stderr = String::from_utf8_lossy(&stderr_bytes.0).into_owned();
    if !status.success() {
        return Err(MediaError::from_command_failure(
            &program, &stdout, &stderr, fallback,
        ));
    }
    Ok(CommandOutput { stdout, stderr })
}

/// Lit un flux jusqu'à EOF en bornant la mémoire conservée.
///
/// Retourne `(octets_conservés, dépassement)`. En cas de dépassement, le flux
/// continue d'être drainé (pour ne pas bloquer le processus enfant) mais les
/// octets supplémentaires sont ignorés.
async fn read_stream_bounded<R>(mut reader: R, limit: usize) -> io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut captured = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 16 * 1024];
    let mut oversized = false;
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        if captured.len() >= limit {
            oversized = true;
            continue;
        }
        let remaining = limit - captured.len();
        let taken = read.min(remaining);
        captured.extend_from_slice(&chunk[..taken]);
        if taken < read {
            oversized = true;
        }
    }
    Ok((captured, oversized))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_binary_is_reported_with_fallback_code() {
        let error = run_command(
            Path::new("lc-media-binary-qui-nexiste-pas"),
            &[],
            Duration::from_secs(2),
            MediaErrorCode::TranscodeFailed,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), MediaErrorCode::TranscodeFailed);
        assert!(error.message().contains("impossible de lancer"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn successful_command_returns_stdout() {
        let args = vec!["/C".to_string(), "echo hello".to_string()];
        let output = run_command(
            Path::new("cmd.exe"),
            &args,
            Duration::from_secs(10),
            MediaErrorCode::DownloadFailed,
        )
        .await
        .unwrap();
        assert!(output.stdout.contains("hello"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn oversized_output_is_rejected() {
        let args = vec![
            "/C".to_string(),
            "for /L %i in (1,1,100) do @echo 0123456789".to_string(),
        ];
        let error = run_command_with_limit(
            Path::new("cmd.exe"),
            &args,
            Duration::from_secs(10),
            MediaErrorCode::DownloadFailed,
            64,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), MediaErrorCode::InvalidMedia);
        assert!(error.message().contains("trop volumineuse"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn oversized_output_is_rejected() {
        let args = vec![
            "-c".to_string(),
            "for i in $(seq 1 100); do echo 0123456789; done".to_string(),
        ];
        let error = run_command_with_limit(
            Path::new("sh"),
            &args,
            Duration::from_secs(10),
            MediaErrorCode::DownloadFailed,
            64,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), MediaErrorCode::InvalidMedia);
        assert!(error.message().contains("trop volumineuse"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn failing_command_is_classified() {
        let args = vec![
            "/C".to_string(),
            "echo invalid data found when processing input 1>&2 & exit /B 1".to_string(),
        ];
        let error = run_command(
            Path::new("cmd.exe"),
            &args,
            Duration::from_secs(10),
            MediaErrorCode::TranscodeFailed,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), MediaErrorCode::InvalidMedia);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_command_returns_stdout() {
        let args = vec!["-c".to_string(), "echo hello".to_string()];
        let output = run_command(
            Path::new("sh"),
            &args,
            Duration::from_secs(10),
            MediaErrorCode::DownloadFailed,
        )
        .await
        .unwrap();
        assert!(output.stdout.contains("hello"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failing_command_is_classified() {
        let args = vec![
            "-c".to_string(),
            "echo 'invalid data found when processing input' 1>&2; exit 1".to_string(),
        ];
        let error = run_command(
            Path::new("sh"),
            &args,
            Duration::from_secs(10),
            MediaErrorCode::TranscodeFailed,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), MediaErrorCode::InvalidMedia);
    }

    #[test]
    fn combined_output_prefers_stderr() {
        let output = CommandOutput {
            stdout: "out".to_string(),
            stderr: "err".to_string(),
        };
        assert_eq!(output.combined(), "err\nout");
        let empty = CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
        };
        assert_eq!(empty.combined(), "");
    }
}
