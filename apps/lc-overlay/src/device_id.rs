//! Stable device identifier, used by the server as a ban defense in depth.
//!
//! This is a fingerprint, not an authentication credential: a patched client
//! can lie about it, and reinstalling Windows yields a new one. The server
//! hashes it per guild and never stores the raw value; this module must never
//! log it either. When no identifier can be read, pairing proceeds without one
//! (older clients stay uncovered, as documented in `docs/rust/guilds-roles.md`).

/// Override for tests, CI and portable installs.
const DEVICE_ID_ENV: &str = "LC_DEVICE_ID";

/// Identifier of this machine, or `None` when it cannot be read.
pub(crate) fn stable_device_id() -> Option<String> {
    let override_value = std::env::var(DEVICE_ID_ENV).ok();
    stable_device_id_from(override_value.as_deref())
}

/// `LC_DEVICE_ID` wins over the machine identifier; a blank override means
/// "unset" so a CI job can fall back to the real machine.
fn stable_device_id_from(override_value: Option<&str>) -> Option<String> {
    if let Some(raw) = override_value {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    machine_device_id()
}

#[cfg(windows)]
fn machine_device_id() -> Option<String> {
    crate::win32::machine_guid()
}

#[cfg(not(windows))]
fn machine_device_id() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_is_trimmed_and_wins_over_the_machine() {
        assert_eq!(
            stable_device_id_from(Some("  machine-abc  ")).as_deref(),
            Some("machine-abc")
        );
        assert_eq!(
            stable_device_id_from(Some("machine-abc")).as_deref(),
            Some("machine-abc")
        );
    }

    #[test]
    fn a_blank_override_is_treated_as_unset() {
        // Falls back to the machine, which a unit test cannot pin (a Windows
        // registry read). The point here is only that no empty identifier is
        // ever sent, hence no hash of the empty string.
        assert!(stable_device_id_from(Some("   ")).is_none_or(|id| !id.is_empty()));
    }
}
