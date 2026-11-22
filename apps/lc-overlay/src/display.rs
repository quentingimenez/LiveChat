//! Sélection et géométrie des écrans, sans dépendance Tauri (testable sur des
//! moniteurs factices). Les identifiants sont ceux du legacy Electron :
//! `displayIndex`, `displayKey` (`x,y,largeur,hauteur` depuis le correctif de
//! sélection ; les configs antérieures gardent `x,y,largeur,hauteur,index`),
//! `displayLabel` (nom du périphérique, ex. `\\.\DISPLAY2`).

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

    /// Clé de géométrie stable : `x,y,largeur,hauteur`.
    ///
    /// L'index d'énumération en est volontairement exclu : il désigne la place
    /// du moniteur dans la liste de l'OS, qui peut changer d'un démarrage à
    /// l'autre (écran virtuel attaché au boot, ordre des adaptateurs). Une clé
    /// qui l'incluait rendait l'écran configuré introuvable au redémarrage, et
    /// la sélection de repli écrasait alors le choix de l'utilisateur.
    pub fn key(&self) -> String {
        format!("{},{},{},{}", self.x, self.y, self.width, self.height)
    }

    /// Clé de l'ancien format (`x,y,largeur,hauteur,index`), écrite par les
    /// configs antérieures et par Electron : lue pour migrer la sélection.
    pub fn legacy_key(&self) -> String {
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

/// Géométrie physique de la fenêtre overlay : position et taille.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl OverlayGeometry {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Géométrie que l'overlay doit prendre pour couvrir `display`.
    pub fn from_display(display: &DisplayInfo) -> Self {
        let (x, y, width, height) = display.overlay_bounds();
        Self::new(x, y, width, height)
    }
}

/// Ce que la fenêtre overlay doit changer pour retrouver son écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometrySync {
    /// La fenêtre est déjà à la bonne place et à la bonne taille.
    Keep,
    /// L'écran a bougé dans le bureau virtuel (disposition des écrans modifiée).
    Move,
    /// L'écran a changé de résolution.
    Resize,
    /// Les deux à la fois.
    MoveAndResize,
}

/// Compare la géométrie de la fenêtre à celle de l'écran cible.
///
/// La garde topmost s'en sert : un changement de résolution, ou une disposition
/// d'écrans modifiée pendant que l'overlay tourne, laisse sinon la fenêtre à son
/// ancienne géométrie. Le média, centré dans la fenêtre, se retrouve alors
/// décalé et peut sortir complètement de l'écran.
pub fn geometry_sync(current: OverlayGeometry, target: OverlayGeometry) -> GeometrySync {
    let moved = current.x != target.x || current.y != target.y;
    let resized = current.width != target.width || current.height != target.height;
    match (moved, resized) {
        (true, true) => GeometrySync::MoveAndResize,
        (true, false) => GeometrySync::Move,
        (false, true) => GeometrySync::Resize,
        (false, false) => GeometrySync::Keep,
    }
}

/// `formatDisplayMenuLabel` legacy.
pub fn format_display_menu_label(display: &DisplayInfo) -> String {
    match display.label() {
        Some(label) => format!("Écran {} - {}", display.index + 1, label),
        None => format!("Écran {}", display.index + 1),
    }
}

/// Label shown by the Settings display list and the identification cards.
pub fn format_settings_label(display: &DisplayInfo, primary: bool) -> String {
    let suffix = if primary { " (principal)" } else { "" };
    format!(
        "Écran {} — {} × {}{suffix}",
        display.index + 1,
        display.width,
        display.height
    )
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

/// Comment l'écran cible a été retrouvé dans la liste des moniteurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    /// Clé de géométrie exacte : l'écran configuré est là, où qu'il soit énuméré.
    Key,
    /// Nom de périphérique identique : l'écran a changé de place ou de taille.
    Label,
    /// Clé legacy d'une configuration antérieure (index inclus) : à migrer.
    LegacyKey,
    /// `displayIndex` de repli, l'index ne désigne pas forcément le même écran.
    Index,
    /// Écran principal de repli.
    Primary,
    /// Premier écran de repli (écran principal inconnu).
    First,
}

impl SelectionSource {
    /// Seules les correspondances sûres peuvent être réécrites en configuration.
    ///
    /// Un repli (`displayIndex`, primaire, premier) ne doit jamais être
    /// persisté : sinon un redémarrage où l'écran n'est pas encore énuméré
    /// remplace définitivement le choix de l'utilisateur.
    pub fn is_confident(self) -> bool {
        matches!(self, Self::Key | Self::Label | Self::LegacyKey)
    }
}

/// Écran retenu et raison de sa sélection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedDisplay {
    pub index: usize,
    pub source: SelectionSource,
}

