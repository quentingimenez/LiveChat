//! État partagé de la coquille applicative.
//!
//! La Phase 8b branche le client WebSocket v2 sur cet état : configuration
//! persistée, buffer d'événements, bindings de raccourcis actifs, snapshot de
//! connexion (statut/version/pairs) et canal de commandes vers la tâche réseau.

use std::collections::{BTreeMap, VecDeque};
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

/// `play` reçu d'une guilde pendant qu'une autre occupe l'écran.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedPlay {
    pub guild_id: String,
    pub job_id: String,
    pub payload: serde_json::Value,
}

/// État géré par Tauri et partagé entre les commandes et les threads.
pub struct AppState {
    store: ConfigStore,
    config: Mutex<OverlayConfig>,
    overlay_ready: AtomicBool,
    pending: EventBuffer,
    active_bindings: Mutex<MemeBindings>,
    /// Instantané de connexion par guilde.
    nets: Mutex<BTreeMap<String, NetSnapshot>>,
    /// Canal de commandes du superviseur de chaque guilde.
    net_commands: Mutex<BTreeMap<String, UnboundedSender<NetCommand>>>,
    /// Lecture à l'écran : `(guilde, job)`.
    playback_job: Mutex<Option<(String, String)>>,
    /// `play` des autres guildes en attente de l'écran.
    local_queue: Mutex<VecDeque<QueuedPlay>>,
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
            nets: Mutex::new(BTreeMap::new()),
            net_commands: Mutex::new(BTreeMap::new()),
            playback_job: Mutex::new(None),
            local_queue: Mutex::new(VecDeque::new()),
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

    /// Guilde active (celle du board et des champs historiques de la config).
    pub fn active_guild(&self) -> Option<String> {
        self.lock_config().guild_id.clone()
    }

    /// Instantané de connexion de la guilde active.
    pub fn net_snapshot(&self) -> NetSnapshot {
        match self.active_guild() {
            Some(guild_id) => self.net_snapshot_for(&guild_id),
            None => NetSnapshot::default(),
        }
    }

    /// Instantané de connexion d'une guilde (défaut si aucun superviseur).
    pub fn net_snapshot_for(&self, guild_id: &str) -> NetSnapshot {
        self.lock_nets().get(guild_id).cloned().unwrap_or_default()
    }

    /// Instantanés de toutes les guildes supervisées.
    pub fn net_snapshots(&self) -> BTreeMap<String, NetSnapshot> {
        self.lock_nets().clone()
    }

