// The board sliders used Chromium's native rendering: a 16 px thumb and a 6 px
// track on every volume and editor control, with no shared style. These checks
// pin the one pill style (4 px track, 9 px thumb), the value fill that CSS
// cannot read by itself, and the places that feed it. Losing a call site would
// bring the native control back on that surface.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { bindRangeFill, paintRange, rangePercent } from '../../web/app/core/range-fill.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8').replace(/\r\n/g, '\n');
const appCss = read('app.css');
const sliderCss = read('styles/sliders.css');
const detailCss = read('styles/media-detail.css');
const settings = read('views/settings.js');
const board = read('views/board.js');
const admin = read('views/admin.js');
const frame = read('views/editor/frame.js');
const image = read('views/editor/image.js');
const mediaDetail = read('views/media-detail.js');

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

test('rangePercent maps a value to its share of the track', () => {
  assert.equal(rangePercent(50, 0, 100), 50);
  assert.equal(rangePercent(0, 0, 100), 0);
  assert.equal(rangePercent(100, 0, 100), 100);
  assert.equal(rangePercent(20, 10, 30), 50);
  assert.equal(rangePercent(25, 10, 30), 75);
  assert.equal(rangePercent(-5, 0, 100), 0);
  assert.equal(rangePercent(150, 0, 100), 100);
  assert.equal(rangePercent('40', '0', '100'), 40);
});

test('rangePercent rejects a broken or empty scale', () => {
  assert.equal(rangePercent(50, 100, 100), 0);
  assert.equal(rangePercent(50, 100, 0), 0);
  assert.equal(rangePercent(NaN, 0, 100), 0);
  assert.equal(rangePercent(50, NaN, 100), 0);
  assert.equal(rangePercent(50, 0, NaN), 0);
});

test('paintRange writes the fill as a custom property', () => {
  const style = {
    name: null,
    value: null,
    setProperty(name, value) {
      this.name = name;
      this.value = value;
    },
  };
  paintRange({ value: '40', min: '0', max: '50', style });
  assert.equal(style.name, '--range-p');
  assert.equal(style.value, '80%');
});

test('bindRangeFill paints once and follows every input event', () => {
  const listeners = [];
  const style = { value: null, setProperty(name, value) { this.value = value; } };
  const input = {
    value: '30',
    min: '0',
    max: '100',
    style,
    addEventListener(type, handler) {
      if (type === 'input') {
        listeners.push(handler);
      }
    },
  };
  bindRangeFill(input);
  assert.equal(style.value, '30%');
  input.value = '70';
  listeners.forEach((handler) => handler());
  assert.equal(style.value, '70%');
});

test('app.css loads the shared slider stylesheet after the feature styles', () => {
  assert.match(appCss, /@import url\('\.\/styles\/sliders\.css'\);/);
  const slider = appCss.indexOf("./styles/sliders.css");
  assert.ok(slider > appCss.indexOf("./styles/editor.css"), 'sliders.css must win over the feature rules');
  assert.ok(slider < appCss.indexOf("./styles/theme-light.css"), 'the light theme still loads last');
});

test('the shared slider style is a small pill with a 9 px thumb', () => {
  const selector = "input[type='range']:not(.hp-volume-range)";
  const base = ruleOf(sliderCss, selector);
  assert.match(base, /height:\s*16px/);
  assert.match(base, /background-size:\s*100% var\(--range-track, 4px\)/);
  assert.match(base, /background-image:\s*linear-gradient\(\s*to right/);
  assert.match(base, /var\(--range-p, 0%\)/);
  assert.match(base, /border-radius:\s*999px/);

  const thumb = ruleOf(sliderCss, `${selector}::-webkit-slider-thumb`);
  assert.match(thumb, /width:\s*9px/);
  assert.match(thumb, /height:\s*9px/);
  assert.match(thumb, /margin-top:\s*3\.5px/);
  assert.match(thumb, /border-radius:\s*50%/);
  assert.match(thumb, /box-shadow:\s*0 0 0 2px var\(--range-ring, var\(--slate-800\)\)/);
  assert.match(thumb, /transition:\s*transform 120ms ease/);
});

test('the player reuses the pill with its white chrome tokens', () => {
  const controls = ruleOf(detailCss, '.hp-controls');
  assert.match(controls, /--range-accent:\s*#fff/);
  assert.match(controls, /--range-rest:\s*rgb\(255 255 255 \/ 26%\)/);
  assert.match(controls, /--range-ring:\s*rgb\(8 10 16 \/ 90%\)/);

  const volume = ruleOf(detailCss, '.hp-ctl-volume');
  assert.doesNotMatch(volume, /accent-color/);
  assert.match(volume, /width:\s*82px/);
});

test('the narrow board slider keeps its native vertical fallback', () => {
  const vertical = ruleOf(sliderCss, "input[type='range'].board-size-range");
  assert.match(vertical, /appearance:\s*auto/);
  assert.match(vertical, /height:\s*96px/);
  assert.match(vertical, /writing-mode:\s*vertical-lr/);
});

test('every range input feeds the shared fill', () => {
  assert.match(settings, /import \{ bindRangeFill \} from '\.\.\/core\/range-fill\.js';/);
  assert.match(settings, /bindRangeFill\(input\)/);
  assert.match(board, /import \{ bindRangeFill \} from '\.\.\/core\/range-fill\.js';/);
  assert.match(board, /bindRangeFill\(input\)/);
  assert.match(admin, /import \{ bindRangeFill \} from '\.\.\/core\/range-fill\.js';/);
  assert.match(admin, /bindRangeFill\(volume\)/);
  assert.match(frame, /import \{ bindRangeFill \} from '\.\.\/\.\.\/core\/range-fill\.js';/);
  assert.match(frame, /bindRangeFill\(input\)/);
  assert.match(image, /import \{ bindRangeFill, paintRange \} from '\.\.\/\.\.\/core\/range-fill\.js';/);
  assert.match(image, /bindRangeFill\(range\)/);
  assert.match(image, /paintRange\(range\)/);
  assert.match(mediaDetail, /import \{ bindRangeFill, paintRange \} from '\.\.\/core\/range-fill\.js';/);
  assert.match(mediaDetail, /bindRangeFill\(volume\)/);
  assert.match(mediaDetail, /paintRange\(volume\)/);
});
