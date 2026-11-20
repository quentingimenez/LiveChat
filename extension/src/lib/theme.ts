// Thème de l'extension : popup, page d'options et widgets injectés.
//
// La préférence vit dans `chrome.storage.local` (partagée par les trois
// contextes) et elle est recopiée dans le localStorage de chaque page : le
// script `theme-guard.js`, chargé en tête du popup et des options, la relit
// avant le premier rendu pour éviter un flash sombre en thème clair.
//
// Trois choix : « Sombre » (défaut), « Clair », « Système » (suit le thème du
// navigateur via `prefers-color-scheme`).

export const THEME_STORAGE_KEY = 'lce.theme.v1';
const THEME_ATTRIBUTE = 'data-lce-theme';
const PREFERS_LIGHT_QUERY = '(prefers-color-scheme: light)';

export type ThemePreference = 'dark' | 'light' | 'system';
export type ResolvedTheme = 'dark' | 'light';

export const THEME_PREFERENCES: readonly ThemePreference[] = ['dark', 'light', 'system'];
export const DEFAULT_THEME_PREFERENCE: ThemePreference = 'dark';

export const normalizeThemePreference = (value: unknown): ThemePreference => {
  return typeof value === 'string' && (THEME_PREFERENCES as readonly string[]).includes(value)
    ? (value as ThemePreference)
    : DEFAULT_THEME_PREFERENCE;
};

export const resolveTheme = (preference: unknown, prefersLight: boolean): ResolvedTheme => {
  const normalized = normalizeThemePreference(preference);

  if (normalized === 'system') {
    return prefersLight ? 'light' : 'dark';
  }

  return normalized;
};

// Thème du navigateur ; hors navigateur (tests unitaires), le défaut sombre.
export const systemPrefersLight = (): boolean => {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false;
  }

  return window.matchMedia(PREFERS_LIGHT_QUERY).matches;
};

const readStorage = async (): Promise<unknown> => {
  if (typeof chrome !== 'undefined' && chrome.storage?.local) {
    try {
      const payload = await chrome.storage.local.get(THEME_STORAGE_KEY);
      return payload[THEME_STORAGE_KEY];
    } catch {
      // storage indisponible : on retombe sur le miroir local.
    }
  }

  if (typeof window === 'undefined') {
    return undefined;
  }

  try {
    return window.localStorage.getItem(THEME_STORAGE_KEY);
  } catch {
    return undefined;
  }
};

export const readThemePreference = async (): Promise<ThemePreference> => {
  return normalizeThemePreference(await readStorage());
};

export const saveThemePreference = async (preference: ThemePreference): Promise<void> => {
  const normalized = normalizeThemePreference(preference);

  if (typeof chrome !== 'undefined' && chrome.storage?.local) {
    await chrome.storage.local.set({ [THEME_STORAGE_KEY]: normalized });
  }

  if (typeof window === 'undefined') {
    return;
  }

  try {
    // Miroir lu par theme-guard.js avant le premier rendu de la page.
    window.localStorage.setItem(THEME_STORAGE_KEY, normalized);
  } catch {
    // Mode privé ou quota : la préférence reste appliquée par le storage.
  }
};

export const applyResolvedTheme = (theme: ResolvedTheme): void => {
  if (typeof document === 'undefined' || !document.documentElement) {
    return;
  }

  document.documentElement.setAttribute(THEME_ATTRIBUTE, theme === 'light' ? 'light' : 'dark');
};

// Applique la préférence (relue du storage quand elle n'est pas fournie).
export const applyThemePreference = async (preference?: ThemePreference): Promise<ResolvedTheme> => {
  const resolved = resolveTheme(
    preference ?? (await readThemePreference()),
    systemPrefersLight(),
  );
  applyResolvedTheme(resolved);
  return resolved;
};

// Réagit aux bascules clair/sombre du navigateur. Un choix explicite ignore le
// média ; seul « Système » est concerné.
export const watchSystemTheme = (onChange: () => void): (() => void) => {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return () => undefined;
  }

  const query = window.matchMedia(PREFERS_LIGHT_QUERY);
  const listener = (): void => {
    void (async () => {
      const preference = await readThemePreference();

      if (preference !== 'system') {
        return;
      }

      applyResolvedTheme(resolveTheme(preference, query.matches));
      onChange();
    })();
  };

  if (typeof query.addEventListener === 'function') {
    query.addEventListener('change', listener);
    return () => query.removeEventListener('change', listener);
  }

  return () => undefined;
};

// Démarrage : applique la préférence enregistrée, puis suit le système et les
// changements faits depuis une autre page de l'extension.
export const initTheme = async (): Promise<ResolvedTheme> => {
  const resolved = await applyThemePreference();

  watchSystemTheme(() => undefined);

  if (typeof chrome !== 'undefined' && chrome.storage?.onChanged) {
    chrome.storage.onChanged.addListener((changes, areaName) => {
      if (areaName !== 'local' || !(THEME_STORAGE_KEY in changes)) {
        return;
      }

      void applyThemePreference(normalizeThemePreference(changes[THEME_STORAGE_KEY]?.newValue));
    });
  }

  return resolved;
};
