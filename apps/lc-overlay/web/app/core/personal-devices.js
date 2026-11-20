// Pure wording helpers of the personal device list in « Mon profil ».
// The server returns overlays, browser extensions and iOS ingest clients in
// one list; these helpers keep the labels testable outside the DOM.

import { lastSeenLabel, platformLabel, presenceLabel } from './device-helpers.js';

export const PERSONAL_DEVICE_KIND_LABELS = {
  overlay: 'PC',
  extension: 'Extension',
  ios: 'Raccourci iOS',
};

export function personalDeviceKindLabel(kind) {
  return PERSONAL_DEVICE_KIND_LABELS[kind] || 'Appareil';
}

// One line under the device name: platform/version/presence for an overlay,
// last use for an extension or iOS shortcut (they have no live socket).
export function personalDeviceSummary(device, now = Date.now()) {
  if (!device) {
    return '';
  }
  if (device.kind !== 'overlay') {
    return lastSeenLabel(device.lastSeenAt, now);
  }
  const parts = [];
  const platform = platformLabel(device.platform);
  if (platform) {
    parts.push(platform);
  }
  if (device.overlayVersion) {
    parts.push(`v${device.overlayVersion}`);
  }
  parts.push(presenceLabel(device, now));
  return parts.join(' · ');
}

export function personalDeviceCountLabel(count) {
  const total = Number.isInteger(count) && count > 0 ? count : 0;
  if (total === 0) {
    return 'Aucun appareil';
  }
  return `${total} appareil${total > 1 ? 's' : ''}`;
}

// The session displaying the dialog cannot revoke itself; the server enforces
// the same rule with 409 cannot_revoke_self.
export function canRevokePersonalDevice(device) {
  return Boolean(device && !device.isSelf);
}
