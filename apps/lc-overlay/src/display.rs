//! Sélection et géométrie des écrans, sans dépendance Tauri (testable sur des
//! moniteurs factices). Les identifiants sont ceux du legacy Electron :
//! `displayIndex`, `displayKey` (`x,y,largeur,hauteur` depuis le correctif de
//! sélection ; les configs antérieures gardent `x,y,largeur,hauteur,index`),
//! `displayLabel` (nom du périphérique, ex. `\\.\DISPLAY2`) et
//! `displayHardwareId` (constructeur, produit et numéro de série lus dans
//! l'EDID : la seule identité que Windows ne réattribue pas).

use crate::config::OverlayConfig;

/// Description d'un moniteur connue du shell.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayInfo {
    pub index: usize,
    pub name: Option<String>,
    /// Nom court du moniteur (`DeviceString` Windows, ex. « G27q-20 ») : sert
    /// uniquement à l'affichage, jamais d'identité persistée ni de clé.
    pub short_name: Option<String>,
    /// Identité matérielle stable du moniteur (EDID), quand Windows la
    /// fournit : elle survit à une renumérotation de `\\.\DISPLAYn`, à un
    /// changement de port et à un changement de résolution.
    pub hardware_id: Option<String>,
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
            short_name: None,
            hardware_id: None,
            x,
            y,
            width,
            height,
            scale_factor,
        }
    }

    /// Renseigne le nom court du moniteur (identité d'affichage seule : il ne
    /// remplace ni `label()` ni la clé, qui restent seuls persistés).
    pub fn with_short_name(mut self, short_name: Option<String>) -> Self {
        self.short_name = short_name
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self
    }

    /// Renseigne l'identité matérielle du moniteur (EDID), quand le système la
    /// fournit : c'est elle que la sélection persiste en premier.
    pub fn with_hardware_id(mut self, hardware_id: Option<String>) -> Self {
        self.hardware_id = hardware_id
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self
    }

    pub fn label(&self) -> Option<String> {
        let label = self.name.as_deref().unwrap_or_default().trim();
        if label.is_empty() {
            None
        } else {
            Some(label.to_string())
        }
    }

    /// Nom court du moniteur tel que Windows l'affiche, quand le pilote le
    /// fournit ; `None` sinon (l'affichage retombe alors sur le nom d'appareil).
    pub fn short_name(&self) -> Option<&str> {
        self.short_name.as_deref()
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

/// `formatDisplayMenuLabel` legacy, enrichi du nom court du moniteur.
pub fn format_display_menu_label(display: &DisplayInfo) -> String {
    let number = display.index + 1;
    if let Some(name) = display.short_name() {
        return format!("Écran {number} - {name}");
    }
    match display.label() {
        Some(label) => format!("Écran {number} - {label}"),
        None => format!("Écran {number}"),
    }
}

/// Label shown by the Settings display list and the identification cards.
pub fn format_settings_label(display: &DisplayInfo, primary: bool) -> String {
    let number = display.index + 1;
    match display.short_name() {
        Some(name) => {
            let suffix = if primary { ", principal" } else { "" };
            format!(
                "Écran {number} — {name} ({} × {}{suffix})",
                display.width, display.height
            )
        }
        None => {
            let suffix = if primary { " (principal)" } else { "" };
            format!(
                "Écran {number} — {} × {}{suffix}",
                display.width, display.height
            )
        }
    }
}

/// Champs de configuration à persister pour mémoriser l'écran choisi.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplaySelection {
    pub display_index: Option<i64>,
    pub display_key: Option<String>,
    pub display_label: Option<String>,
    pub display_hardware_id: Option<String>,
}

impl DisplaySelection {
    pub fn from_display(display: &DisplayInfo) -> Self {
        Self {
            display_index: Some(display.index as i64),
            display_key: Some(display.key()),
            display_label: display.label(),
            display_hardware_id: display.hardware_id.clone(),
        }
    }
}

/// Comment l'écran cible a été retrouvé dans la liste des moniteurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    /// Identité matérielle (EDID) identique : le moniteur physique, même s'il a
    /// été renommé, déplacé ou changé de résolution.
    HardwareId,
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
    /// remplace définitivement le choix de l'utilisateur. Le nom
    /// `\\.\DISPLAYn` non plus : Windows le réattribue d'un démarrage à l'autre
    /// et après un rebranchement, si bien qu'un label périmé peut désigner un
    /// autre moniteur — le persister « réinitialisait » l'overlay sur l'écran
    /// principal.
    pub fn is_confident(self) -> bool {
        matches!(self, Self::HardwareId | Self::Key | Self::LegacyKey)
    }

    /// La correspondance nomme un écran précis, même si elle ne suffit pas à
    /// réécrire la sélection (le nom `\\.\DISPLAYn` peut être périmé).
    pub fn is_specific(self) -> bool {
        matches!(
            self,
            Self::HardwareId | Self::Key | Self::Label | Self::LegacyKey
        )
    }
}

