//! Événements `overlay://play`, `overlay://stop`, `overlay://settings` et
//! bufferisation tout-ou-rien tant que le renderer overlay n'est pas prêt
//! (docs/rust/overlay-bridge.md §5.3 et §8.1).
//!
//! La fenêtre overlay peut être masquée : on garde au plus le dernier `play`
//! et le dernier `stop`, puis on rejoue dans l'ordre reçu dès que le renderer
//! signale qu'il est prêt. Passé [`READY_TIMEOUT`], les événements en attente
//! sont perdus (même comportement que le legacy Electron).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Wry};

use crate::config::OverlayConfigView;
use crate::state::AppState;
use crate::windows::{BOARD_LABEL, OVERLAY_LABEL};

pub const EVENT_PLAY: &str = "overlay://play";
pub const EVENT_STOP: &str = "overlay://stop";
pub const EVENT_SETTINGS: &str = "overlay://settings";

/// Délai maximal de rétention des événements avant que le renderer ne soit prêt.
pub const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Nombre maximal d'événements bufferisés (deux types : play, stop).
pub const MAX_PENDING: usize = 2;

/// Événement en attente de livraison à la fenêtre overlay.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OverlayEvent {
    Play { payload: Value },
    Stop { payload: Value },
}

impl OverlayEvent {
    pub fn play(payload: Value) -> Self {
        Self::Play { payload }
    }

    pub fn stop(payload: Value) -> Self {
        Self::Stop { payload }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Play { .. } => EVENT_PLAY,
            Self::Stop { .. } => EVENT_STOP,
        }
    }

    pub fn payload(&self) -> &Value {
        match self {
            Self::Play { payload } | Self::Stop { payload } => payload,
        }
    }

    fn is_same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Play { .. }, Self::Play { .. }) | (Self::Stop { .. }, Self::Stop { .. })
        )
    }
}

/// Buffer borné conservant le dernier `play` et le dernier `stop`.
#[derive(Debug, Default)]
pub struct EventBuffer {
    events: Mutex<Vec<OverlayEvent>>,
    timeout_armed: AtomicBool,
}

impl EventBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<OverlayEvent>> {
        self.events
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Ajoute un événement en conservant au plus le dernier de chaque type.
    pub fn enqueue(&self, event: OverlayEvent) {
        let mut events = self.lock();
        events.retain(|pending| !pending.is_same_kind(&event));
        events.push(event);
        while events.len() > MAX_PENDING {
            events.remove(0);
        }
    }

    /// Vide le buffer et retourne les événements dans leur ordre d'arrivée.
    pub fn drain(&self) -> Vec<OverlayEvent> {
        std::mem::take(&mut *self.lock())
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Arme le timeout de purge ; retourne `true` si c'est cet appel qui l'arme.
    pub fn arm_timeout(&self) -> bool {
        !self.timeout_armed.swap(true, Ordering::SeqCst)
    }

    pub fn disarm_timeout(&self) {
        self.timeout_armed.store(false, Ordering::SeqCst);
    }

    pub fn timeout_armed(&self) -> bool {
        self.timeout_armed.load(Ordering::SeqCst)
    }
}

/// Émet un événement vers la seule fenêtre overlay.
pub fn emit_to_overlay(app: &AppHandle<Wry>, event: &OverlayEvent) -> tauri::Result<()> {
    app.emit_to(
        EventTarget::labeled(OVERLAY_LABEL),
        event.name(),
        event.payload().clone(),
    )
}

/// Émet `overlay://settings` vers overlay et board (jamais pairing).
pub fn emit_settings(app: &AppHandle<Wry>, view: &OverlayConfigView) -> tauri::Result<()> {
    app.emit_to(
        EventTarget::labeled(OVERLAY_LABEL),
        EVENT_SETTINGS,
        view.clone(),
    )?;
    app.emit_to(
        EventTarget::labeled(BOARD_LABEL),
        EVENT_SETTINGS,
        view.clone(),
    )
}

/// Livre immédiatement si le renderer est prêt, sinon bufferise et arme le
/// timeout de purge. Ne panique jamais (les erreurs d'émission sont loguées).
pub fn deliver_or_buffer(app: &AppHandle<Wry>, overlay_ready: bool, event: OverlayEvent) {
    if overlay_ready {
        if let Err(error) = emit_to_overlay(app, &event) {
            tracing::warn!(event = event.name(), %error, "échec d'émission de l'événement overlay");
        }
        return;
    }

    let state = app.state::<AppState>();
    let buffer = state.pending_events();
    buffer.enqueue(event);
    if buffer.arm_timeout() {
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(READY_TIMEOUT);
            let state = handle.state::<AppState>();
            let buffer = state.pending_events();
            if !state.overlay_ready() {
                let dropped = buffer.drain();
                if !dropped.is_empty() {
                    tracing::warn!(
                        count = dropped.len(),
                        "renderer overlay non prêt après 5 s, événements abandonnés"
                    );
                }
            }
            buffer.disarm_timeout();
        });
    }
}

/// Rejoue les événements en attente, puis libère le timeout.
pub fn flush_pending(app: &AppHandle<Wry>) {
    let state = app.state::<AppState>();
    let buffer = state.pending_events();
    let pending = buffer.drain();
    buffer.disarm_timeout();
    for event in pending {
        if let Err(error) = emit_to_overlay(app, &event) {
            tracing::warn!(event = event.name(), %error, "échec de rejeu d'un événement overlay");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn buffer_keeps_last_event_of_each_kind_in_arrival_order() {
        let buffer = EventBuffer::new();
        buffer.enqueue(OverlayEvent::play(json!({"jobId": "a"})));
        buffer.enqueue(OverlayEvent::stop(json!({"jobId": "a"})));
        buffer.enqueue(OverlayEvent::play(json!({"jobId": "b"})));
        let pending = buffer.drain();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].name(), EVENT_STOP);
        assert_eq!(pending[1].name(), EVENT_PLAY);
        assert_eq!(pending[1].payload(), &json!({"jobId": "b"}));
        assert!(buffer.is_empty());
    }

    #[test]
    fn buffer_is_bounded_by_max_pending() {
        let buffer = EventBuffer::new();
        buffer.enqueue(OverlayEvent::stop(json!({})));
        buffer.enqueue(OverlayEvent::play(json!({})));
        buffer.enqueue(OverlayEvent::stop(json!({"jobId": "c"})));
        assert!(buffer.len() <= MAX_PENDING);
        let pending = buffer.drain();
        assert_eq!(pending.last().map(OverlayEvent::name), Some(EVENT_STOP));
    }

    #[test]
    fn timeout_can_only_be_armed_once_until_disarmed() {
        let buffer = EventBuffer::new();
        assert!(buffer.arm_timeout());
        assert!(!buffer.arm_timeout());
        buffer.disarm_timeout();
        assert!(buffer.arm_timeout());
    }

    #[test]
    fn event_names_match_bridge_contract() {
        assert_eq!(OverlayEvent::play(json!({})).name(), "overlay://play");
        assert_eq!(OverlayEvent::stop(json!({})).name(), "overlay://stop");
        assert_eq!(EVENT_SETTINGS, "overlay://settings");
    }
}
