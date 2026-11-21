// Croix de fermeture des dialogues : elle doit être un bouton stylé, posé
// dans la colonne de contenu et non par-dessus la barre de défilement.
//
// Régression : la croix du cadre commun n'avait que `icon-btn`, sans `btn` :
// le style natif du <button> (fond gris, bordure 2px outset) revenait, et la
// croix en position absolue sur le cadre recouvrait la barre de défilement de
// « Mon profil ». Le correctif la place dans une ligne de titre collante,
// à l'intérieur du corps défilant : son bord droit s'arrête au bord de la
// barre, et `right: 10px` la laisse dans la colonne de contenu.
//
// Sans DOM dans les dépendances de l'overlay, le test protège les invariants
// de structure (ui.js) et de mise en page (dialog.css) qui portent ce
// comportement.

import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

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

function declarationsOf(css, selector) {
  const declarations = ruleOf(css, selector);
  assert.ok(declarations, `règle ${selector} introuvable`);
  return declarations;
}

const ui = read('core/ui.js');
const css = read('styles/dialog.css');

test('la croix de fermeture reste un bouton de l’interface', () => {
  const classes = /class: '([^']*\bdialog-close\b[^']*)'/.exec(ui);
  assert.ok(classes, 'bouton .dialog-close introuvable dans ui.js');
  const tokens = classes[1].split(/\s+/);
  for (const token of ['btn', 'is-quiet', 'icon-btn', 'dialog-close']) {
    assert.ok(tokens.includes(token), `classe ${token} absente de « ${classes[1]} »`);
  }
});

test('la croix des dialogues titrés vit dans la ligne de titre défilante', () => {
  // Le titre n'est déplacé que s'il ouvre le corps : les autres dialogues
  // gardent la croix posée sur le cadre (repli plus bas).
  assert.match(ui, /if \(title && body\.firstElementChild === title\)/);
  assert.match(ui, /body\.prepend\(h\('div', \{ class: 'dialog-head' \}, title, closeButton\)\)/);
});

test('le focus initial est choisi avant que la croix entre dans le corps', () => {
  // Sinon `preferred` verrait la croix et les dialogues sans champ s'ouvriraient
  // dessus au lieu de leur premier contrôle.
  assert.ok(ui.indexOf('const preferred') !== -1, 'sélection du focus introuvable');
  assert.ok(ui.indexOf('const preferred') < ui.indexOf("body.prepend(h('div', { class: 'dialog-head' }"));
});

test('la ligne de titre couvre le corps sans décaler le titre', () => {
  const head = declarationsOf(css, '.dialog-head');
  assert.match(head, /position:\s*sticky/);
  assert.match(head, /top:\s*0/);
  // Les marges négatives annulent le padding du corps sur les côtés : la
  // surface collante va jusqu'à la barre de défilement, tandis que le padding
  // garde le titre à sa place (24 px du bord du contenu).
  assert.match(head, /margin:\s*0\s+-24px/);
  assert.match(head, /padding:\s*0\s+24px/);
});

test('la croix est calée dans la colonne, pas sur la barre de défilement', () => {
  const close = declarationsOf(css, '.dialog-head .dialog-close');
  assert.match(close, /position:\s*absolute/);
  assert.match(close, /right:\s*10px/);
  assert.match(close, /top:\s*10px/);
});

test('le titre réserve la place de la croix et ne colle plus tout seul', () => {
  const title = declarationsOf(css, '.dialog h2');
  assert.match(title, /padding:\s*22px\s+40px\s+4px\s+0/);
  // Deux classes : l’override gagne sur `.dialog h2` quel que soit l’ordre.
  assert.match(declarationsOf(css, '.dialog .dialog-head h2'), /position:\s*static/);
});

test('un dialogue sans titre garde la croix sur le cadre', () => {
  const overlay = declarationsOf(css, '.dialog > .dialog-close');
  assert.match(overlay, /position:\s*absolute/);
  assert.match(overlay, /right:\s*10px/);
});

test('le premier enfant du corps ne rajoute pas de marge sous la ligne', () => {
  const first = declarationsOf(css, '.dialog-body > :first-child:not(h2):not(.dialog-head)');
  assert.match(first, /margin-top:\s*22px/);
});

test('tout bouton icône emprunte le style des boutons', () => {
  // Une croix réduite à `icon-btn` reprend le style natif du <button> (fond
  // gris, bordure 2px). L'audit couvre les boutons à venir, pas seulement la
  // croix du cadre.
  const appDir = fileURLToPath(new URL('../../web/app/', import.meta.url));
  const files = readdirSync(appDir, { recursive: true })
    .filter((name) => String(name).endsWith('.js'))
    .map((name) => join(appDir, String(name)));
  const found = [];
  for (const path of files) {
    const source = readFileSync(path, 'utf8');
    const pattern = /\bclass: '([^']*\bicon-btn\b[^']*)'/g;
    let match;
    while ((match = pattern.exec(source)) !== null) {
      found.push({
        path,
        classes: match[1],
        line: source.slice(0, match.index).split('\n').length,
      });
    }
  }
  assert.ok(found.length >= 6, `seulement ${found.length} bouton(s) icône trouvé(s)`);
  for (const { path, classes, line } of found) {
    const tokens = classes.split(/\s+/);
    assert.ok(tokens.includes('btn'), `${basename(path)}:${line} : classe btn absente de « ${classes} »`);
  }
});
