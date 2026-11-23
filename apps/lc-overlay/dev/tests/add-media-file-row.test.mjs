// Ligne d'un fichier choisi dans « Ajouter un média » : le bouton « Modifier
// l’image » reste dans le groupe de droite et la taille du média se place
// juste à sa gauche ; la croix de retrait ferme la ligne, après lui.
//
// Avant le premier correctif, l'ordre des enfants était nom → bouton → taille,
// et la ligne répartissait l'espace libre entre les trois (space-between) : le
// bouton flottait au milieu et la taille partait seule à l'extrême droite. La
// taille prend maintenant l'espace libre avant elle, et les actions la suivent.
//
// L'ordre du DOM est aussi l'ordre de lecture et de tabulation : le bouton de
// modification précède la croix, dernière cible de la ligne. Une vidéo ou un
// son, sans bouton de modification, garde sa taille à droite devant la croix.

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

// Declared height of a rule, in pixels, or null.
function pxHeight(rule) {
  const match = rule && rule.match(/(?:^|;|\s)height:\s*(\d+)px/);
  return match ? Number(match[1]) : null;
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

test('la croix de retrait ferme la ligne, après le bouton de modification', () => {
  const row = fileListRow();
  const edit = row.indexOf("class: 'btn file-edit");
  const remove = row.indexOf('file-remove');

  assert.notEqual(remove, -1, 'la ligne doit porter une croix de retrait');
  assert.ok(edit < remove, 'la croix vient après le bouton de modification, en dernier');
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
  // the size and the buttons stay together at the right end.
  assert.ok(size, 'the size rule must exist');
  assert.match(size, /margin-left:\s*auto/, 'la taille pousse les boutons à droite et vient juste avant eux');
  assert.match(size, /flex:\s*none/, 'la taille ne doit ni grandir ni rétrécir');
});

test('la liste réserve la place de sa barre de défilement', () => {
  const list = ruleOf(read('styles/dialog.css'), '.file-list');

  assert.ok(list, 'la règle de la liste doit exister');
  assert.match(list, /overflow:\s*auto/, 'une longue importation défile dans la liste');
  // Sans gouttière réservée, la barre apparaît avec le quatrième média et
  // décale d'un coup toutes les lignes, croix comprise, sous le pointeur.
  assert.match(list, /scrollbar-gutter:\s*stable/, 'la barre de défilement ne doit pas décaler les lignes');
  assert.match(list, /max-height:\s*\d+px/, 'la liste garde sa hauteur bornée');
});

test('la croix de retrait tient dans la ligne comme le bouton de modification', () => {
  const remove = ruleOf(read('styles/dialog.css'), '.file-list .file-remove');
  const edit = ruleOf(read('styles/editor.css'), '.file-list .file-edit');

  assert.ok(remove, 'la règle de la croix doit exister');
  assert.ok(edit, 'la règle du bouton de modification doit exister');
  const height = pxHeight(remove);
  assert.ok(height > 0, 'la croix doit avoir une hauteur déclarée en pixels');
  assert.equal(height, pxHeight(edit), 'la croix doit avoir la hauteur du bouton de modification');
  assert.match(remove, new RegExp(`width:\\s*${height}px`), 'une icône seule : la croix est carrée');
  assert.match(remove, /padding:\s*0/, 'l’icône occupe le bouton, sans marge interne');
  assert.match(remove, /justify-content:\s*center/, 'l’icône est centrée dans le bouton');
  assert.match(remove, /flex:\s*none/, 'la croix ne doit ni grandir ni rétrécir');
});
