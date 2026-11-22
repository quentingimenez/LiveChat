// The editor panel ends with a quiet summary of the media being edited: its
// file, its weight, its length and the size of the picture. It belongs to no
// tool, so it stays at the bottom whatever is open, and its "Image" and
// "Durée" lines follow the recipe (crop, cut, speed).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

test('mediaInfoFacts lists the file and what the recipe makes of it', async () => {
  const { mediaInfoFacts } = await import('../../web/app/core/editor-helpers.js');
  assert.deepEqual(
    mediaInfoFacts({
      name: 'sample.mp4',
      mime: 'video/mp4',
      bytes: 1_572_864,
      kind: 'video',
      duration: 6,
      resultDuration: 3.5,
      width: 1280,
      height: 720,
      resultWidth: 200,
      resultHeight: 128,
    }),
    [
      ['Fichier', 'sample.mp4'],
      ['Type', 'Vidéo · MP4'],
      ['Poids', '1,5 Mo'],
      ['Durée', '0:06,0 → 0:03,5'],
      ['Image', '1280 × 720 px → 200 × 128 px'],
    ],
  );
});

test('mediaInfoFacts: an untouched media shows one value per line', async () => {
  const { mediaInfoFacts } = await import('../../web/app/core/editor-helpers.js');
  assert.deepEqual(
    mediaInfoFacts({
      name: 'sample.mp4',
      mime: 'video/mp4',
      kind: 'video',
      duration: 6,
      resultDuration: 6,
      width: 1280,
      height: 720,
      resultWidth: 1280,
      resultHeight: 720,
    }),
    [
      ['Fichier', 'sample.mp4'],
      ['Type', 'Vidéo · MP4'],
      ['Durée', '0:06,0'],
      ['Image', '1280 × 720 px'],
    ],
  );
});

test('mediaInfoFacts: a sound tells whether it has artwork', async () => {
  const { mediaInfoFacts } = await import('../../web/app/core/editor-helpers.js');
  assert.deepEqual(
    mediaInfoFacts({
      name: 'sample.mp3',
      mime: 'audio/mpeg',
      kind: 'audio',
      duration: 3,
      hasArtwork: true,
      artworkWidth: 360,
      artworkHeight: 360,
    }),
    [
      ['Fichier', 'sample.mp3'],
      ['Type', 'Son · MP3'],
      ['Durée', '0:03,0'],
      ['Pochette', '360 × 360 px'],
    ],
  );
  assert.deepEqual(
    mediaInfoFacts({ name: 'sample.mp3', mime: 'audio/mpeg', kind: 'audio', duration: 3, hasArtwork: false }),
    [
      ['Fichier', 'sample.mp3'],
      ['Type', 'Son · MP3'],
      ['Durée', '0:03,0'],
      ['Pochette', 'aucune'],
    ],
  );
});

test('mediaInfoFacts: a picture shows its crop and its size on the overlay', async () => {
  const { mediaInfoFacts } = await import('../../web/app/core/editor-helpers.js');
  assert.deepEqual(
    mediaInfoFacts({
      mime: 'image/png',
      bytes: 204_800,
      kind: 'image',
      width: 1200,
      height: 1200,
      resultWidth: 1080,
      resultHeight: 1080,
      displayWidth: 540,
      displayHeight: 540,
    }),
    [
      ['Type', 'Image · PNG'],
      ['Poids', '200 ko'],
      ['Image', '1200 × 1200 px → 1080 × 1080 px'],
      ['À l’écran', '540 × 540 px'],
    ],
  );
  // The overlay shows the picture at its own size: no separate row.
  assert.deepEqual(
    mediaInfoFacts({ mime: 'image/png', kind: 'image', width: 800, height: 600, resultWidth: 800, resultHeight: 600, displayWidth: 800, displayHeight: 600 }),
    [
      ['Type', 'Image · PNG'],
      ['Image', '800 × 600 px'],
    ],
  );
});

test('mediaTypeLabel names the formats it knows and stays quiet otherwise', async () => {
  const { mediaTypeLabel } = await import('../../web/app/core/editor-helpers.js');
  assert.equal(mediaTypeLabel('video/mp4'), 'Vidéo · MP4');
  assert.equal(mediaTypeLabel('video/quicktime'), 'Vidéo · MOV');
  assert.equal(mediaTypeLabel('audio/mpeg'), 'Son · MP3');
  assert.equal(mediaTypeLabel('image/png'), 'Image · PNG');
  assert.equal(mediaTypeLabel('image/jpeg'), 'Image · JPEG');
  assert.equal(mediaTypeLabel('image/svg+xml'), 'Image · SVG');
  assert.equal(mediaTypeLabel('text/plain'), '');
  assert.equal(mediaTypeLabel(''), '');
});

test('formatFileSize reads sizes in French units', async () => {
  const { formatFileSize } = await import('../../web/app/core/editor-helpers.js');
  assert.equal(formatFileSize(0), '');
  assert.equal(formatFileSize(512), '512 o');
  assert.equal(formatFileSize(30_720), '30 ko');
  assert.equal(formatFileSize(1_572_864), '1,5 Mo');
});

test('the panel ends with the media summary, outside the tool sections', () => {
  const av = source('views/editor/av.js');
  assert.match(av, /mediaInfoFacts/, 'av.js must build the summary from the shared helper');
  assert.match(av, /class: 'editor-info'/);
  const render = av.slice(av.indexOf('function renderPanel'), av.indexOf('function renderPanel') + 3200);
  assert.match(render, /infoPanel\(\)/, 'the summary must be appended to the panel');
  // It follows the recipe, so a commit has to refresh it.
  const after = av.slice(av.indexOf('function afterChange'), av.indexOf('function commit'));
  assert.match(after, /syncInfoNode\(\)/);
  const css = source('styles/editor.css');
  assert.match(css, /\.editor-info \{[^}]*position: sticky/);
  assert.match(css, /\.editor-panel \{[^}]*display: flex/, 'the summary sits at the bottom of the panel');
});

test('the picture editor gets the same summary, without duration', () => {
  const image = source('views/editor/image.js');
  assert.match(image, /mediaInfoFacts\(\{/, 'image.js must build the summary from the shared helper');
  assert.match(image, /kind: 'image'/);
  // Its size comes from the picture it opened, not from the box being edited.
  assert.match(image, /const sourceWidth = doc\.base\.width/);
  assert.match(image, /const sourceHeight = doc\.base\.height/);
  const render = image.slice(image.indexOf('function renderPanel'), image.indexOf('function renderPanel') + 1200);
  assert.match(render, /infoPanel\(\)/, 'the summary must be appended to the panel');
  // The display tool and the history change the rows without rebuilding the panel.
  assert.match(image, /function syncInfoNode\(\)/);
  const commit = image.slice(image.indexOf('function commit'), image.indexOf('function restore'));
  assert.match(commit, /syncInfoNode\(\)/);
  const scale = image.slice(image.indexOf('function setDisplayScale'), image.indexOf('function setDisplayScale') + 900);
  assert.match(scale, /syncInfoNode\(\)/);
});