    /// Guildes rejointes et leur état de connexion, pour l'interface (aucun
    /// jeton n'y figure).
    pub fn guilds_payload(&self) -> serde_json::Value {
        let config = self.config_snapshot();
        let nets = self.net_snapshots();
        let playing = self.playback_guild();
        let guilds = config
            .membership_views()
            .into_iter()
            .map(|view| {
                let net = nets.get(&view.guild_id).cloned().unwrap_or_default();
                let is_playing = playing.as_deref() == Some(view.guild_id.as_str());
                serde_json::json!({
                    "membership": view,
                    "connection": net,
                    "statusLabel": net.status.label(),
                    "playing": is_playing,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "activeGuildId": config.guild_id,
            "enabled": config.enabled,
            "profile": {
                "displayName": config.profile_name,
                "avatar": config.profile_avatar,
            },
            "guilds": guilds,
        })
    }

    fn with_net<F>(&self, guild_id: &str, mutate: F)
    where
        F: FnOnce(&mut NetSnapshot),
    {
        let mut guard = self.lock_nets();
        mutate(guard.entry(guild_id.to_string()).or_default());
    }

    pub fn set_net_status(&self, guild_id: &str, status: ConnectionStatus, reason: Option<String>) {
        self.with_net(guild_id, |net| {
            net.status = status;
            net.reason = reason;
            if status == ConnectionStatus::Connected {
                net.last_error = None;
            }
        });
    }

    /// Applique un `welcome` : identité serveur, session et statut connecté.
    pub fn set_net_welcome(&self, guild_id: &str, payload: &lc_protocol::WelcomePayload) {
        self.with_net(guild_id, |net| {
            net.status = ConnectionStatus::Connected;
            net.reason = None;
            net.last_error = None;
            net.server_version = Some(payload.server_version.clone());
            net.min_version = None;
            net.guild_id = Some(payload.guild_id.clone());
            net.client_id = Some(payload.client_id.clone());
            net.session_mode = Some(session_mode_label(payload.session_mode).to_string());
            net.peers.clear();
        });
    }

    pub fn set_net_peers(&self, guild_id: &str, labels: Vec<String>) {
        self.with_net(guild_id, |net| net.peers = labels);
    }

    /// Met à jour le résumé de session exposé au tray après un `settings`.
    pub fn set_net_settings_summary(&self, guild_id: &str, session_mode: lc_protocol::SessionMode) {
        self.with_net(guild_id, |net| {
            net.guild_id = Some(guild_id.to_string());
            net.session_mode = Some(session_mode_label(session_mode).to_string());
        });
    }

    pub fn set_net_error(&self, guild_id: &str, message: String) {
        self.with_net(guild_id, |net| net.last_error = Some(message));
    }

    pub fn set_net_command_sender(&self, guild_id: &str, sender: UnboundedSender<NetCommand>) {
        self.lock_net_commands()
            .insert(guild_id.to_string(), sender);
    }

    /// `true` si un superviseur est enregistré pour la guilde.
    pub fn has_supervisor(&self, guild_id: &str) -> bool {
        self.lock_net_commands().contains_key(guild_id)
    }

    /// Oublie le superviseur d'une guilde (il s'est arrêté).
    pub fn remove_net(&self, guild_id: &str) {
        self.lock_net_commands().remove(guild_id);
        self.lock_nets().remove(guild_id);
    }

    /// Guildes ayant un superviseur.
    pub fn supervised_guilds(&self) -> Vec<String> {
        self.lock_net_commands().keys().cloned().collect()
    }

    /// Envoie une commande au superviseur de la guilde active. `false` s'il
    /// n'est pas démarré (tests unitaires) ou si le canal est fermé.
    pub fn send_net_command(&self, command: NetCommand) -> bool {
        match self.active_guild() {
            Some(guild_id) => self.send_net_command_to(&guild_id, command),
            None => false,
        }
    }

    /// Envoie une commande au superviseur d'une guilde donnée.
    pub fn send_net_command_to(&self, guild_id: &str, command: NetCommand) -> bool {
        let guard = self.lock_net_commands();
        match guard.get(guild_id) {
            Some(sender) => sender.send(command).is_ok(),
            None => false,
        }
    }

    pub fn set_playback(&self, guild_id: &str, job_id: String) {
        *self.lock_playback() = Some((guild_id.to_string(), job_id));
    }

    pub fn clear_playback(&self) {
        *self.lock_playback() = None;
    }

    /// Job à l'écran, toutes guildes confondues.
    pub fn playback_job(&self) -> Option<String> {
        self.lock_playback().as_ref().map(|(_, job)| job.clone())
    }

    /// Guilde propriétaire du job à l'écran.
    pub fn playback_guild(&self) -> Option<String> {
        self.lock_playback()
            .as_ref()
            .map(|(guild, _)| guild.clone())
    }

    /// Met en attente le `play` d'une guilde pendant qu'une autre joue.
    pub fn enqueue_play(&self, queued: QueuedPlay) {
        let mut guard = self.lock_queue();
        guard.retain(|existing| existing.job_id != queued.job_id);
        guard.push_back(queued);
    }

    /// Prochain `play` en attente.
    pub fn dequeue_play(&self) -> Option<QueuedPlay> {
        self.lock_queue().pop_front()
    }

    /// Retire les `play` en attente d'une guilde (un job précis, ou tous).
    /// Retourne le nombre d'entrées retirées.
    pub fn drop_queued(&self, guild_id: &str, job_id: Option<&str>) -> usize {
        let mut guard = self.lock_queue();
        let before = guard.len();
        guard.retain(|queued| {
            queued.guild_id != guild_id || job_id.is_some_and(|job_id| queued.job_id != job_id)
        });
        before - guard.len()
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

    fn lock_nets(&self) -> MutexGuard<'_, BTreeMap<String, NetSnapshot>> {
        self.nets
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_net_commands(&self) -> MutexGuard<'_, BTreeMap<String, UnboundedSender<NetCommand>>> {
        self.net_commands
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_queue(&self) -> MutexGuard<'_, VecDeque<QueuedPlay>> {
        self.local_queue
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_playback(&self) -> MutexGuard<'_, Option<(String, String)>> {
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
        assert!(state.active_guild().is_none());
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
        state.set_net_status(
            "111",
            ConnectionStatus::Reconnecting,
            Some("test".to_string()),
        );
        let snapshot = state.net_snapshot_for("111");
        assert_eq!(snapshot.status, ConnectionStatus::Reconnecting);
        assert_eq!(snapshot.reason.as_deref(), Some("test"));
        state.set_net_error("111", "boom".to_string());
        assert_eq!(
            state.net_snapshot_for("111").last_error.as_deref(),
            Some("boom")
        );
        state.set_net_status("111", ConnectionStatus::Connected, None);
        assert!(state.net_snapshot_for("111").last_error.is_none());
        // Une autre guilde garde son propre état.
        assert_eq!(
            state.net_snapshot_for("222").status,
            ConnectionStatus::Disabled
        );
        assert_eq!(state.net_snapshots().len(), 1);
    }

    fn paired_state(guilds: &[&str]) -> (tempfile::TempDir, AppState) {
        let (dir, store) = temp_store();
        let state = AppState::new(store);
        state
            .update_config(|config| {
                for guild in guilds {
                    config.join(crate::config::Membership {
                        server_url: "https://example.test".to_string(),
                        guild_id: (*guild).to_string(),
                        client_token: format!("token-{guild}"),
                        ..Default::default()
                    });
                }
            })
            .expect("update");
        (dir, state)
    }

    #[test]
    fn commands_are_routed_per_guild() {
        let (_dir, state) = paired_state(&["111", "222"]);
        // La dernière guilde rejointe est active.
        assert_eq!(state.active_guild().as_deref(), Some("222"));
        let (first_tx, mut first_rx) = tokio::sync::mpsc::unbounded_channel();
        let (second_tx, mut second_rx) = tokio::sync::mpsc::unbounded_channel();
        state.set_net_command_sender("111", first_tx);
        state.set_net_command_sender("222", second_tx);

        assert!(state.send_net_command(NetCommand::Reconnect));
        assert!(matches!(second_rx.try_recv(), Ok(NetCommand::Reconnect)));
        assert!(first_rx.try_recv().is_err());

        assert!(state.send_net_command_to("111", NetCommand::Shutdown));
        assert!(matches!(first_rx.try_recv(), Ok(NetCommand::Shutdown)));
        assert!(!state.send_net_command_to("333", NetCommand::Reconnect));

        state.remove_net("111");
        assert!(!state.has_supervisor("111"));
        assert_eq!(state.supervised_guilds(), vec!["222".to_string()]);
    }

    #[test]
    fn playback_owner_and_local_queue() {
        let (_dir, state) = paired_state(&["111", "222"]);
        state.set_playback("111", "job-a".to_string());
        assert_eq!(state.playback_guild().as_deref(), Some("111"));
        assert_eq!(state.playback_job().as_deref(), Some("job-a"));

        let queued = |guild: &str, job: &str| QueuedPlay {
            guild_id: guild.to_string(),
            job_id: job.to_string(),
            payload: serde_json::json!({"jobId": job}),
        };
        state.enqueue_play(queued("222", "job-b"));
        state.enqueue_play(queued("222", "job-b"));
        state.enqueue_play(queued("222", "job-c"));
        state.enqueue_play(queued("111", "job-d"));

        // Un `stop` ciblé ne retire que ce job ; sans job, toute la guilde.
        assert_eq!(state.drop_queued("222", Some("job-c")), 1);
        assert_eq!(state.drop_queued("111", None), 1);
        assert_eq!(
            state.dequeue_play().map(|play| play.job_id).as_deref(),
            Some("job-b")
        );
        assert!(state.dequeue_play().is_none());

        state.clear_playback();
        assert!(state.playback_guild().is_none());
    }
}
