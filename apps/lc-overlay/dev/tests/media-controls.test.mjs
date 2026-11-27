// The details window used Chromium's native controls: its progress bar is
// drawn in a closed shadow tree, so it can neither be styled nor given a
// linear animation. These checks pin the replacement: one control bar in the
// stage, built only with a player, driving the same linear fill as the hover
// preview and the media element itself through pointers and keys. They also
// pin the native controls off everywhere (fresh, prefetched and carried
// players) and the bar's destruction on every close path, otherwise the
// animation would keep running on a removed element.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { ratioAt, seekBy, seekToRatio } from '../../web/app/core/media-controls.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const mediaDetail = read('views/media-detail.js').replace(/\r\n/g, '\n');
const detailCss = read('styles/media-detail.css');

// Declarations of the rule whose selector list contains `selector`.
function ruleOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim());
    if (selectors.includes(selector)) {
      return match[2];
    }
  }
  return '';
}

test('seekBy moves by the step and stays inside the media', () => {
  assert.equal(seekBy(10, 30, 5), 15);
  assert.equal(seekBy(10, 30, -5), 5);
  assert.equal(seekBy(0, 30, -5), 0);
  assert.equal(seekBy(30, 30, 5), 30);
  assert.equal(seekBy(28, 30, 5), 30);
});

test('seekBy tolerates an unknown duration or a broken position', () => {
  assert.equal(seekBy(10, NaN, 5), 10);
  assert.equal(seekBy(10, 0, 5), 10);
  assert.equal(seekBy(10, Infinity, 5), 10);
  assert.equal(seekBy(-3, 30, 5), 5);
  assert.equal(seekBy(NaN, 30, 5), 5);
  assert.equal(seekBy(10, 30, NaN), 10);
});

test('seekToRatio converts a pointer ratio to a time inside the media', () => {
  assert.equal(seekToRatio(0.5, 30), 15);
  assert.equal(seekToRatio(0, 30), 0);
  assert.equal(seekToRatio(1, 30), 30);
  assert.equal(seekToRatio(1.5, 30), 30);
  assert.equal(seekToRatio(-1, 30), 0);
  assert.equal(seekToRatio(NaN, 30), 0);
  assert.equal(seekToRatio(0.5, 0), 0);
  assert.equal(seekToRatio(0.5, NaN), 0);
});

test('ratioAt maps the pointer to the track, clamped at both ends', () => {
  const rect = { left: 100, width: 200 };
  assert.equal(ratioAt(200, rect), 0.5);
  assert.equal(ratioAt(100, rect), 0);
  assert.equal(ratioAt(300, rect), 1);
  assert.equal(ratioAt(50, rect), 0);
  assert.equal(ratioAt(350, rect), 1);
  assert.equal(ratioAt(NaN, rect), 0);
  assert.equal(ratioAt(200, { left: 100, width: 0 }), 0);
  assert.equal(ratioAt(200, null), 0);
});

test('the details stage gets a control bar built with its player', () => {
  const attach = /\n  function attachPlayerControls\(([^)]*)\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(attach, 'attachPlayerControls is found');
  const [, params, body] = attach;
  assert.match(params, /\bplayer\b/);
  assert.match(params, /\bkind\b/);
  assert.match(params, /\bstage\b/);
  assert.match(body, /player\.controls = false/);
  assert.match(body, /class: 'hp-controls'/);
  assert.match(body, /role: 'slider'/);
  assert.match(body, /'aria-label': 'Progression'/);
  assert.match(body, /createPreviewProgress\(\{ player, fill \}\)/);
  assert.match(body, /return \{\s*root,\s*destroy\(\)/);
});

test('the control bar drives play, volume, seek and fullscreen', () => {
  const attach = /\n  function attachPlayerControls\(([^)]*)\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  const body = attach[2];
  assert.match(body, /player\.play\(\)/);
  assert.match(body, /player\.pause\(\)/);
  assert.match(body, /seekBy\(/);
  assert.match(body, /player\.currentTime = seekToRatio\(/);
  assert.match(body, /ratioAt\(/);
  assert.match(body, /player\.muted = true/);
  assert.match(body, /player\.muted = false/);
  assert.match(body, /requestFullscreen/);
  assert.match(body, /hp-ctl-fill/);
});

test('no native controls remain, fresh or carried', () => {
  assert.doesNotMatch(mediaDetail, /\.controls = true/, 'the native bar would sit under the custom one');
  assert.match(mediaDetail, /controls = false/, 'detailVideo and detailAudio opt out');
  assert.match(mediaDetail, /controls = false;/, 'prefetched and carried players opt out too');
});

test('buildStage hands the controls to the page', () => {
  assert.match(mediaDetail, /const controls = player \? attachPlayerControls\(/);
  assert.match(mediaDetail, /return \{ stage, player, kind, poster, controls \};/);
  assert.match(mediaDetail, /page\._controls = built\.controls;/);
});

test('every close path destroys the controls', () => {
  const count = (mediaDetail.match(/_controls\.destroy\(\)/g) || []).length;
  assert.ok(count >= 3, `controls destroyed on the close paths (found ${count})`);
});

test('carrying a preview player stops its preview progress controller', () => {
  assert.match(
    mediaDetail,
    /if \(carry && carry\.source\) \{[\s\S]*?carry\.source\._progress[\s\S]*?carry\.source\.remove\(\);/,
    'the preview fill is left animating on a detached player otherwise',
  );
});

test('the progress bar is the same linear fill as the hover preview', () => {
  assert.match(mediaDetail, /import \{ createPreviewProgress \} from '\.\.\/core\/preview-progress\.js';/);
  assert.match(mediaDetail, /const progress = createPreviewProgress\(\{ player, fill \}\);/);
});

test('the control bar sits over the stage without a CSS step', () => {
  const bar = ruleOf(detailCss, '.hp-controls');
  assert.match(bar, /position:\s*absolute/);
  assert.match(bar, /inset:\s*auto 0 0/);
  assert.match(bar, /z-index:\s*2/);

  const fill = ruleOf(detailCss, '.hp-ctl-fill');
  assert.match(fill, /transform:\s*scaleX\(0\)/);
  assert.match(fill, /transform-origin:\s*left center/);
  assert.doesNotMatch(fill, /transition:/, 'the fill is animated by the compositor, not a short glide');

  const track = ruleOf(detailCss, '.hp-ctl-track');
  assert.match(track, /touch-action:\s*none/);
  assert.match(track, /cursor:\s*pointer/);
});