/// `getTargetDisplay` legacy, adapté : `displayKey` (ou nom, ou clé legacy) ->
/// `displayIndex` -> primaire -> premier. La source permet à l'appelant de ne
/// persister que les sélections sûres.
///
/// Limite documentée : le `displayId` numérique d'Electron n'a pas d'équivalent
/// stable côté Tauri/Win32 ; il est conservé dans le fichier (rollback Electron)
/// mais n'entre pas dans la résolution.
pub fn resolve_display(
    config: &OverlayConfig,
    displays: &[DisplayInfo],
    primary_index: Option<usize>,
) -> Option<SelectedDisplay> {
    if displays.is_empty() {
        return None;
    }
    if let Some(key) = config.display_key.as_deref() {
        if let Some(index) = displays.iter().position(|display| display.key() == key) {
            return Some(SelectedDisplay {
                index,
                source: SelectionSource::Key,
            });
        }
    }
    // Le nom du périphérique survit à un changement d'ordre d'énumération, de
    // place ou de résolution : c'est l'identité la plus fiable de l'écran.
    if let Some(label) = config.display_label.as_deref() {
        if let Some(index) = displays
            .iter()
            .position(|display| display.label().as_deref() == Some(label))
        {
            return Some(SelectedDisplay {
                index,
                source: SelectionSource::Label,
            });
        }
    }
    if let Some(key) = config.display_key.as_deref() {
        if let Some(index) = displays
            .iter()
            .position(|display| display.legacy_key() == key)
        {
            return Some(SelectedDisplay {
                index,
                source: SelectionSource::LegacyKey,
            });
        }
    }
    if let Some(index) = config.display_index {
        if index >= 0 {
            if let Ok(index) = usize::try_from(index) {
                if index < displays.len() {
                    return Some(SelectedDisplay {
                        index,
                        source: SelectionSource::Index,
                    });
                }
            }
        }
    }
    match primary_index {
        Some(index) if index < displays.len() => Some(SelectedDisplay {
            index,
            source: SelectionSource::Primary,
        }),
        _ => Some(SelectedDisplay {
            index: 0,
            source: SelectionSource::First,
        }),
    }
}

