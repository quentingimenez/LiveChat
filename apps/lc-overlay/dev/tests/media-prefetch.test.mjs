// The board warms a card's media while the pointer rests on it, so the hover
// preview and the detail window open on bytes the browser already has. These
// tests pin the debounce (a sweep across the grid loads nothing), the one-at-a-
// time rule, the expiry guard and the handoff of the loading element.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  createMediaPrefetcher,
  PREFETCH_DELAY_MS,
  PREFETCH_LIMIT,
} from '../../web/app/core/media-prefetch.js';
import { HOVER_TIMING } from '../../web/app/core/detail-motion.js';

const VIDEO_URL = 'https://cdn.test/media/video.mp4?exp=9999999999&sig=abc';
const OTHER_URL = 'https://cdn.test/media/other.mp4?exp=9999999999&sig=def';
const IMAGE_URL = 'https://cdn.test/media/picture.png?exp=9999999999&sig=ghi';

function createClock(start = 0) {
  let now = start;
  let nextId = 1;
  const timers = new Map();
  return {
    now: () => now,
    setTimeout(callback, delay) {
      const id = nextId;
      nextId += 1;
      timers.set(id, { at: now + delay, callback });
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
    pending: () => timers.size,
    advance(ms) {
      now += ms;
      let guard = 1000;
      while (guard > 0) {
        guard -= 1;
        let due = null;
        for (const [id, timer] of timers) {
          if (timer.at <= now
            && (!due || timer.at < due.timer.at || (timer.at === due.timer.at && id < due.id))) {
            due = { id, timer };
          }
        }
        if (!due) {
          return;
        }
        timers.delete(due.id);
        due.timer.callback();
      }
      throw new Error('timer loop did not settle');
    },
  };
}

function fakeElement(tagName) {
  const listeners = new Map();
  const classes = new Set();
  const element = {
    tagName: tagName.toUpperCase(),
    className: '',
    src: '',
    attributes: {},
    pauseCalls: 0,
    loadCalls: 0,
    classList: {
      add: (...names) => names.forEach((name) => classes.add(name)),
      remove: (...names) => names.forEach((name) => classes.delete(name)),
      contains: (name) => classes.has(name),
    },
    addEventListener(type, callback) {
      const list = listeners.get(type) || [];
      list.push(callback);
      listeners.set(type, list);
    },
    dispatch(type) {
      for (const callback of listeners.get(type) || []) {
        callback({ type });
      }
    },
    setAttribute(name, value) {
      element.attributes[name] = String(value);
    },
    getAttribute(name) {
      return Object.hasOwn(element.attributes, name) ? element.attributes[name] : null;
    },
    removeAttribute(name) {
      delete element.attributes[name];
      if (name === 'src') {
        element.src = '';
      }
    },
    pause() {
      element.pauseCalls += 1;
    },
    load() {
      element.loadCalls += 1;
    },
  };
  return element;
}

function setup(overrides = {}) {
  const clock = createClock();
  const created = [];
  const document = {
    createElement(tagName) {
      const element = fakeElement(tagName);
      created.push(element);
      return element;
    },
  };
  const prefetcher = createMediaPrefetcher({ document, clock, ...overrides });
  return { clock, created, prefetcher };
}

const video = (url = VIDEO_URL, expiresAt = 0) => ({ kind: 'video', url, expiresAt });
const image = (url = IMAGE_URL, expiresAt = 0) => ({ kind: 'image', url, expiresAt });

test('the prefetch starts before the cold hover preview delay', () => {
  // The media is meant to be in flight by the time the preview opens: a longer
  // debounce would make the preview wait for it again.
  assert.ok(PREFETCH_DELAY_MS > 0);
  assert.ok(PREFETCH_DELAY_MS < HOVER_TIMING.cold);
  assert.ok(PREFETCH_LIMIT >= 1);
});

test('a resting pointer loads the card media after the debounce', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());

  assert.equal(prefetcher.pendingCard(), 'card-a');
  clock.advance(PREFETCH_DELAY_MS - 1);
  assert.equal(created.length, 0, 'nothing loads before the delay');
  clock.advance(1);

  assert.equal(created.length, 1);
  assert.equal(created[0].tagName, 'VIDEO');
  assert.equal(created[0].src, VIDEO_URL);
  assert.equal(created[0].preload, 'auto');
  assert.equal(created[0].muted, true);
  assert.equal(prefetcher.size(), 1);
  assert.equal(prefetcher.pendingCard(), null);
});

test('leaving the card before the debounce loads nothing', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS / 2);
  prefetcher.cancel('card-a');
  clock.advance(PREFETCH_DELAY_MS * 10);

  assert.equal(created.length, 0);
  assert.equal(prefetcher.size(), 0);
});

