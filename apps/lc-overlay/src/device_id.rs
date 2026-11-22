//! Stable device identifier, used by the server as a ban defense in depth.
//!
//! This is a fingerprint, not an authentication credential: a patched client
//! can lie about it, and reinstalling Windows yields a new one. The server
//! hashes it per guild and never stores the raw value; this module must never
//! log it either. When no identifier can be read, pairing proceeds without one
//! (older clients stay uncovered, as documented in `docs/rust/guilds-roles.md`).
//!
//! `LC_DEVICE_ID` is a debug-build override for tests, CI and portable
//! installs. A release build always reads the machine identifier: otherwise
//! the ban defense would be one environment variable away from bypass.

/// Debug/test override; ignored by release builds.
const DEVICE_ID_ENV: &str = "LC_DEVICE_ID";

/// Identifier of this machine, or `None` when it cannot be read.
pub(crate) fn stable_device_id() -> Option<String> {
    let override_value = std::env::var(DEVICE_ID_ENV).ok();
    stable_device_id_from(override_value.as_deref(), cfg!(debug_assertions))
}

/// `LC_DEVICE_ID` wins over the machine identifier in debug/test builds only;
/// a blank override means "unset" so a CI job can fall back to the real
/// machine.
fn stable_device_id_from(override_value: Option<&str>, allow_override: bool) -> Option<String> {
    if allow_override {
        if let Some(raw) = override_value {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
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
    fn the_override_is_trimmed_and_wins_in_debug_builds() {
        assert_eq!(
            stable_device_id_from(Some("  machine-abc  "), true).as_deref(),
            Some("machine-abc")
        );
        assert_eq!(
            stable_device_id_from(Some("machine-abc"), true).as_deref(),
            Some("machine-abc")
        );
    }

    #[test]
    fn a_release_build_ignores_the_override() {
        // The machine identifier cannot be pinned from a unit test (a Windows
        // registry read); the point is that the environment value never wins.
        assert_ne!(
            stable_device_id_from(Some("machine-abc"), false).as_deref(),
            Some("machine-abc")
        );
    }

    #[test]
    fn a_blank_override_is_treated_as_unset() {
        // Falls back to the machine, which a unit test cannot pin (a Windows
        // registry read). The point here is only that no empty identifier is
        // ever sent, hence no hash of the empty string.
        assert!(stable_device_id_from(Some("   "), true).is_none_or(|id| !id.is_empty()));
    }
}
