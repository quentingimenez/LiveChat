// Thème clair de la board : préférence locale (core/theme.js), feuille de
// tokens clairs (styles/theme-light.css) et injection dans la page Paramètres.
//
// Les valeurs claires ne sont pas recopiées ici : les tests vérifient les
// invariants (règle portant `data-theme`, tokens utilisés déclarés, plus aucun
// littéral de couleur thémable), pas la teinte exacte.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  DEFAULT_THEME_PREFERENCE,
  normalizeThemePreference,
  resolveTheme,
} from '../../web/app/core/theme.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Declarations of the rule whose selector is exactly `selector`.
function ruleOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    if (match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim() === selector) {
      return match[2];
    }
  }
  return null;
}

// Every custom property declared by a stylesheet.
function declaredTokens(css) {
  return new Set(Array.from(css.matchAll(/(--[a-z0-9-]+)\s*:/g), (match) => match[1]));
}

// base.css declares the tokens: it is where the dark values live. Every other
// feature stylesheet must go through them.
const STYLESHEETS = [
  'board.css',
  'admin.css',
  'queue.css',
  'dialog.css',
  'contextmenu.css',
  'toast.css',
  'say.css',
  'settings.css',
  'editor.css',
];

// Colours that moved into tokens when the light theme was added. They may
// still appear in base.css (where the tokens are declared) and in
// theme-light.css, nowhere else.
const THEMED_LITERALS = [
  '#f7c466',
  'rgba(23, 27, 37',
  '#4a2c2c',
  '#f0b4b4',
  '#7fd1a2',
  '#101a33',
  '#9bbcf8',
  '#2a3040',
  '#222736',
  '#11141c',
];

test('the stored preference falls back to dark, never to an unstyled board', () => {
  assert.equal(DEFAULT_THEME_PREFERENCE, 'dark');
  for (const value of [undefined, null, '', 'Light', 'auto', 42]) {
    assert.equal(normalizeThemePreference(value), DEFAULT_THEME_PREFERENCE);
  }
  for (const value of ['dark', 'light', 'system']) {
    assert.equal(normalizeThemePreference(value), value);
  }
});

test('an explicit choice ignores Windows, « Système » follows it', () => {
  assert.equal(resolveTheme('dark', true), 'dark');
  assert.equal(resolveTheme('light', false), 'light');
  assert.equal(resolveTheme('system', true), 'light');
  assert.equal(resolveTheme('system', false), 'dark');
  assert.equal(resolveTheme('nonsense', true), 'dark');
});

test('the light theme is a token swap scoped to data-theme="light"', () => {
  const css = read('styles/theme-light.css');
  const light = ruleOf(css, ":root[data-theme='light']");
  assert.ok(light, "theme-light.css must scope its palette to :root[data-theme='light']");
  assert.match(light, /color-scheme:\s*light/);
  for (const token of ['--slate-900', '--slate-800', '--text', '--line', '--amber-text', '--accent-ink', '--scrim']) {
    assert.match(light, new RegExp(`${token}\\s*:`), `${token} must be redefined by the light theme`);
  }
});

test('every token used by the light sheet is declared somewhere', () => {
  const light = read('styles/theme-light.css');
  const known = new Set([...declaredTokens(read('styles/base.css')), ...declaredTokens(light)]);
  for (const match of light.matchAll(/var\((--[a-z0-9-]+)\)/g)) {
    assert.ok(known.has(match[1]), `${match[1]} is used by theme-light.css but never declared`);
  }
});

test('the board still boots dark', () => {
  const root = ruleOf(read('styles/base.css'), ':root');
  assert.ok(root, 'base.css must declare its palette on :root');
  assert.match(root, /color-scheme:\s*dark/);
});

test('the themeable colours are tokens, not hardcoded values', () => {
  for (const name of STYLESHEETS) {
    const css = read(`styles/${name}`).toLowerCase();
    for (const literal of THEMED_LITERALS) {
      assert.ok(!css.includes(literal), `${name} still hardcodes ${literal}`);
    }
  }
});

test('the modal veils are tokens (the crop mask stays a media-editor literal)', () => {
  assert.match(ruleOf(read('styles/dialog.css'), '.backdrop'), /background:\s*var\(--veil\)/);
  assert.match(ruleOf(read('styles/editor.css'), '.editor-veil'), /background:\s*var\(--veil\)/);
});

test('the light sheet loads last so its rules win the cascade', () => {
  const imports = Array.from(
    read('app.css').matchAll(/@import url\('\.\/styles\/([a-z-]+\.css)'\);/g),
    (match) => match[1],
  );
  assert.ok(imports.includes('base.css'));
  assert.equal(imports[imports.length - 1], 'theme-light.css');
});

test('the board applies its theme before the first render', () => {
  const main = read('main.js');
  assert.match(main, /import \{ initTheme \} from '\.\/core\/theme\.js';/);
  const boot = main.indexOf('initTheme();');
  assert.ok(boot > 0, 'main.js must call initTheme()');
  assert.ok(boot < main.indexOf('renderRail('), 'initTheme() must run before the rail is rendered');
});

test('Paramètres offers the three choices and applies them at once', () => {
  const settings = read('views/settings.js');
  assert.match(settings, /id: 'settings-theme'/);
  for (const label of ['Sombre', 'Clair', 'Système']) {
    assert.match(settings, new RegExp(`'${label}'`));
  }
  assert.match(settings, /setThemePreference\(event\.target\.value\)/);
  const order = ['renderStartup(settings)', 'renderAppearance()', 'renderAbout(settings)'].map((call) =>
    settings.indexOf(call),
  );
  assert.ok(order.every((index) => index > 0));
  assert.deepEqual([...order].sort((a, b) => a - b), order, 'Apparence sits between Démarrage and À propos');
});

test('the About logo follows the surface', () => {
  assert.match(read('assets/logo.svg'), /stroke="#E9EBF3"/);
  assert.match(read('assets/logo-light.svg'), /stroke="#171B25"/);
  assert.match(read('views/settings.js'), /resolvedTheme\(\) === 'light' \? LOGO_LIGHT_URL : LOGO_URL/);
});
