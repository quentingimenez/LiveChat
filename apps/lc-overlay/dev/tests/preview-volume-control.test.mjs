// The hover preview's mute button reveals a vertical level bar styled like the
// card's playback line. These checks pin the control: the bar trims the overlay
// gain (100 % sounds exactly like the overlay), it replaces the mute button on
// hover and sits on the button's line, it borrows the playback line's thickness,
// rail and glow, it fills its whole height at 100 %, it is built with the player
// and never with a still, it only answers once the preview is open, and a press
// on it never toggles playback.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

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

const attach = /\n  function attachPlayer\(([^)]*)\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
assert.ok(attach, 'attachPlayer is found');
const attachParams = attach[1];
const attachBody = attach[2];

test('the mute button owns a vertical volume bar, built with the player', () => {
  assert.match(attachParams, /\bdesc\b/, 'attachPlayer receives the card description');
  assert.match(attachBody, /className = 'hp-volume'/);
  assert.match(attachBody, /className = 'hp-mute'/);
  assert.match(attachBody, /className = 'hp-volume-pop'/);
  assert.match(attachBody, /className = 'hp-volume-range'/);
  assert.match(attachBody, /slider\.type = 'range'/);
  assert.match(attachBody, /slider\.min = '0'/);
  assert.match(attachBody, /slider\.max = '100'/);
  assert.match(attachBody, /slider\.step = '1'/);
  assert.match(attachBody, /aria-label', 'Volume de l’aperçu'/);
  assert.match(attachBody, /aria-orientation', 'vertical'/);
});

test('the bar trims the overlay gain instead of setting an absolute volume', () => {
  assert.match(attachBody, /trimVolume\(desc && desc\.volume, previewLevel\)/);
  assert.match(attachBody, /const level = clampVolume\(Number\(slider\.value\) \/ 100\)/);
  assert.match(attachBody, /trimVolume\(desc && desc\.volume, level\)/);
  assert.doesNotMatch(attachBody, /player\.volume = Number\(slider\.value\)/);
});

test('the bar mutes at zero and the mute click restores the last audible level', () => {
  assert.match(attachBody, /if \(level > 0\) \{[\s\S]*?lastAudibleLevel = level;[\s\S]*?player\.muted = false;[\s\S]*?\} else \{[\s\S]*?player\.muted = true;[\s\S]*?\}/);
  assert.match(attachBody, /if \(previewLevel === 0\) \{[\s\S]*?previewLevel = lastAudibleLevel;/);
  assert.match(attachBody, /const audible = !player\.muted && player\.volume > 0/);
  assert.match(attachBody, /speakerIcon\(!audible\)/);
  assert.match(attachBody, /aria-pressed', String\(audible\)/);
});

test('the level survives from one preview to the next', () => {
  const controller = mediaDetail.indexOf('export function createMediaDetail');
  const level = mediaDetail.indexOf('let previewLevel = 1;');
  const last = mediaDetail.indexOf('let lastAudibleLevel = 1;');
  const attachAt = mediaDetail.indexOf('function attachPlayer');
  assert.ok(controller >= 0 && level > controller && last > level && attachAt > last, 'shared state lives in the controller closure');
});

test('pressing the bar or the button never toggles the preview playback', () => {
  const guards = mediaDetail.match(/event\.target\.closest\('\.hp-volume'\)/g) || [];
  assert.equal(guards.length, 4, 'pointerdown, click, dblclick and keydown all ignore the control');
  assert.doesNotMatch(mediaDetail, /event\.target\.closest\('\.hp-mute'\)/);
  assert.match(mediaDetail, /attachPlayer\(el, media, player, card, desc\)/);
});

test('the bar is hidden until an open preview is hovered, or reached by keyboard', () => {
  const pop = ruleOf(detailCss, '.hp-volume-pop');
  assert.match(pop, /opacity:\s*0/);
  assert.match(pop, /pointer-events:\s*none/);
  const hover = ruleOf(detailCss, '.hp-overlay.is-in .hp-volume:hover .hp-volume-pop');
  assert.match(hover, /opacity:\s*1/);
  assert.match(hover, /pointer-events:\s*auto/);
  const focus = ruleOf(detailCss, '.hp-overlay.is-in .hp-volume:has(:focus-visible) .hp-volume-pop');
  assert.match(focus, /opacity:\s*1/);
});

test('the wrapper stays out of the opening and owns the hover surface once open', () => {
  // A click must reach the video during the opening, when the mute button is
  // still hidden. Once open, the wrapper keeps the hover surface alive when
  // the button steps aside; the media handlers ignore the whole control.
  assert.match(ruleOf(detailCss, '.hp-volume'), /pointer-events:\s*none/);
  assert.match(ruleOf(detailCss, '.hp-overlay.is-in .hp-volume'), /pointer-events:\s*auto/);
  assert.match(ruleOf(detailCss, '.hp-mute'), /pointer-events:\s*auto/);
  assert.match(ruleOf(detailCss, '.hp-overlay.is-in .hp-mute'), /pointer-events:\s*auto/);
});

test('hovering the control leaves only the level bar', () => {
  const hide = ruleOf(detailCss, '.hp-overlay.is-in .hp-volume:hover .hp-mute');
  assert.match(hide, /opacity:\s*0/, 'the mute button leaves the stage');
  assert.match(hide, /pointer-events:\s*none/);
  assert.match(hide, /transition:\s*opacity var\(--motion-reveal\)/, 'it fades out at the control speed');
});

test('the bar is a vertical range with a visible fill', () => {
  const range = ruleOf(detailCss, '.hp-volume-range');
  assert.match(range, /writing-mode:\s*vertical-lr/);
  assert.match(range, /direction:\s*rtl/);
  assert.match(range, /appearance:\s*none/);
  const track = ruleOf(detailCss, '.hp-volume-range::-webkit-slider-runnable-track');
  assert.match(track, /linear-gradient/);
  assert.match(track, /var\(--hp-volume-level/);
  const thumb = ruleOf(detailCss, '.hp-volume-range::-webkit-slider-thumb');
  assert.match(thumb, /width:\s*3px/);
  assert.match(thumb, /border-radius:\s*50%/);
});

test('the level bar copies the playback line: thickness, rail and glow', () => {
  const progress = ruleOf(detailCss, '.hp-progress');
  const fill = ruleOf(detailCss, '.hp-progress span');
  const track = ruleOf(detailCss, '.hp-volume-range::-webkit-slider-runnable-track');
  const thumb = ruleOf(detailCss, '.hp-volume-range::-webkit-slider-thumb');
  const bar = /height:\s*(\d+(?:\.\d+)?)px/.exec(progress);
  const line = /width:\s*(\d+(?:\.\d+)?)px/.exec(track);
  assert.ok(bar && line, 'both lines state their thickness');
  assert.equal(line[1], bar[1], 'the volume line is as thick as the card playback bar');
  assert.match(track, /rgba\(8, 10, 16, 0\.55\)/, 'same dark rail');
  assert.match(fill, /background:\s*#fff/);
  assert.match(track, /#fff/, 'same white fill');
  assert.match(fill, /box-shadow:\s*0 0 6px rgba\(255, 255, 255, 0\.45\)/);
  assert.match(thumb, /box-shadow:\s*0 0 6px rgba\(255, 255, 255, 0\.45\)/, 'the fill keeps its glow');
  assert.match(ruleOf(detailCss, '.hp-progress:hover'), /rgba\(8, 10, 16, 0\.72\)/);
  assert.match(ruleOf(detailCss, '.hp-volume-range:hover::-webkit-slider-runnable-track'), /rgba\(8, 10, 16, 0\.72\)/, 'the rail lifts on hover too');
  assert.doesNotMatch(track, /var\(--blue\)/);
});

test('the level bar is a bare line, not a pill', () => {
  const pop = ruleOf(detailCss, '.hp-volume-pop');
  assert.doesNotMatch(pop, /background:/);
  assert.doesNotMatch(pop, /box-shadow:/);
  assert.doesNotMatch(pop, /border-radius:/);
});

test('the bar sits on the mute button and fills its whole height', () => {
  const pop = ruleOf(detailCss, '.hp-volume-pop');
  assert.match(pop, /top:\s*0/, 'it starts at the button top');
  assert.match(pop, /right:\s*1px/, 'its centre is the button centre, not a column beside it');
  assert.doesNotMatch(pop, /calc\(100%/, 'it is no longer pushed to the left of the button');
  assert.doesNotMatch(pop, /padding:/, 'no inset keeps the track from the top');
  const popHeight = /height:\s*(\d+(?:\.\d+)?)px/.exec(pop);
  const range = ruleOf(detailCss, '.hp-volume-range');
  const rangeHeight = /height:\s*(\d+(?:\.\d+)?)px/.exec(range);
  assert.ok(popHeight && rangeHeight, 'the control states its height');
  assert.equal(rangeHeight[1], popHeight[1], 'the track fills the whole hit area');
  const track = ruleOf(detailCss, '.hp-volume-range::-webkit-slider-runnable-track');
  assert.match(track, /#fff 0 var\(--hp-volume-level, 100%\)/);
  assert.match(track, /rgba\(8, 10, 16, 0\.55\) var\(--hp-volume-level, 100%\) 100%/);
});

test('a still preview has no volume control', () => {
  const build = /\n  function build\(card, desc\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(build, 'build is found');
  const call = build[1].indexOf('attachPlayer(el, media, player, card, desc)');
  const still = build[1].indexOf('attachStillOpen(media, card)');
  assert.ok(call >= 0 && still > call, 'the bar is attached with a player, never to a still');
});
