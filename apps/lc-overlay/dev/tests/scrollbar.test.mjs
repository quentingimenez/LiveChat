// Barre de défilement « Atelier » : les conteneurs de l'overlay reçoivent une
// barre fine, arrondie et tokenisée à la place de la barre native de WebView2.
//
// Le test lit le CSS source (pas de DOM dans les tests de l'overlay) : la
// sélection des conteneurs, la géométrie du pouce, les tokens des deux thèmes,
// et les surfaces qui doivent garder leur barre masquée (profil de guilde,
// colonne d'aperçu).
//
// Il mord : sans la règle ::-webkit-scrollbar-button, les flèches natives
// reviennent ; un scrollbar-color ajouté désactive tous les pseudo-éléments ;
// un token remplacé par un hex casse le thème clair.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const CONTAINERS = [
  '.view',
  '.rail',
  '.dialog:not(.is-profile) .dialog-body',
  '.file-list',
  '.editor-rail',
  '.editor-panel',
  '.display-preview-scroll',
];

const BAR = `:is(${CONTAINERS.join(', ')})`;

// Declarations of the rule whose selector is exactly `selector`.
function declarationsOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    const found = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .replace(/\s+/g, ' ')
      .trim();
    if (found === selector) {
      return match[2];
    }
  }
  assert.fail(`règle ${selector} introuvable`);
}

const css = read('styles/scrollbar.css');
const code = css.replace(/\/\*[\s\S]*?\*\//g, '');

test('la barre Atelier couvre les sept conteneurs défilants', () => {
  for (const container of CONTAINERS) {
    assert.ok(code.includes(container), `${container} doit faire partie de la sélection`);
  }
});

test("la voie fait 12 px, sans fond, et le pouce suit les tokens du thème", () => {
  const bar = declarationsOf(css, `${BAR}::-webkit-scrollbar`);
  assert.match(bar, /width:\s*12px/);
  assert.match(bar, /height:\s*12px/);
  assert.match(bar, /background:\s*transparent/);

  const track = declarationsOf(css, `${BAR}::-webkit-scrollbar-track`);
  assert.match(track, /background:\s*transparent/);

  const thumb = declarationsOf(css, `${BAR}::-webkit-scrollbar-thumb`);
  assert.match(thumb, /background:\s*var\(--field-line\)/);
  assert.match(thumb, /border:\s*3px solid transparent/);
  assert.match(thumb, /border-radius:\s*999px/);
  assert.match(thumb, /background-clip:\s*padding-box/);
  assert.match(thumb, /min-height:\s*40px/);
  assert.match(thumb, /min-width:\s*40px/);
});

test("le pouce s'épaissit et s'éclaire au survol puis à la saisie", () => {
  const hover = declarationsOf(css, `${BAR}::-webkit-scrollbar-thumb:hover`);
  assert.match(hover, /background:\s*var\(--text-faint\)/);
  assert.match(hover, /background-clip:\s*padding-box/);
  assert.match(hover, /border-width:\s*2px/);

  const active = declarationsOf(css, `${BAR}::-webkit-scrollbar-thumb:active`);
  assert.match(active, /background:\s*var\(--text-soft\)/);
  assert.match(active, /background-clip:\s*padding-box/);
  assert.match(active, /border-width:\s*2px/);
});

test('les flèches et le coin natifs ne sont plus dessinés', () => {
  const button = declarationsOf(css, `${BAR}::-webkit-scrollbar-button`);
  assert.match(button, /display:\s*none/);

  const corner = declarationsOf(css, `${BAR}::-webkit-scrollbar-corner`);
  assert.match(corner, /background:\s*transparent/);
});

test('aucune propriété standard ne réactive la barre native', () => {
  assert.doesNotMatch(code, /scrollbar-(?:color|width)\s*:/);
});

test('les couleurs restent des tokens, jamais des hex', () => {
  assert.doesNotMatch(code, /#[0-9a-f]{3,8}\b/i);
});

test('le profil et la colonne d’aperçu restent hors de la sélection', () => {
  assert.match(code, /\.dialog:not\(\.is-profile\) \.dialog-body/);
  assert.doesNotMatch(code, /\.dialog\.is-profile/);
  assert.doesNotMatch(code, /\.hp-page-info/);
});

test('les deux surfaces masquées gardent leur barre masquée', () => {
  const dialog = read('styles/dialog.css');
  const profile = declarationsOf(dialog, '.dialog.is-profile .dialog-body');
  assert.match(profile, /scrollbar-width:\s*none/);

  const mediaDetail = read('styles/media-detail.css');
  const pageInfo = declarationsOf(mediaDetail, '.hp-page-info');
  assert.match(pageInfo, /scrollbar-width:\s*none/);
});

test('scrollbar.css est importé avant les surcharges du thème clair', () => {
  const app = read('app.css');
  const barImport = app.indexOf("@import url('./styles/scrollbar.css');");
  const themeImport = app.indexOf("@import url('./styles/theme-light.css');");
  assert.notEqual(barImport, -1, 'app.css doit importer styles/scrollbar.css');
  assert.ok(themeImport > barImport, 'scrollbar.css doit précéder theme-light.css');
});
