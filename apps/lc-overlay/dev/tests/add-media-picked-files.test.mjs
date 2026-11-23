// « Ajouter un média » garde une seule importation en cours : rouvrir
// « Choisir des fichiers » ajoute les médias choisis à ceux déjà listés au lieu
// de les remplacer, sans jamais ajouter deux fois le même média, et la croix
// d'une ligne retire ce média avant l'envoi.
//
// Avant ce correctif, le gestionnaire de changement assignait
// `chosenFiles = Array.from(fileInput.files || [])` : un second choix
// remplaçait toute l'importation en cours, les médias déjà préparés étaient
// perdus sans trace. Le sélecteur est maintenant vidé après chaque choix,
// sinon le même fichier ne redéclencherait pas de change une fois sa ligne
// retirée (un input inchangé n'émet pas d'événement). Un fichier déjà listé
// est ignoré — un média qui apparaîtrait deux fois dans l'importation serait
// envoyé deux fois au board — et le dialogue le dit sous le sélecteur. Un
// fichier déposé sur le dialogue ouvert (core/media-drop.js) suit la même règle
// qu'un choix : il rejoint l'importation sans y entrer deux fois.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { addPickedFiles, duplicatePickMessage, removePickedFile, samePickedFile } from '../../web/app/core/add-media-rules.js';

const view = readFileSync(new URL('../../web/app/views/add-media.js', import.meta.url), 'utf8');

const file = (name, size = 1024) => ({ name, size, type: 'image/png' });

// The picker's change handler, up to the list it fills.
function pickerHandler() {
  const start = view.indexOf('onChange: () => {');
  assert.notEqual(start, -1, 'le sélecteur de fichiers doit réagir à un choix');
  const end = view.indexOf('const fileList = h(', start);
  assert.notEqual(end, -1, 'la liste doit suivre le sélecteur');
  return view.slice(start, end);
}

// Source of `renderFileList`, up to the form built after it.
function fileRow() {
  const start = view.indexOf('function renderFileList');
  assert.notEqual(start, -1, 'renderFileList doit exister');
  const end = view.indexOf('const form = h(', start);
  assert.notEqual(end, -1, 'le formulaire doit suivre la liste');
  return view.slice(start, end);
}

// The drop surface filled once the form is built, up to the link branch.
function dropHandler() {
  const start = view.indexOf('drop: (transfer) => {');
  assert.notEqual(start, -1, 'le dialogue ouvert doit prendre les dépôts');
  const end = view.indexOf('linkUrl.value = media.link', start);
  assert.notEqual(end, -1, 'le dépôt doit distinguer les fichiers du lien');
  return view.slice(start, end);
}

test('un deuxième choix s’ajoute à l’importation en cours', () => {
  const first = [file('videoframe_1871.png')];
  const after = addPickedFiles(first, [file('sample.mp4', 2048)]);

  // Le bug : la liste ne contenait plus que le second choix.
  assert.deepEqual(after.map((entry) => entry.name), ['videoframe_1871.png', 'sample.mp4']);
  assert.deepEqual(first.map((entry) => entry.name), ['videoframe_1871.png'], 'la liste déjà choisie ne change pas');
  assert.notEqual(after, first, 'la liste renvoyée est une nouvelle liste');
});

test('un choix multiple s’ajoute dans l’ordre du sélecteur', () => {
  const after = addPickedFiles([file('a.png')], [file('b.png'), file('c.mp3', 512)]);
  assert.deepEqual(after.map((entry) => entry.name), ['a.png', 'b.png', 'c.mp3']);
});

test('un média déjà choisi n’est pas ajouté une seconde fois', () => {
  const chat = file('chat.png');
  const after = addPickedFiles([chat], [file('chat.png')]);

  assert.equal(after.length, 1, 'le même média ne peut pas entrer deux fois dans l’importation');
  assert.equal(after[0], chat, 'la ligne gardée est celle déjà préparée, à sa place');
});

test('le même média répété dans un seul choix n’entre qu’une fois', () => {
  const after = addPickedFiles([], [file('chat.png'), file('chat.png')]);
  assert.equal(after.length, 1);
});

test('deux fichiers de même nom mais de contenus différents sont deux médias', () => {
  // Deux dossiers, deux fichiers : le nom seul ne dit pas que c'est le même.
  const after = addPickedFiles([file('chat.png', 1024)], [file('chat.png', 2048)]);
  assert.equal(after.length, 2, 'un fichier de même nom mais d’autre taille reste un autre média');
});

