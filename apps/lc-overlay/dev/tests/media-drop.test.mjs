// « Ajouter un média » ouvert, un lien ou un fichier déposé dessus ne faisait
// rien. Le board ne voyait un dépôt que dans sa zone principale (`.main`) :
// la fenêtre est dessinée à côté, dans `#dialog-root`, donc `inMainArea`
// refusait le dépôt ; et le `preventDefault` du dragover empêchait en plus le
// champ de le recevoir nativement. Rien n'arrivait.
//
// Correctif : le board demande à la fenêtre ouverte si elle prend le dépôt
// (`dropTarget`) et lui remet le transfert, qu'elle lit avec `droppedMedia`
// (fichiers ajoutés à la sélection, lien prérempli).
//
// Sans DOM dans les dépendances de l'overlay, les tests couvrent la décision et
// la lecture du transfert, puis protègent par invariants de source le câblage
// qui les relie aux deux vues.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { DROP_VEIL_TEXT, droppedMedia, dropTarget } from '../../web/app/core/media-drop.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Un DataTransfer minimal : les deux membres que le renderer lit.
function transfer({ files = [], text = {} } = {}) {
  return { files, getData: (type) => text[type] ?? '' };
}

const clip = { name: 'clip.mp4', type: 'video/mp4', size: 12 };

// --- what a drop brings ---

test('les fichiers déposés priment sur le lien du même transfert', () => {
  const media = droppedMedia(transfer({ files: [clip], text: { 'text/uri-list': 'https://x.test/a' } }));
  assert.deepEqual(media, { files: [clip], link: null });
});

test('plusieurs fichiers déposés sont tous gardés, dans l’ordre', () => {
  const second = { name: 'image.png', type: 'image/png', size: 3 };
  const media = droppedMedia(transfer({ files: [clip, second] }));
  assert.equal(media.files.length, 2);
  assert.deepEqual(media.files.map((file) => file.name), ['clip.mp4', 'image.png']);
});

test('un lien vient de l’onglet glissé (text/uri-list), à défaut du texte', () => {
  assert.equal(droppedMedia(transfer({ text: { 'text/uri-list': 'https://x.test/a' } })).link, 'https://x.test/a');
  assert.equal(droppedMedia(transfer({ text: { 'text/plain': 'https://y.test/b' } })).link, 'https://y.test/b');
  // Le format uri-list commence par ses commentaires : seule la ligne URL compte.
  assert.equal(droppedMedia(transfer({ text: { 'text/uri-list': '# glissé\r\nhttps://z.test/c\r\n' } })).link, 'https://z.test/c');
});

test('un dépôt sans fichier ni lien web n’est pas un média', () => {
  assert.equal(droppedMedia(transfer()), null);
  assert.equal(droppedMedia(transfer({ text: { 'text/plain': 'du texte' } })), null);
  // Un fichier local ne s’importe pas par chemin.
  assert.equal(droppedMedia(transfer({ text: { 'text/uri-list': 'file:///C:/a.mp4' } })), null);
});

test('un transfert absent ne fait pas échouer la lecture', () => {
  assert.equal(droppedMedia(undefined), null);
  assert.equal(droppedMedia(null), null);
  assert.equal(droppedMedia({ files: [] }), null);
});

// --- where a drop lands ---

test('la fenêtre ouverte prend le dépôt, même quand le board le refuserait', () => {
  // Régression : la fenêtre n'était pas une destination du tout, et son fond
  // (`#dialog-root`) est hors de la zone principale.
  assert.equal(dropTarget({ dialogOpen: true, onBoard: true }), 'dialog');
  assert.equal(dropTarget({ dialogOpen: true, onBoard: false }), 'dialog');
});

test('sans fenêtre ouverte, seul un dépôt dans la zone principale compte', () => {
  assert.equal(dropTarget({ dialogOpen: false, onBoard: true }), 'board');
  assert.equal(dropTarget({ dialogOpen: false, onBoard: false }), null);
});

test('le voile nomme la destination, pas seulement le board', () => {
  assert.match(DROP_VEIL_TEXT.board, /board/);
  assert.match(DROP_VEIL_TEXT.dialog, /formulaire/);
  assert.notEqual(DROP_VEIL_TEXT.dialog, DROP_VEIL_TEXT.board);
});

