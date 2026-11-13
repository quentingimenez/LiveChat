//! Board settings contract: the `SettingsView` projection and the pure
//! validation of `settings_set` payloads.
//!
//! Applying side effects (persistence, display placement, shortcut
//! registration, autostart, overlay event) belongs to `commands.rs`.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager, Wry};

use crate::config::{LayoutAnchor, LayoutZone, OverlayConfig, OverlayLayout, ShortcutSettings};
use crate::display::DisplayInfo;
use crate::state::AppState;
use crate::updater;

/// One display as exposed by `settings_get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDisplay {
    pub index: usize,
    pub key: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// Local default profile offered to new guilds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsProfile {
    pub display_name: Option<String>,
    pub avatar: Option<String>,
}

/// Full board settings payload (`settings_get`, `settings_set` and
/// `overlay://device-settings`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub version: String,
    pub updater_enabled: bool,
    pub displays: Vec<SettingsDisplay>,
    pub selected_display_key: Option<String>,
    pub volume: f64,
    pub guild_volumes: BTreeMap<String, f64>,
    pub general_mute: bool,
    pub show_text: bool,
    pub auto_start: bool,
    pub shortcuts: ShortcutSettings,
    pub layout: OverlayLayout,
    /// The display zone is being placed on the overlay screen.
    pub placing: bool,
    pub profile: SettingsProfile,
}

/// `settings_set` answer. On failure nothing has been applied.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSetResponse {
    pub ok: bool,
    pub settings: SettingsView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl SettingsSetResponse {
    pub fn success(settings: SettingsView) -> Self {
        Self {
            ok: true,
            settings,
            field: None,
            message: None,
        }
    }

    pub fn failure(
        field: impl Into<String>,
        message: impl Into<String>,
        settings: SettingsView,
    ) -> Self {
        Self {
            ok: false,
            settings,
            field: Some(field.into()),
            message: Some(message.into()),
        }
    }
}

/// Answer of `settings_identify_displays`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsIdentifyDisplaysResponse {
    pub count: usize,
}

/// Answer of `settings_test_media` and `settings_placement_start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsActionResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A validation failure, mapped to the board answer shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError {
    pub field: String,
    pub message: String,
}

impl SettingsError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

fn display_views(displays: &[DisplayInfo], primary_index: Option<usize>) -> Vec<SettingsDisplay> {
    displays
        .iter()
        .enumerate()
        .map(|(index, display)| SettingsDisplay {
            index,
            key: display.key(),
            label: crate::display::format_settings_label(display, primary_index == Some(index)),
            width: display.width,
            height: display.height,
            primary: primary_index == Some(index),
        })
        .collect()
}

/// Builds the settings projection without touching Tauri, for unit tests.
pub fn build_settings_view(
    config: &OverlayConfig,
    displays: &[DisplayInfo],
    primary_index: Option<usize>,
    auto_start: bool,
) -> SettingsView {
    SettingsView {
        version: env!("CARGO_PKG_VERSION").to_string(),
        updater_enabled: updater::UPDATER_ENABLED_FOR_RELEASE,
        displays: display_views(displays, primary_index),
        selected_display_key: config.display_key.clone(),
        volume: config.volume,
        guild_volumes: config
            .memberships
            .iter()
            .map(|membership| (membership.guild_id.clone(), membership.volume))
            .collect(),
        general_mute: !config.enabled,
        show_text: config.show_text,
        auto_start,
        shortcuts: config.shortcuts.clone(),
        layout: config.layout.clone(),
        placing: false,
        profile: SettingsProfile {
            display_name: config.profile_name.clone(),
            avatar: config.profile_avatar.clone(),
        },
    }
}

/// Current `SettingsView` for the board, including live display enumeration.
pub fn settings_view(app: &AppHandle<Wry>) -> SettingsView {
    let config = app.state::<AppState>().config_snapshot();
    let (displays, primary_index) = crate::windows::collect_displays(app);
    let auto_start = crate::device_settings::autostart_enabled(app);
    let mut view = build_settings_view(&config, &displays, primary_index, auto_start);
    view.placing = crate::placement::is_active();
    view
}

/// Local `overlay://play` payload used by `settings_test_media`.
///
/// It deliberately has no `jobId`: the renderer opens no playback session and
/// therefore never reports this test to a server.
pub fn test_media_payload(config: &OverlayConfig) -> Value {
    local_card_payload(config, 5, "Média de test")
}

