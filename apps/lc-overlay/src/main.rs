// Pas de console visible en release (même comportement que l'overlay Electron).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = lc_overlay::run() {
        eprintln!("lc-overlay: démarrage impossible: {error}");
        std::process::exit(1);
    }
}
