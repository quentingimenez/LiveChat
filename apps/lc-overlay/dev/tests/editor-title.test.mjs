// What this file protects: the media title is edited in the editor bar with the
// board drawer's field (pencil on hover, Enter or leaving the field saves,
// Escape goes back), it is saved through the same route as the drawer, and the
// drawer's caption no longer glues its text to the field's border.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { titleAfterRename, titleToSave } from '../../web/app/views/editor/title-field.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

test('un brouillon inchangé ne part pas vers le serveur', () => {
  assert.equal(titleToSave('', ''), null);
  assert.equal(titleToSave('   ', ''), null);
  assert.equal(titleToSave('  Bruh  ', 'Bruh'), null);
  // Effacer le titre est un vrai changement.
  assert.equal(titleToSave('', 'Bruh'), '');
  assert.equal(titleToSave('  Nope ', 'Bruh'), 'Nope');
});

test('le titre affiché retombe sur celui déduit de la source', () => {
  assert.equal(titleAfterRename('Bruh', 'Bruh'), 'Bruh');
  assert.equal(titleAfterRename('  Neuf ', 'tiktok.com'), 'Neuf');
  // Un titre effacé laisse la barre montrer le titre déduit, comme la board.
  assert.equal(titleAfterRename('', 'tiktok.com'), 'tiktok.com');
  assert.equal(titleAfterRename('   ', 'tiktok.com'), 'tiktok.com');
  assert.equal(titleAfterRename(null, 'tiktok.com'), 'tiktok.com');
});

test('le champ du titre vit dans la barre, pas dans un panneau', () => {
  const frame = read('views/editor/frame.js');
  assert.match(frame, /import \{ createTitleField \} from '\.\/title-field\.js'/);
  assert.match(frame, /createTitleField\(\{ title, value: titleValue, rename \}\)/);
  assert.match(frame, /titleField\.node/);
  assert.doesNotMatch(frame, /titleLabel/);
  // Entrée enregistre, Échap annule : le cadre intercepte avant l'éditeur.
  assert.match(frame, /titleField\.owns\(event\.target\)/);
  assert.match(frame, /titleField\.commit\(\)/);
  assert.match(frame, /titleField\.cancel\(\)/);
  // Once settled, the keyboard goes back to the field, not the page.
  assert.match(frame, /titleField\.focus\(\)/);
});

test('le champ reprend les gestes du tiroir', () => {
  const source = read('views/editor/title-field.js');
  assert.match(source, /class: `inline-edit is-title/);
  assert.match(source, /icon\('edit'\)/);
  assert.match(source, /maxlength: String\(maxlength\)/);
  assert.match(source, /placeholder: EMPTY_LABEL/);
  assert.match(source, /const EMPTY_LABEL = 'Ajouter un titre'/);
  // Une vraie sortie de champ enregistre ; un champ retiré par un rendu, non.
  assert.match(source, /if \(input\.isConnected\) \{\s*commit\(\);/);
  // La sauvegarde laisse le clavier dans la barre : Échap atteint encore l'éditeur.
  assert.match(source, /const hadFocus = node\.contains\(document\.activeElement\)/);
  assert.match(source, /if \(hadFocus\) \{\s*field\.focus\(\);/);
});

test('l’éditeur enregistre le titre par la route du tiroir', () => {
  const index = read('views/editor/index.js');
  assert.match(index, /api\('PATCH', `\/overlay\/media-board\/items\/\$\{encodeURIComponent\(item\.id\)\}`, \{ title: value \}\)/);
  assert.match(index, /item\.title = saved;/);
  assert.match(index, /requestItemsReload\(\)/);
  // Même droit que le tiroir : seul un média éditable se renomme.
  assert.match(index, /rename: item\.canEdit \? \(next\) => renameItem\(item, next\) : null/);
  assert.match(index, /titleValue: item\.title \|\| ''/);
});

test('le champ du titre garde la largeur de la barre', () => {
  const css = read('styles/editor.css');
  assert.match(css, /\.editor-title \{[^}]*display: flex/);
  const field = css.match(/\.editor-title \.inline-edit \{[^}]*\}/);
  assert.ok(field, 'règle .editor-title .inline-edit absente');
  assert.match(field[0], /max-width: min\(36vw, 100%\)/);
  const input = css.match(/\.editor-title \.inline-input\.is-title \{[^}]*\}/);
  assert.ok(input, 'règle .editor-title .inline-input.is-title absente');
  assert.match(input[0], /height: 28px/);
});
