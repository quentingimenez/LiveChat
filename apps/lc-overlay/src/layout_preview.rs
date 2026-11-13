//! Live layout preview.
//!
//! While the user edits the display settings, the board sends each draft
//! layout through `settings_preview_layout`. The overlay applies it at once
//! and, when nothing is playing, shows a template card (the test media with a
//! frozen countdown). [`LAYOUT_PREVIEW_HOLD`] after the last change, the
//! template goes away and the overlay is back on the saved layout.
//!
//! The draft is never saved here: the board saves the value itself once the
//! control is released (`settings_set`).

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Wry};

use crate::config::OverlayLayout;
use crate::events::{self, OverlayEvent};
use crate::settings;
use crate::state::AppState;
use crate::windows;

/// How long the template stays after the last layout change.
pub const LAYOUT_PREVIEW_HOLD: Duration = Duration::from_secs(4);

/// Hold state of the preview, kept apart from Tauri so it can be tested.
#[derive(Debug, Default)]
pub struct PreviewHold {
    deadline: Option<Instant>,
    waiter: bool,
    template_shown: bool,
}

/// What [`PreviewHold::extend`] asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extended {
    /// First change of a preview: the template may have to be shown.
    pub started: bool,
    /// No waiter is running: one must be spawned to end the preview.
    pub spawn_waiter: bool,
}

/// Outcome of a waiter's check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldPoll {
    /// A later change pushed the end back: sleep this long, then check again.
    Wait(Duration),
    /// The preview is over and must be ended.
    Expired { template_shown: bool },
    /// Another card replaced the template: nothing to end.
    Cancelled,
}

impl PreviewHold {
    pub const fn new() -> Self {
        Self {
            deadline: None,
            waiter: false,
            template_shown: false,
        }
    }

    /// Pushes the end of the preview to `now + hold`.
    pub fn extend(&mut self, now: Instant, hold: Duration) -> Extended {
        let started = self.deadline.is_none();
        self.deadline = Some(now + hold);
        let spawn_waiter = !self.waiter;
        self.waiter = true;
        Extended {
            started,
            spawn_waiter,
        }
    }

    pub fn mark_template_shown(&mut self) {
        self.template_shown = true;
    }

    /// Drops the preview without ending it: the template was replaced.
    pub fn cancel(&mut self) {
        self.deadline = None;
        self.template_shown = false;
    }

    pub fn is_active(&self) -> bool {
        self.deadline.is_some()
    }

    /// Checked by the waiter once its sleep is over.
    pub fn poll(&mut self, now: Instant) -> HoldPoll {
        match self.deadline {
            None => {
                self.waiter = false;
                HoldPoll::Cancelled
            }
            Some(deadline) if deadline > now => HoldPoll::Wait(deadline - now),
            Some(_) => {
                self.deadline = None;
                self.waiter = false;
                HoldPoll::Expired {
                    template_shown: std::mem::take(&mut self.template_shown),
                }
            }
        }
    }
}

static HOLD: Mutex<PreviewHold> = Mutex::new(PreviewHold::new());

fn hold() -> MutexGuard<'static, PreviewHold> {
    HOLD.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Shows `layout` on the overlay right away, with the template card when
/// nothing plays, and pushes the end of the preview back.
///
/// Runs on the main thread (the board command is synchronous), like the
/// waiter's check, so a change made right at the deadline is never lost.
pub fn show(app: &AppHandle<Wry>, layout: OverlayLayout) {
    let state = app.state::<AppState>();
    let mut view = state.view();
    view.layout = layout;
    if let Err(error) = events::emit_settings(app, &view) {
        tracing::debug!(%error, "échec d'émission de la mise en page d'aperçu");
    }

    let extended = hold().extend(Instant::now(), LAYOUT_PREVIEW_HOLD);
    if extended.started && state.playback_job().is_none() {
        windows::show_overlay_preview(app);
        let payload = settings::layout_preview_payload(&state.config_snapshot());
        match events::emit_to_overlay(app, &OverlayEvent::play(payload)) {
            Ok(()) => hold().mark_template_shown(),
            Err(error) => tracing::warn!(%error, "échec d'affichage du gabarit d'aperçu"),
        }
    }
    if extended.spawn_waiter {
        spawn_waiter(app.clone());
    }
}

/// Another local card replaced the template (`settings_test_media`).
pub fn cancel() {
    hold().cancel();
}

/// A preview is running: the overlay window must stay visible.
pub fn is_active() -> bool {
    hold().is_active()
}