/// Countdown shown, frozen, on the layout preview template.
pub const LAYOUT_PREVIEW_COUNTDOWN_SEC: u64 = 10;

/// Local `overlay://play` payload of the layout preview template.
///
/// Same card as the test media, with `hold`: the renderer shows the countdown
/// frozen and keeps the card until the shell sends `overlay://stop`.
pub fn layout_preview_payload(config: &OverlayConfig) -> Value {
    let mut payload = local_card_payload(
        config,
        LAYOUT_PREVIEW_COUNTDOWN_SEC,
        "Aperçu de la mise en page",
    );
    payload["hold"] = Value::Bool(true);
    payload
}

fn local_card_payload(config: &OverlayConfig, duration_sec: u64, text: &str) -> Value {
    let author_name = config
        .profile_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("LiveChat");
    serde_json::json!({
        "durationSec": duration_sec,
        "test": true,
        "media": {
            "kind": "image",
            "url": "assets/test-media.svg",
        },
        "author": {
            "enabled": true,
            "name": author_name,
            "image": config.profile_avatar,
        },
        "text": {
            "enabled": true,
            "value": text,
        },
    })
}

/// Validates a `settings_preview_layout` payload (`{ "layout": {...} }`) and
/// returns the layout it describes on top of the saved one, without saving it.
pub fn preview_layout(
    config: &OverlayConfig,
    payload: &Value,
) -> Result<OverlayLayout, SettingsError> {
    let object = request_object(payload)?;
    if !object.contains_key("layout") {
        return Err(SettingsError::new("layout", "objet JSON attendu"));
    }
    let mut candidate = config.clone();
    apply_layout_patch(&mut candidate, object)?;
    Ok(candidate.layout)
}

fn request_object(payload: &Value) -> Result<&Map<String, Value>, SettingsError> {
    payload
        .as_object()
        .ok_or_else(|| SettingsError::new("payload", "objet JSON attendu"))
}

fn bool_value(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, SettingsError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(SettingsError::new(key, "valeur booléenne attendue")),
    }
}

fn text_or_null(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<Option<Option<String>>, SettingsError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(value)) => Ok(Some(Some(value.clone()))),
        Some(_) => Err(SettingsError::new(field, "chaîne ou null attendu")),
    }
}

fn number_value(
    object: &Map<String, Value>,
    key: &str,
    field: &str,
) -> Result<Option<f64>, SettingsError> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| SettingsError::new(field, "nombre attendu")),
    }
}

fn apply_volume_patch(
    config: &mut OverlayConfig,
    object: &Map<String, Value>,
) -> Result<(), SettingsError> {
    if let Some(volume) = number_value(object, "volume", "volume")? {
        config.volume = crate::config::normalize_volume(volume);
    }

    let Some(raw_guild_volumes) = object.get("guildVolumes") else {
        return Ok(());
    };
    let Some(guild_volumes) = raw_guild_volumes.as_object() else {
        return Err(SettingsError::new("guildVolumes", "objet JSON attendu"));
    };
    for (guild_id, raw_volume) in guild_volumes {
        let Some(volume) = raw_volume.as_f64().filter(|number| number.is_finite()) else {
            return Err(SettingsError::new(
                format!("guildVolumes.{guild_id}"),
                "volume numérique attendu",
            ));
        };
        if let Some(membership) = config
            .memberships
            .iter_mut()
            .find(|membership| membership.guild_id == *guild_id)
        {
            membership.volume = crate::config::normalize_volume(volume);
        }
    }
    Ok(())
}

fn apply_shortcuts_patch(
    config: &mut OverlayConfig,
    object: &Map<String, Value>,
) -> Result<(), SettingsError> {
    let Some(raw_shortcuts) = object.get("shortcuts") else {
        return Ok(());
    };
    match raw_shortcuts {
        Value::Null => config.shortcuts = ShortcutSettings::default(),
        Value::Object(shortcuts) => {
            if let Some(stop) = text_or_null(shortcuts, "stop", "shortcuts.stop")? {
                config.shortcuts.stop = stop;
            }
            if let Some(mute_all) = text_or_null(shortcuts, "muteAll", "shortcuts.muteAll")? {
                config.shortcuts.mute_all = mute_all;
            }
        }
        _ => {
            return Err(SettingsError::new(
                "shortcuts",
                "objet JSON ou null attendu",
            ))
        }
    }
    crate::shortcuts::validate_shortcut_settings(&config.shortcuts, &config.media_bindings)
        .map_err(|error| SettingsError::new(error.field, error.message))
}

