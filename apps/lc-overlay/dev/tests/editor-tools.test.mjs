// The toolbox shared by the media editors: a rail is a list of tools, the
// panel shows the picked one and nothing else.
//
// The same ids, labels, glyphs and grouping feed the picture editor, the video
// editor and the audio editor. This file pins that model so the three rails
// cannot drift apart, and pins the two exceptions that live outside the rule:
// the inspector of a selected object (it replaces the panel of Sélection) and
// the cover of a video (it keeps the display panel company).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { EDITORS, panelKeys, railGroups, toolById, toolHint } from '../../web/app/views/editor/tools.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const ids = (groups) => groups.flat().map((entry) => entry.id);
const all = (editor, options) => ids(railGroups(editor, options));

test('chaque éditeur a une rail, groupée, sans doublon', () => {
  assert.deepEqual(EDITORS.sort(), ['audio', 'image', 'video']);
  for (const editor of EDITORS) {
    const groups = railGroups(editor, { allowDisplayScale: true, hasArtwork: true, output: 'same' });
    assert.ok(groups.length > 0, `${editor} n’a aucun groupe`);
    assert.ok(groups.every((group) => group.length > 0), `${editor} a un groupe vide`);
    const list = ids(groups);
    assert.equal(new Set(list).size, list.length, `${editor} répète un outil`);
    for (const entry of groups.flat()) {
      assert.equal(typeof entry.label, 'string');
      assert.ok(entry.label.length > 0);
      assert.equal(typeof entry.glyph, 'string');
    }
  }
});

test('la rail vidéo range les outils de montage sous les outils d’image', () => {
  const groups = railGroups('video', { allowDisplayScale: true, output: 'same', hasArtwork: false });
  assert.deepEqual(groups.map((group) => group.map((entry) => entry.id)), [
    ['select', 'crop'],
    ['trim', 'speed', 'sound'],
    ['draw', 'shape', 'text', 'emoji', 'adjust'],
    ['blur'],
    ['display'],
  ]);
});

test('la rail audio n’a ni sélection ni dessin', () => {
  const groups = railGroups('audio', { allowDisplayScale: true, output: 'same', hasArtwork: true });
  assert.deepEqual(groups.map((group) => group.map((entry) => entry.id)), [
    ['trim', 'speed', 'sound'],
    ['display'],
  ]);
});

test('un résultat audio retire les outils qui travaillent l’image', () => {
  const options = { allowDisplayScale: true, output: 'audio', hasArtwork: false };
  const list = all('video', options);
  for (const id of ['select', 'crop', 'draw', 'shape', 'text', 'emoji', 'adjust', 'blur', 'display']) {
    assert.ok(!list.includes(id), `${id} ne devrait pas exister pour un résultat audio`);
  }
  assert.deepEqual(railGroups('video', options).map((group) => group.map((entry) => entry.id)), [['trim', 'speed', 'sound']]);
});

test('la taille à l’affichage n’apparaît que si une image est réglable', () => {
  assert.ok(all('image', { allowDisplayScale: false }).every((id) => id !== 'display'));
  assert.ok(all('image', { allowDisplayScale: true }).includes('display'));
  // Un son sans pochette n’a pas d’image à dimensionner.
  assert.ok(!all('audio', { allowDisplayScale: true, hasArtwork: false }).includes('display'));
  assert.ok(all('audio', { allowDisplayScale: true, hasArtwork: true }).includes('display'));
  // Vidéo muette en sortie : la carte n’affiche plus d’image.
  assert.ok(!all('video', { allowDisplayScale: true, output: 'audio' }).includes('display'));
});

test('le panneau d’un outil ne montre que cet outil', () => {
  assert.deepEqual(panelKeys('image', 'draw'), ['draw']);
  assert.deepEqual(panelKeys('video', 'trim'), ['trim']);
  assert.deepEqual(panelKeys('video', 'adjust'), ['adjust']);
  assert.deepEqual(panelKeys('audio', 'sound'), ['sound']);
  assert.throws(() => panelKeys('image', 'trim'), /unknown image tool: trim/);
});

test('sélection : l’inspecteur remplace l’aide, il ne s’y ajoute pas', () => {
  assert.deepEqual(panelKeys('image', 'select'), ['select']);
  assert.deepEqual(panelKeys('image', 'select', { hasSelection: true }), ['selection']);
  assert.deepEqual(panelKeys('video', 'select', { hasSelection: true }), ['selection']);
});

test('affichage : la taille garde la miniature à côté d’elle', () => {
  assert.deepEqual(panelKeys('video', 'display'), ['display', 'cover']);
  assert.deepEqual(panelKeys('image', 'display'), ['display', 'cover']);
});

test('les outils se nomment par ce qu’ils font', () => {
  assert.equal(toolById('adjust').label, 'Lumière et couleurs');
  assert.equal(toolById('display').label, 'Affichage');
  assert.equal(toolById('trim').label, 'Découper');
  assert.equal(toolById('inconnu'), null);
  const labels = EDITORS.flatMap((editor) => railGroups(editor, { allowDisplayScale: true, hasArtwork: true }).flat());
  assert.ok(labels.every((entry) => !/réglages/i.test(entry.label)));
});

test('l’aide de Lumière et couleurs nomme la lumière, les couleurs et le flou', () => {
  assert.match(toolHint('adjust', 'image'), /lumière/i);
  assert.match(toolHint('adjust', 'image'), /couleurs/i);
  assert.match(toolHint('adjust', 'image'), /flou/i);
  assert.match(toolHint('adjust', 'video'), /vidéo/i);
  // Un outil écrit pour l’image parle bien de la vidéo quand il la sert.
  assert.match(toolHint('crop', 'video'), /vidéo/i);
});

test('les trois éditeurs lisent le même modèle', () => {
  const image = read('views/editor/image.js');
  const video = read('views/editor/video-annotations.js');
  const av = read('views/editor/av.js');
  assert.match(image, /import \{[^}]*railGroups[^}]*toolHint[^}]*\} from '\.\/tools\.js'/);
  assert.match(video, /import \{[^}]*toolById[^}]*toolHint[^}]*\} from '\.\/tools\.js'/);
  assert.match(av, /import \{[^}]*panelKeys[^}]*railGroups[^}]*toolHint[^}]*\} from '\.\/tools\.js'/);
  // Les anciens catalogues locaux ne doivent pas revenir.
  assert.doesNotMatch(image, /const TOOLS = \[/);
  assert.doesNotMatch(video, /const ANNOTATION_TOOLS = \[/);
});

test('l’éditeur audio/vidéo ne construit son panneau qu’à partir de la rail', () => {
  const source = read('views/editor/av.js');
  // Une seule écriture du panneau, pilotée par les clés du modèle.
  assert.equal((source.match(/frame\.panel\.replaceChildren/g) || []).length, 1);
  assert.match(source, /panelKeys\(mediaType\(\), tool, \{ hasSelection \}\)/);
  // La rail est celle du modèle, pas une seconde liste dans cette vue.
  assert.doesNotMatch(source, /const ANNOTATION_TOOLS/);
  assert.doesNotMatch(source, /annotations\.panelSections\(/);
  assert.doesNotMatch(source, /annotations\.renderRail\(/);
});

test('le format de sortie quitte le panneau pour la barre', () => {
  const source = read('views/editor/av.js');
  assert.match(source, /frame\.setExtra\(/);
  assert.doesNotMatch(source, /section\(\s*'Enregistrer en'/);
});