test('a newer card replaces the pending one instead of stacking loads', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS / 2);
  prefetcher.schedule('card-b', video(OTHER_URL));
  clock.advance(PREFETCH_DELAY_MS / 2);

  assert.equal(created.length, 0, 'the first card was dropped with its timer');
  clock.advance(PREFETCH_DELAY_MS / 2);
  assert.equal(created.length, 1);
  assert.equal(created[0].src, OTHER_URL);
});

test('hovering the same card again does not restart the debounce', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS / 2);
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS / 2);

  assert.equal(created.length, 1, 'the original timer still fired at its first deadline');
});

test('an already warm card is not prefetched twice', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS);
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS * 2);

  assert.equal(created.length, 1);
  assert.equal(prefetcher.size(), 1);
});

test('take hands the loading element over exactly once', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS);

  const taken = prefetcher.take(video());
  assert.deepEqual(taken, { kind: 'video', element: created[0], ready: false });
  assert.equal(prefetcher.size(), 0);
  assert.equal(prefetcher.take(video()), null, 'the detail window cannot take it twice');
});

test('canplay marks the prefetched element as ready', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS);
  created[0].dispatch('canplay');

  assert.equal(prefetcher.take(video()).ready, true);
});

test('take refuses another kind and releases the prefetched element', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS);

  assert.equal(prefetcher.take({ kind: 'image', url: VIDEO_URL, expiresAt: 0 }), null);
  assert.equal(created[0].src, '');
  assert.equal(created[0].pauseCalls, 1);
  assert.equal(prefetcher.size(), 0);
});

test('a signed URL too close to its expiry is never prefetched', () => {
  const { clock, created, prefetcher } = setup();
  // One second short of the guard: the window would open on a URL that dies
  // while the media loads.
  prefetcher.schedule('card-a', video(VIDEO_URL, clock.now() + 59 * 1000));
  clock.advance(PREFETCH_DELAY_MS * 2);

  assert.equal(created.length, 0);
});

test('take drops a prefetched URL that expired while waiting', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video(VIDEO_URL, 200 * 1000));
  clock.advance(PREFETCH_DELAY_MS);
  assert.equal(created.length, 1);

  clock.advance(140 * 1000);
  assert.equal(prefetcher.take(video(VIDEO_URL, 200 * 1000)), null);
  assert.equal(created[0].src, '');
});

test('the oldest prefetch is released above the limit', () => {
  const { clock, created, prefetcher } = setup({ limit: 2 });
  const urls = [VIDEO_URL, OTHER_URL, 'https://cdn.test/media/third.mp4?exp=9999999999'];
  for (const url of urls) {
    prefetcher.schedule(`card-${urls.indexOf(url)}`, video(url));
    clock.advance(PREFETCH_DELAY_MS);
  }

  assert.equal(prefetcher.size(), 2);
  assert.equal(created.length, 3);
  assert.equal(created[0].src, '', 'the first media was released');
  assert.ok(created[0].pauseCalls > 0);
  assert.equal(created[1].src, OTHER_URL);
  assert.equal(created[2].src, urls[2]);
});

test('an unknown kind or a missing URL is ignored', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', { kind: 'pdf', url: VIDEO_URL, expiresAt: 0 });
  prefetcher.schedule('card-b', { kind: 'video', url: '', expiresAt: 0 });
  assert.equal(prefetcher.pendingCard(), null);
  clock.advance(PREFETCH_DELAY_MS * 2);

  assert.equal(created.length, 0);
});

test('a media that fails to load is dropped', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS);
  created[0].dispatch('error');

  assert.equal(prefetcher.size(), 0);
  assert.equal(prefetcher.take(video()), null);
});

test('images prefetch with the same handoff', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', image());
  clock.advance(PREFETCH_DELAY_MS);

  assert.equal(created[0].tagName, 'IMG');
  assert.equal(created[0].src, IMAGE_URL);
  const taken = prefetcher.take(image());
  assert.equal(taken.kind, 'image');
  assert.equal(taken.element, created[0]);
});

test('clear releases the pending and the warm media', () => {
  const { clock, created, prefetcher } = setup();
  prefetcher.schedule('card-a', video());
  clock.advance(PREFETCH_DELAY_MS);
  prefetcher.schedule('card-b', video(OTHER_URL));
  prefetcher.clear();
  clock.advance(PREFETCH_DELAY_MS);

  assert.equal(created.length, 1, 'the pending card never loaded');
  assert.equal(created[0].src, '');
  assert.equal(prefetcher.size(), 0);
});
