// The hover preview's progress bar used to be painted on `timeupdate`, which
// Chromium fires about four times a second, behind a 90 ms CSS transition: the
// fill moved for 90 ms, then stayed frozen for the remaining 160 ms — the
// stutter this file guards against. The fill is now a linear animation from the
// current position to the end of the media, run by the compositor; the media
// clock only restarts it on play, pause, seek, stall, drift and the next loop.
// These checks pin the pure timing and the player/fill state machine, and the
// wiring that replaced the per-tick paint.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  PROGRESS_DRIFT,
  createPreviewProgress,
  previewProgressFrames,
  progressRatio,
} from '../../web/app/core/preview-progress.js';

const read = (path) =>
  readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const mediaDetail = read('views/media-detail.js');
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

// The player only needs the surface the module reads: EventTarget, the playback
// state, the clocks and a `paused` flag the test moves like the browser would.
class FakePlayer extends EventTarget {
  constructor(overrides = {}) {
    super();
    this.currentTime = 0;
    this.duration = 30;
    this.paused = true;
    this.seeking = false;
    this.playbackRate = 1;
    Object.assign(this, overrides);
  }

  fire(type) {
    this.dispatchEvent(new Event(type));
  }
}

function fakeFill({ animatable = true } = {}) {
  const animations = [];
  const fill = {
    style: { transform: '' },
    animations,
    animate(frames, options) {
      const animation = {
        frames,
        options,
        canceled: false,
        progress: 0,
        onfinish: null,
        cancel() {
          this.canceled = true;
        },
        effect: {
          getComputedTiming: () => ({ progress: animation.progress }),
        },
      };
      animations.push(animation);
      return animation;
    },
  };
  if (!animatable) {
    delete fill.animate;
  }
  return fill;
}

function setup(overrides) {
  const player = new FakePlayer(overrides);
  const fill = fakeFill();
  return { player, fill, progress: createPreviewProgress({ player, fill }) };
}

function play(player) {
  player.paused = false;
  player.fire('play');
}

test('a resumed fill starts from the displayed ratio', () => {
  const plan = previewProgressFrames({
    currentTime: 12,
    duration: 30,
    playing: true,
    startRatio: 0.45,
  });
  assert.deepEqual(plan.frames, [
    { transform: 'scaleX(0.45)' },
    { transform: 'scaleX(1)' },
  ]);
  assert.equal(plan.ratio, 0.45);
  assert.deepEqual(plan.options, { duration: 18000, easing: 'linear', fill: 'forwards' });
});

test('the ratio is bounded and an unknown duration reads as empty', () => {
  assert.equal(progressRatio(3, 12), 0.25);
  assert.equal(progressRatio(0, 12), 0);
  assert.equal(progressRatio(-4, 12), 0);
  assert.equal(progressRatio(20, 12), 1);
  assert.equal(progressRatio(3, 0), 0);
  assert.equal(progressRatio(3, NaN), 0);
  assert.equal(progressRatio(3, Infinity), 0);
  assert.equal(progressRatio(NaN, 12), 0);
});

test('frames cover the time left, at a linear pace and the playback rate', () => {
  const plan = previewProgressFrames({ currentTime: 3, duration: 12, playing: true });
  assert.deepEqual(plan.frames, [
    { transform: 'scaleX(0.25)' },
    { transform: 'scaleX(1)' },
  ]);
  assert.deepEqual(plan.options, { duration: 9000, easing: 'linear', fill: 'forwards' });

  const fast = previewProgressFrames({ currentTime: 3, duration: 12, playing: true, playbackRate: 2 });
  assert.equal(fast.options.duration, 4500, 'at double speed the fill crosses the same span in half the time');

  for (const playbackRate of [0, -1, NaN]) {
    assert.equal(
      previewProgressFrames({ currentTime: 3, duration: 12, playing: true, playbackRate }).options.duration,
      9000,
      'an unusable rate falls back to normal speed',
    );
  }
});

test('nothing animates while paused, unknown or already at the end', () => {
  assert.equal(previewProgressFrames({ currentTime: 0, duration: 12 }), null, 'playing defaults to false');
  assert.equal(previewProgressFrames({ currentTime: 0, duration: 12, playing: false }), null);
  assert.equal(previewProgressFrames({ currentTime: 0, duration: NaN, playing: true }), null);
  assert.equal(previewProgressFrames({ currentTime: 12, duration: 12, playing: true }), null);
  assert.equal(previewProgressFrames({ currentTime: 15, duration: 12, playing: true }), null);
});

test('a player waiting on the reader shows an empty, still bar', () => {
  const { fill } = setup();
  assert.equal(fill.style.transform, 'scaleX(0)');
  assert.equal(fill.animations.length, 0);
});

test('play starts one linear animation for the remaining time', () => {
  const { player, fill } = setup({ duration: 12 });
  play(player);

  assert.equal(fill.animations.length, 1);
  const [animation] = fill.animations;
  assert.deepEqual(animation.frames, [
    { transform: 'scaleX(0)' },
    { transform: 'scaleX(1)' },
  ]);
  assert.deepEqual(animation.options, { duration: 12000, easing: 'linear', fill: 'forwards' });
  assert.equal(fill.style.transform, 'scaleX(0)', 'the static position stays under the animation');
});