fn apply_layout_patch(
    config: &mut OverlayConfig,
    object: &Map<String, Value>,
) -> Result<(), SettingsError> {
    let Some(raw_layout) = object.get("layout") else {
        return Ok(());
    };
    if raw_layout.is_null() {
        config.layout = OverlayLayout::default();
        return Ok(());
    }
    let Some(layout) = raw_layout.as_object() else {
        return Err(SettingsError::new("layout", "objet JSON attendu"));
    };
    if let Some(raw_align) = layout.get("align") {
        if raw_align.is_null() {
            config.layout.align = LayoutAnchor::Center;
        } else {
            match serde_json::from_value::<LayoutAnchor>(raw_align.clone()) {
                Ok(align) => config.layout.align = align,
                Err(_) => return Err(SettingsError::new("layout.align", "alignement inconnu")),
            }
        }
    }
    match layout.get("zone") {
        None => {}
        Some(Value::Null) => config.layout.zone = LayoutZone::default(),
        Some(Value::Object(zone)) => {
            let target = &mut config.layout.zone;
            for (key, slot) in [
                ("x", &mut target.x),
                ("y", &mut target.y),
                ("width", &mut target.width),
                ("height", &mut target.height),
            ] {
                if let Some(value) = number_value(zone, key, &format!("layout.zone.{key}"))? {
                    *slot = value;
                }
            }
        }
        Some(_) => {
            return Err(SettingsError::new(
                "layout.zone",
                "objet JSON ou null attendu",
            ))
        }
    }
    config.layout.normalize();
    Ok(())
}

fn apply_display_patch(
    config: &mut OverlayConfig,
    object: &Map<String, Value>,
    displays: &[DisplayInfo],
) -> Result<(), SettingsError> {
    let Some(raw_key) = object.get("selectedDisplayKey") else {
        return Ok(());
    };
    match raw_key {
        Value::Null => {
            config.display_id = None;
            config.display_index = None;
            config.display_key = None;
            config.display_label = None;
        }
        Value::String(key) => {
            let key = key.trim();
            let Some(display) = displays.iter().find(|display| display.key() == key) else {
                return Err(SettingsError::new("selectedDisplayKey", "écran inconnu"));
            };
            let selection = crate::display::DisplaySelection::from_display(display);
            config.display_index = selection.display_index;
            config.display_key = selection.display_key;
            config.display_label = selection.display_label;
        }
        _ => {
            return Err(SettingsError::new(
                "selectedDisplayKey",
                "chaîne ou null attendu",
            ))
        }
    }
    Ok(())
}

fn apply_profile_patch(
    config: &mut OverlayConfig,
    object: &Map<String, Value>,
) -> Result<(), SettingsError> {
    let Some(raw_profile) = object.get("profile") else {
        return Ok(());
    };
    if raw_profile.is_null() {
        config.profile_name = None;
        config.profile_avatar = None;
        return Ok(());
    }
    let Some(profile) = raw_profile.as_object() else {
        return Err(SettingsError::new("profile", "objet JSON attendu"));
    };
    if let Some(display_name) = text_or_null(profile, "displayName", "profile.displayName")? {
        config.profile_name = display_name;
    }
    if let Some(avatar) = text_or_null(profile, "avatar", "profile.avatar")? {
        config.profile_avatar = avatar;
    }
    Ok(())
}

