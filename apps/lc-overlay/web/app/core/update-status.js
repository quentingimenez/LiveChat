// Shared updater status (`settings.update`): the single source of truth for
// the board window.
//
// The shell reports the updater state inside the device settings: the
// `settings_get` command and the `overlay://device-settings` event both carry
// an `update` object (`{ state, currentVersion, availableVersion, message }`,
// see src/updater.rs). settings.js feeds this store; the Settings page and the
// board banner (views/update-notice.js) read the same value from here instead
// of registering their own listeners.
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

// Which update banner to show — pure. Returns `null` when the banner must
// stay hidden, else `{ kind, version, message }`:
// - `kind: 'ready'`: the version is downloaded and waiting. The banner offers
//   to install it, unless the user postponed exactly that version
//   (`dismissedVersion`, kept in localStorage): a newer version shows it again.
// - `kind: 'downloading'`: the version is known and still downloading. An
//   informational line without buttons; postponing only applies to the
//   install prompt, not to this transient state.
//
// Every other state (idle, checking, up_to_date, error, unknown) hides the
// banner, and a `ready` status without a version never shows a broken
// sentence.
export function updateNotice(status, dismissedVersion = null) {
  if (!status || typeof status !== 'object') {
    return null;
  }
  const version =
    typeof status.availableVersion === 'string' && status.availableVersion ? status.availableVersion : null;
  if (status.state === 'ready') {
    if (!version || version === dismissedVersion) {
      return null;
    }
    return { kind: 'ready', version, message: `LiveChat ${version} est prête à être installée.` };
  }
  if (status.state === 'downloading' && version) {
    return { kind: 'downloading', version, message: `Téléchargement de LiveChat ${version}…` };
  }
  return null;
}
