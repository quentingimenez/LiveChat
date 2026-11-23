//! État partagé de la coquille applicative.
//!
//! La Phase 8b branche le client WebSocket v2 sur cet état : configuration
//! persistée, buffer d'événements, bindings de raccourcis actifs, snapshot de
//! connexion (statut/version/pairs) et canal de commandes vers la tâche réseau.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{ConfigStore, MediaBindings, OverlayConfig, OverlayConfigView};
use crate::events::EventBuffer;
use crate::net::ws::{ConnectionStatus, NetCommand, NetSnapshot};
use crate::screen::{PlayPresentation, ReportDecision, ScreenArbiter, StopDecision};

/// Cadence maximale d'envoi de `playback.state` (contrat v2 : 1 msg/s).
const PLAYBACK_STATE_MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Why a guild was automatically removed from the local configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuildRemovalReason {
    /// The guild was deleted by an administrator.
    GuildDeleted,
    /// The member was removed from the guild.
    MemberRemoved,
    /// The member was banned from the guild.
    MemberBanned,
    /// The overlay device was revoked.
    Revoked,
    /// The guild token is no longer valid.
    InvalidToken,
}

/// Guild removal waiting for the board to acknowledge it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuildRemoval {
    pub guild_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_name: Option<String>,
    pub reason: GuildRemovalReason,
}

pub use crate::screen::QueuedPlay;

/// État géré par Tauri et partagé entre les commandes et les threads.
pub struct AppState {
    store: ConfigStore,
    config: Mutex<OverlayConfig>,
    overlay_ready: AtomicBool,
    pending: EventBuffer,
    active_bindings: Mutex<MediaBindings>,
    /// Instantané de connexion par guilde.
    nets: Mutex<BTreeMap<String, NetSnapshot>>,
    /// Canal de commandes du superviseur de chaque guilde.
    net_commands: Mutex<BTreeMap<String, UnboundedSender<NetCommand>>>,
    /// Arbitrage de l'écran unique entre les guildes : propriétaire courant,
    /// file locale et routage des rapports de lecture.
    screen: Mutex<ScreenArbiter>,
    /// `true` quand le propriétaire de l'écran a changé depuis la dernière
    /// publication : le board n'apprend `playing` (pastille « à l'antenne » du
    /// rail) que par `overlay://guilds`, il faut donc le republier.
    screen_owner_changed: AtomicBool,
    last_state_sent: Mutex<Option<Instant>>,
    /// View requested by the tray before the board was loaded.
    pending_board_view: Mutex<Option<String>>,
    /// Removed guilds not consumed yet by the board.
    pending_guild_removals: Mutex<Vec<GuildRemoval>>,
}

