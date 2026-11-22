//! Empreinte matérielle stable, défense en profondeur du bannissement.
//!
//! L'empreinte combine l'UUID SMBIOS de la carte mère, le numéro de série du
//! disque système (quand il est lisible) et le modèle du processeur. Elle
//! survit à une réinstallation de Windows tant que le matériel ne change pas.
//! Ce n'est pas une authentification : un client modifié peut mentir, et un
//! changement de matériel change l'empreinte. Le serveur la hache par guilde et
//! ne stocke jamais la valeur brute ; ce module ne doit jamais la journaliser.
//!
//! Quand aucun composant fiable n'est lisible (ou seulement des valeurs
//! d'usine), `None` est renvoyé : les anciens clients restent hors du contrôle
//! par appareil, comme documenté dans `docs/rust/guilds-roles.md`.

/// Longueur maximale d'un composant avant concaténation.
const MAX_COMPONENT_CHARS: usize = 96;

/// Valeurs d'usine ou textuelles qui ne distinguent aucune machine.
const PLACEHOLDERS: &[&str] = &[
    "to be filled by o.e.m.",
    "to be filled by oem",
    "default string",
    "base board serial number",
    "chassis serial number",
    "system serial number",
    "not specified",
    "not available",
    "not applicable",
    "unknown",
    "none",
    "n/a",
    "oem",
];

/// Empreinte brute de cette machine, ou `None` si aucun composant fiable.
pub(crate) fn hardware_fingerprint() -> Option<String> {
    fingerprint(
        machine_board_uuid().as_deref(),
        machine_disk_serial().as_deref(),
        machine_processor().as_deref(),
    )
}

#[cfg(windows)]
fn machine_board_uuid() -> Option<String> {
    crate::win32::smbios_board_uuid()
}

#[cfg(windows)]
fn machine_disk_serial() -> Option<String> {
    crate::win32::system_disk_serial()
}

#[cfg(windows)]
fn machine_processor() -> Option<String> {
    crate::win32::processor_name()
}

#[cfg(not(windows))]
fn machine_board_uuid() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn machine_disk_serial() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn machine_processor() -> Option<String> {
    None
}

/// Construit l'empreinte, dans un ordre fixe et sans composant absent.
fn fingerprint(
    board_uuid: Option<&str>,
    disk_serial: Option<&str>,
    processor: Option<&str>,
) -> Option<String> {
    let mut components = Vec::new();
    if let Some(uuid) = normalize_board_uuid(board_uuid) {
        components.push(format!("board={uuid}"));
    }
    if let Some(serial) = normalize_component(disk_serial) {
        components.push(format!("disk={serial}"));
    }
    if let Some(model) = normalize_component(processor) {
        components.push(format!("cpu={model}"));
    }
    if components.is_empty() {
        None
    } else {
        Some(components.join(";"))
    }
}

/// Normalise une valeur d'usine (`None`, vide, placeholder) en `None`.
fn normalize_component(raw: Option<&str>) -> Option<String> {
    let collapsed = raw?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || is_placeholder(&collapsed) {
        return None;
    }
    Some(collapsed.chars().take(MAX_COMPONENT_CHARS).collect())
}

/// UUID SMBIOS d'usine, servi tel quel par de nombreuses cartes mères.
const FACTORY_BOARD_UUID: &str = "03000200040005000006000700080009";

/// Normalise l'UUID SMBIOS en 32 caractères hexadécimaux, ou l'écarte.
fn normalize_board_uuid(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim().to_ascii_lowercase();
    let hex: String = value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect();
    if hex.len() != 32 {
        return None;
    }
    if hex == FACTORY_BOARD_UUID
        || hex.chars().all(|character| character == '0')
        || hex.chars().all(|character| character == 'f')
    {
        return None;
    }
    Some(hex)
}

fn is_placeholder(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    PLACEHOLDERS
        .iter()
        .any(|placeholder| lowercase == *placeholder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic_and_ordered() {
        let first = fingerprint(
            Some("4c4c4544-0031-3510-8043-b2c04f4d4e31"),
            Some(" WD-WCC4N1234567 "),
            Some("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz"),
        );
        let second = fingerprint(
            Some("4c4c4544003135108043b2c04f4d4e31"),
            Some("WD-WCC4N1234567"),
            Some("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz"),
        );
        assert_eq!(first, second);
        let value = first.expect("three reliable components");
        assert!(
            value.starts_with("board=4c4c4544003135108043b2c04f4d4e31;disk=WD-WCC4N1234567;cpu=")
        );
    }

    #[test]
    fn fingerprint_omits_missing_and_placeholder_components() {
        assert_eq!(
            fingerprint(
                Some("   "),
                Some("To be filled by O.E.M."),
                Some(" Ryzen 7 ")
            ),
            Some("cpu=Ryzen 7".to_string())
        );
        assert_eq!(
            fingerprint(None, None, Some("Default string")),
            None,
            "an all-placeholder machine carries no usable fingerprint"
        );
        assert_eq!(fingerprint(None, None, None), None);
    }

    #[test]
    fn fingerprint_rejects_unusable_board_uuids() {
        assert_eq!(
            fingerprint(Some("00000000-0000-0000-0000-000000000000"), None, None),
            None
        );
        assert_eq!(
            fingerprint(Some("ffffffffffffffffffffffffffffffff"), None, None),
            None
        );
        // The SMBIOS factory UUID is shared by many boards: it identifies
        // nothing.
        assert_eq!(
            fingerprint(Some("03000200-0400-0500-0006-000700080009"), None, None),
            None
        );
        assert_eq!(
            fingerprint(Some("03000200040005000006000700080009"), None, None),
            None
        );
        assert_eq!(fingerprint(Some("not-a-uuid"), None, None), None);
    }

    #[test]
    fn fingerprint_truncates_oversized_components() {
        let long = "A".repeat(MAX_COMPONENT_CHARS + 20);
        let value = fingerprint(None, Some(&long), None).expect("one component");
        assert_eq!(value.len(), "disk=".len() + MAX_COMPONENT_CHARS);
    }

    #[test]
    fn live_fingerprint_is_absent_or_usable() {
        assert!(hardware_fingerprint()
            .is_none_or(|value| { !value.is_empty() && value.chars().count() <= 256 }));
    }
}