test('un média se reconnaît à son nom, sa taille, son type et sa date', () => {
  const chat = { name: 'chat.png', size: 1024, type: 'image/png', lastModified: 1_700_000_000_000 };
  const copy = { name: 'chat.png', size: 1024, type: 'image/png', lastModified: 1_700_000_000_000 };

  assert.ok(samePickedFile(chat, copy), 'le même fichier choisi deux fois doit être reconnu');
  assert.ok(!samePickedFile(chat, { ...copy, size: 1025 }), 'une autre taille : autre fichier');
  assert.ok(!samePickedFile(chat, { ...copy, type: 'video/mp4' }), 'un autre type : autre fichier');
  assert.ok(!samePickedFile(chat, { ...copy, lastModified: 1 }), 'une autre date : autre fichier');
  assert.ok(!samePickedFile(chat, { ...copy, name: 'chien.png' }), 'un autre nom : autre fichier');
});

test('retirer un média ne retire que sa ligne', () => {
  const files = [file('a.png'), file('b.png'), file('c.png')];
  const left = removePickedFile(files, 1);

  assert.deepEqual(left.map((entry) => entry.name), ['a.png', 'c.png']);
  assert.equal(files.length, 3, 'l’importation d’origine ne doit pas être modifiée');
});

test('retirer un média édité garde le média édité voisin', () => {
  const edited = { ...file('edited.png'), edited: true };
  const files = [file('a.png'), file('b.png'), edited];
  const left = removePickedFile(files, 0);

  assert.equal(left.length, 2);
  assert.equal(left[1], edited, 'le média édité reste le même objet, à sa place');
});

test('une position qui n’existe plus ne retire rien', () => {
  const files = [file('a.png'), file('b.png')];
  // Un rendu périmé peut déclencher un index hors liste : rien ne doit être
  // retiré au hasard.
  for (const index of [-1, 2, 1.5, Number.NaN]) {
    const left = removePickedFile(files, index);
    assert.deepEqual(left.map((entry) => entry.name), ['a.png', 'b.png']);
    assert.notEqual(left, files, 'la liste renvoyée reste une copie');
  }
});

test('un nouveau choix ne remplace plus la liste dans le dialogue', () => {
  const handler = pickerHandler();

  assert.match(handler, /addPickedFiles\(chosenFiles/, 'le choix doit rejoindre les fichiers déjà choisis');
  assert.doesNotMatch(handler, /chosenFiles = Array\.from/, 'le choix ne doit plus remplacer toute l’importation');
  assert.match(handler, /fileInput\.value = ''/, 'le sélecteur doit être vidé pour pouvoir rechoisir le même fichier');
});

test('un choix sans nouveauté le dit sous le sélecteur', () => {
  const handler = pickerHandler();
  const panel = view.slice(view.indexOf('const filePanel = h('), view.indexOf('const linkButton'));
  const hint = view.slice(view.indexOf('const duplicateHint = h('), view.indexOf('const fileMessage'));

  assert.match(panel, /fileList,\s*duplicateHint,/, 'la liste doit s’accompagner d’un message pour les doublons');
  assert.match(hint, /role: 'status'/, 'le message est annoncé et lu, pas seulement affiché');
  assert.match(hint, /hidden: true/, 'le message reste muet tant qu’aucun doublon n’est choisi');
  assert.match(handler, /duplicateHint\.hidden = ignored === 0/, 'le message ne reste pas quand le choix apporte un média');
  assert.match(handler, /duplicatePickMessage\(ignored\)/, 'le message compte les médias ignorés');
  assert.equal(duplicatePickMessage(1), 'Média déjà dans l’importation.');
  assert.equal(duplicatePickMessage(2), '2 médias déjà dans l’importation.');
});

test('un fichier déposé suit la même règle qu’un choix', () => {
  const drop = dropHandler();

  // Le formulaire prend aussi un dépôt (core/media-drop.js) : un fichier
  // déposé rejoint l'importation en cours et ne doit pas y entrer deux fois.
  assert.match(drop, /addPickedFiles\(chosenFiles, media\.files\)/, 'le dépôt doit rejoindre les fichiers déjà choisis');
  assert.doesNotMatch(drop, /chosenFiles = chosenFiles\.concat\(/, 'le dépôt ne doit pas ajouter un média déjà listé');
  assert.match(drop, /duplicatePickMessage\(ignored\)/, 'le dépôt dit aussi quand le média est déjà listé');
  assert.match(drop, /setMode\('file'\)/, 'un fichier déposé ouvre le formulaire fichier');
});

test('la croix d’une ligne retire son média de l’importation', () => {
  const row = fileRow();

  assert.match(row, /file-remove/, 'la ligne doit porter une croix de retrait');
  assert.match(row, /aria-label': `Retirer \$\{file\.name\}`/, 'la croix doit dire quel média elle retire');
  assert.match(row, /removePickedFile\(chosenFiles, index\)/, 'la croix doit retirer le média de l’importation');
  assert.match(row, /icon\('close'\)/, 'le bouton de retrait est une croix');
  assert.match(row, /duplicateHint\.hidden = true/, 'le média retiré redevient choisissable : le message de doublon tombe');
});