/// Applies a partial `settings_set` payload to a copy of `base`.
///
/// Every field is validated before returning: a failure means the caller must
/// not apply anything. Volumes are clamped, the zone is kept on screen, unknown
/// alignments and invalid shortcut accelerators are rejected.
pub fn apply_settings_patch(
    base: &OverlayConfig,
    payload: &Value,
    displays: &[DisplayInfo],
) -> Result<OverlayConfig, SettingsError> {
    let object = request_object(payload)?;
    let mut config = base.clone();

    apply_volume_patch(&mut config, object)?;

    if let Some(enabled) = bool_value(object, "generalMute")? {
        config.enabled = !enabled;
    }
    if let Some(show_text) = bool_value(object, "showText")? {
        config.show_text = show_text;
    }
    if let Some(auto_start) = bool_value(object, "autoStart")? {
        config.auto_start = auto_start;
    }

    apply_shortcuts_patch(&mut config, object)?;
    apply_layout_patch(&mut config, object)?;
    apply_display_patch(&mut config, object, displays)?;
    apply_profile_patch(&mut config, object)?;

    config.normalize();
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Membership;
    use serde_json::json;

    pub(crate) fn sample_displays() -> Vec<DisplayInfo> {
        vec![
            DisplayInfo::new(0, Some("DISPLAY1".to_string()), 0, 0, 2560, 1440, 1.0),
            DisplayInfo::new(1, Some("DISPLAY2".to_string()), -1920, 0, 1920, 1080, 1.0),
        ]
    }

    fn paired_config() -> OverlayConfig {
        let mut config = OverlayConfig::default();
        config.join(Membership {
            server_url: "https://example.test".to_string(),
            guild_id: "111".to_string(),
            client_token: "token".to_string(),
            ..Membership::default()
        });
        config
    }

    #[test]
    fn settings_set_volume_is_clamped_and_guild_volumes_are_merged() {
        let config = paired_config();
        let patched = apply_settings_patch(
            &config,
            &json!({ "volume": 4.0, "guildVolumes": { "111": -2.0, "unknown": 0.5 } }),
            &sample_displays(),
        )
        .expect("valid payload");
        assert_eq!(patched.volume, 1.0);
        assert_eq!(
            patched
                .membership("111")
                .map(|membership| membership.volume),
            Some(0.0)
        );
        assert_eq!(patched.memberships.len(), 1);
    }

    #[test]
    fn settings_set_rejects_unknown_align_without_mutating_the_base() {
        let config = paired_config();
        let before = config.clone();
        let error = apply_settings_patch(
            &config,
            &json!({ "volume": 0.4, "layout": { "align": "middle" } }),
            &sample_displays(),
        )
        .expect_err("unknown align");
        assert_eq!(error.field, "layout.align");
        assert_eq!(config, before);

        let error = apply_settings_patch(
            &config,
            &json!({ "layout": { "zone": { "x": "left" } } }),
            &sample_displays(),
        )
        .expect_err("zone numbers only");
        assert_eq!(error.field, "layout.zone.x");
    }

    #[test]
    fn settings_set_zone_is_partial_and_kept_on_screen() {
        let mut config = paired_config();
        config.layout.zone = LayoutZone {
            x: 0.1,
            y: 0.1,
            width: 0.5,
            height: 0.5,
        };
        let patched = apply_settings_patch(
            &config,
            &json!({ "layout": { "align": "right", "zone": { "x": 0.8 } } }),
            &sample_displays(),
        )
        .expect("valid layout");
        assert_eq!(patched.layout.align, LayoutAnchor::Right);
        assert_eq!(
            patched.layout.zone,
            LayoutZone {
                x: 0.5,
                y: 0.1,
                width: 0.5,
                height: 0.5,
            }
        );

        let reset = apply_settings_patch(&config, &json!({ "layout": { "zone": null } }), &[])
            .expect("reset zone");
        assert_eq!(reset.layout.zone, LayoutZone::default());
    }

    #[test]
    fn settings_set_rejects_duplicate_global_shortcut() {
        let error = apply_settings_patch(
            &paired_config(),
            &json!({ "shortcuts": { "stop": "Ctrl+Shift+S", "muteAll": "Ctrl+Shift+S" } }),
            &sample_displays(),
        )
        .expect_err("duplicate shortcut");
        assert_eq!(error.field, "shortcuts.muteAll");
    }

    #[test]
    fn settings_set_rejects_a_shortcut_bound_to_a_media() {
        let mut config = paired_config();
        config.media_bindings.insert(
            "CommandOrControl+Shift+S".to_string(),
            "media-1".to_string(),
        );
        let error = apply_settings_patch(
            &config,
            &json!({ "shortcuts": { "stop": "Ctrl+Shift+S" } }),
            &sample_displays(),
        )
        .expect_err("media binding collision");
        assert_eq!(error.field, "shortcuts.stop");
    }

    #[test]
    fn settings_set_response_matches_the_board_shape() {
        let view = build_settings_view(&paired_config(), &sample_displays(), Some(0), false);
        let failure =
            serde_json::to_value(SettingsSetResponse::failure("volume", "test", view.clone()))
                .expect("failure json");
        assert_eq!(failure["ok"], json!(false));
        assert_eq!(failure["field"], json!("volume"));
        assert_eq!(failure["message"], json!("test"));
        assert_eq!(failure["settings"]["volume"], json!(1.0));

        let success =
            serde_json::to_value(SettingsSetResponse::success(view)).expect("success json");
        assert_eq!(success["ok"], json!(true));
        assert!(success.get("field").is_none());
        assert!(success.get("message").is_none());
    }

    #[test]
    fn shortcut_cleared_with_null() {
        let mut config = paired_config();
        config.shortcuts.stop = Some("Ctrl+Shift+S".to_string());
        let patched = apply_settings_patch(
            &config,
            &json!({ "shortcuts": { "stop": null } }),
            &sample_displays(),
        )
        .expect("clear shortcut");
        assert!(patched.shortcuts.stop.is_none());
    }

    #[test]
    fn display_keys_are_validated_and_applied() {
        let displays = sample_displays();
        let key = displays[1].key();
        let patched = apply_settings_patch(
            &paired_config(),
            &json!({ "selectedDisplayKey": key }),
            &displays,
        )
        .expect("valid display");
        assert_eq!(patched.display_index, Some(1));
        assert_eq!(patched.display_key.as_deref(), Some(key.as_str()));

        let error = apply_settings_patch(
            &paired_config(),
            &json!({ "selectedDisplayKey": "missing" }),
            &displays,
        )
        .expect_err("unknown display");
        assert_eq!(error.field, "selectedDisplayKey");
    }

    #[test]
    fn test_media_payload_is_local_and_has_no_job_id() {
        let mut config = paired_config();
        config.profile_name = Some("Quentin".to_string());
        let payload = test_media_payload(&config);
        assert!(payload.get("jobId").is_none());
        assert_eq!(payload["test"], json!(true));
        assert_eq!(payload["durationSec"], json!(5));
        assert_eq!(payload["author"]["name"], json!("Quentin"));
        assert_eq!(payload["text"]["value"], json!("Média de test"));
        assert_eq!(payload["media"]["kind"], json!("image"));
        assert_eq!(payload["media"]["url"], json!("assets/test-media.svg"));

        config.profile_name = Some("   ".to_string());
        assert_eq!(
            test_media_payload(&config)["author"]["name"],
            json!("LiveChat")
        );
    }

    #[test]
    fn layout_preview_payload_holds_the_test_card() {
        let payload = layout_preview_payload(&paired_config());
        assert!(payload.get("jobId").is_none());
        assert_eq!(payload["test"], json!(true));
        assert_eq!(payload["hold"], json!(true));
        assert_eq!(payload["durationSec"], json!(LAYOUT_PREVIEW_COUNTDOWN_SEC));
        assert_eq!(payload["media"]["url"], json!("assets/test-media.svg"));
        assert_eq!(payload["text"]["value"], json!("Aperçu de la mise en page"));
    }

    #[test]
    fn preview_layout_applies_the_draft_without_touching_the_config() {
        let mut config = paired_config();
        config.layout.zone.width = 0.4;
        let layout = preview_layout(&config, &json!({ "layout": { "align": "bottom-right" } }))
            .expect("valid draft");
        assert_eq!(layout.align, LayoutAnchor::BottomRight);
        assert_eq!(
            layout.zone.width, 0.4,
            "fields left out keep the saved value"
        );
        assert_eq!(config.layout.align, LayoutAnchor::Center);

        let error = preview_layout(&config, &json!({ "layout": { "align": "middle" } }))
            .expect_err("unknown align");
        assert_eq!(error.field, "layout.align");
        let error = preview_layout(&config, &json!({})).expect_err("layout required");
        assert_eq!(error.field, "layout");
    }

    #[test]
    fn settings_view_matches_the_board_contract() {
        let mut config = paired_config();
        config.layout.zone.width = 0.7;
        config.shortcuts.stop = Some("Ctrl+Shift+S".to_string());
        config.profile_name = Some("Quentin".to_string());
        let displays = sample_displays();
        let view = build_settings_view(&config, &displays, Some(0), true);
        assert_eq!(view.version, env!("CARGO_PKG_VERSION"));
        assert!(!view.general_mute);
        assert_eq!(view.guild_volumes.get("111"), Some(&1.0));
        assert_eq!(view.displays[0].label, "Écran 1 — 2560 × 1440 (principal)");
        assert!(view.displays[0].primary);
        assert_eq!(view.displays[1].label, "Écran 2 — 1920 × 1080");
        assert_eq!(view.layout.zone.width, 0.7);
        assert!(view.auto_start);
        assert!(!view.placing);

        let value = serde_json::to_value(view).expect("serialize");
        assert_eq!(value["placing"], json!(false));
        assert_eq!(value["layout"]["zone"]["width"], json!(0.7));
        assert_eq!(value["shortcuts"]["stop"], json!("Ctrl+Shift+S"));
        assert_eq!(value["profile"]["displayName"], json!("Quentin"));
    }
}
