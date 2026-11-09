//! Coquille applicative Tauri 2 de l'overlay LiveChat (Phase 8a).
//!
//! Cette crate construit les trois fenêtres (overlay transparent click-through,
//! pairing 460×580, board 1180×780), le tray, le single-instance, l'autostart,
//! le squelette des raccourcis globaux et l'updater. Le renderer existant
//! (`web/bridge.js`) est embarqué ; le client WebSocket et la logique métier
//! arrivent en Phase 8b sans changer le contrat exposé.
//!
//! Le seul module autorisé à utiliser `unsafe` est [`win32`] (bloc Win32 isolé
//! et documenté), d'où `deny` au niveau crate plutôt que `forbid`, comme
//! expliqué dans `docs/rust/lc-overlay-shell.md`.
#![deny(unsafe_code)]
// Les tests unitaires peuvent utiliser unwrap/expect/panic et des littéraux de
// test ; le contrat « pas de unwrap/expect/panic » ne concerne que le code de
// production (cf. docs/rust/lc-overlay-shell.md).
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::field_reassign_with_default
    )
)]

pub mod commands;
pub mod config;
pub mod display;
pub mod events;
pub mod shortcuts;
pub mod state;
pub mod tray;
pub mod updater;
pub mod win32;
pub mod windows;

use std::error::Error;
use std::path::PathBuf;

use tauri::{Manager, Wry};

use crate::config::ConfigStore;
use crate::state::AppState;

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("lc_overlay=info,tauri=warn,tauri_plugin_global_shortcut=warn")
    });
    if let Err(error) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init()
    {
        eprintln!("lc-overlay: initialisation du logging impossible: {error}");
    }
}

/// Nom de la valeur `Run` utilisée par le plugin autostart.
///
/// Par défaut `LiveChat` : c'est la même valeur (casse ignorée) que celle de
/// l'Electron historique, donc l'activer pendant une coexistence side-by-side
/// écraserait la clé de l'ancienne application (cf. docs/rust/lc-overlay-shell.md).
/// Les builds bêta peuvent définir `LIVECHAT_PRODUCT_NAME="LiveChat Next"` pour
/// utiliser une valeur distincte.
fn autostart_app_name() -> String {
    std::env::var("LIVECHAT_PRODUCT_NAME").unwrap_or_else(|_| "LiveChat".to_string())
}

fn app_config_dir(app: &tauri::App<Wry>) -> Option<PathBuf> {
    match app.path().app_config_dir() {
        Ok(path) => Some(path),
        Err(error) => {
            tracing::warn!(%error, "app_config_dir() indisponible");
            None
        }
    }
}

fn prepare_store(app: &tauri::App<Wry>) -> ConfigStore {
    let fallback = app_config_dir(app);
    let path = config::resolve_config_path_from_env(fallback.as_deref());
    tracing::info!(chemin = %path.display(), "fichier de configuration forcé");
    let store = ConfigStore::new(path);
    if let Some(fallback) = fallback.as_deref() {
        match store.migrate_from(&fallback.join(config::CONFIG_FILE_NAME)) {
            Ok(true) => tracing::info!("configuration migrée depuis app_config_dir()"),
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "migration de configuration impossible"),
        }
    }
    match store.ensure_backup() {
        Ok(true) => {
            tracing::info!(backup = %store.backup_path().display(), "sauvegarde .bak créée")
        }
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "création du fichier .bak impossible"),
    }
    store
}

fn handle_second_instance(app: &tauri::AppHandle<Wry>, argv: Vec<String>) {
    tracing::info!(
        ?argv,
        "deuxième instance : transfert vers l'instance principale"
    );
    windows::handle_second_instance(app, &argv);
}

fn setup(app: &mut tauri::App<Wry>) -> Result<(), Box<dyn Error>> {
    let store = prepare_store(app);
    let handle = app.handle().clone();
    // L'état doit être géré avant toute commande/tray qui lit `app.state()`.
    app.manage(AppState::new(store));
    if let Err(error) = tray::create_tray(&handle) {
        tracing::warn!(%error, "création du tray impossible");
    }
    windows::bootstrap_windows(&handle);
    shortcuts::restore_bindings(&handle);
    Ok(())
}

fn handler_closures() -> impl Fn(
    &tauri::AppHandle<Wry>,
    &tauri_plugin_global_shortcut::Shortcut,
    tauri_plugin_global_shortcut::ShortcutEvent,
) + Send
       + Sync
       + 'static {
    |app, shortcut, event| {
        if matches!(
            event.state(),
            tauri_plugin_global_shortcut::ShortcutState::Pressed
        ) {
            shortcuts::handle_shortcut_pressed(app, shortcut);
        }
    }
}

/// Démarre l'application overlay.
pub fn run() -> Result<(), Box<dyn Error>> {
    init_tracing();

    let app = tauri::Builder::<Wry>::default()
        .plugin(tauri_plugin_single_instance::init(
            |app: &tauri::AppHandle<Wry>, argv: Vec<String>, _cwd: String| {
                handle_second_instance(app, argv);
            },
        ))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(autostart_app_name())
                .args(["--from-autostart"])
                .build(),
        )
        .plugin(tauri_plugin_fs::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(handler_closures())
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::overlay_get_config,
            commands::overlay_renderer_ready,
            commands::overlay_report_error,
            commands::overlay_report_playback_state,
            commands::overlay_report_playback_stop,
            commands::pairing_consume,
            commands::meme_board_get_bindings,
            commands::meme_board_set_bindings,
            commands::meme_board_trigger,
            commands::meme_board_stop,
            commands::overlay_check_updates,
            commands::overlay_install_update,
        ])
        .setup(setup)
        .build(tauri::generate_context!())?;

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            shortcuts::unregister_all(app_handle);
            tracing::info!("overlay : sortie propre");
        }
    });

    Ok(())
}
