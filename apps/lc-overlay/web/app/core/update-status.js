// Shared updater status (`settings.update`): the single source of truth for
// the Settings page.
//
// The shell reports the updater state inside the device settings: the
// `settings_get` command and the `overlay://device-settings` event both carry
// an `update` object (`{ state, currentVersion, availableVersion, message }`,
// see src/updater.rs). settings.js feeds this store and reads it back for the
// « À propos » section; no other view shows update state.
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
