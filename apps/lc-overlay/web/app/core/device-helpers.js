// Pure helpers of the administration « Appareils » list: version comparison
// against the newest device seen, presence wording and labels.

export const SESSION_MODE_LABELS = {
  normal: 'Contrôle complet',
  invite_read_only: 'Lecture seule',
};

export const PLATFORM_LABELS = {
  windows: 'Windows',
  linux: 'Linux',
  macos: 'macOS',
};

export function sessionModeLabel(mode) {
  return SESSION_MODE_LABELS[mode] || mode || '—';
}

export function platformLabel(platform) {
  if (!platform) {
    return '';
  }
  return PLATFORM_LABELS[platform] || platform;
}

// Numeric components of a version string, or null when none can be read.
export function parseVersion(version) {
  if (typeof version !== 'string' || !version.trim()) {
    return null;
  }
  const parts = version.trim().split('.').map((part) => Number.parseInt(part, 10));
  if (!parts.every((part) => Number.isInteger(part) && part >= 0)) {
    return null;
  }
  return parts;
}

// -1, 0 or 1; null when one of the versions cannot be read. Missing
// components count as zero ("3.7" and "3.7.0" are equal).
export function compareVersions(left, right) {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) {
    return null;
  }
  const length = Math.max(a.length, b.length, 3);
  for (let index = 0; index < length; index += 1) {
    const x = a[index] ?? 0;
    const y = b[index] ?? 0;
    if (x !== y) {
      return x < y ? -1 : 1;
    }
  }
  return 0;
}

// Newest readable overlay version of the list, or null.
export function newestVersion(devices) {
  let newest = null;
  for (const device of devices || []) {
    const version = device && device.overlayVersion;
    if (!parseVersion(version)) {
      continue;
    }
    if (newest === null || compareVersions(version, newest) > 0) {
      newest = version;
    }
  }
  return newest;
}

// A device is obsolete when it runs an older version than the newest one seen
// in the list. An unknown version is never flagged.
export function isObsolete(version, newest) {
  if (!parseVersion(version) || !parseVersion(newest)) {
    return false;
  }
  return compareVersions(version, newest) < 0;
}

export function lastSeenLabel(iso, now = Date.now()) {
  if (!iso) {
    return 'Jamais vu';
  }
  const timestamp = new Date(iso).getTime();
  if (!Number.isFinite(timestamp)) {
    return 'Jamais vu';
  }
  const minutes = Math.max(0, Math.round((now - timestamp) / 60000));
  if (minutes < 2) {
    return 'Vu à l’instant';
  }
  if (minutes < 60) {
    return `Vu il y a ${minutes} min`;
  }
  if (minutes < 1440) {
    return `Vu il y a ${Math.round(minutes / 60)} h`;
  }
  return `Vu il y a ${Math.round(minutes / 1440)} j`;
}

export function presenceLabel(device, now = Date.now()) {
  if (!device) {
    return '—';
  }
  if (device.connected) {
    return 'En ligne';
  }
  if (device.guildSettings && device.guildSettings.paused) {
    return 'Appareil coupé';
  }
  return lastSeenLabel(device.lastSeenAt, now);
}
