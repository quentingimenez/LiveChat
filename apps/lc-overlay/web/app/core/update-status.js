// Shared updater status (`settings.update`): the single source of truth for
// the Settings page and the board's update banner.
//
// The shell reports the updater state inside the device settings: the
// `settings_get` command and the `overlay://device-settings` event both carry
// an `update` object (`{ state, currentVersion, availableVersion, message }`,
// see src/updater.rs). settings.js feeds this store and reads it back for the
// « Mise à jour » section; update-notice.js reads it for the banner shown when
// automatic installs are disabled.
//
// No import: this module also loads under Node for its unit tests.

// Last status recorded by `setUpdateStatus` (`null` until the shell speaks).
let current = null;
const listeners = new Set();

export function currentUpdateStatus() {
  return current;
}

// Records a status (from a device-settings payload or an `overlay_check_updates`
// answer) and notifies the subscribers. `null` or a non-object clears it.
export function setUpdateStatus(status) {
  current = status && typeof status === 'object' ? status : null;
  for (const listener of Array.from(listeners)) {
    listener(current);
  }
}

// Subscribes to status changes; returns the unsubscribe function.
export function subscribeUpdateStatus(listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

// What the board banner should show for a status (pure, for tests).
//
// The banner is the manual path: when the device installs updates on its own,
// the board stays silent (zero-click stays invisible). A downloaded update is
// only announced while it waits for the user, and « Plus tard » hides the
// version it was postponed for.
export function updateNotice(status, { autoUpdate = true, dismissedVersion = null } = {}) {
  if (!status || typeof status !== 'object') {
    return null;
  }
  const version =
    typeof status.availableVersion === 'string' && status.availableVersion.trim()
      ? status.availableVersion.trim()
      : null;
  if (!version) {
    return null;
  }
  if (status.state === 'ready' && !autoUpdate && version !== dismissedVersion) {
    return {
      kind: 'ready',
      version,
      message: `LiveChat ${version} est prête à être installée.`,
    };
  }
  if (status.state === 'downloading' && !autoUpdate) {
    return {
      kind: 'downloading',
      version,
      message: `Téléchargement de LiveChat ${version}…`,
    };
  }
  return null;
}
