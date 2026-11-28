//! Ouvre le dépôt du projet dans le navigateur par défaut.
//!
//! La board ne peut pas naviguer vers une page distante : le WebView
//! remplacerait l'overlay, et Tauri n'ouvre pas de nouvelle fenêtre pour un
//! `target="_blank"`. Cette commande confie donc une URL fixe au système, sans
//! jamais accepter d'adresse venue du renderer.

use std::process::Command;

use tauri::{WebviewWindow, Wry};

/// Dépôt public du projet LiveChat.
pub const REPOSITORY_URL: &str = "https://github.com/quentingimenez/LiveChat";

/// Ouvre le dépôt GitHub dans le navigateur par défaut.
///
/// L'URL est constante et la commande reste réservée à la board (matrice IPC
/// de `commands.rs`) : rien de ce que le renderer envoie n'atteint le shell.
#[tauri::command]
pub fn open_repository(window: WebviewWindow<Wry>) -> Result<(), String> {
    if !crate::commands::command_allowed(window.label(), "open_repository") {
        return Err("Fenêtre non autorisée.".to_string());
    }
    open_in_browser(REPOSITORY_URL)
}

#[cfg(windows)]
fn open_in_browser(url: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    // Évite l'apparition fugace de la console de `cmd`.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    Command::new("cmd")
        .args(["/C", "start", "", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Impossible d'ouvrir le navigateur : {error}"))
}

#[cfg(not(windows))]
fn open_in_browser(url: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Impossible d'ouvrir le navigateur : {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_url_points_at_the_public_project() {
        assert_eq!(REPOSITORY_URL, "https://github.com/quentingimenez/LiveChat");
        assert!(!REPOSITORY_URL.ends_with('/'));
    }
}