test('a timeupdate that agrees with the animation does not restart it', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;

  // Eight ticks of a quarter second, the old paint clock: each one must find
  // the compositor animation exactly where the media clock is.
  for (let tick = 1; tick <= 8; tick += 1) {
    player.currentTime = tick * 1.25;
    animation.progress = player.currentTime / player.duration;
    player.fire('timeupdate');
  }

  assert.equal(fill.animations.length, 1, 'no per-tick repaint, no per-tick restart');
  assert.equal(animation.canceled, false);
});

test('a drifting animation is restarted from the media position', () => {
  const { player, fill } = setup();
  play(player);
  const [first] = fill.animations;

  first.progress = 0.1;
  player.currentTime = 9;
  player.fire('timeupdate');

  assert.equal(fill.animations.length, 2);
  assert.equal(first.canceled, true);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0.3)' });
  assert.deepEqual(fill.animations[1].options, { duration: 21000, easing: 'linear', fill: 'forwards' });
});

test('the drift tolerance is a quarter second, not a visible jump', () => {
  assert.equal(PROGRESS_DRIFT, 0.25);
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;

  animation.progress = 10 / 30;
  player.currentTime = 10 + PROGRESS_DRIFT - 0.01;
  player.fire('timeupdate');
  assert.equal(fill.animations.length, 1, 'a lag inside the tolerance is left to the animation');

  animation.progress = 10 / 30;
  player.currentTime = 10 + PROGRESS_DRIFT + 0.01;
  player.fire('timeupdate');
  assert.equal(fill.animations.length, 2);
});

test('pause cancels the animation and leaves the exact ratio', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;
  animation.progress = 0.25;
  player.currentTime = 7.5;

  player.paused = true;
  player.fire('pause');

  assert.equal(animation.canceled, true);
  assert.equal(fill.animations.length, 1, 'a paused bar starts nothing');
  assert.equal(fill.style.transform, 'scaleX(0.25)');

  play(player);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0.25)' });
  assert.deepEqual(fill.animations[1].options, { duration: 22500, easing: 'linear', fill: 'forwards' });
});

test('pause freezes at the displayed ratio, not at the last media frame', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;
  animation.progress = 0.3;
  player.currentTime = 7.5;

  player.paused = true;
  player.fire('pause');
  assert.equal(fill.style.transform, 'scaleX(0.3)');

  player.fire('timeupdate');
  assert.equal(fill.style.transform, 'scaleX(0.3)', 'a late tick does not snap the bar back');

  player.paused = false;
  player.fire('play');
  assert.equal(fill.animations.length, 2);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0.3)' });
  assert.deepEqual(fill.animations[1].options, { duration: 22500, easing: 'linear', fill: 'forwards' });

  player.fire('playing');
  assert.equal(fill.animations.length, 2, 'playing does not restart the resumed animation');
});

test('a seek after a pause drops the frozen position', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;
  animation.progress = 0.3;
  player.currentTime = 7.5;
  player.paused = true;
  player.fire('pause');

  player.currentTime = 6;
  player.seeking = true;
  player.fire('seeking');
  assert.equal(fill.style.transform, 'scaleX(0.2)');

  player.seeking = false;
  player.fire('seeked');
  assert.equal(fill.style.transform, 'scaleX(0.2)');

  player.paused = false;
  player.fire('play');
  assert.deepEqual(fill.animations.at(-1).frames[0], { transform: 'scaleX(0.2)' });
  assert.deepEqual(fill.animations.at(-1).options, { duration: 24000, easing: 'linear', fill: 'forwards' });
});

test('waiting freezes the bar, playing starts it again from the same point', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;
  player.currentTime = 7.5;
  animation.progress = 0.25;

  player.fire('waiting');
  assert.equal(animation.canceled, true);
  assert.equal(fill.style.transform, 'scaleX(0.25)');

  player.fire('timeupdate');
  assert.equal(fill.animations.length, 1, 'a stalled bar does not creep forward');

  player.fire('playing');
  assert.equal(fill.animations.length, 2);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0.25)' });
  assert.deepEqual(fill.animations[1].options, { duration: 22500, easing: 'linear', fill: 'forwards' });
});

test('a seek freezes the bar at the pointer and resumes from there', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;
  animation.progress = 0.1;

  player.currentTime = 4;
  player.seeking = true;
  player.fire('seeking');
  assert.equal(animation.canceled, true);
  assert.equal(fill.style.transform, 'scaleX(0.13333333333333333)');

  player.seeking = false;
  player.currentTime = 6;
  player.fire('seeked');
  assert.equal(fill.animations.length, 2);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0.2)' });
  assert.deepEqual(fill.animations[1].options, { duration: 24000, easing: 'linear', fill: 'forwards' });
});

