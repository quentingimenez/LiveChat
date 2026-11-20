// Board theme (dark/light): the renderer owns it.
//
// The board is a local page: its theme is a display preference of this
// machine, so it lives in the WebView's localStorage instead of the shell
// config — no round trip through Rust, nothing published to the stream. The
// `data-theme` attribute on <html> switches the token set declared in
// styles/theme-light.css; the dark theme is the default and needs no file.
//
// Three choices: « Sombre », « Clair », « Système » (follows the Windows
// theme through `prefers-color-scheme`).
//
// No import: this module also loads under Node for its unit tests.

export const THEME_STORAGE_KEY = 'livechat.theme';
export const THEME_ATTRIBUTE = 'data-theme';
export const THEME_PREFERENCES = ['dark', 'light', 'system'];
export const DEFAULT_THEME_PREFERENCE = 'dark';

const PREFERS_LIGHT_QUERY = '(prefers-color-scheme: light)';

// Theme actually painted on <html> (`dark` until a theme is applied).
let current = 'dark';
const listeners = new Set();

function notify() {
  for (const listener of Array.from(listeners)) {
    listener(current);
  }
}

// Unknown or missing values fall back to the default: an old profile, a
// hand-edited localStorage entry or a value from another version never
// produces an unstyled board.
export function normalizeThemePreference(value) {
  return THEME_PREFERENCES.includes(value) ? value : DEFAULT_THEME_PREFERENCE;
}

// Which theme a preference means — pure. `prefersLight` is the OS answer at
// the time of the call.
export function resolveTheme(preference, prefersLight) {
  const normalized = normalizeThemePreference(preference);
  if (normalized === 'system') {
    return prefersLight ? 'light' : 'dark';
  }
  return normalized;
}

// Whether Windows is currently in its light theme. Outside a browser (unit
// tests) there is no media query: the answer is the dark default.
export function systemPrefersLight() {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false;
  }
  return window.matchMedia(PREFERS_LIGHT_QUERY).matches;
}

// Star, or the « Système » resolution as last applied.
export function resolvedTheme() {
  return current;
}

// Stored preference; `dark` when localStorage is unavailable (private mode).
export function currentThemePreference() {
  try {
    return normalizeThemePreference(window.localStorage.getItem(THEME_STORAGE_KEY));
  } catch {
    return DEFAULT_THEME_PREFERENCE;
  }
}

// Paints `theme` on <html> and remembers it for the subscribers.
export function applyTheme(theme) {
  current = theme === 'light' ? 'light' : 'dark';
  if (typeof document !== 'undefined' && document.documentElement) {
    document.documentElement.setAttribute(THEME_ATTRIBUTE, current);
  }
  notify();
  return current;
}

// Applies and persists a preference. Returns the theme it resolved to.
export function setThemePreference(preference) {
  const normalized = normalizeThemePreference(preference);
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, normalized);
  } catch {
    // Quota or private mode: the choice still applies to this session.
  }
  return applyTheme(resolveTheme(normalized, systemPrefersLight()));
}

// Subscribes to theme changes; returns the unsubscribe function.
export function subscribeTheme(listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

// Re-resolves the stored preference whenever Windows switches theme. Only
// « Système » is affected; an explicit choice ignores the media query.
function watchSystemTheme() {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return;
  }
  const query = window.matchMedia(PREFERS_LIGHT_QUERY);
  const onChange = () => {
    if (currentThemePreference() !== 'system') {
      return;
    }
    applyTheme(resolveTheme('system', query.matches));
  };
  if (typeof query.addEventListener === 'function') {
    query.addEventListener('change', onChange);
  }
}

// Boots the theme: applies the stored preference before the first render and
// keeps « Système » in step with Windows.
export function initTheme() {
  applyTheme(resolveTheme(currentThemePreference(), systemPrefersLight()));
  watchSystemTheme();
  return current;
}
