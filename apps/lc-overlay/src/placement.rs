//! On-screen placement of the display zone.
//!
//! `settings_placement_start` (board) turns the overlay into an ordinary,
//! clickable and focusable window on its screen. The renderer draws the zone
//! with a handle at each corner and edge over the template card, and the user
//! moves and resizes it like a window. `overlay_placement_finish` (overlay)
//! saves the zone or cancels; `settings_placement_cancel` (board) cancels too,
//! in case the overlay screen cannot be reached. The overlay then goes back to
//! a click-through layer and the board comes back to the front.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Wry};

use crate::config::{LayoutZone, OverlayLayout};
use crate::events::{self, OverlayEvent};
use crate::layout_preview;
use crate::settings;
use crate::state::AppState;
use crate::windows::{self, OVERLAY_LABEL};

/// Placement start / end, sent to the overlay renderer only.
pub const EVENT_PLACEMENT: &str = "overlay://placement";

static PLACING: AtomicBool = AtomicBool::new(false);
static TEMPLATE_SHOWN: AtomicBool = AtomicBool::new(false);

/// `overlay://placement` payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementPayload {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<OverlayLayout>,
}

/// Why the placement could not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    /// The overlay page has not reported ready: nothing would draw the zone,
    /// and a clickable transparent window would block the screen.
    OverlayNotReady,
}

impl StartError {
    pub fn reason(self) -> &'static str {
        match self {
            Self::OverlayNotReady => "overlay_not_ready",
        }
    }
}

/// The zone is being placed: the overlay window must stay clickable.
pub fn is_active() -> bool {
    PLACING.load(Ordering::SeqCst)
}

/// Parses the `overlay_placement_finish` payload: `{ "zone": {...} }` saves
/// that zone, `{ "zone": null }` cancels.
pub fn parse_finish(payload: &Value) -> Result<Option<LayoutZone>, String> {
    let Some(object) = payload.as_object() else {
        return Err("objet JSON attendu".to_string());
    };
    match object.get("zone") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(zone)) => {
            let mut parsed = LayoutZone::default();
            for (key, slot) in [
                ("x", &mut parsed.x),
                ("y", &mut parsed.y),
                ("width", &mut parsed.width),
                ("height", &mut parsed.height),
            ] {
                match zone.get(key).and_then(Value::as_f64) {
                    Some(value) if value.is_finite() => *slot = value,
                    _ => return Err(format!("zone.{key} : nombre attendu")),
                }
            }
            parsed.normalize();
            Ok(Some(parsed))
        }
        Some(_) => Err("zone : objet JSON ou null attendu".to_string()),
    }
}

/// Starts the placement; a second call only brings the overlay back to the
/// front.
pub fn start(app: &AppHandle<Wry>) -> Result<(), StartError> {
    let state = app.state::<AppState>();
    if !state.overlay_ready() {
        return Err(StartError::OverlayNotReady);
    }
    if PLACING.swap(true, Ordering::SeqCst) {
        windows::enter_placement(app);
        return Ok(());
    }
    // The placement shows its own template for as long as it lasts.
    layout_preview::cancel();
    windows::enter_placement(app);

    let config = state.config_snapshot();
    if state.playback_job().is_none() {
        let payload = settings::layout_preview_payload(&config);
        match events::emit_to_overlay(app, &OverlayEvent::play(payload)) {
            Ok(()) => TEMPLATE_SHOWN.store(true, Ordering::SeqCst),
            Err(error) => tracing::warn!(%error, "échec d'affichage du gabarit de placement"),
        }
    }
    emit_placement(
        app,
        &PlacementPayload {
            active: true,
            layout: Some(config.layout),
        },
    );
    emit_board_settings(app);
    Ok(())
}

/// Ends the placement, saving `zone` when given. Does nothing when no
/// placement is running.
pub fn finish(app: &AppHandle<Wry>, zone: Option<LayoutZone>) -> Result<(), String> {
    if !PLACING.swap(false, Ordering::SeqCst) {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let saved = match zone {
        Some(zone) => state
            .update_config(|config| config.layout.zone = zone)
            .map(|_config| ()),
        None => Ok(()),
    };

    emit_placement(
        app,
        &PlacementPayload {
            active: false,
            layout: None,
        },
    );
    // Back to the saved layout (the new zone, or the previous one).
    if let Err(error) = events::emit_settings(app, &state.view()) {
        tracing::debug!(%error, "échec d'émission de la mise en page après placement");
    }
    if TEMPLATE_SHOWN.swap(false, Ordering::SeqCst) && state.playback_job().is_none() {
        let stop = OverlayEvent::stop(serde_json::json!({ "reason": "placement_end" }));
        if let Err(error) = events::emit_to_overlay(app, &stop) {
            tracing::debug!(%error, "échec du retrait du gabarit de placement");
        }
    }
    windows::exit_placement(app);
    emit_board_settings(app);
    windows::show_board(app);
    saved
}

fn emit_placement(app: &AppHandle<Wry>, payload: &PlacementPayload) {
    if let Err(error) = app.emit_to(
        EventTarget::labeled(OVERLAY_LABEL),
        EVENT_PLACEMENT,
        payload,
    ) {
        tracing::warn!(%error, "échec d'émission de overlay://placement");
    }
}

fn emit_board_settings(app: &AppHandle<Wry>) {
    if let Err(error) = events::emit_device_settings(app) {
        tracing::debug!(%error, "échec d'émission overlay://device-settings (placement)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_finish_with_a_zone_saves_it_normalized() {
        let zone = parse_finish(&json!({
            "zone": { "x": 0.9, "y": 0.1, "width": 0.3, "height": 0.2 }
        }))
        .expect("valid zone")
        .expect("zone to save");
        assert_eq!(
            zone,
            LayoutZone {
                x: 0.7,
                y: 0.1,
                width: 0.3,
                height: 0.2,
            }
        );
    }

    #[test]
    fn a_finish_without_zone_cancels() {
        assert_eq!(parse_finish(&json!({ "zone": null })), Ok(None));
        assert_eq!(parse_finish(&json!({})), Ok(None));
    }

    #[test]
    fn a_finish_needs_every_zone_number() {
        let error = parse_finish(&json!({ "zone": { "x": 0.1, "y": 0.1, "width": 0.5 } }))
            .expect_err("height missing");
        assert!(error.contains("zone.height"), "{error}");
        assert!(parse_finish(&json!({ "zone": "full" })).is_err());
        assert!(parse_finish(&json!([])).is_err());
    }

    #[test]
    fn the_placement_payload_matches_the_renderer_contract() {
        let start = serde_json::to_value(PlacementPayload {
            active: true,
            layout: Some(OverlayLayout::default()),
        })
        .expect("serialize");
        assert_eq!(
            start,
            json!({
                "active": true,
                "layout": {
                    "zone": { "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 },
                    "align": "center"
                }
            })
        );
        let end = serde_json::to_value(PlacementPayload {
            active: false,
            layout: None,
        })
        .expect("serialize");
        assert_eq!(end, json!({ "active": false }));
    }
}