/// Écran retenu et raison de sa sélection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedDisplay {
    pub index: usize,
    pub source: SelectionSource,
}

/// `getTargetDisplay` legacy, adapté : identité matérielle, puis `displayKey`
/// (ou nom, ou clé legacy) -> `displayIndex` -> primaire -> premier. La source
/// permet à l'appelant de ne persister que les sélections sûres.
///
/// L'identité matérielle passe avant la géométrie : un moniteur déplacé dans le
/// bureau virtuel, ou qui a changé de résolution, reste le même, alors que sa
/// clé de géométrie désigne alors un autre écran — ou aucun.
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
    if let Some(hardware_id) = config.display_hardware_id.as_deref() {
        if let Some(index) = displays
            .iter()
            .position(|display| display.hardware_id.as_deref() == Some(hardware_id))
        {
            return Some(SelectedDisplay {
                index,
                source: SelectionSource::HardwareId,
            });
        }
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
    // place ou de résolution ; Windows le réattribue en revanche d'un
    // démarrage à l'autre : il place l'overlay, sans être persisté.
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

/// La fenêtre est entièrement posée sur un des écrans énumérés.
///
/// Sert de garde aux sélections non sûres : tant que l'overlay remplit un
/// écran, une énumération transitoire (réveil, changement de mode, écran
/// temporairement absent) ne doit pas le déplacer. Un écran débranché, lui,
/// laisse la fenêtre hors de tout écran : le repli peut alors la récupérer.
pub fn is_on_a_display(current: OverlayGeometry, displays: &[DisplayInfo]) -> bool {
    displays.iter().any(|display| {
        let left = i64::from(display.x);
        let top = i64::from(display.y);
        let right = left + i64::from(display.width);
        let bottom = top + i64::from(display.height);
        let x = i64::from(current.x);
        let y = i64::from(current.y);
        x >= left
            && y >= top
            && x + i64::from(current.width) <= right
            && y + i64::from(current.height) <= bottom
    })
}

/// La configuration garde une sélection d'écran, fût-elle périmée.
///
/// C'est elle qu'un repli doit préserver. Une installation neuve n'en a aucune :
/// la fenêtre est encore à sa taille par défaut et doit adopter l'écran de
/// secours au lieu de rester dans ce rectangle.
pub fn has_recorded_selection(config: &OverlayConfig) -> bool {
    config.display_hardware_id.is_some()
        || config.display_key.is_some()
        || config.display_label.is_some()
        || config.display_index.is_some()
        || config.display_id.is_some()
}

/// Géométrie que l'overlay doit prendre à ce tick, `None` s'il reste où il est.
///
/// Un écran qui a changé de place ou de résolution est suivi ; une sélection
/// non sûre (repli, nom réattribué) ne déplace en revanche jamais une fenêtre
/// encore posée sur un écran : le choix de l'utilisateur est retrouvé au tick
/// suivant, quand l'énumération redevient complète.
pub fn next_overlay_geometry(
    current: OverlayGeometry,
    target: &DisplayInfo,
    source: SelectionSource,
    displays: &[DisplayInfo],
    recorded: bool,
) -> Option<OverlayGeometry> {
    let target_geometry = OverlayGeometry::from_display(target);
    if geometry_sync(current, target_geometry) == GeometrySync::Keep {
        return None;
    }
    if recorded && !source.is_confident() && is_on_a_display(current, displays) {
        return None;
    }
    Some(target_geometry)
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
    fn menu_label_prefers_the_monitor_short_name() {
        let named = display(0, 0, Some("DISPLAY1")).with_short_name(Some("G27q-20".to_string()));
        assert_eq!(format_display_menu_label(&named), "Écran 1 - G27q-20");
        let blank = display(1, -2560, Some("DISPLAY2")).with_short_name(Some("   ".to_string()));
        assert_eq!(format_display_menu_label(&blank), "Écran 2 - DISPLAY2");
    }

    #[test]
    fn settings_label_puts_the_short_name_before_the_dimensions() {
        let named = display(0, 0, Some("DISPLAY1")).with_short_name(Some("G27q-20".to_string()));
        assert_eq!(
            format_settings_label(&named, true),
            "Écran 1 — G27q-20 (2560 × 1440, principal)"
        );
        let unnamed = display(1, -2560, None);
        assert_eq!(
            format_settings_label(&unnamed, false),
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
        let named = display(1, -2560, Some("DISPLAY2"))
            .with_hardware_id(Some("SAM:707A:Z9XK400001".to_string()));
        let selection = DisplaySelection::from_display(&named);
        assert_eq!(selection.display_index, Some(1));
        assert_eq!(selection.display_key.as_deref(), Some("-2560,0,2560,1440"));
        assert_eq!(selection.display_label.as_deref(), Some("DISPLAY2"));
        assert_eq!(
            selection.display_hardware_id.as_deref(),
            Some("SAM:707A:Z9XK400001")
        );
        assert_eq!(
            DisplaySelection::from_display(&display(0, 0, None)).display_hardware_id,
            None
        );
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
    fn a_legacy_key_is_placed_through_the_label_without_being_persisted() {
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
        assert!(
            !selected.source.is_confident(),
            "le nom place l'overlay mais ne remplace pas la sélection enregistrée"
        );
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

    #[test]
    fn a_renumbered_display_label_is_not_a_confident_identity() {
        // Windows réattribue `\\.\DISPLAYn` d'un démarrage à l'autre : après un
        // redémarrage ou un rebranchement, DISPLAY2 peut nommer l'écran
        // principal. Retrouver l'écran par ce nom ne doit donc jamais passer
        // pour un choix sûr, sinon un label périmé réécrit définitivement la
        // sélection de l'utilisateur.
        let renumbered = vec![
            display(0, 0, Some("\\\\.\\DISPLAY2")),
            display(1, -1920, Some("\\\\.\\DISPLAY1")),
        ];
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440".to_string());
        config.display_label = Some("\\\\.\\DISPLAY2".to_string());
        let selected = resolve_display(&config, &renumbered, Some(0)).expect("selection");
        assert_eq!(selected.index, 0, "le nom retrouve le moniteur renuméroté");
        assert_eq!(selected.source, SelectionSource::Label);
        assert!(
            !selected.source.is_confident(),
            "un nom `\\\\.\\DISPLAYn` renuméroté n'est pas une identité sûre"
        );
    }

    #[test]
    fn the_hardware_identity_wins_over_a_geometry_taken_by_another_screen() {
        // Les deux moniteurs ont échangé leurs places : la clé enregistrée
        // désigne désormais l'autre écran, l'EDID désigne toujours celui de
        // l'utilisateur.
        let swapped = vec![
            display(0, -2560, Some("\\\\.\\DISPLAY2"))
                .with_hardware_id(Some("SAM:707A:PRINCIPAL".to_string())),
            display(1, 0, Some("\\\\.\\DISPLAY1"))
                .with_hardware_id(Some("SAM:707A:BUREAU".to_string())),
        ];
        let mut config = OverlayConfig::default();
        config.display_key = Some("0,0,2560,1440".to_string());
        config.display_hardware_id = Some("SAM:707A:BUREAU".to_string());
        let selected = resolve_display(&config, &swapped, Some(0)).expect("selection");
        assert_eq!(selected.index, 1);
        assert_eq!(selected.source, SelectionSource::HardwareId);
        assert!(selected.source.is_confident());
    }

    #[test]
    fn a_renumbered_label_does_not_beat_the_hardware_identity() {
        // Après un rebranchement, le moniteur de l'utilisateur est nommé
        // DISPLAY1 et le nom enregistré (DISPLAY2) désigne le principal.
        let renumbered = vec![
            display(0, 0, Some("\\\\.\\DISPLAY2"))
                .with_hardware_id(Some("SAM:707A:PRINCIPAL".to_string())),
            display(1, -1920, Some("\\\\.\\DISPLAY1"))
                .with_hardware_id(Some("SAM:707A:BUREAU".to_string())),
        ];
        let mut config = OverlayConfig::default();
        config.display_key = Some("-2560,0,2560,1440".to_string());
        config.display_label = Some("\\\\.\\DISPLAY2".to_string());
        config.display_hardware_id = Some("SAM:707A:BUREAU".to_string());
        let selected = resolve_display(&config, &renumbered, Some(0)).expect("selection");
        assert_eq!(selected.index, 1);
        assert_eq!(selected.source, SelectionSource::HardwareId);
    }

    #[test]
    fn a_window_is_on_a_screen_only_when_it_fits_entirely() {
        let displays = displays();
        let on_second = OverlayGeometry::new(-2560, 0, 2560, 1440);
        assert!(is_on_a_display(on_second, &displays));
        let slightly_off = OverlayGeometry::new(-2559, 0, 2560, 1440);
        assert!(!is_on_a_display(slightly_off, &displays));
        let straddling = OverlayGeometry::new(-1280, 0, 2560, 1440);
        assert!(!is_on_a_display(straddling, &displays));
        let too_tall = OverlayGeometry::new(-2560, 0, 2560, 4000);
        assert!(!is_on_a_display(too_tall, &displays));
        assert!(!is_on_a_display(on_second, &[]));
    }

    #[test]
    fn a_transient_fallback_never_pulls_the_overlay_off_its_screen() {
        // Réveil, changement de mode ou ré-énumération : l'écran configuré
        // manque le temps d'un tick et la sélection retombe sur le principal.
        // L'overlay occupe encore son écran : il ne doit pas sauter.
        let displays = displays();
        let current = OverlayGeometry::new(-2560, 0, 2560, 1440);
        for source in [SelectionSource::Primary, SelectionSource::Index] {
            assert_eq!(
                next_overlay_geometry(current, &displays[0], source, &displays, true),
                None,
                "repli {source:?} : l'overlay reste sur son écran"
            );
        }
    }

    #[test]
    fn a_first_launch_adopts_the_fallback_screen() {
        // Installation neuve : aucune sélection enregistrée, et la fenêtre est
        // encore à sa taille par défaut (1280×720). Le repli doit la faire
        // adopter l'écran principal, pas la laisser dans ce rectangle.
        let displays = displays();
        let mut config = OverlayConfig::default();
        assert!(!has_recorded_selection(&config));
        assert_eq!(
            next_overlay_geometry(
                OverlayGeometry::new(0, 0, 1280, 720),
                &displays[0],
                SelectionSource::Primary,
                &displays,
                has_recorded_selection(&config)
            ),
            Some(OverlayGeometry::new(0, 0, 2560, 1440)),
            "sans sélection enregistrée, le repli remplit l'écran"
        );
        // Une sélection, même périmée, protège la fenêtre en place.
        config.display_key = Some("-2560,0,2560,1440".to_string());
        assert!(has_recorded_selection(&config));
        assert_eq!(
            next_overlay_geometry(
                OverlayGeometry::new(0, 0, 1280, 720),
                &displays[1],
                SelectionSource::Primary,
                &displays,
                has_recorded_selection(&config)
            ),
            None
        );
    }

    #[test]
    fn a_confident_selection_follows_a_screen_that_moved() {
        let displays = displays();
        let current = OverlayGeometry::new(-2560, 0, 2560, 1440);
        let moved = display(1, -1920, Some("DISPLAY2"));
        assert_eq!(
            next_overlay_geometry(current, &moved, SelectionSource::Key, &displays, true),
            Some(OverlayGeometry::new(-1920, 0, 2560, 1440))
        );
    }

    #[test]
    fn a_fallback_recovers_an_overlay_left_off_every_screen() {
        // Écran débranché : la fenêtre n'est plus posée sur aucun écran, le
        // repli la ramène sur le principal.
        let disconnected = vec![display(0, 0, Some("DISPLAY1"))];
        let current = OverlayGeometry::new(-2560, 0, 2560, 1440);
        assert_eq!(
            next_overlay_geometry(
                current,
                &disconnected[0],
                SelectionSource::Primary,
                &disconnected,
                true
            ),
            Some(OverlayGeometry::new(0, 0, 2560, 1440))
        );
    }

    #[test]
    fn an_overlay_already_filling_its_screen_is_left_alone() {
        let displays = displays();
        let current = OverlayGeometry::new(-2560, 0, 2560, 1440);
        assert_eq!(
            next_overlay_geometry(
                current,
                &displays[1],
                SelectionSource::HardwareId,
                &displays,
                true
            ),
            None
        );
        assert_eq!(
            next_overlay_geometry(
                current,
                &displays[1],
                SelectionSource::Label,
                &displays,
                true
            ),
            None,
            "un nom renuméroté ne déplace pas une fenêtre encore posée sur son écran"
        );
    }
}
