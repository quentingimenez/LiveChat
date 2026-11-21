// Éditeur de média — champs de découpe et frise.
//
// Deux régressions que ce fichier protège :
//  - les champs début/fin de « Découper » étaient des inputs de 96 px posés
//    dans une `.row` : ils ne prenaient pas la largeur de la section,
//    contrairement aux boutons « Début ici » / « Fin ici » ;
//  - la frise est un canvas peint avec des couleurs sombres recopiées dans
//    av.js, donc la piste audio restait noire avec le thème clair.
//
// Les valeurs ne sont pas recopiées : le test lit les feuilles de style et la
// source de l'éditeur, et vérifie les invariants (grille pleine largeur,
// couleurs en tokens, thème dans la clé de repeinture, contraste des traits).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const av = read('views/editor/av.js');
const editor = read('styles/editor.css');
const base = read('styles/base.css');
const themeLight = read('styles/theme-light.css');

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

function tokensOf(css, selector) {
  const rule = ruleOf(css, selector);
  assert.ok(rule, `règle ${selector} introuvable`);
  const tokens = new Map();
  for (const match of rule.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) {
    tokens.set(match[1], match[2].trim());
  }
  return tokens;
}

function channel(value) {
  const c = value / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  const value = String(hex).trim().replace('#', '');
  assert.match(value, /^[0-9a-f]{6}$/i, `couleur attendue en #rrggbb : ${hex}`);
  const channels = [0, 2, 4].map((offset) => channel(parseInt(value.slice(offset, offset + 2), 16)));
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(foreground, background) {
  const one = luminance(foreground);
  const two = luminance(background);
  return (Math.max(one, two) + 0.05) / (Math.min(one, two) + 0.05);
}

const TL_TOKENS = [
  '--tl-bg',
  '--tl-wave',
  '--tl-removed',
  '--tl-removed-hatch',
  '--tl-edge',
  '--tl-edge-grip',
  '--tl-selection',
  '--tl-selection-edge',
  '--tl-head',
];

test('les champs de découpe occupent la largeur de la section', () => {
  const times = ruleOf(editor, '.editor-times');
  assert.ok(times, 'editor.css doit définir .editor-times');
  assert.match(times, /display:\s*grid/);
  assert.match(
    times,
    /grid-template-columns:\s*minmax\(0,\s*1fr\)\s+auto\s+minmax\(0,\s*1fr\)/,
    'les deux champs doivent occuper les deux colonnes, la flèche au milieu',
  );

  const field = ruleOf(editor, '.field.tl-time');
  assert.ok(field, 'editor.css doit définir .field.tl-time');
  assert.match(field, /width:\s*100%/, 'le champ doit remplir sa colonne');
  assert.match(field, /text-align:\s*center/, 'le temps doit rester centré dans le champ');

  assert.match(av, /class: 'editor-times'/);
  assert.match(av, /class: 'editor-times-arrow', 'aria-hidden': 'true'/);
  assert.match(av, /class: 'field tl-time'/);
  assert.doesNotMatch(
    av,
    /class: 'row' \}, startField/,
    'les champs ne doivent plus être posés dans une .row de largeur libre',
  );
});

test('la frise lit ses couleurs dans les tokens du thème', () => {
  assert.match(av, /getComputedStyle\(track\)/, 'av.js doit lire les tokens de la piste');
  for (const token of TL_TOKENS) {
    assert.ok(av.includes(token), `av.js doit lire ${token}`);
  }
  assert.doesNotMatch(
    av,
    /#[0-9a-f]{3,8}\b|rgba?\(/i,
    'les couleurs de la frise doivent venir des feuilles de style',
  );
});

test('le thème clair redéfinit toutes les couleurs de la frise', () => {
  const dark = tokensOf(base, ':root');
  const light = tokensOf(themeLight, ":root[data-theme='light']");
  for (const token of TL_TOKENS) {
    assert.ok(dark.has(token), `base.css doit déclarer ${token}`);
    assert.ok(light.has(token), `theme-light.css doit surcharger ${token}`);
    assert.notEqual(light.get(token), dark.get(token), `${token} doit changer avec le thème`);
  }
  assert.match(ruleOf(editor, '.tl-track'), /background:\s*var\(--tl-bg\)/);
});

test('la frise se repeint quand le thème change', () => {
  assert.match(av, /document\.documentElement\.dataset\.theme/);
  assert.match(av, /const key = \[width, theme,/, 'le thème doit faire partie de la clé de repeinture');
});

const darkTokens = tokensOf(base, ':root');
const lightTokens = new Map([...darkTokens, ...tokensOf(themeLight, ":root[data-theme='light']")]);

// Traits dessinés sur le fond de la piste : 3:1 (WCAG 1.4.11).
const TL_PAIRS = [
  ['--tl-wave', '--tl-bg', 'forme d’onde'],
  ['--tl-edge', '--tl-bg', 'bords clairs de la découpe'],
  ['--tl-selection-edge', '--tl-bg', 'bords de la sélection'],
  ['--tl-head', '--tl-bg', 'tête de lecture'],
];

for (const [name, tokens] of [
  ['sombre', darkTokens],
  ['clair', lightTokens],
]) {
  test(`frise : les traits restent lisibles sur le fond (${name})`, () => {
    for (const [foreground, background, purpose] of TL_PAIRS) {
      const ratio = contrast(tokens.get(foreground), tokens.get(background));
      assert.ok(
        ratio >= 3,
        `${name} : ${purpose} = ${ratio.toFixed(2)}:1, sous 3:1`,
      );
    }
  });
}
