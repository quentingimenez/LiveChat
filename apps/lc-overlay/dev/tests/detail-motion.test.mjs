// Motion helpers for the board's media detail window. These tests pin the
// geometry strings handed to the Web Animations API and the frame accounting
// of the opening timeline, with a fake clock: the module itself must stay
// pure and never touch window, document or performance.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  contentRect,
  containRect,
  flightTransform,
  growFrame,
  HOLD_DISTANCE,
  holdReleased,
  HOVER_TIMING,
  isVisibleIn,
  MOTION_DEFAULTS,
  pageFlightFrames,
  mediaTrackFrames,
  parseDuration,
  placePreview,
  playTimeline,
  readMotion,
  returnRadius,
  showDelay,
  VOLUME_DRAG_MARGIN,
  volumeDragMargin,
} from '../../web/app/core/detail-motion.js';

// A clock the test drives by hand: animation frames are queued until the test
// flushes them with a timestamp, and every timeout waits for fireTimer().
const createClock = () => {
  const frames = [];
  const timers = new Map();
  let nextTimer = 1;

  return {
    frames,
    timers,
    raf(callback) {
      frames.push(callback);
    },
    setTimeout(callback, delay) {
      const id = nextTimer;
      nextTimer += 1;
      timers.set(id, { callback, delay });
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
    flushFrame(now) {
      const callbacks = frames.splice(0, frames.length);
      for (const callback of callbacks) callback(now);
    },
    fireTimer(delay) {
      const entry = [...timers.entries()].find(([, timer]) => timer.delay === delay);
      assert.ok(entry, `no ${delay} ms timer armed`);
      const [id, timer] = entry;
      timers.delete(id);
      timer.callback();
    },
  };
};

const animation = () => ({
  paused: 0,
  currentTime: null,
  pause() {
    this.paused += 1;
  },
});

const beginTimeline = (clock, animations = [animation(), animation()], duration = 320) => {
  const calls = { start: 0, done: 0 };
  const done = playTimeline(animations, duration, {
    onStart: () => {
      calls.start += 1;
    },
    onDone: () => {
      calls.done += 1;
    },
  }, clock);
  return { animations, calls, done };
};

// The timeline starts on the third nested frame; this flushes exactly those.
const startFrames = (clock) => {
  clock.flushFrame(0);
  clock.flushFrame(16);
  clock.flushFrame(32);
};

// A CSSStyleDeclaration stand-in: getPropertyValue knows the declared custom
// properties, like the computed style of the document element.
const fakeStyle = (values) => ({
  getPropertyValue(name) {
    return Object.prototype.hasOwnProperty.call(values, name) ? values[name] : '';
  },
});

test('MOTION_DEFAULTS is the one table the styles and the window agree on', () => {
  assert.deepEqual(MOTION_DEFAULTS, {
    detailOpen: 320,
    detailClose: 210,
    previewClose: 150,
    backdropIn: 320,
    backdropLead: 90,
    easeEnter: 'cubic-bezier(0.22, 0.61, 0.36, 1)',
    easeMove: 'cubic-bezier(0.4, 0, 0.2, 1)',
  });
});

test('HOVER_TIMING is the hover preview timing table', () => {
  assert.deepEqual(HOVER_TIMING, {
    cold: 250,
    warm: 60,
    warmWindow: 400,
    hide: 60,
  });
});

test('the level drag margin reaches above and below the preview', () => {
  assert.equal(VOLUME_DRAG_MARGIN, 32);
  const rect = { left: 100, top: 200, width: 460, height: 369 };
  assert.deepEqual(volumeDragMargin(rect), {
    left: 100,
    top: 168,
    width: 460,
    height: 433,
  });
  assert.deepEqual(rect, { left: 100, top: 200, width: 460, height: 369 }, 'the preview rect is left alone');
  assert.deepEqual(volumeDragMargin(rect, 0), rect, 'a zero margin is the preview itself');
});

test('showDelay waits cold once, then stays warm while a preview is open or just closed', () => {
  const never = Number.NEGATIVE_INFINITY;
  assert.equal(showDelay({ previewOpen: false, now: 5_000, lastClosedAt: never }), 250);
  assert.equal(showDelay({ previewOpen: true, now: 5_000, lastClosedAt: never }), 60);
  assert.equal(showDelay({ previewOpen: false, now: 1_399, lastClosedAt: 1_000 }), 60);
  // Exactly warmWindow after the close is no longer warm.
  assert.equal(showDelay({ previewOpen: false, now: 1_400, lastClosedAt: 1_000 }), 250);
  assert.equal(showDelay({ previewOpen: false, now: 2_000, lastClosedAt: 1_000 }), 250);
});

test('showDelay treats a missing or unusable close time as never closed', () => {
  for (const lastClosedAt of [undefined, Number.NaN, Number.NEGATIVE_INFINITY]) {
    assert.equal(
      showDelay({ previewOpen: false, now: 5_000, lastClosedAt }),
      250,
      `${lastClosedAt} must stay cold`,
    );
    assert.equal(
      showDelay({ previewOpen: true, now: 5_000, lastClosedAt }),
      60,
      `${lastClosedAt} is still warm while a preview is open`,
    );
  }
  assert.equal(showDelay({ previewOpen: false, now: 5_000 }), 250, 'an absent lastClosedAt is cold');
});

test('showDelay honours a custom timing table', () => {
  const timing = { cold: 500, warm: 20, warmWindow: 100, hide: 30 };
  assert.equal(showDelay({ previewOpen: false, now: 1_050, lastClosedAt: 1_000 }, timing), 20);
  assert.equal(showDelay({ previewOpen: false, now: 1_100, lastClosedAt: 1_000 }, timing), 500);
  assert.equal(showDelay({ previewOpen: true, now: 9_000, lastClosedAt: 1_000 }, timing), 20);
  assert.equal(
    showDelay({ previewOpen: false, now: 9_000, lastClosedAt: Number.NEGATIVE_INFINITY }, timing),
    500,
  );
});

test('parseDuration reads both CSS duration units, trimmed', () => {
  assert.equal(parseDuration('320ms', 1), 320);
  assert.equal(parseDuration('0.32s', 1), 320);
  assert.equal(parseDuration(' 150ms ', 1), 150);
  assert.equal(parseDuration('0s', 1), 0);
});

test('parseDuration falls back on anything that is not a usable duration', () => {
  for (const text of ['', '   ', 'abc', 'ms', '-5ms', '-0.2s', 'NaN', null, undefined]) {
    assert.equal(parseDuration(text, 210), 210, `${JSON.stringify(text)} must fall back`);
  }
});

test('readMotion reads every token, curves trimmed', () => {
  const style = fakeStyle({
    '--motion-detail-open': '400ms',
    // Values that differ from MOTION_DEFAULTS: a token read under the wrong
    // name would fall back and still look right otherwise.
    '--motion-detail-close': '0.25s',
    '--motion-preview-close': '90ms',
    '--motion-backdrop-in': '70ms',
    '--motion-backdrop-lead': '40ms',
    '--ease-enter': '  cubic-bezier(0.1, 0.2, 0.3, 1)  ',
    '--ease-move': ' linear ',
  });
  assert.deepEqual(readMotion(style), {
    detailOpen: 400,
    detailClose: 250,
    previewClose: 90,
    backdropIn: 70,
    backdropLead: 40,
    easeEnter: 'cubic-bezier(0.1, 0.2, 0.3, 1)',
    easeMove: 'linear',
  });
});

test('readMotion falls back to MOTION_DEFAULTS for missing and invalid values', () => {
  assert.deepEqual(readMotion(fakeStyle({})), MOTION_DEFAULTS);
  assert.deepEqual(readMotion(fakeStyle({
    '--motion-detail-open': 'abc',
    '--motion-detail-close': '-5ms',
    '--motion-preview-close': '',
    '--ease-enter': '   ',
    '--ease-move': null,
  })), MOTION_DEFAULTS);
  assert.deepEqual(readMotion(null), MOTION_DEFAULTS);
  // An object that is not a style declaration reads nothing, it never throws.
  assert.deepEqual(readMotion({}), MOTION_DEFAULTS);
});

test('parseDuration reads decimals and rejects a duration buried in other text', () => {
  assert.equal(parseDuration('12.5ms', 1), 12.5);
  assert.equal(parseDuration('.25s', 1), 250);
  for (const text of ['x320ms', '320msx', '1 320ms']) {
    assert.equal(parseDuration(text, 210), 210, `${JSON.stringify(text)} must fall back`);
  }
});

test('containRect letterboxes a wide media in a tall box', () => {
  const rect = containRect({ left: 10, top: 20, width: 200, height: 400 }, 2);
  assert.deepEqual(rect, { left: 10, top: 170, width: 200, height: 100 });
});

test('containRect pillarboxes a tall media in a wide box', () => {
  const rect = containRect({ left: 0, top: 0, width: 400, height: 200 }, 0.5);
  assert.deepEqual(rect, { left: 150, top: 0, width: 100, height: 200 });
});

test('containRect returns a box copy for an unknown ratio or an empty box', () => {
  const box = { left: 10, top: 20, width: 300, height: 200 };
  const copy = containRect(box, 0);
  assert.deepEqual(copy, box);
  assert.notEqual(copy, box);
  assert.deepEqual(
    containRect({ left: 1, top: 2, width: 0, height: 50 }, 1.6),
    { left: 1, top: 2, width: 0, height: 50 },
  );
});

test('contentRect subtracts borders and padding and clamps to zero', () => {
  assert.deepEqual(
    contentRect(
      { left: 10, top: 20, width: 100, height: 80 },
      { left: 2, right: 3, top: 4, bottom: 5 },
    ),
    { left: 12, top: 24, width: 95, height: 71 },
  );
  assert.deepEqual(
    contentRect(
      { left: 10, top: 20, width: 10, height: 5 },
      { left: 8, right: 8, top: 4, bottom: 4 },
    ),
    { left: 18, top: 24, width: 0, height: 0 },
  );
});

test('flightTransform maps a box laid out at the destination onto the source', () => {
  assert.equal(
    flightTransform(
      { left: 10, top: 20, width: 100, height: 50 },
      { left: 0, top: 0, width: 200, height: 100 },
    ),
    'translate(10px, 20px) scale(0.5, 0.5)',
  );
});

test('isVisibleIn: the vertical middle must be strictly inside the area', () => {
  const area = { top: 0, bottom: 200 };
  assert.equal(isVisibleIn({ left: 0, top: 100, width: 10, height: 10 }, area), true);
  assert.equal(isVisibleIn({ left: 0, top: -20, width: 10, height: 10 }, area), false);
  assert.equal(isVisibleIn({ left: 0, top: 300, width: 10, height: 10 }, area), false);
  assert.equal(isVisibleIn({ left: 0, top: 100, width: 0, height: 10 }, area), false);
  assert.equal(isVisibleIn({ left: 0, top: 100, width: 10, height: 0 }, area), false);
  assert.equal(isVisibleIn({ left: 0, top: -5, width: 10, height: 10 }, area), false);
});

test('holdReleased waits for a real 4 px move, diagonals included', () => {
  assert.equal(HOLD_DISTANCE, 4);
  assert.equal(holdReleased({ x: 10, y: 10 }, { x: 13, y: 10 }), false);
  assert.equal(holdReleased({ x: 10, y: 10 }, { x: 14, y: 10 }), true);
  assert.equal(holdReleased({ x: 10, y: 10 }, { x: 13, y: 13 }), true);
});

test('placePreview centres a preview that fits inside the area', () => {
  const area = { left: 0, right: 1000, top: 0, bottom: 800 };
  assert.deepEqual(
    placePreview(
      { left: 450, top: 375, width: 100, height: 100 },
      { width: 200, height: 100 },
      area,
    ),
    { left: 400, top: 375 },
  );
});

test('placePreview clamps at the edges and returns the final box only', () => {
  const area = { left: 0, right: 1000, top: 0, bottom: 800 };
  const placed = placePreview(
    { left: 0, top: 350, width: 100, height: 100 },
    { width: 200, height: 100 },
    area,
  );
  assert.deepEqual(placed, { left: 0, top: 350 });
  // dx, dy and scale belong to growFrame, which owns the start frame.
  assert.deepEqual(Object.keys(placed).sort(), ['left', 'top']);
});

test('returnRadius keeps thumb radii unless they are square at the card top', () => {
  assert.equal(returnRadius('0px 0px 0px 0px', 100, 100.5, '12px'), '12px 12px 0px 0px');
  assert.equal(returnRadius('10px 10px 0px 0px', 100, 100, '12px'), '10px 10px 0px 0px');
  assert.equal(returnRadius('0px 0px 0px 0px', 110, 100, '12px'), '0px 0px 0px 0px');
});

test('playTimeline pauses every animation before the first frame', () => {
  const clock = createClock();
  const animations = [animation(), animation(), animation()];
  const { calls } = beginTimeline(clock, animations);

  assert.deepEqual(animations.map((item) => item.paused), [1, 1, 1]);
  assert.equal(calls.start, 0);
  assert.equal(calls.done, 0);
});

test('playTimeline waits three frames and starts on the third', () => {
  const clock = createClock();
  const { animations, calls } = beginTimeline(clock);

  assert.equal(clock.frames.length, 1);
  clock.flushFrame(0);
  assert.equal(calls.start, 0);
  assert.equal(clock.frames.length, 1);
  clock.flushFrame(16);
  assert.equal(calls.start, 0);
  clock.flushFrame(32);
  assert.equal(calls.start, 1);
  assert.equal(calls.done, 0);
  for (const item of animations) assert.equal(item.currentTime, null);

  // The 400 ms prepaint timer was cleared and the end-of-timeline safety armed.
  assert.equal(clock.timers.size, 1);
  const [safety] = clock.timers.values();
  assert.equal(safety.delay, 320 * 3 + 500);
});

test('playTimeline caps a slow frame at 20 ms', () => {
  const clock = createClock();
  const { animations, calls } = beginTimeline(clock);
  startFrames(clock);

  clock.flushFrame(1000); // first frame only records the clock
  assert.equal(animations[0].currentTime, 0);
  assert.equal(animations[1].currentTime, 0);
  clock.flushFrame(1100); // 100 ms elapsed, capped to 20 ms
  assert.equal(animations[0].currentTime, 20);
  assert.equal(animations[1].currentTime, 20);
  assert.equal(calls.done, 0);
});

test('playTimeline finishes once at the duration and lands the animations there', () => {
  const clock = createClock();
  const { animations, calls } = beginTimeline(clock, [animation(), animation()], 100);
  startFrames(clock);

  clock.flushFrame(1000);
  let now = 1000;
  for (let step = 0; step < 5; step += 1) {
    now += 20;
    clock.flushFrame(now);
  }

  assert.equal(calls.done, 1);
  assert.deepEqual(animations.map((item) => item.currentTime), [100, 100]);
  assert.equal(clock.timers.size, 0);

  clock.flushFrame(2000);
  assert.equal(calls.done, 1);
});

test('playTimeline starts on the 400 ms safety when no frame ever comes', () => {
  const clock = createClock();
  const { animations, calls } = beginTimeline(clock);
  assert.equal(calls.start, 0);

  clock.fireTimer(400);
  assert.equal(calls.start, 1);
  assert.equal(clock.timers.size, 1);
  const [safety] = clock.timers.values();
  assert.equal(safety.delay, 320 * 3 + 500);

  clock.flushFrame(0); // the lazy frame chain and the first step
  clock.flushFrame(100);
  assert.equal(animations[0].currentTime, 20);
  assert.equal(calls.start, 1);
});

test('playTimeline can be finished early and stays finished', () => {
  const clock = createClock();
  const { animations, calls, done } = beginTimeline(clock);
  done();
  assert.equal(calls.done, 1);
  assert.equal(clock.timers.size, 0);

  done();
  for (let frame = 0; frame < 5; frame += 1) clock.flushFrame(frame * 16);

  assert.equal(calls.start, 0);
  assert.equal(calls.done, 1);
  assert.equal(animations[0].currentTime, null);
});

test('flightTransform measures from the destination box, wherever it sits', () => {
  const from = { left: 40, top: 30, width: 100, height: 50 };
  const to = { left: 140, top: 230, width: 200, height: 100 };
  assert.equal(flightTransform(from, to), 'translate(-100px, -200px) scale(0.5, 0.5)');
});

test('isVisibleIn uses the middle of a card that does not start at zero', () => {
  // Middle at 300: inside [100, 400], not inside [310, 900] nor [0, 300].
  const card = { left: 0, top: 250, width: 100, height: 100 };
  assert.equal(isVisibleIn(card, { top: 100, bottom: 400 }), true);
  assert.equal(isVisibleIn(card, { top: 310, bottom: 900 }), false);
  assert.equal(isVisibleIn(card, { top: 0, bottom: 300 }), false);
});

test('placePreview keeps a preview inside the right and bottom edges', () => {
  const area = { left: 0, right: 1000, top: 0, bottom: 800 };
  assert.deepEqual(
    placePreview({ left: 950, top: 750, width: 100, height: 100 }, { width: 200, height: 100 }, area),
    { left: 800, top: 700 },
  );
});

test('growFrame puts the preview first frame exactly on the card', () => {
  // The viewer's card (210 x 208, 130 px thumbnail) and the preview it opens
  // into: the scaled preview rect is the card's to the pixel, the media band
  // is the thumbnail, the body starts under it and what lies below the card's
  // bottom edge is clipped.
  assert.deepEqual(
    growFrame({
      card: { left: 548, top: 201, width: 210, height: 208 },
      thumbHeight: 130,
      cardRadius: 10,
      box: { left: 522, top: 209, width: 260, height: 368 },
      mediaHeight: 258,
    }),
    {
      dx: 26,
      dy: -8,
      scale: 0.808,
      crop: 48.524,
      shift: -97.048,
      clipBottom: 110.476,
      radius: 12.381,
    },
  );
});

test('growFrame leaves a media shorter than the band uncropped and unshifted', () => {
  const frame = growFrame({
    card: { left: 548, top: 201, width: 210, height: 208 },
    thumbHeight: 130,
    cardRadius: 10,
    box: { left: 522, top: 209, width: 260, height: 368 },
    mediaHeight: 140,
  });
  assert.equal(frame.crop, 0);
  assert.equal(frame.shift, 0);
});

test('growFrame clamps the scale and clips only below the card bottom edge', () => {
  const card = { left: 100, top: 100, width: 300, height: 200 };
  const box = { left: 80, top: 90, width: 260, height: 368 };
  const frame = growFrame({
    card,
    thumbHeight: 130,
    cardRadius: 10,
    box,
    mediaHeight: 258,
  });
  assert.equal(frame.scale, 1);
  assert.equal(frame.clipBottom, box.height - card.height);
  assert.equal(frame.radius, 10);

  // A preview shorter than the card: nothing to clip below, never negative.
  const short = growFrame({
    card: { ...card, height: 400 },
    thumbHeight: 130,
    cardRadius: 10,
    box,
    mediaHeight: 258,
  });
  assert.equal(short.scale, 1);
  assert.equal(short.clipBottom, 0);

  // The other bound: a card much narrower than its preview still never
  // shrinks below 0.35, or the text would become unreadable.
  const narrow = growFrame({
    card: { ...card, width: 50 },
    thumbHeight: 130,
    cardRadius: 10,
    box,
    mediaHeight: 258,
  });
  assert.equal(narrow.scale, 0.35);
});

test('returnRadius only borrows the card corners for four exact zero radii', () => {
  // A 10 px top-left corner ends with « 0px 0px 0px 0px » too: not square.
  assert.equal(returnRadius('10px 0px 0px 0px', 100, 100, '12px'), '10px 0px 0px 0px');
  assert.equal(returnRadius('0px 0px 0px 0px 5px', 100, 100, '12px'), '0px 0px 0px 0px 5px');
  // Exactly 2 px below the card top is not flush any more.
  assert.equal(returnRadius('0px 0px 0px 0px', 102, 100, '12px'), '0px 0px 0px 0px');
});

test('playTimeline runs without an onStart callback', () => {
  const clock = createClock();
  const animations = [animation()];
  let done = 0;
  playTimeline(animations, 40, { onDone: () => { done += 1; } }, clock);
  startFrames(clock);
  clock.flushFrame(100);
  clock.flushFrame(120);
  clock.flushFrame(140);
  assert.equal(animations[0].currentTime, 40);
  assert.equal(done, 1);
});

test('playTimeline starts once when the safety timer beats the third frame', () => {
  const clock = createClock();
  const { calls } = beginTimeline(clock);
  clock.fireTimer(400);
  startFrames(clock); // the frame chain still reaches its third frame
  assert.equal(calls.start, 1);
});

test('playTimeline advances by the real gap of a fast frame and freezes once done', () => {
  const clock = createClock();
  const { animations, done } = beginTimeline(clock);
  startFrames(clock);
  clock.flushFrame(1000);
  clock.flushFrame(1005);
  assert.equal(animations[0].currentTime, 5);
  done();
  clock.flushFrame(1015);
  assert.equal(animations[0].currentTime, 5);
});

test('pageFlightFrames carries the content from the origin rect to the page', () => {
  // Worked example of the brief: a card at (100, 200), 200 x 150, opening
  // into a 800 x 600 page at (300, 100), with the radii interpolated and
  // divided by the scale so the transformed corners keep their CSS size.
  const frames = pageFlightFrames({
    origin: { left: 100, top: 200, width: 200, height: 150 },
    page: { left: 300, top: 100, width: 800, height: 600 },
    originRadius: 10,
    pageRadius: 16,
    steps: 2,
  });
  assert.deepEqual(frames, [
    {
      offset: 0,
      transform: 'translate(-200px, 100px) scale(0.25)',
      clipPath: 'inset(0px 0px 0px 0px round 40px)',
    },
    {
      offset: 0.5,
      transform: 'translate(-100px, 50px) scale(0.625)',
      clipPath: 'inset(0px 0px 0px 0px round 20.8px)',
    },
    {
      offset: 1,
      transform: 'translate(0px, 0px) scale(1)',
      clipPath: 'inset(0px 0px 0px 0px round 16px)',
    },
  ]);
});

test('pageFlightFrames clips what lies below a short origin frame', () => {
  // A wide, 50 px tall card unfolds into a 600 px tall page: the content below
  // the frame's bottom edge is cut in the page's own (unscaled) pixels.
  const frames = pageFlightFrames({
    origin: { left: 100, top: 200, width: 200, height: 50 },
    page: { left: 300, top: 100, width: 800, height: 600 },
    originRadius: 10,
    pageRadius: 16,
    steps: 1,
  });
  assert.equal(frames.length, 2);
  assert.deepEqual(frames[0], {
    offset: 0,
    transform: 'translate(-200px, 100px) scale(0.25)',
    clipPath: 'inset(0px 0px 400px 0px round 40px)',
  });
});

test('pageFlightFrames cuts a mid-flight frame at the interpolated height', () => {
  // Halfway, the frame is 500 x 325 px: at scale 0.625 the page shows
  // 325 / 0.625 = 520 of its 600 px, so 80 px are cut below.
  const frames = pageFlightFrames({
    origin: { left: 100, top: 200, width: 200, height: 50 },
    page: { left: 300, top: 100, width: 800, height: 600 },
    originRadius: 10,
    pageRadius: 16,
    steps: 2,
  });
  assert.equal(frames[1].clipPath, 'inset(0px 0px 80px 0px round 20.8px)');
});

test('pageFlightFrames treats a step count below one as one step', () => {
  for (const steps of [0, -3]) {
    const frames = pageFlightFrames({
      origin: { left: 100, top: 200, width: 200, height: 150 },
      page: { left: 300, top: 100, width: 800, height: 600 },
      originRadius: 10,
      pageRadius: 16,
      steps,
    });
    assert.equal(frames.length, 2, `steps ${steps} must yield two frames`);
    assert.deepEqual(frames.map((frame) => frame.offset), [0, 1]);
    assert.equal(frames[1].transform, 'translate(0px, 0px) scale(1)');
  }
});

test('pageFlightFrames rounds to three decimals and never writes a negative zero', () => {
  const frames = pageFlightFrames({
    // Negative sub-thousandth offsets on every axis: rounding lands on -0.
    origin: { left: 299.9996, top: 99.9996, width: 800, height: 600 },
    page: { left: 300, top: 100, width: 800, height: 600 },
    originRadius: 16,
    pageRadius: 16,
    steps: 1,
  });
  assert.equal(frames[0].transform, 'translate(0px, 0px) scale(1)');
  for (const frame of frames) {
    assert.doesNotMatch(frame.transform, /-0(?:px)?\b/);
    assert.doesNotMatch(frame.clipPath, /-0(?:px)?\b/);
  }
});

test('pageFlightFrames ends exactly at the page, whatever the origin', () => {
  const frames = pageFlightFrames({
    origin: { left: 0.1, top: 0.2, width: 400.1, height: 200.1 },
    page: { left: 100.2, top: 50.3, width: 800.2, height: 600.3 },
    originRadius: 9.5,
    pageRadius: 16,
    steps: 7,
  });
  const last = frames.at(-1);
  assert.equal(frames.length, 8);
  assert.equal(last.offset, 1);
  assert.equal(last.transform, 'translate(0px, 0px) scale(1)');
  assert.equal(last.clipPath, 'inset(0px 0px 0px 0px round 16px)');
});

// The media of the detail window, 400 x 200 at (400, 200) in the laid-out
// page, carried by the same container transform as pageFlightFrames.
const TRACK = {
  origin: { left: 100, top: 200, width: 200, height: 150 },
  page: { left: 300, top: 100, width: 800, height: 600 },
  media: { left: 400, top: 200, width: 400, height: 200 },
  outside: { left: 120, top: 210, width: 160, height: 60 },
  steps: 2,
};

test('mediaTrackFrames opens from the outside rect onto the media of the moving page', () => {
  const frames = mediaTrackFrames(TRACK);
  assert.deepEqual(frames, [
    { offset: 0, rect: { left: 120, top: 210, width: 160, height: 60 } },
    // Halfway the page sits at (200, 150) scaled 0.625, so its media is at
    // (262.5, 212.5) 250 x 125; the carried media has covered 1 - 0.5^2 = 75 %
    // of the way from the outside rect to it.
    { offset: 0.5, rect: { left: 226.875, top: 211.875, width: 227.5, height: 108.75 } },
    { offset: 1, rect: { left: 400, top: 200, width: 400, height: 200 } },
  ]);
});

test('mediaTrackFrames closes from the media of the shrinking page onto the outside rect', () => {
  const frames = mediaTrackFrames({ ...TRACK, leaving: true, steps: 4 });
  assert.equal(frames.length, 5);
  assert.deepEqual(frames[0], { offset: 0, rect: TRACK.media });
  assert.deepEqual(frames[4], { offset: 1, rect: TRACK.outside });
  // A quarter of the way the page is at offset 0.75 of the opening: (250, 125)
  // scaled 0.8125, its media at (331.25, 206.25) 325 x 162.5; the media has only
  // left it by 0.25^2 = 6.25 % toward the outside rect.
  assert.deepEqual(frames[1], {
    offset: 0.25,
    rect: { left: 318.047, top: 206.484, width: 314.688, height: 156.094 },
  });
  // Halfway: the same point as the opening's halfway, mirrored.
  assert.deepEqual(frames[2].rect, { left: 226.875, top: 211.875, width: 227.5, height: 108.75 });
});

test('mediaTrackFrames treats a step count below one as one step', () => {
  const frames = mediaTrackFrames({ ...TRACK, steps: 0 });
  assert.deepEqual(frames.map((frame) => frame.offset), [0, 1]);
});
