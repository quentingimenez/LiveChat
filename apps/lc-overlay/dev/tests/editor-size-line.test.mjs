// The "Affichage" panel describes the size of the media once the recipe is
// applied. A fractional crop (`198.53329781346287 × 128.63910187455946 px`)
// broke the line and widened the panel past its column, so the size is now
// given in the whole pixels ffmpeg keeps and the panel cannot be widened.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

test('croppedPictureSize keeps the whole pixels ffmpeg crops to', async () => {
  const { croppedPictureSize } = await import('../../web/app/core/editor-helpers.js');
  // Full frame: untouched.
  assert.deepEqual(croppedPictureSize({ width: 1280, height: 720 }), { width: 1280, height: 720 });
  // A fractional crop is truncated to an even number of pixels per side, as
  // `crop=w='max(2,trunc(iw*w/2)*2)'` does on the server.
  assert.deepEqual(
    croppedPictureSize({ width: 1280, height: 720, crop: { width: 0.1551, height: 0.1786 } }),
    { width: 198, height: 128 },
  );
  // No side goes below two pixels, and the fractions are clamped.
  assert.deepEqual(croppedPictureSize({ width: 1280, height: 720, crop: { width: 0.0001, height: 2 } }), {
    width: 2,
    height: 720,
  });
  // A quarter turn swaps the sides, the way `transpose` does.
  assert.deepEqual(
    croppedPictureSize({ width: 1280, height: 720, crop: { width: 0.5, height: 0.5 }, quarterTurns: 1 }),
    { width: 360, height: 640 },
  );
  // A picture without a known size stays unknown.
  assert.equal(croppedPictureSize({ width: 0, height: 0 }), null);
});

test('the Affichage line shows those pixels instead of the raw fractions', () => {
  const av = source('views/editor/av.js');
  assert.match(av, /croppedPictureSize/, "av.js must crop the size the way the server does");
  // The size line must come from the pixel size, not from `resultPictureSize()`
  // (which keeps the fractions for the preview).
  const sizePanel = av.slice(av.indexOf('function sizePanel'), av.indexOf('function sizePanel') + 2600);
  assert.match(sizePanel, /resultPixelSize\(\)/);
  assert.doesNotMatch(sizePanel, /resultPictureSize\(\) \|\|/);
});

test('the panel cannot be widened by a long line', () => {
  const css = source('styles/editor.css');
  // A grid track wider than the 288px column pushed the content out of the page.
  assert.match(css, /\.editor-section \{[^}]*grid-template-columns: minmax\(0, 1fr\)/);
  assert.match(css, /\.editor-section \.meta \{[^}]*overflow-wrap: anywhere/);
});
