import { describe, expect, it } from 'vitest';

import { inpageThemeStyles } from '../content/inpage-theme';
import { DEFAULT_THEME_PREFERENCE, normalizeThemePreference, resolveTheme } from './theme';

describe('préférence de thème', () => {
  it('retombe sur le sombre pour une valeur inconnue', () => {
    expect(DEFAULT_THEME_PREFERENCE).toBe('dark');

    for (const value of [undefined, null, '', 'Light', 'auto', 42]) {
      expect(normalizeThemePreference(value)).toBe('dark');
    }

    for (const value of ['dark', 'light', 'system']) {
      expect(normalizeThemePreference(value)).toBe(value);
    }
  });

  it('« Système » suit le navigateur, un choix explicite non', () => {
    expect(resolveTheme('dark', true)).toBe('dark');
    expect(resolveTheme('light', false)).toBe('light');
    expect(resolveTheme('system', true)).toBe('light');
    expect(resolveTheme('system', false)).toBe('dark');
    expect(resolveTheme('nonsense', true)).toBe('dark');
  });
});

describe('feuille claire des widgets injectés', () => {
  it('ne contient que des règles préfixées par html[data-lce-theme="light"]', () => {
    const selectors = Array.from(inpageThemeStyles.matchAll(/([^{}]+)\{/g), (match) => match[1])
      .flatMap((group) => group.split(','))
      .map((selector) => selector.trim())
      .filter(Boolean);

    expect(selectors.length).toBeGreaterThan(10);

    for (const selector of selectors) {
      expect(selector.startsWith("html[data-lce-theme='light'] ")).toBe(true);
    }
  });
});
