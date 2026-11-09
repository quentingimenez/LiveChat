//! Exécution contrôlée des binaires externes (`ffmpeg`, `ffprobe`, `yt-dlp`).
//!
//! Chaque processus est lancé avec un timeout, un `kill_on_drop`, et une
//! classification d'erreur cohérente avec le reste du pipeline.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::error::{MediaError, MediaErrorCode, MediaResult};

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
///   [`MediaError::from_command_failure`].
pub async fn run_command(
    binary: &Path,
    args: &[String],
    timeout: Duration,
    fallback: MediaErrorCode,
) -> MediaResult<CommandOutput> {
    let program = binary.display().to_string();
    let child = Command::new(binary)
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

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return Err(MediaError::new(
                fallback,
                format!("échec de l'exécution de {program} : {error}"),
            ));
        }
        Err(_) => {
            return Err(MediaError::new(
                MediaErrorCode::DownloadTimeout,
                format!(
                    "le processus {program} a dépassé le délai de {:.1} s",
                    timeout.as_secs_f64()
                ),
            ));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(MediaError::from_command_failure(
            &program, &stdout, &stderr, fallback,
        ));
    }
    Ok(CommandOutput { stdout, stderr })
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
