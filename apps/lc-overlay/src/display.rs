//! Sélection et géométrie des écrans, sans dépendance Tauri (testable sur des
//! moniteurs factices). Les identifiants sont ceux du legacy Electron :
//! `displayIndex`, `displayKey` (`x,y,largeur,hauteur,index`), `displayLabel`.

use crate::config::OverlayConfig;

/// Description d'un moniteur connue du shell.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayInfo {
    pub index: usize,
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

impl DisplayInfo {
    pub fn new(
        index: usize,
        name: Option<String>,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        scale_factor: f64,
    ) -> Self {
        Self {
            index,
            name,
            x,
            y,
            width,
            height,
            scale_factor,
        }
    }

    pub fn label(&self) -> Option<String> {
        let label = self.name.as_deref().unwrap_or_default().trim();
        if label.is_empty() {
            None
        } else {
            Some(label.to_string())
        }
    }

    /// Identifiant stable utilisé par le legacy Electron (même formule).
    pub fn key(&self) -> String {
        format!(
            "{},{},{},{},{}",
            self.x, self.y, self.width, self.height, self.index
        )
    }

    /// Bornes physiques de l'overlay (plein écran, comme le legacy).
    pub fn overlay_bounds(&self) -> (i32, i32, u32, u32) {
        (self.x, self.y, self.width, self.height)
    }
}

/// `formatDisplayMenuLabel` legacy.
pub fn format_display_menu_label(display: &DisplayInfo) -> String {
    match display.label() {
        Some(label) => format!("Écran {} - {}", display.index + 1, label),
        None => format!("Écran {}", display.index + 1),
    }
}

/// Champs de configuration à persister pour mémoriser l'écran choisi.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplaySelection {
    pub display_index: Option<i64>,
    pub display_key: Option<String>,
    pub display_label: Option<String>,
}

impl DisplaySelection {
    pub fn from_display(display: &DisplayInfo) -> Self {
        Self {
            display_index: Some(display.index as i64),
            display_key: Some(display.key()),
            display_label: display.label(),
        }
    }
}

/// `getTargetDisplay` legacy, adapté : `displayKey` -> `displayIndex` -> primaire.
///
/// Limite documentée : le `displayId` numérique d'Electron n'a pas d'équivalent
/// stable côté Tauri/Win32 ; il est conservé dans le fichier (rollback Electron)
/// mais n'entre pas dans la résolution.
pub fn select_display(
    config: &OverlayConfig,
    displays: &[DisplayInfo],
    primary_index: Option<usize>,
) -> Option<usize> {
    if displays.is_empty() {
        return None;
    }
    if let Some(key) = config.display_key.as_deref() {
        if let Some(index) = displays.iter().position(|display| display.key() == key) {
            return Some(index);
        }
    }
    if let Some(index) = config.display_index {
        if index >= 0 {
            if let Ok(index) = usize::try_from(index) {
                if index < displays.len() {
                    return Some(index);
                }
            }
        }
    }
    match primary_index {
        Some(index) if index < displays.len() => Some(index),
        _ => Some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(index: usize, x: i32, name: Option<&str>) -> DisplayInfo {
        DisplayInfo::new(index, name.map(str::to_string), x, 0, 2560, 1440, 1.0)
    }

    fn displays() -> Vec<DisplayInfo> {
        vec![
            display(0, 0, Some("DISPLAY1")),
            display(1, -2560, Some("DISPLAY2")),
        ]
    }

    #[test]
    fn display_key_matches_legacy_format() {
        assert_eq!(display(0, 0, None).key(), "0,0,2560,1440,0");
        assert_eq!(display(1, -2560, None).key(), "-2560,0,2560,1440,1");
    }

    #[test]
    fn label_is_trimmed_and_empty_becomes_none() {
        assert_eq!(
            display(0, 0, Some("  Écran 1  ")).label(),
            Some("Écran 1".to_string())
        );
        assert_eq!(display(0, 0, Some("   ")).label(), None);
        assert_eq!(display(0, 0, None).label(), None);
    }

    #[test]
    fn menu_label_matches_legacy() {
        assert_eq!(
            format_display_menu_label(&display(0, 0, Some("DISPLAY1"))),
            "Écran 1 - DISPLAY1"
        );
        assert_eq!(
            format_display_menu_label(&display(1, -2560, None)),
            "Écran 2"
        );
    }

    #[test]
    fn selection_prefers_display_key() {
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440,1".to_string());
        config.display_index = Some(0);
        assert_eq!(select_display(&config, &displays(), Some(0)), Some(1));
    }

    #[test]
    fn selection_falls_back_to_index() {
        let mut config = OverlayConfig::default();
        config.display_index = Some(1);
        assert_eq!(select_display(&config, &displays(), Some(0)), Some(1));
    }

    #[test]
    fn selection_ignores_negative_and_out_of_range_index() {
        let mut config = OverlayConfig::default();
        config.display_index = Some(-1);
        assert_eq!(select_display(&config, &displays(), Some(1)), Some(1));
        config.display_index = Some(9);
        assert_eq!(select_display(&config, &displays(), Some(1)), Some(1));
    }

    #[test]
    fn selection_falls_back_to_primary_then_first() {
        let config = OverlayConfig::default();
        assert_eq!(select_display(&config, &displays(), Some(1)), Some(1));
        assert_eq!(select_display(&config, &displays(), None), Some(0));
        assert_eq!(select_display(&config, &[], Some(0)), None);
    }

    #[test]
    fn overlay_bounds_are_full_display() {
        let (x, y, width, height) = display(1, -2560, None).overlay_bounds();
        assert_eq!((x, y, width, height), (-2560, 0, 2560, 1440));
    }

    #[test]
    fn selection_roundtrips_through_config_fields() {
        let selection = DisplaySelection::from_display(&display(1, -2560, Some("DISPLAY2")));
        assert_eq!(selection.display_index, Some(1));
        assert_eq!(
            selection.display_key.as_deref(),
            Some("-2560,0,2560,1440,1")
        );
        assert_eq!(selection.display_label.as_deref(), Some("DISPLAY2"));
    }
}
