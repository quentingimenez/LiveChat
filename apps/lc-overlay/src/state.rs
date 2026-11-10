//! État partagé de la coquille applicative.
//!
//! La Phase 8b branche le client WebSocket v2 sur cet état : configuration
//! persistée, buffer d'événements, bindings de raccourcis actifs, snapshot de
//! connexion (statut/version/pairs) et canal de commandes vers la tâche réseau.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::config::{ConfigStore, MemeBindings, OverlayConfig, OverlayConfigView};
use crate::events::EventBuffer;
use crate::net::ws::{ConnectionStatus, NetCommand, NetSnapshot};

/// Cadence maximale d'envoi de `playback.state` (contrat v2 : 1 msg/s).
const PLAYBACK_STATE_MIN_INTERVAL: Duration = Duration::from_secs(1);

/// État géré par Tauri et partagé entre les commandes et les threads.
pub struct AppState {
    store: ConfigStore,
    config: Mutex<OverlayConfig>,
    overlay_ready: AtomicBool,
    pending: EventBuffer,
    active_bindings: Mutex<MemeBindings>,
    net: Mutex<NetSnapshot>,
    net_commands: Mutex<Option<UnboundedSender<NetCommand>>>,
    playback_job: Mutex<Option<String>>,
    last_state_sent: Mutex<Option<Instant>>,
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
            net: Mutex::new(NetSnapshot::default()),
            net_commands: Mutex::new(None),
            playback_job: Mutex::new(None),
            last_state_sent: Mutex::new(None),
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

    // --- Client WebSocket v2 -------------------------------------------------

    pub fn net_snapshot(&self) -> NetSnapshot {
        self.lock_net().clone()
    }

    pub fn set_net_status(&self, status: ConnectionStatus, reason: Option<String>) {
        let mut guard = self.lock_net();
        guard.status = status;
        guard.reason = reason;
        if status == ConnectionStatus::Connected {
            guard.last_error = None;
        }
    }

    pub fn set_net_min_version(&self, min_version: Option<String>) {
        self.lock_net().min_version = min_version;
    }

    /// Applique un `welcome` : identité serveur, session et statut connecté.
    pub fn set_net_welcome(&self, payload: &lc_protocol::WelcomePayload) {
        let mut guard = self.lock_net();
        guard.status = ConnectionStatus::Connected;
        guard.reason = None;
        guard.last_error = None;
        guard.server_version = Some(payload.server_version.clone());
        guard.min_version = None;
        guard.guild_id = Some(payload.guild_id.clone());
        guard.client_id = Some(payload.client_id.clone());
        guard.session_mode = Some(session_mode_label(payload.session_mode).to_string());
        guard.peers.clear();
    }

    pub fn set_net_peers(&self, labels: Vec<String>) {
        self.lock_net().peers = labels;
    }

    /// Met à jour le résumé de session exposé au tray après un `settings`.
    pub fn set_net_settings_summary(&self, guild_id: &str, session_mode: lc_protocol::SessionMode) {
        let mut guard = self.lock_net();
        guard.guild_id = Some(guild_id.to_string());
        guard.session_mode = Some(session_mode_label(session_mode).to_string());
    }

    pub fn set_net_error(&self, message: String) {
        let mut guard = self.lock_net();
        guard.last_error = Some(message);
    }

    pub fn set_net_command_sender(&self, sender: UnboundedSender<NetCommand>) {
        *self.lock_net_commands() = Some(sender);
    }

    /// Envoie une commande à la tâche réseau. `false` si le superviseur n'est
    /// pas démarré (tests unitaires) ou si le canal est fermé.
    pub fn send_net_command(&self, command: NetCommand) -> bool {
        let guard = self.lock_net_commands();
        match guard.as_ref() {
            Some(sender) => sender.send(command).is_ok(),
            None => false,
        }
    }

    pub fn set_playback(&self, job_id: String) {
        *self.lock_playback() = Some(job_id);
    }

    pub fn clear_playback(&self) {
        *self.lock_playback() = None;
    }

    pub fn playback_job(&self) -> Option<String> {
        self.lock_playback().clone()
    }

    /// Limite l'envoi de `playback.state` à un message par seconde (contrat v2).
    pub fn try_record_playback_state(&self) -> bool {
        let mut guard = self
            .last_state_sent
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = Instant::now();
        match *guard {
            Some(previous) if now.duration_since(previous) < PLAYBACK_STATE_MIN_INTERVAL => false,
            _ => {
                *guard = Some(now);
                true
            }
        }
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

    fn lock_net(&self) -> MutexGuard<'_, NetSnapshot> {
        self.net.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_net_commands(&self) -> MutexGuard<'_, Option<UnboundedSender<NetCommand>>> {
        self.net_commands
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_playback(&self) -> MutexGuard<'_, Option<String>> {
        self.playback_job
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn session_mode_label(session_mode: lc_protocol::SessionMode) -> &'static str {
    match session_mode {
        lc_protocol::SessionMode::Normal => "normal",
        lc_protocol::SessionMode::InviteReadOnly => "invite_read_only",
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

    #[test]
    fn net_snapshot_starts_disabled_without_sender() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        assert_eq!(state.net_snapshot().status, ConnectionStatus::Disabled);
        assert!(!state.send_net_command(NetCommand::Reconnect));
    }

    #[test]
    fn playback_state_is_throttled_to_one_per_second() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        assert!(state.try_record_playback_state());
        assert!(!state.try_record_playback_state());
    }

    #[test]
    fn net_status_and_errors_are_tracked() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        state.set_net_status(ConnectionStatus::Reconnecting, Some("test".to_string()));
        let snapshot = state.net_snapshot();
        assert_eq!(snapshot.status, ConnectionStatus::Reconnecting);
        assert_eq!(snapshot.reason.as_deref(), Some("test"));
        state.set_net_error("boom".to_string());
        assert_eq!(state.net_snapshot().last_error.as_deref(), Some("boom"));
        state.set_net_status(ConnectionStatus::Connected, None);
        assert!(state.net_snapshot().last_error.is_none());
    }
}
