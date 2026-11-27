// The hover preview's mute button reveals a vertical level bar styled like the
// card's playback line. These checks pin the control: the bar sets an absolute
// 0–100 % level (100 % is full volume, independent of the device settings, and
// the preview starts at the card's default level), the mute button stays
// visible while the bar opens below it, the bar borrows the playback line's
// thickness, rail and glow, it fills its whole height at 100 % and still fits
// the shortest preview, it is built with the player and never with a still, it
// only answers once the preview is open, a drag keeps a height margin around
// the preview, and a press on it never toggles playback.

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

test('the bar sets an absolute level instead of trimming the overlay gain', () => {
  assert.doesNotMatch(attachBody, /trimVolume/);
  assert.match(attachBody, /const defaultLevel = clampVolume\(desc && desc\.volume\)/);
  assert.match(attachBody, /const level = clampVolume\(Number\(slider\.value\) \/ 100\)/);
  assert.match(attachBody, /player\.volume = level;/, 'a move sets that exact level');
  assert.match(attachBody, /player\.volume = currentLevel\(\);/, 'before any move, the card default applies');
  assert.doesNotMatch(attachBody, /player\.volume = Number\(slider\.value\)/);
});

test('the bar mutes at zero and the mute click restores the last audible level', () => {
  assert.match(attachBody, /if \(level > 0\) \{[\s\S]*?lastAudibleLevel = level;[\s\S]*?player\.muted = false;[\s\S]*?\} else \{[\s\S]*?player\.muted = true;[\s\S]*?\}/);
  assert.match(attachBody, /if \(previewLevel === 0\) \{[\s\S]*?previewLevel = lastAudibleLevel;/);
  assert.match(attachBody, /const audible = !player\.muted && player\.volume > 0/);
  assert.match(attachBody, /speakerIcon\(!audible\)/);
  assert.match(attachBody, /aria-pressed', String\(audible\)/);
});

test('the level survives from one preview to the next and follows the card default until moved', () => {
  const controller = mediaDetail.indexOf('export function createMediaDetail');
  const level = mediaDetail.indexOf('let previewLevel = null;');
  const last = mediaDetail.indexOf('let lastAudibleLevel = null;');
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
  // the pointer moves from the button down onto the bar; the media handlers
  // ignore the whole control.
  assert.match(ruleOf(detailCss, '.hp-volume'), /pointer-events:\s*none/);
  assert.match(ruleOf(detailCss, '.hp-overlay.is-in .hp-volume'), /pointer-events:\s*auto/);
  assert.match(ruleOf(detailCss, '.hp-mute'), /pointer-events:\s*auto/);
  assert.match(ruleOf(detailCss, '.hp-overlay.is-in .hp-mute'), /pointer-events:\s*auto/);
});

test('hovering the control keeps the mute button visible', () => {
  // The button no longer steps aside for the bar: it stays the way in and out
  // of the sound, and the bar opens below it.
  assert.equal(
    ruleOf(detailCss, '.hp-overlay.is-in .hp-volume:hover .hp-mute'),
    '',
    'no rule hides the mute button on hover',
  );
  assert.doesNotMatch(
    detailCss,
    /\.hp-volume:hover[^{]*\.hp-mute[^{]*\{[^}]*opacity:\s*0/,
    'the button never fades out while the bar is open',
  );
  const open = ruleOf(detailCss, '.hp-overlay.is-in .hp-mute');
  assert.match(open, /opacity:\s*1/);
  assert.match(open, /pointer-events:\s*auto/);
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

test('the bar hangs under the mute button and fits the shortest preview', () => {
  const pop = ruleOf(detailCss, '.hp-volume-pop');
  assert.match(pop, /top:\s*32px/, 'it starts under the 32 px button, not on it');
  assert.match(pop, /right:\s*1px/, 'its centre is the button centre, not a column beside it');
  assert.doesNotMatch(pop, /calc\(100%/, 'it is no longer pushed onto the button line');
  assert.doesNotMatch(pop, /padding:/, 'no inset keeps the track from its top');

  const wrapper = ruleOf(detailCss, '.hp-volume');
  const wrapperTop = /top:\s*(\d+(?:\.\d+)?)px/.exec(wrapper);
  const popTop = /top:\s*(\d+(?:\.\d+)?)px/.exec(pop);
  const popHeight = /height:\s*(\d+(?:\.\d+)?)px/.exec(pop);
  const range = ruleOf(detailCss, '.hp-volume-range');
  const rangeHeight = /height:\s*(\d+(?:\.\d+)?)px/.exec(range);
  assert.ok(wrapperTop && popTop && popHeight && rangeHeight, 'the control states its geometry');
  assert.equal(rangeHeight[1], popHeight[1], 'the track fills the whole hit area');

  // The bar must end inside the shortest preview media: mediaSize in
  // views/media-detail.js floors the media height, and .hp-media clips the
  // overflow. Note the bar is measured from the wrapper, itself inset from
  // the media top.
  const shortest = /height:\s*Math\.max\((\d+), Math\.round\(height\)\)/.exec(mediaDetail);
  assert.ok(shortest, 'the shortest preview media height is stated in mediaSize');
  const bottom = Number(wrapperTop[1]) + Number(popTop[1]) + Number(popHeight[1]);
  assert.ok(
    bottom <= Number(shortest[1]),
    `the bar ends at ${bottom} px, inside the shortest preview (${shortest[1]} px)`,
  );

  const track = ruleOf(detailCss, '.hp-volume-range::-webkit-slider-runnable-track');
  assert.match(track, /#fff 0 var\(--hp-volume-level, 100%\)/);
  assert.match(track, /rgba\(8, 10, 16, 0\.55\) var\(--hp-volume-level, 100%\) 100%/);
});

test('reduced motion does not bring back the hiding hand-off', () => {
  const reduced = detailCss.slice(detailCss.indexOf('@media (prefers-reduced-motion: reduce)'));
  assert.doesNotMatch(
    reduced,
    /\.hp-volume:hover \.hp-mute/,
    'the button stays beside the bar in every motion mode',
  );
});

test('a level drag keeps a height margin around the preview', () => {
  assert.match(mediaDetail, /let volumeMargin = null;/);
  assert.match(mediaDetail, /let volumeDragging = false;/);
  assert.match(mediaDetail, /function attachVolumeDragMargin\(\)/);
  assert.match(
    mediaDetail,
    /function scheduleHide\(\) \{[\s\S]*?if \(volumeDragging\) \{[\s\S]*?return;/,
    'a held slider never lets the preview hide',
  );
  assert.match(mediaDetail, /className = 'hp-volume-drag-margin'/);
  assert.match(mediaDetail, /margin\.addEventListener\('pointerenter', cancelHide\)/);
  assert.match(mediaDetail, /margin\.addEventListener\('pointerleave',/);
  assert.match(mediaDetail, /scheduleHide\(\)/);
  assert.match(mediaDetail, /volumeDragMargin\(previewEl\.getBoundingClientRect\(\), VOLUME_DRAG_MARGIN\)/);
  assert.match(attachBody, /slider\.addEventListener\('pointerdown', \(\) => \{[\s\S]*?attachVolumeDragMargin\(\)/);
  assert.match(attachBody, /slider\.addEventListener\('pointerup', finishVolumeDrag\)/);
  assert.match(attachBody, /slider\.addEventListener\('pointercancel', finishVolumeDrag\)/);
  assert.match(
    mediaDetail,
    /if \(volumeMargin && volumeMargin\.matches\(':hover'\)\) \{\s*return;/,
    'the controller treats the margin as part of the hover surface',
  );
  assert.match(mediaDetail, /function close\(immediate\) \{[\s\S]*?clearVolumeDragMargin\(\)/, 'closing drops the margin');
  const band = ruleOf(detailCss, '.hp-volume-drag-margin');
  assert.match(band, /position:\s*fixed/);
  assert.match(band, /z-index:\s*89/);
  assert.match(band, /background:\s*transparent/);
  assert.match(band, /pointer-events:\s*auto/);
});

test('a release inside the band keeps it even when the band was never hovered', () => {
  const finish = /\n  function finishVolumeDrag\(([^)]*)\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(finish, 'finishVolumeDrag must exist');
  assert.match(finish[1], /event/, 'the release event carries the pointer position');
  assert.match(finish[2], /clientX/, 'the released x is read');
  assert.match(finish[2], /clientY/, 'the released y is read');
  assert.match(finish[2], /getBoundingClientRect\(\)/, 'the band rectangle is measured at release');
  assert.match(finish[2], /volumeDragMarginContains/, 'the release point is tested against the band');
  assert.doesNotMatch(
    finish[2],
    /const inMargin = Boolean\(volumeMargin && volumeMargin\.matches\(':hover'\)\);/,
    'hover must only be the fallback for a release without coordinates',
  );
});

test('a still preview has no volume control', () => {
  const build = /\n  function build\(card, desc\) \{([\s\S]*?)\n  \}\n/.exec(mediaDetail);
  assert.ok(build, 'build is found');
  const call = build[1].indexOf('attachPlayer(el, media, player, card, desc)');
  const still = build[1].indexOf('attachStillOpen(media, card)');
  assert.ok(call >= 0 && still > call, 'the bar is attached with a player, never to a still');
});
