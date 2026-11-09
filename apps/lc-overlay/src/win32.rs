//! Bloc Win32 isolé du shell overlay.
//!
//! Exigences du contrat Fenêtre (docs/rust/overlay-bridge.md §5.3) :
//! l'overlay doit rester au-dessus des autres fenêtres sans jamais recevoir le
//! focus, rester hors de la barre des tâches, être cliquable-au-travers et
//! conserver ses styles lorsque WebView2 ou l'utilisateur les modifient.
//!
//! Tauri/tao pose déjà `WS_EX_LAYERED | WS_EX_TRANSPARENT` via
//! `set_ignore_cursor_events(true)`, `WS_EX_NOACTIVATE` via `set_focusable(false)`
//! et `WS_EX_TOOLWINDOW` via `set_skip_taskbar(true)`. Ce module réapplique les
//! quatre styles en ceinture et bretelles (le POC Phase 0 a montré qu'ils
//! pouvaient manquer selon le timing du `show()`).
//!
//! Ce fichier est le SEUL du crate autorisé à utiliser `unsafe` : il est
//! documenté comme tel dans `docs/rust/lc-overlay-shell.md`.
#![allow(unsafe_code)]

use windows::core::Result as WindowsResult;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, WINDOW_EX_STYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT,
};

/// Styles étendus requis par l'overlay transparent cliquable-au-travers.
pub const OVERLAY_EX_STYLE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(
    WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0,
);

/// Styles étendus à vérifier dans les logs après application.
pub fn describe_ex_style(ex_style: isize) -> String {
    let raw = ex_style as u32;
    let mut flags = Vec::new();
    if raw & WS_EX_LAYERED.0 != 0 {
        flags.push("WS_EX_LAYERED");
    }
    if raw & WS_EX_TRANSPARENT.0 != 0 {
        flags.push("WS_EX_TRANSPARENT");
    }
    if raw & WS_EX_NOACTIVATE.0 != 0 {
        flags.push("WS_EX_NOACTIVATE");
    }
    if raw & WS_EX_TOOLWINDOW.0 != 0 {
        flags.push("WS_EX_TOOLWINDOW");
    }
    if flags.is_empty() {
        format!("0x{raw:08X}")
    } else {
        format!("0x{raw:08X} [{}]", flags.join("|"))
    }
}

/// Retourne les styles étendus courants de la fenêtre.
pub fn ex_style(hwnd: HWND) -> isize {
    // SAFETY: `hwnd` provient d'une fenêtre Tauri vivante ; GetWindowLongPtrW ne
    // fait que lire l'état de la fenêtre.
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) }
}

/// Ajoute les styles overlay manquants (idempotent) et retourne les styles finaux.
pub fn ensure_overlay_ex_style(hwnd: HWND) -> isize {
    let current = ex_style(hwnd);
    let desired = (current as u32) | OVERLAY_EX_STYLE.0;
    if desired != current as u32 {
        // SAFETY: même HWND vivant ; l'appel ne modifie que les styles étendus et
        // le retour précédent est ignoré volontairement (échec = style inchangé).
        let _previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired as isize) };
        desired as isize
    } else {
        current
    }
}

/// Réaffirme `HWND_TOPMOST` sans activer ni déplacer la fenêtre.
pub fn reassert_topmost(hwnd: HWND) -> WindowsResult<()> {
    // SAFETY: `hwnd` provient d'une fenêtre Tauri vivante ; les flags interdisent
    // tout déplacement/redimensionnement/activation.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_reports_each_overlay_flag() {
        let described = describe_ex_style(OVERLAY_EX_STYLE.0 as isize);
        assert!(described.contains("WS_EX_LAYERED"));
        assert!(described.contains("WS_EX_TRANSPARENT"));
        assert!(described.contains("WS_EX_NOACTIVATE"));
        assert!(described.contains("WS_EX_TOOLWINDOW"));
    }

    #[test]
    fn describe_falls_back_to_raw_hex() {
        assert_eq!(describe_ex_style(0), "0x00000000");
    }
}
