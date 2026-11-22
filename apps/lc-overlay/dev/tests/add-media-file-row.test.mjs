// Ligne d'un fichier choisi dans « Ajouter un média » : le bouton « Modifier
// l’image » reste collé à droite et la taille du média se place juste à sa
// gauche.
//
// Avant ce correctif, l'ordre des enfants était nom → bouton → taille, et la
// ligne répartissait l'espace libre entre les trois (space-between) : le
// bouton flottait au milieu et la taille partait seule à l'extrême droite. La
// taille prend maintenant l'espace libre avant elle et le bouton la suit.
//
// L'ordre du DOM est aussi l'ordre de lecture et de tabulation : le bouton
// passe donc en dernier. Une vidéo ou un son, sans bouton, garde sa taille à
// droite (même règle, la taille est le dernier élément).

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

// Source of `renderFileList`, up to the form built after it.
function fileListRow() {
  const view = read('views/add-media.js');
  const start = view.indexOf('function renderFileList');
  assert.notEqual(start, -1, 'renderFileList must exist');
  const end = view.indexOf('const form = h(', start);
  assert.notEqual(end, -1, 'the form must follow renderFileList');
  return view.slice(start, end);
}

test('la taille du média précède le bouton de modification', () => {
  const row = fileListRow();
  const name = row.indexOf("class: 'file-name");
  const size = row.indexOf("class: 'file-size");
  const edit = row.indexOf("class: 'btn file-edit");

  assert.notEqual(name, -1, 'the row must keep the file name');
  assert.notEqual(size, -1, 'the row must keep the media size');
  assert.notEqual(edit, -1, 'the row must keep the edit button');
  assert.ok(name < size, 'le nom ouvre la ligne');
  assert.ok(size < edit, 'la taille doit être juste avant le bouton, pas après lui');
});

test('l’espace libre de la ligne est pris avant la taille', () => {
  const css = read('styles/dialog.css');
  const row = ruleOf(css, '.file-list li');
  const size = ruleOf(css, '.file-size');

  assert.ok(row, 'the file row rule must exist');
  assert.match(row, /display:\s*flex/, 'the row lays its parts out as a flex line');
  assert.match(row, /align-items:\s*center/);
  // A grid item keeps its content width unless told otherwise: a long name
  // would then widen the row past the list, with the button out of sight.
  assert.match(row, /min-width:\s*0/, 'le nom doit pouvoir se réduire, pas chasser le bouton de la ligne');
  // The free space goes to a margin before the size, not between every child:
  // the size and the button stay together at the right end.
  assert.ok(size, 'the size rule must exist');
  assert.match(size, /margin-left:\s*auto/, 'la taille pousse le bouton à droite et vient juste avant lui');
  assert.match(size, /flex:\s*none/, 'la taille ne doit ni grandir ni rétrécir');
});