/// Index seul de l'écran cible, sans la source (raccourci de test).
///
/// Le code de production passe par [`resolve_display`] : la source décide si la
/// sélection peut être persistée.
#[cfg(test)]
pub fn select_display(
    config: &OverlayConfig,
    displays: &[DisplayInfo],
    primary_index: Option<usize>,
) -> Option<usize> {
    resolve_display(config, displays, primary_index).map(|selected| selected.index)
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
    fn display_key_is_stable_geometry() {
        // L'index d'énumération ne fait plus partie de la clé : la clé de
        // l'écran ne change pas quand l'ordre de la liste change.
        assert_eq!(display(0, 0, None).key(), "0,0,2560,1440");
        assert_eq!(display(1, -2560, None).key(), "-2560,0,2560,1440");
        let mut reordered = display(1, -2560, None);
        reordered.index = 0;
        assert_eq!(reordered.key(), "-2560,0,2560,1440");
    }

    #[test]
    fn legacy_key_keeps_the_enumeration_index() {
        assert_eq!(display(0, 0, None).legacy_key(), "0,0,2560,1440,0");
        assert_eq!(display(1, -2560, None).legacy_key(), "-2560,0,2560,1440,1");
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
    fn settings_label_uses_dimensions_and_primary() {
        assert_eq!(
            format_settings_label(&display(0, 0, Some("DISPLAY1")), true),
            "Écran 1 — 2560 × 1440 (principal)"
        );
        assert_eq!(
            format_settings_label(&display(1, -2560, None), false),
            "Écran 2 — 2560 × 1440"
        );
    }

    #[test]
    fn selection_keeps_the_screen_when_the_enumeration_order_changes() {
        // Au redémarrage, Windows peut énumérer DISPLAY2 avant DISPLAY1 :
        // l'index stocké ne désigne plus le même écran, la géométrie si.
        let reordered = vec![
            display(0, -2560, Some("DISPLAY2")),
            display(1, 0, Some("DISPLAY1")),
        ];
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440".to_string());
        config.display_label = Some("DISPLAY2".to_string());
        assert_eq!(
            select_display(&config, &reordered, Some(1)),
            Some(0),
            "l'écran configuré doit rester DISPLAY2 même si son index a changé"
        );
    }

    #[test]
    fn selection_recovers_a_legacy_key_through_the_display_label() {
        // Configuration écrite avant le correctif : clé « x,y,w,h,index » et
        // nom du moniteur. L'ordre d'énumération a changé au redémarrage.
        let reordered = vec![
            display(0, -2560, Some("\\\\.\\DISPLAY2")),
            display(1, 0, Some("\\\\.\\DISPLAY1")),
        ];
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440,1".to_string());
        config.display_index = Some(1);
        config.display_label = Some("\\\\.\\DISPLAY2".to_string());
        assert_eq!(
            select_display(&config, &reordered, Some(1)),
            Some(0),
            "le nom du périphérique doit retrouver l'écran malgré l'index périmé"
        );
    }

    #[test]
    fn selection_prefers_display_key() {
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440".to_string());
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
    fn overlay_geometry_comes_from_the_display_bounds() {
        assert_eq!(
            OverlayGeometry::from_display(&display(1, -2560, None)),
            OverlayGeometry::new(-2560, 0, 2560, 1440)
        );
    }

    #[test]
    fn an_overlay_already_on_its_screen_is_kept() {
        let geometry = OverlayGeometry::new(0, 0, 2560, 1440);
        assert_eq!(geometry_sync(geometry, geometry), GeometrySync::Keep);
    }

    #[test]
    fn a_resolution_change_resizes_the_window() {
        let current = OverlayGeometry::new(0, 0, 2560, 1440);
        let target = OverlayGeometry::new(0, 0, 1920, 1080);
        assert_eq!(geometry_sync(current, target), GeometrySync::Resize);
    }

    #[test]
    fn a_display_moved_in_the_desktop_moves_the_window() {
        // Primary screen resolution change shifts the secondary one: same
        // size, new x.
        let current = OverlayGeometry::new(-2560, 0, 2560, 1440);
        let target = OverlayGeometry::new(-1920, 0, 2560, 1440);
        assert_eq!(geometry_sync(current, target), GeometrySync::Move);
    }

    #[test]
    fn a_display_moved_and_resized_does_both() {
        let current = OverlayGeometry::new(-2560, 0, 2560, 1440);
        let target = OverlayGeometry::new(0, 0, 1920, 1080);
        assert_eq!(geometry_sync(current, target), GeometrySync::MoveAndResize);
    }

    #[test]
    fn selection_roundtrips_through_config_fields() {
        let selection = DisplaySelection::from_display(&display(1, -2560, Some("DISPLAY2")));
        assert_eq!(selection.display_index, Some(1));
        assert_eq!(selection.display_key.as_deref(), Some("-2560,0,2560,1440"));
        assert_eq!(selection.display_label.as_deref(), Some("DISPLAY2"));
    }

    #[test]
    fn a_reordered_screen_is_selected_by_its_key() {
        let reordered = vec![
            display(0, -2560, Some("DISPLAY2")),
            display(1, 0, Some("DISPLAY1")),
        ];
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440".to_string());
        let selected = resolve_display(&config, &reordered, Some(1)).expect("selection");
        assert_eq!(selected.index, 0);
        assert_eq!(selected.source, SelectionSource::Key);
        assert!(selected.source.is_confident());
    }

    #[test]
    fn a_legacy_key_is_accepted_and_migrated_through_the_label() {
        let reordered = vec![
            display(0, -2560, Some("\\\\.\\DISPLAY2")),
            display(1, 0, Some("\\\\.\\DISPLAY1")),
        ];
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440,1".to_string());
        config.display_index = Some(1);
        config.display_label = Some("\\\\.\\DISPLAY2".to_string());
        let selected = resolve_display(&config, &reordered, Some(1)).expect("selection");
        assert_eq!(selected.index, 0);
        assert_eq!(selected.source, SelectionSource::Label);
        assert!(selected.source.is_confident());
    }

    #[test]
    fn a_legacy_key_without_a_label_is_still_recognised() {
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440,1".to_string());
        let selected = resolve_display(&config, &displays(), Some(0)).expect("selection");
        assert_eq!(selected.index, 1);
        assert_eq!(selected.source, SelectionSource::LegacyKey);
        assert!(selected.source.is_confident());
    }

    #[test]
    fn a_screen_that_changed_geometry_is_found_by_its_label() {
        let resized = vec![DisplayInfo::new(
            0,
            Some("\\\\.\\DISPLAY2".to_string()),
            -1920,
            0,
            1920,
            1080,
            1.0,
        )];
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440".to_string());
        config.display_label = Some("\\\\.\\DISPLAY2".to_string());
        let selected = resolve_display(&config, &resized, Some(0)).expect("selection");
        assert_eq!(selected.index, 0);
        assert_eq!(selected.source, SelectionSource::Label);
    }

    #[test]
    fn a_fallback_selection_is_never_confident() {
        // Aucun écran configuré : le primaire sert d'emplacement, sans être
        // enregistré comme choix de l'utilisateur.
        let mut config = OverlayConfig::default();
        let primary = resolve_display(&config, &displays(), Some(0)).expect("selection");
        assert_eq!(primary.index, 0);
        assert_eq!(primary.source, SelectionSource::Primary);
        assert!(!primary.source.is_confident());

        config.display_index = Some(1);
        let by_index = resolve_display(&config, &displays(), Some(0)).expect("selection");
        assert_eq!(by_index.index, 1);
        assert_eq!(by_index.source, SelectionSource::Index);
        assert!(!by_index.source.is_confident());

        let unset = OverlayConfig::default();
        let first = resolve_display(&unset, &displays(), None).expect("selection");
        assert_eq!(first.index, 0);
        assert_eq!(first.source, SelectionSource::First);
        assert!(!first.source.is_confident());
    }
}
