//! Application overlay LiveChat (Tauri 2) : shell Rust, bridge renderer et fenêtres.
//!
//! Squelette de la Phase 0 : lib vide, volontairement sans dépendance Tauri pour
//! garder `cargo check --workspace` rapide. La Phase 8 ajoutera le `crate-type`
//! staticlib/cdylib, le binaire Tauri et le bridge `window.livechatOverlay`.
#![forbid(unsafe_code)]
