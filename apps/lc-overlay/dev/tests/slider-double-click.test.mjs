// A double click on a slider (settings volumes, the device volume dialog, the
// image and audio/video editor tools, the display scale) puts it back to the
// default of its setting. `resetOnDoubleClick` plays the move through the
// `input` and `change` listeners, so the value shown, the stored setting and
// the editor history follow exactly as after a manual move.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { resetOnDoubleClick, SLIDER_RESET_HINT } from '../../web/app/core/dom.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// The helper only uses `value`, `title`, `addEventListener` and `dispatchEvent`
// on the input: a fake range is enough to drive it without a DOM.
function fakeRange(value) {
  const listeners = new Map();
  const dispatched = [];
  return {
    value: String(value),
    title: '',
    dispatched,
    addEventListener(type, listener) {
      listeners.set(type, listener);
    },
    dispatchEvent(event) {
      dispatched.push(event.type);
      const listener = listeners.get(event.type);
      if (listener) {
        listener(event);
      }
      return true;
    },
    fire(type) {
      const listener = listeners.get(type);
      if (listener) {
        listener(new Event(type, { bubbles: true }));
      }
    },
  };
}

test('a double click resets the slider and replays it as a user move', () => {
  const input = fakeRange(37);
  const seen = [];
  input.addEventListener('input', () => seen.push(['input', input.value]));
  input.addEventListener('change', () => seen.push(['change', input.value]));
  resetOnDoubleClick(input, 100);

  assert.equal(input.title, SLIDER_RESET_HINT);
  input.fire('dblclick');

  assert.equal(input.value, '100');
  assert.deepEqual(input.dispatched, ['input', 'change']);
  assert.deepEqual(seen, [
    ['input', '100'],
    ['change', '100'],
  ]);
});

test('a double click on the default value changes nothing', () => {
  const input = fakeRange(100);
  input.addEventListener('input', () => assert.fail('an unchanged slider must not fire input'));
  resetOnDoubleClick(input, 100);

  input.fire('dblclick');

  assert.equal(input.value, '100');
  assert.deepEqual(input.dispatched, []);
});

test('a slider without a numeric default keeps no double-click reset', () => {
  const input = fakeRange(10);
  resetOnDoubleClick(input, undefined);
  input.fire('dblclick');

  assert.equal(input.value, '10');
  assert.equal(input.title, '');
});

test('the slider factory and the settings range both wire a default', () => {
  assert.match(read('views/editor/frame.js'), /resetOnDoubleClick\(input, defaultValue\)/);
  assert.match(read('views/settings.js'), /resetOnDoubleClick\(input, defaultValue\)/);
});

test('the settings volume sliders reset to 100 %', () => {
  const settings = read('views/settings.js');
  assert.equal(settings.match(/defaultValue: DEFAULT_VOLUME_PERCENT/g)?.length, 2);
  assert.match(read('views/guild-settings.js'), /defaultValue: DEFAULT_VOLUME_PERCENT/);
});

test('the device volume dialog resets to 100 %', () => {
  assert.match(read('views/admin.js'), /resetOnDoubleClick\(volume, DEFAULT_VOLUME_PERCENT\)/);
});

test('the display scale slider resets to its shipped 100 %', () => {
  assert.match(read('views/editor/image.js'), /resetOnDoubleClick\(range, DEFAULT_DISPLAY_SCALE_PERCENT\)/);
});

test('every editor slider declares its default', () => {
  for (const path of ['views/editor/image.js', 'views/editor/av.js']) {
    const source = read(path);
    const sliders = (source.match(/slider\(\{/g) || []).length;
    const defaults = (source.match(/defaultValue:/g) || []).length;
    assert.equal(defaults, sliders, `${path}: ${sliders} sliders, ${defaults} defaults`);
  }
});
