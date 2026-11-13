//! Coquille applicative Tauri 2 de l'overlay LiveChat (Phase 8a).
//!
//! Cette crate construit les deux fenêtres (overlay transparent click-through,
//! board 1180×780), le tray, le single-instance, l'autostart, le squelette des
//! raccourcis globaux et l'updater. L'appairage se fait désormais dans la board.
//! Le renderer existant (`web/bridge.js`) est embarqué ; le client WebSocket et
//! la logique métier arrivent en Phase 8b sans changer le contrat exposé.
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
pub mod device_settings;
pub mod display;
pub mod events;
pub mod net;
pub mod shortcuts;
pub mod state;
pub mod tray;
pub mod updater;
#[cfg(windows)]
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
/// Par défaut le `productName` de la configuration Tauri : `LiveChat` pour le
/// canal stable (même valeur, casse ignorée, que l'Electron historique — activer
/// l'autostart pendant une coexistence side-by-side écraserait donc la clé de
/// l'ancienne application, voir docs/rust/lc-overlay-shell.md) et
/// `LiveChat Next` pour la bêta, ce qui donne deux valeurs `Run` distinctes.
/// `LIVECHAT_PRODUCT_NAME` reste un override explicite (tests, lancements
/// portables).
fn autostart_app_name(product_name: &str) -> String {
    std::env::var("LIVECHAT_PRODUCT_NAME").unwrap_or_else(|_| product_name.to_string())
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

/// Résolution du chemin de configuration AVANT la création des fenêtres.
///
/// L'état applicatif est géré via `Builder::manage` (et non `App::manage` dans
/// `setup`) pour être disponible dès le chargement initial des webviews : les
/// pages appellent `overlay_get_config` pendant leur boot, avant que `setup`
/// ne s'exécute, et une course provoquerait `state not managed`.
/// Conséquence : le fallback `app_config_dir()` n'est pas disponible ici, mais
/// sur Windows `%APPDATA%` existe toujours et `resolve_config_path_from_env`
/// l'utilise en priorité ; le fallback n'est qu'un garde-fou.
fn initial_store() -> ConfigStore {
    let fallback = std::env::temp_dir().join("lc-overlay");
    let path = config::resolve_config_path_from_env(Some(fallback.as_path()));
    tracing::info!(chemin = %path.display(), "fichier de configuration forcé");
    ConfigStore::new(path)
}

/// Migration éventuelle et sauvegarde `.bak`, une fois l'`App` construite.
fn finalize_store(app: &tauri::App<Wry>) {
    let state = app.state::<AppState>();
    if let Some(fallback) = app_config_dir(app) {
        match state
            .store()
            .migrate_from(&fallback.join(config::CONFIG_FILE_NAME))
        {
            Ok(true) => tracing::info!("configuration migrée depuis app_config_dir()"),
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, "migration de configuration impossible"),
        }
    }
    match state.store().ensure_backup() {
        Ok(true) => tracing::info!(
            backup = %state.store().backup_path().display(),
            "sauvegarde .bak créée"
        ),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "création du fichier .bak impossible"),
    }
}

fn handle_second_instance(app: &tauri::AppHandle<Wry>, argv: Vec<String>) {
    tracing::info!(
        ?argv,
        "deuxième instance : transfert vers l'instance principale"
    );
    windows::handle_second_instance(app, &argv);
}

fn setup(app: &mut tauri::App<Wry>) -> Result<(), Box<dyn Error>> {
    finalize_store(app);
    let handle = app.handle().clone();
    if let Err(error) = tray::create_tray(&handle) {
        tracing::warn!(%error, "création du tray impossible");
    }
    windows::bootstrap_windows(&handle);
    shortcuts::restore_bindings(&handle);
    // Le superviseur WebSocket démarre en dernier : la configuration, le tray et
    // les raccourcis sont prêts avant la première connexion.
    net::ws::start(&handle);
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

    // Contexte construit tôt : le `productName` sert de nom de valeur `Run` au
    // plugin autostart (LiveChat vs LiveChat Next pour la coexistence bêta).
    let context = tauri::generate_context!();
    let product_name = context
        .config()
        .product_name
        .clone()
        .unwrap_or_else(|| "LiveChat".to_string());

    let app = tauri::Builder::<Wry>::default()
        // Géré avant la construction des fenêtres configurées (les webviews
        // peuvent appeler des commandes dès leur chargement initial).
        .manage(AppState::new(initial_store()))
        .plugin(tauri_plugin_single_instance::init(
            |app: &tauri::AppHandle<Wry>, argv: Vec<String>, _cwd: String| {
                handle_second_instance(app, argv);
            },
        ))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(autostart_app_name(&product_name))
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
            commands::media_board_get_bindings,
            commands::media_board_set_bindings,
            commands::media_board_trigger,
            commands::media_board_stop,
            commands::guilds_list,
            commands::guild_select,
            commands::guild_set_muted,
            commands::guild_leave,
            commands::guild_api,
            commands::guild_media_url,
            commands::overlay_check_updates,
            commands::overlay_install_update,
            commands::board_take_pending_view,
        ])
        .setup(setup)
        .build(context)?;

    app.run(|app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            net::ws::request_shutdown(app_handle);
            shortcuts::unregister_all(app_handle);
            tracing::info!("overlay : sortie propre");
        }
    });

    Ok(())
}
