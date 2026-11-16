const GIB = 1024 * 1024 * 1024;
const MIB = 1024 * 1024;

/** French "octets" formatter used by the mockup (Mo / Go, decimal comma). */
export function bytes(value: number | null | undefined): string {
  const safe = typeof value === 'number' && Number.isFinite(value) ? Math.max(0, value) : 0;
  if (safe >= GIB) {
    return `${(safe / GIB).toFixed(1).replace('.', ',')} Go`;
  }
  return `${Math.round(safe / MIB)} Mo`;
}

/** `count one` / `count many` with a French plural rule. */
export function plural(count: number, one: string, many: string): string {
  return `${count} ${count > 1 ? many : one}`;
}

/**
 * Relative French time from an ISO date. `now` is injectable so tests do not
 * depend on the wall clock.
 */
export function ago(iso: string | null | undefined, now: number = Date.now()): string {
  if (!iso) {
    return 'jamais';
  }
  const timestamp = Date.parse(iso);
  if (!Number.isFinite(timestamp)) {
    return 'jamais';
  }
  const minutes = Math.max(0, Math.round((now - timestamp) / 60_000));
  if (minutes < 1) {
    return 'à l’instant';
  }
  if (minutes < 60) {
    return `il y a ${minutes} min`;
  }
  if (minutes < 60 * 48) {
    return `il y a ${Math.round(minutes / 60)} h`;
  }
  return `il y a ${Math.round(minutes / 1440)} j`;
}

/** Expiry delay shown in code tables (`jamais`, `expiré`, `dans 3 h`). */
export function expiresIn(iso: string | null | undefined, now: number = Date.now()): string {
  if (!iso) {
    return 'jamais';
  }
  const timestamp = Date.parse(iso);
  if (!Number.isFinite(timestamp)) {
    return 'jamais';
  }
  const minutes = Math.round((timestamp - now) / 60_000);
  if (minutes <= 0) {
    return 'expiré';
  }
  if (minutes < 60) {
    return `dans ${minutes} min`;
  }
  if (minutes < 60 * 48) {
    return `dans ${Math.round(minutes / 60)} h`;
  }
  return `dans ${Math.round(minutes / 1440)} j`;
}

/** Initials for the avatar stand-ins (Discord images stay optional in v2). */
export function initials(name: string | null | undefined): string {
  const cleaned = String(name ?? '')
    .replace(/[^\p{L}\p{N} ]/gu, '')
    .split(' ')
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? '')
    .join('');
  return cleaned;
}

/** Loose semver comparison for the overlay minimum version. */
export function versionBelow(version: string | null | undefined, minimum: string): boolean {
  if (!version) {
    return true;
  }
  const left = version.split('.').map((part) => Number.parseInt(part, 10) || 0);
  const right = minimum.split('.').map((part) => Number.parseInt(part, 10) || 0);
  for (let index = 0; index < 3; index += 1) {
    const a = left[index] ?? 0;
    const b = right[index] ?? 0;
    if (a !== b) {
      return a < b;
    }
  }
  return false;
}

export function percent(used: number, maximum: number): number {
  if (!Number.isFinite(used) || !Number.isFinite(maximum) || maximum <= 0) {
    return 0;
  }
  return Math.min(100, Math.max(0, Math.round((used / maximum) * 100)));
}

export function duration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) {
    return '—';
  }
  const whole = Math.round(seconds);
  if (whole < 60) {
    return `${whole} s`;
  }
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  if (minutes < 60) {
    return rest ? `${minutes} min ${String(rest).padStart(2, '0')} s` : `${minutes} min`;
  }
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return remainingMinutes ? `${hours} h ${String(remainingMinutes).padStart(2, '0')} min` : `${hours} h`;
}

const DATE_TIME_FORMAT = new Intl.DateTimeFormat('fr-FR', {
  dateStyle: 'short',
  timeStyle: 'medium',
});

export function dateTime(iso: string | null | undefined): string {
  if (!iso) {
    return '—';
  }
  const timestamp = Date.parse(iso);
  if (!Number.isFinite(timestamp)) {
    return '—';
  }
  return DATE_TIME_FORMAT.format(new Date(timestamp));
}

/** Code display: four characters, dash, the rest (mockup invite style). */
export function codeDisplay(code: string): string {
  if (code.length <= 4) {
    return code;
  }
  return `${code.slice(0, 4)}-${code.slice(4)}`;
}

export function number(value: number | null | undefined): string {
  return new Intl.NumberFormat('fr-FR').format(value ?? 0);
}

/** Compact server uptime (`6 j 4 h`, `3 h 12 min`, `42 s`). */
export function uptime(totalSeconds: number | null | undefined): string {
  const seconds = Math.max(0, Math.floor(totalSeconds ?? 0));
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) {
    return `${days} j ${hours} h`;
  }
  if (hours > 0) {
    return `${hours} h ${minutes} min`;
  }
  if (minutes > 0) {
    return `${minutes} min`;
  }
  return `${seconds} s`;
}
