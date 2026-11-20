// Contraste des tokens de l'interface, dans les deux thèmes.
//
// Les couleurs ne sont pas recopiées : le test lit les valeurs déclarées par
// base.css (thème sombre) et theme-light.css (surcharges claires), puis
// applique le calcul WCAG 2.2. Deux familles :
//  - texte (12–14 px) : 4,5:1 (SC 1.4.3) ;
//  - bordure des champs : 3:1 (SC 1.4.11), des deux côtés du trait (fond du
//    champ et surface qui le porte).
//
// Ce test mord : remettre l'ancien --text-faint (#717a93 en sombre, #6f7a90 en
// clair) le fait échouer, et remettre l'ancienne bordure --line (#3a4259 en
// sombre, #d5dce8 en clair) dans --field-line aussi.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

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

const dark = tokensOf(read('styles/base.css'), ':root');
const light = new Map([...dark, ...tokensOf(read('styles/theme-light.css'), ":root[data-theme='light']")]);

// [texte, fond, pourquoi] — les couples réellement posés par les composants.
const PAIRS = [
  ['--text', '--slate-900', 'texte principal sur la surface de page'],
  ['--text', '--slate-800', 'texte principal sur les cartes'],
  ['--text', '--slate-700', 'texte principal sur les surfaces surélevées'],
  ['--text', '--slate-600', 'texte principal sur les boutons'],
  ['--text-soft', '--slate-900', 'texte secondaire sur la page'],
  ['--text-soft', '--slate-800', 'texte secondaire sur les cartes'],
  ['--text-soft', '--slate-700', 'texte secondaire sur les surfaces surélevées'],
  ['--text-faint', '--slate-900', 'texte discret sur la page'],
  ['--text-faint', '--slate-800', 'texte discret sur les cartes'],
  ['--text-faint', '--slate-700', 'texte discret sur les surfaces surélevées'],
  ['--amber-text', '--slate-800', 'accent ambre en texte'],
  ['--amber-text', '--slate-900', 'accent ambre sur la page'],
  ['--red-text', '--slate-800', 'danger en texte'],
  ['--green-text', '--slate-800', 'succès en texte'],
  ['--blue-text', '--slate-800', 'information en texte'],
  ['--amber-ink', '--amber', 'texte sur un bouton ambre'],
  ['--accent-ink', '--blue', 'texte sur un bouton primaire'],
  ['--badge-admin-text', '--badge-admin-bg', 'badge administrateur'],
  ['--badge-guest-text', '--badge-guest-bg', 'badge invité'],
];

// [bordure, fond, pourquoi] — la bordure d'un champ doit tenir 3:1 des deux
// côtés du trait : sur le fond du champ et sur la surface qui le porte.
const FIELD_PAIRS = [
  ['--field-line', '--slate-900', 'bordure de champ sur le fond du champ'],
  ['--field-line', '--slate-800', 'bordure de champ sur une carte ou un dialogue'],
  ['--field-line', '--slate-700', 'bordure de champ sur une surface surélevée'],
];

function assertTheme(name, tokens) {
  for (const [foreground, background, purpose] of PAIRS) {
    const fg = tokens.get(foreground);
    const bg = tokens.get(background);
    assert.ok(fg, `${name} : ${foreground} n'est pas déclaré`);
    assert.ok(bg, `${name} : ${background} n'est pas déclaré`);
    const ratio = contrast(fg, bg);
    assert.ok(
      ratio >= 4.5,
      `${name} : ${foreground} (${fg}) sur ${background} (${bg}) = ${ratio.toFixed(2)}:1, ` +
        `sous 4,5:1 — ${purpose}`,
    );
  }
}

function assertFieldBoundaries(name, tokens) {
  for (const [border, background, purpose] of FIELD_PAIRS) {
    const line = tokens.get(border);
    const bg = tokens.get(background);
    assert.ok(line, `${name} : ${border} n'est pas déclaré`);
    assert.ok(bg, `${name} : ${background} n'est pas déclaré`);
    const ratio = contrast(line, bg);
    assert.ok(
      ratio >= 3,
      `${name} : ${border} (${line}) sur ${background} (${bg}) = ${ratio.toFixed(2)}:1, ` +
        `sous 3:1 — ${purpose}`,
    );
  }
}

test('thème sombre : chaque couple texte/fond atteint 4,5:1', () => {
  assertTheme('sombre', dark);
});

test('thème clair : chaque couple texte/fond atteint 4,5:1', () => {
  assertTheme('clair', light);
});

test('le thème clair surcharge bien les tokens de texte', () => {
  for (const token of ['--text', '--text-soft', '--text-faint', '--amber-text', '--red-text', '--green-text', '--blue-text']) {
    assert.notEqual(light.get(token), dark.get(token), `${token} doit être redéfini par le thème clair`);
  }
});

test('thème sombre : les bordures de champ atteignent 3:1', () => {
  assertFieldBoundaries('sombre', dark);
});

test('thème clair : les bordures de champ atteignent 3:1', () => {
  assertFieldBoundaries('clair', light);
});

test('le thème clair surcharge aussi la bordure des champs', () => {
  assert.notEqual(light.get('--field-line'), dark.get('--field-line'), '--field-line doit être redéfini par le thème clair');
});

// Sans ce branchement, les couples ci-dessus passeraient alors que les champs
// gardent l'ancienne bordure --line.
test('les contrôles de saisie peignent leur bordure avec --field-line', () => {
  const css = read('styles/base.css');
  for (const selector of ['.field', ".switch input[type='checkbox']"]) {
    const rule = ruleOf(css, selector);
    assert.ok(rule, `règle ${selector} introuvable`);
    assert.match(rule, /border[^;]*var\(--field-line\)/, `${selector} doit utiliser --field-line pour sa bordure`);
  }
});