test('a loop restarts the fill from zero, without waiting for a tick', () => {
  const { player, fill } = setup();
  play(player);
  const [first] = fill.animations;
  first.progress = 1;
  player.currentTime = 0;

  first.onfinish();

  assert.equal(first.canceled, true);
  assert.equal(fill.animations.length, 2);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0)' });
  assert.deepEqual(fill.animations[1].options, { duration: 30000, easing: 'linear', fill: 'forwards' });

  // The superseded animation may still deliver its finish callback: it must not
  // start a third animation.
  first.onfinish();
  assert.equal(fill.animations.length, 2);
});

test('a loop that surfaces as seeking, seeked and playing is handled too', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;
  animation.progress = 0.99;

  player.currentTime = 0;
  player.seeking = true;
  player.fire('seeking');
  assert.equal(animation.canceled, true);
  assert.equal(fill.style.transform, 'scaleX(0)');

  player.seeking = false;
  player.fire('seeked');
  assert.equal(fill.animations.length, 2);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0)' });
});

test('a backwards jump detected on timeupdate restarts the fill', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;
  animation.progress = 0.9;

  player.currentTime = 0;
  player.fire('timeupdate');

  assert.equal(fill.animations.length, 2);
  assert.deepEqual(fill.animations[1].frames[0], { transform: 'scaleX(0)' });
});

test('the duration arriving late starts the animation', () => {
  const { player, fill } = setup({ duration: NaN });
  play(player);
  assert.equal(fill.animations.length, 0, 'no duration, no endpoint');

  player.duration = 12;
  player.fire('durationchange');
  assert.equal(fill.animations.length, 1);
  assert.deepEqual(fill.animations[0].options, { duration: 12000, easing: 'linear', fill: 'forwards' });
});

test('a playback rate change retimes the remaining span', () => {
  const { player, fill } = setup();
  play(player);
  player.currentTime = 6;
  player.playbackRate = 2;
  player.fire('ratechange');

  assert.equal(fill.animations.length, 2);
  assert.deepEqual(fill.animations[1].options, { duration: 12000, easing: 'linear', fill: 'forwards' });
});

test('emptied puts the bar back to zero and cuts the animation', () => {
  const { player, fill } = setup();
  play(player);
  const [animation] = fill.animations;

  player.currentTime = 0;
  player.duration = NaN;
  player.fire('emptied');

  assert.equal(animation.canceled, true);
  assert.equal(fill.style.transform, 'scaleX(0)');
});

test('without Element.animate the static ratio still follows the clock', () => {
  const player = new FakePlayer();
  const fill = fakeFill({ animatable: false });
  createPreviewProgress({ player, fill });

  play(player);
  assert.equal(fill.animations.length, 0);
  assert.equal(fill.style.transform, 'scaleX(0)');

  player.currentTime = 15;
  player.fire('timeupdate');
  assert.equal(fill.style.transform, 'scaleX(0.5)', 'the fallback keeps the old behaviour rather than freezing');
});

test('destroy stops the animation and detaches the player', () => {
  const { player, fill, progress } = setup();
  play(player);
  const [animation] = fill.animations;

  progress.destroy();
  assert.equal(animation.canceled, true);

  player.paused = false;
  player.fire('play');
  player.currentTime = 5;
  player.fire('timeupdate');
  assert.equal(fill.animations.length, 1, 'a destroyed controller listens to nothing');
  assert.equal(fill.style.transform, 'scaleX(0)');

  assert.doesNotThrow(() => progress.destroy(), 'destroy is idempotent');
});

test('the preview wires the fill to the animation, not to timeupdate', () => {
  assert.match(mediaDetail, /import \{ createPreviewProgress \} from '\.\.\/core\/preview-progress\.js';/);

  const attach = /\n  function attachProgress\(player\) \{([\s\S]*?)\n  \}/.exec(mediaDetail);
  assert.ok(attach, 'attachProgress is found');
  const body = attach[1];
  assert.match(body, /const progress = createPreviewProgress\(\{ player, fill \}\);/);
  assert.match(body, /progress\.paint\(\)/, 'a seek paints the pointer position at once');
  assert.match(body, /return \{ track, destroy: progress\.destroy \};/);
  assert.doesNotMatch(body, /addEventListener\('timeupdate'/, 'timeupdate no longer paints the fill');
});

test('the preview stores the controller and destroys it when it closes', () => {
  assert.match(mediaDetail, /const progress = player \? attachProgress\(player\) : null;/);
  assert.match(mediaDetail, /media\.after\(progress\.track\)/);
  assert.match(mediaDetail, /el\._progress = progress;/);
  assert.match(
    mediaDetail,
    /if \(el\._progress\) \{\s*el\._progress\.destroy\(\);\s*\}\s*if \(el\._player\) \{/,
    'the animation is cut before the player pauses',
  );
});

test('the fill starts empty and no longer steps through a CSS transition', () => {
  const fill = ruleOf(detailCss, '.hp-progress span');
  assert.match(fill, /transform:\s*scaleX\(0\)/);
  assert.match(fill, /transform-origin:\s*left center/);
  assert.doesNotMatch(fill, /transition:/, 'no short glide followed by a freeze');
  assert.match(fill, /background:\s*#fff/);
  assert.match(fill, /box-shadow:\s*0 0 6px rgba\(255, 255, 255, 0\.45\)/);
});
