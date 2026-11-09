//! État partagé de la coquille applicative.
//!
//! Le shell 8a ne contient pas encore de client WebSocket : l'état expose les
//! points d'accroche (configuration persistée, buffer d'événements, bindings de
//! raccourcis actifs) que la Phase 8b branchera sur ses commandes réseau.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::config::{ConfigStore, MemeBindings, OverlayConfig, OverlayConfigView};
use crate::events::EventBuffer;

/// État géré par Tauri et partagé entre les commandes et les threads.
pub struct AppState {
    store: ConfigStore,
    config: Mutex<OverlayConfig>,
    overlay_ready: AtomicBool,
    pending: EventBuffer,
    active_bindings: Mutex<MemeBindings>,
}

impl AppState {
    pub fn new(store: ConfigStore) -> Self {
        let config = store.load();
        Self {
            store,
            config: Mutex::new(config),
            overlay_ready: AtomicBool::new(false),
            pending: EventBuffer::new(),
            active_bindings: Mutex::new(MemeBindings::default()),
        }
    }

    pub fn store(&self) -> &ConfigStore {
        &self.store
    }

    /// Chemin forcé `%APPDATA%\LiveChat\config.json`.
    pub fn config_path(&self) -> &Path {
        self.store.path()
    }

    pub fn config_snapshot(&self) -> OverlayConfig {
        self.lock_config().clone()
    }

    /// Projection 8 champs envoyée au renderer.
    pub fn view(&self) -> OverlayConfigView {
        self.config_snapshot().view()
    }

    /// Applique une mutation puis persiste le fichier. En cas d'échec d'écriture,
    /// la valeur en mémoire est laissée inchangée (sémantique `saveConfig` legacy).
    pub fn update_config<F>(&self, mutate: F) -> Result<OverlayConfig, String>
    where
        F: FnOnce(&mut OverlayConfig),
    {
        let mut guard = self.lock_config();
        let mut next = guard.clone();
        mutate(&mut next);
        next.normalize();
        self.store.save(&next).map_err(|error| error.to_string())?;
        *guard = next.clone();
        Ok(next)
    }

    pub fn overlay_ready(&self) -> bool {
        self.overlay_ready.load(Ordering::SeqCst)
    }

    pub fn mark_overlay_ready(&self) {
        self.overlay_ready.store(true, Ordering::SeqCst);
    }

    pub fn pending_events(&self) -> &EventBuffer {
        &self.pending
    }

    pub fn active_bindings(&self) -> MemeBindings {
        self.lock_bindings().clone()
    }

    pub fn set_active_bindings(&self, bindings: MemeBindings) {
        *self.lock_bindings() = bindings;
    }

    fn lock_config(&self) -> MutexGuard<'_, OverlayConfig> {
        self.config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_bindings(&self) -> MutexGuard<'_, MemeBindings> {
        self.active_bindings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ConfigStore::new(dir.path().join("config.json"));
        (dir, store)
    }

    #[test]
    fn new_state_loads_persisted_config() {
        let (_dir, store) = temp_store();
        let mut config = OverlayConfig::default();
        config.volume = 0.25;
        store.save(&config).expect("save");
        let state = AppState::new(store);
        assert!((state.config_snapshot().volume - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn update_config_mutates_and_persists() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        let updated = state
            .update_config(|config| {
                config.show_text = false;
                config.server_url = Some("http://localhost:3000/".to_string());
            })
            .expect("update");
        assert!(!updated.show_text);
        // Le legacy conserve `serverUrl` tel quel dans le fichier (la
        // normalisation `normalizeServerUrl` n'intervient qu'à l'usage, dans
        // les flux d'appairage/connexion).
        assert_eq!(
            updated.server_url.as_deref(),
            Some("http://localhost:3000/")
        );
        assert!(!state.config_snapshot().show_text);
    }

    #[test]
    fn overlay_ready_flag_toggles_once() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        assert!(!state.overlay_ready());
        state.mark_overlay_ready();
        assert!(state.overlay_ready());
    }

    #[test]
    fn active_bindings_are_replaceable() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        let mut bindings = MemeBindings::default();
        bindings.insert("Ctrl+Shift+1".to_string(), "meme-1".to_string());
        state.set_active_bindings(bindings.clone());
        assert_eq!(state.active_bindings(), bindings);
    }
}