// --- the wiring, recorded from the sources ---

const board = read('views/board.js');
const addMedia = read('views/add-media.js');
const ui = read('core/ui.js');
const boardCss = read('styles/board.css');
const dialogCss = read('styles/dialog.css');

function declarationsOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    if (match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim() === selector) {
      return match[2];
    }
  }
  return null;
}

function valueOf(css, selector, property) {
  const declarations = declarationsOf(css, selector);
  assert.ok(declarations, `règle ${selector} introuvable`);
  const value = new RegExp(`(?:^|;)\\s*${property}:\\s*([^;]+)`).exec(declarations);
  assert.ok(value, `${property} absent de ${selector}`);
  return value[1].trim();
}

test('le board remet le dépôt à la fenêtre ouverte au lieu d’importer derrière', () => {
  const dropHandler = board.slice(board.indexOf("document.addEventListener('drop'"));
  assert.ok(dropHandler.length > 0, 'gestionnaire de drop introuvable');
  assert.match(dropHandler, /dropOnAddDialog\(event\.dataTransfer\)/);
  // La décision passe par la fenêtre, pas par la seule zone principale.
  assert.match(board, /dropTarget\(\{ dialogOpen: addDialogOpen\(\), onBoard: /);
});

test('le voile suit la destination et couvre la fenêtre au-dessus du dialogue', () => {
  assert.match(board, /classList\.toggle\('is-modal', destination === 'dialog'\)/);
  assert.match(board, /textContent = DROP_VEIL_TEXT\[destination\]/);
  assert.match(valueOf(boardCss, '.drop-veil.is-modal', 'inset'), /^0/);
  // Le voile modal n'ajoute pas d'assombrissement à lui : c'est le fond du
  // dialogue qui prend celui du voile, sous un formulaire qui reste lisible.
  assert.equal(valueOf(boardCss, '.drop-veil.is-modal', 'background'), 'none');
  assert.match(valueOf(boardCss, '.drop-veil', 'background'), /--scrim/);
  assert.match(valueOf(boardCss, '#dialog-root .backdrop.is-drop-target', 'background'), /--scrim/);
  assert.match(board, /highlightAddDialog\(destination === 'dialog'\)/);
  assert.match(board, /highlightAddDialog\(false\)/);
  assert.match(addMedia, /export function highlightAddDialog\(active\)/);
  // Le voile doit passer devant le fond du dialogue : sinon le formulaire le
  // recouvre et le dépôt ne lui parvient pas.
  const backdrop = Number(valueOf(dialogCss, '.backdrop', 'z-index'));
  const veil = Number(valueOf(boardCss, '.drop-veil.is-modal', 'z-index'));
  assert.ok(veil > backdrop, `voile ${veil} derrière le fond du dialogue ${backdrop}`);
});

test('la fenêtre pose son puits de dépôt et le retire en se fermant', () => {
  assert.match(addMedia, /export function addDialogOpen\(\)/);
  assert.match(addMedia, /export function dropOnAddDialog\(transfer\)/);
  // Une fenêtre fermée ne prend plus rien : le puits n'existe que tant que son
  // formulaire est dans le document.
  assert.match(addMedia, /isOpen: \(\) => form\.isConnected/);
  assert.match(addMedia, /onClose: \(\) => \{ dropSurface = null; \}/);
  assert.match(ui, /options\.onClose\(\)/);
});

test('un éditeur ouvert par-dessus la fenêtre prend le dépôt pour lui', () => {
  // L'éditeur de média est plein écran sur `#dialog-root` : le dépôt ne doit pas
  // remplir le formulaire caché dessous.
  assert.match(addMedia, /!document\.querySelector\('#dialog-root > \.editor'\)/);
});

test('un dépôt de fichiers remplit la sélection sans la remplacer', () => {
  assert.match(addMedia, /chosenFiles = chosenFiles\.concat\(media\.files\)/);
  assert.match(addMedia, /setMode\('file'\);\s*\n\s*renderFileList\(\);/);
});

test('un dépôt de lien préremplit le champ de lien, sans toucher aux fichiers', () => {
  assert.match(addMedia, /linkUrl\.value = media\.link/);
  assert.match(addMedia, /setMode\('link'\)/);
});