impl AppState {
    pub fn new(store: ConfigStore) -> Self {
        let config = store.load();
        Self {
            store,
            config: Mutex::new(config),
            overlay_ready: AtomicBool::new(false),
            pending: EventBuffer::new(),
            active_bindings: Mutex::new(MediaBindings::default()),
            nets: Mutex::new(BTreeMap::new()),
            net_commands: Mutex::new(BTreeMap::new()),
            screen: Mutex::new(ScreenArbiter::default()),
            screen_owner_changed: AtomicBool::new(false),
            last_state_sent: Mutex::new(None),
            pending_board_view: Mutex::new(None),
            pending_guild_removals: Mutex::new(Vec::new()),
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

    pub fn active_bindings(&self) -> MediaBindings {
        self.lock_bindings().clone()
    }

    pub fn set_active_bindings(&self, bindings: MediaBindings) {
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

    /// Note le changement de propriétaire de l'écran provoqué par une
    /// transition. La pastille « à l'antenne » du rail ne vient que du champ
    /// `playing` d'`overlay://guilds` : le board doit être republié dès que
    /// l'écran change de main, sans attendre qu'il redemande l'état.
    fn note_screen_owner(&self, before: Option<String>, after: Option<&str>) {
        if before.as_deref() != after {
            self.screen_owner_changed.store(true, Ordering::SeqCst);
        }
    }

    /// Consomme le drapeau de changement de propriétaire de l'écran.
    ///
    /// `true` quand une transition a changé la guilde à l'antenne depuis la
    /// dernière publication : l'appelant doit émettre `overlay://guilds`.
    pub fn take_screen_owner_change(&self) -> bool {
        self.screen_owner_changed.swap(false, Ordering::SeqCst)
    }

    /// Décide du sort d'un `play` : affichage immédiat ou mise en file locale.
    ///
    /// `showable` dit si l'overlay peut afficher quelque chose (activé et
    /// appairé) : sinon le `play` attend son tour au lieu de devenir un
    /// propriétaire fantôme que le renderer n'a jamais reçu.
    pub fn present_play(
        &self,
        guild_id: &str,
        job_id: &str,
        payload: serde_json::Value,
        showable: bool,
    ) -> PlayPresentation {
        let mut screen = self.lock_screen();
        let before = screen.screen_guild().map(str::to_owned);
        let presentation = screen.present(guild_id, job_id, payload, showable);
        self.note_screen_owner(before, screen.screen_guild());
        presentation
    }

    /// Affiche le prochain `play` en attente si l'écran est libre.
    pub fn advance_screen(&self, showable: bool) -> Option<QueuedPlay> {
        let mut screen = self.lock_screen();
        let before = screen.screen_guild().map(str::to_owned);
        let next = screen.advance(showable);
        self.note_screen_owner(before, screen.screen_guild());
        next
    }

    /// `stop` reçu du serveur pour une guilde : purge sa file, et libère
    /// l'écran seulement si elle l'occupe encore.
    pub fn server_stop(
        &self,
        guild_id: &str,
        job_id: Option<&str>,
        showable: bool,
    ) -> StopDecision {
        let mut screen = self.lock_screen();
        let before = screen.screen_guild().map(str::to_owned);
        let decision = screen.server_stop(guild_id, job_id, showable);
        self.note_screen_owner(before, screen.screen_guild());
        decision
    }

    /// Fin de session d'une guilde : purge sa file et libère son écran.
    pub fn release_guild(&self, guild_id: &str, showable: bool) -> StopDecision {
        let mut screen = self.lock_screen();
        let before = screen.screen_guild().map(str::to_owned);
        let decision = screen.release_guild(guild_id, showable);
        self.note_screen_owner(before, screen.screen_guild());
        decision
    }

    /// Rapport de fin de lecture du renderer, routé vers la guilde du job.
    pub fn report_stop(&self, job_id: Option<&str>, showable: bool) -> ReportDecision {
        let mut screen = self.lock_screen();
        let before = screen.screen_guild().map(str::to_owned);
        let decision = screen.report_stop(job_id, showable);
        self.note_screen_owner(before, screen.screen_guild());
        decision
    }

    /// Job à l'écran, toutes guildes confondues.
    pub fn playback_job(&self) -> Option<String> {
        self.lock_screen()
            .screen()
            .map(|owner| owner.job_id.clone())
    }

    /// Guilde propriétaire du job à l'écran.
    pub fn playback_guild(&self) -> Option<String> {
        self.lock_screen().screen_guild().map(str::to_owned)
    }

    /// Guilde propriétaire d'un job : l'écran, puis les derniers affichés.
    pub fn job_owner(&self, job_id: &str) -> Option<String> {
        self.lock_screen().owner_of(job_id).map(str::to_owned)
    }

    /// `(job_id, durée en ms)` des `play` d'une guilde encore en file locale,
    /// dans l'ordre d'attente.
    pub fn queued_jobs_for(&self, guild_id: &str) -> Vec<(String, u64)> {
        self.lock_screen().queued_jobs_for(guild_id)
    }

    /// Remembers the view requested by the tray for the board (e.g. `settings`).
    pub fn set_pending_board_view(&self, view: impl Into<String>) {
        *self.lock_pending_board_view() = Some(view.into());
    }

    /// Returns and clears the pending view for the board.
    pub fn take_pending_board_view(&self) -> Option<String> {
        self.lock_pending_board_view().take()
    }

    /// Queues a removed guild for the board (also emitted as an event).
    pub fn push_guild_removal(&self, removal: GuildRemoval) {
        self.lock_guild_removals().push(removal);
    }

    /// Returns and clears every guild removal waiting for the board.
    pub fn take_guild_removals(&self) -> Vec<GuildRemoval> {
        std::mem::take(&mut *self.lock_guild_removals())
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

    fn lock_bindings(&self) -> MutexGuard<'_, MediaBindings> {
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

    fn lock_screen(&self) -> MutexGuard<'_, ScreenArbiter> {
        self.screen
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_pending_board_view(&self) -> MutexGuard<'_, Option<String>> {
        self.pending_board_view
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_guild_removals(&self) -> MutexGuard<'_, Vec<GuildRemoval>> {
        self.pending_guild_removals
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
        let mut bindings = MediaBindings::default();
        bindings.insert("Ctrl+Shift+1".to_string(), "media-1".to_string());
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
    fn pending_board_view_is_taken_once() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        assert!(state.take_pending_board_view().is_none());
        state.set_pending_board_view("settings");
        assert_eq!(state.take_pending_board_view().as_deref(), Some("settings"));
        assert!(state.take_pending_board_view().is_none());
    }

    #[test]
    fn guild_removals_are_taken_once() {
        let (_dir, store) = temp_store();
        let state = AppState::new(store);
        assert!(state.take_guild_removals().is_empty());

        let removal = GuildRemoval {
            guild_id: "111".to_string(),
            guild_name: Some("Les Sardines".to_string()),
            reason: GuildRemovalReason::GuildDeleted,
        };
        state.push_guild_removal(removal.clone());
        let second = GuildRemoval {
            guild_id: "222".to_string(),
            guild_name: None,
            reason: GuildRemovalReason::InvalidToken,
        };
        state.push_guild_removal(second.clone());

        assert_eq!(state.take_guild_removals(), vec![removal, second]);
        assert!(state.take_guild_removals().is_empty());
    }

    #[test]
    fn guild_removal_serializes_camel_case() {
        let removal = GuildRemoval {
            guild_id: "111".to_string(),
            guild_name: Some("Les Sardines".to_string()),
            reason: GuildRemovalReason::MemberRemoved,
        };
        assert_eq!(
            serde_json::to_value(&removal).expect("json"),
            serde_json::json!({
                "guildId": "111",
                "guildName": "Les Sardines",
                "reason": "member_removed",
            })
        );
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
    fn screen_arbitration_is_routed_through_the_state() {
        let (_dir, state) = paired_state(&["111", "222"]);
        assert!(state.playback_job().is_none());

        let displayed =
            state.present_play("111", "job-a", serde_json::json!({"jobId": "job-a"}), true);
        assert!(matches!(displayed, PlayPresentation::Display(_)));
        assert_eq!(state.playback_guild().as_deref(), Some("111"));
        assert_eq!(state.playback_job().as_deref(), Some("job-a"));

        let queued =
            state.present_play("222", "job-b", serde_json::json!({"jobId": "job-b"}), true);
        assert_eq!(queued, PlayPresentation::Queued);
        assert_eq!(state.playback_guild().as_deref(), Some("111"));

        // Un rapport de fin libère l'écran et affiche le play suivant.
        let decision = state.report_stop(Some("job-a"), true);
        assert_eq!(
            decision.ended_for.as_ref().map(|job| job.guild_id.as_str()),
            Some("111")
        );
        assert!(decision.display_cleared);
        assert_eq!(
            decision.next.as_ref().map(|play| play.job_id.as_str()),
            Some("job-b")
        );
        assert_eq!(state.playback_job().as_deref(), Some("job-b"));
        // Le job terminé reste connu pour router une trame tardive.
        assert_eq!(state.job_owner("job-a").as_deref(), Some("111"));
    }

    #[test]
    fn screen_owner_changes_are_flagged_for_the_board() {
        let (_dir, state) = paired_state(&["111", "222"]);
        // Rien à republier tant que l'écran n'a pas changé de main.
        assert!(!state.take_screen_owner_change());

        // Le board n'apprend `playing` (pastille « à l'antenne » du rail) que
        // par `overlay://guilds` : afficher un média doit le faire republier,
        // sinon la pastille reste éteinte jusqu'au prochain changement de
        // guilde.
        let displayed =
            state.present_play("111", "job-a", serde_json::json!({"jobId": "job-a"}), true);
        assert!(matches!(displayed, PlayPresentation::Display(_)));
        assert!(state.take_screen_owner_change());
        // Le drapeau se consomme : aucune republication en boucle.
        assert!(!state.take_screen_owner_change());

        // Un `stop` d'une guilde qui n'occupe pas l'écran non plus.
        assert!(!state.server_stop("222", None, true).display_cleared);
        assert!(!state.take_screen_owner_change());

        // Un `play` mis en file ne change rien à l'antenne.
        assert_eq!(
            state.present_play("222", "job-b", serde_json::json!({"jobId": "job-b"}), true),
            PlayPresentation::Queued
        );
        assert!(!state.take_screen_owner_change());

        // La fin de lecture libère l'écran et affiche le média suivant :
        // l'antenne change de guilde, le board doit le savoir.
        let decision = state.report_stop(Some("job-a"), true);
        assert_eq!(
            decision.next.as_ref().map(|next| next.job_id.as_str()),
            Some("job-b")
        );
        assert!(state.take_screen_owner_change());

        // Un rapport tardif pour un job déjà terminé ne touche pas l'écran.
        assert!(!state.report_stop(Some("job-a"), true).display_cleared);
        assert!(!state.take_screen_owner_change());

        // Fin du média affiché sans suivant : l'écran redevient libre, la
        // pastille doit s'éteindre.
        let cleared = state.report_stop(Some("job-b"), true);
        assert!(cleared.display_cleared);
        assert!(cleared.next.is_none());
        assert!(state.take_screen_owner_change());
    }

    #[test]
    fn guilds_payload_marks_the_guild_on_air() {
        let (_dir, state) = paired_state(&["111", "222"]);
        let playing = |state: &AppState, guild_id: &str| {
            let payload = state.guilds_payload();
            let guilds = payload["guilds"].as_array().cloned().unwrap_or_default();
            guilds
                .into_iter()
                .find(|guild| guild["membership"]["guildId"] == serde_json::json!(guild_id))
                .map(|guild| guild["playing"].clone())
                .unwrap_or(serde_json::Value::Null)
        };
        assert_eq!(playing(&state, "111"), serde_json::json!(false));
        state.present_play("111", "job-a", serde_json::json!({"jobId": "job-a"}), true);
        assert_eq!(playing(&state, "111"), serde_json::json!(true));
        // Une seule guilde est à l'antenne, toutes guildes confondues.
        assert_eq!(playing(&state, "222"), serde_json::json!(false));
        state.report_stop(Some("job-a"), true);
        assert_eq!(playing(&state, "111"), serde_json::json!(false));
    }

    #[test]
    fn queued_plays_report_durations_per_guild() {
        let (_dir, state) = paired_state(&["111", "222"]);
        state.present_play(
            "111",
            "job-a",
            serde_json::json!({ "durationSec": 9 }),
            true,
        );
        let queued = |state: &AppState, job_id: &str, payload: serde_json::Value| {
            assert_eq!(
                state.present_play("222", job_id, payload, true),
                PlayPresentation::Queued
            );
        };
        queued(&state, "job-b", serde_json::json!({ "durationSec": 7 }));
        queued(&state, "job-c", serde_json::json!({}));
        // Durée invalide (chaîne) : repli sur 5 s ; durée démesurée : plafonnée.
        queued(
            &state,
            "job-e",
            serde_json::json!({ "durationSec": "sept" }),
        );
        queued(
            &state,
            "job-f",
            serde_json::json!({ "durationSec": 1_000_000_000 }),
        );

        assert_eq!(
            state.queued_jobs_for("222"),
            vec![
                ("job-b".to_string(), 7_000),
                ("job-c".to_string(), 5_000),
                ("job-e".to_string(), 5_000),
                ("job-f".to_string(), lc_protocol::MAX_PLAYBACK_REMAINING_MS),
            ]
        );
        // La guilde à l'écran n'a rien en file ; une guilde inconnue non plus.
        assert!(state.queued_jobs_for("111").is_empty());
        assert!(state.queued_jobs_for("333").is_empty());
    }
}