fn spawn_waiter(app: AppHandle<Wry>) {
    std::thread::spawn(move || {
        let mut wait = LAYOUT_PREVIEW_HOLD;
        loop {
            std::thread::sleep(wait);
            let (sender, receiver) = std::sync::mpsc::channel();
            let handle = app.clone();
            let scheduled = app.run_on_main_thread(move || {
                let poll = hold().poll(Instant::now());
                if let HoldPoll::Expired { template_shown } = poll {
                    end(&handle, template_shown);
                }
                let _sent = sender.send(poll);
            });
            if let Err(error) = scheduled {
                tracing::debug!(%error, "fin de l'aperçu de mise en page impossible");
                return;
            }
            match receiver.recv() {
                Ok(HoldPoll::Wait(remaining)) => wait = remaining,
                _ => return,
            }
        }
    });
}

/// Back to the saved layout; the template goes away unless a media replaced it.
fn end(app: &AppHandle<Wry>, template_shown: bool) {
    let state = app.state::<AppState>();
    if let Err(error) = events::emit_settings(app, &state.view()) {
        tracing::debug!(%error, "échec du retour à la mise en page enregistrée");
    }
    if !template_shown || state.playback_job().is_some() {
        return;
    }
    let stop = OverlayEvent::stop(serde_json::json!({ "reason": "layout_preview_end" }));
    if let Err(error) = events::emit_to_overlay(app, &stop) {
        tracing::debug!(%error, "échec du retrait du gabarit d'aperçu");
    }
    if !windows::overlay_should_show(&state.config_snapshot()) {
        windows::hide_overlay(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD_FOR_TESTS: Duration = Duration::from_secs(4);

    #[test]
    fn the_first_change_starts_the_preview_and_its_waiter() {
        let mut hold = PreviewHold::new();
        let now = Instant::now();
        assert_eq!(
            hold.extend(now, HOLD_FOR_TESTS),
            Extended {
                started: true,
                spawn_waiter: true
            }
        );
        assert!(hold.is_active());
        assert_eq!(
            hold.extend(now + Duration::from_secs(1), HOLD_FOR_TESTS),
            Extended {
                started: false,
                spawn_waiter: false
            }
        );
    }

    #[test]
    fn a_later_change_pushes_the_end_back() {
        let mut hold = PreviewHold::new();
        let start = Instant::now();
        hold.extend(start, HOLD_FOR_TESTS);
        hold.extend(start + Duration::from_secs(3), HOLD_FOR_TESTS);
        assert_eq!(
            hold.poll(start + Duration::from_secs(4)),
            HoldPoll::Wait(Duration::from_secs(3))
        );
        assert_eq!(
            hold.poll(start + Duration::from_secs(7)),
            HoldPoll::Expired {
                template_shown: false
            }
        );
        assert!(!hold.is_active());
    }

    #[test]
    fn the_expiry_reports_the_template_once() {
        let mut hold = PreviewHold::new();
        let start = Instant::now();
        hold.extend(start, HOLD_FOR_TESTS);
        hold.mark_template_shown();
        assert_eq!(
            hold.poll(start + HOLD_FOR_TESTS),
            HoldPoll::Expired {
                template_shown: true
            }
        );
        let next = hold.extend(start + Duration::from_secs(10), HOLD_FOR_TESTS);
        assert!(next.started && next.spawn_waiter);
        assert_eq!(
            hold.poll(start + Duration::from_secs(14)),
            HoldPoll::Expired {
                template_shown: false
            }
        );
    }

    #[test]
    fn a_cancelled_preview_ends_its_waiter_without_ending_anything() {
        let mut hold = PreviewHold::new();
        let start = Instant::now();
        hold.extend(start, HOLD_FOR_TESTS);
        hold.mark_template_shown();
        hold.cancel();
        assert!(!hold.is_active());
        assert_eq!(hold.poll(start + HOLD_FOR_TESTS), HoldPoll::Cancelled);
        let next = hold.extend(start + Duration::from_secs(5), HOLD_FOR_TESTS);
        assert!(next.started && next.spawn_waiter);
    }

    #[test]
    fn a_change_after_a_cancel_keeps_the_running_waiter() {
        let mut hold = PreviewHold::new();
        let start = Instant::now();
        hold.extend(start, HOLD_FOR_TESTS);
        hold.cancel();
        let next = hold.extend(start + Duration::from_secs(1), HOLD_FOR_TESTS);
        assert_eq!(
            next,
            Extended {
                started: true,
                spawn_waiter: false
            }
        );
        assert_eq!(
            hold.poll(start + HOLD_FOR_TESTS),
            HoldPoll::Wait(Duration::from_secs(1))
        );
    }
}
