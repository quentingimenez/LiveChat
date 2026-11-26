// Settings page: « Retour » may leave the centred 880 px column on a wide
// board and stick in the left margin while the page scrolls. At the minimum
// window size it keeps its ordinary in-flow position.
//
// The decision is a pure helper; the CSS and the scroll/resize wiring are
// checked as source, since the overlay tests have no DOM.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  SETTINGS_BACK_GAP,
  SETTINGS_COLUMN_WIDTH,
  settingsBackInMargin,
} from '../../web/app/core/settings-back.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

function declarationsOf(css, selector) {
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
  assert.fail(`règle ${selector} introuvable`);
}

const BUTTON_WIDTH = 82;

test('la marge est calculée depuis la largeur du scroller et la colonne', () => {
  assert.equal(SETTINGS_COLUMN_WIDTH, 880);
  assert.equal(SETTINGS_BACK_GAP, 12);

  // 960×640 board: the scroller is narrower than the column plus the button.
  assert.equal(settingsBackInMargin({ scrollerWidth: 892, buttonWidth: BUTTON_WIDTH }), false);
  // 1400×880 board: the left margin holds the button with room to spare.
  assert.equal(settingsBackInMargin({ scrollerWidth: 1332, buttonWidth: BUTTON_WIDTH }), true);
});

test('la bascule se fait exactement à la largeur utile', () => {
  const threshold = SETTINGS_COLUMN_WIDTH + 2 * (BUTTON_WIDTH + SETTINGS_BACK_GAP);
  assert.equal(settingsBackInMargin({ scrollerWidth: threshold, buttonWidth: BUTTON_WIDTH }), true);
  assert.equal(
    settingsBackInMargin({ scrollerWidth: threshold - 1, buttonWidth: BUTTON_WIDTH }),
    false,
  );
  assert.equal(settingsBackInMargin({ scrollerWidth: threshold, buttonWidth: 0 }), false);
  assert.equal(settingsBackInMargin({ scrollerWidth: Number.NaN, buttonWidth: BUTTON_WIDTH }), false);
});

test('le bouton sort de l’en-tête et reste dans la marge au défilement', () => {
  const source = read('views/settings.js');
  const head = source.slice(source.indexOf('function renderHead'), source.indexOf('function settingsPage'));
  assert.ok(head.length > 0);
  assert.doesNotMatch(head, /settings-back/);
  assert.match(source, /function settingsPage\(\.\.\.children\)/);
  assert.match(source, /settingsBack\(\),\s*renderHead\(\)/);
  assert.match(source, /settingsBackInMargin\(\{/);
  assert.match(source, /scrollerWidth: view\.clientWidth/);
  assert.match(source, /buttonWidth: button\.offsetWidth/);
  assert.match(source, /addEventListener\('scroll', scheduleBackPlacement/);
  assert.match(source, /addEventListener\('resize', scheduleBackPlacement/);
});

test('la classe is-in-margin porte le sticky, le décalage et la transition', () => {
  const css = read('styles/settings.css');
  const rule = declarationsOf(css, '.settings-back.is-in-margin');
  assert.match(rule, /position:\s*sticky/);
  assert.match(rule, /top:\s*12px/);
  assert.match(rule, /margin-left:\s*0/);
  assert.match(rule, /transform:\s*translateX\(calc\(-100% - 12px\)\)/);
  assert.match(rule, /transition:\s*transform 120ms ease/);
  assert.match(rule, /background:\s*var\(--slate-800\)/);

  // Without the class the button keeps the current margin and no sticky.
  const base = declarationsOf(css, '.settings-back');
  assert.doesNotMatch(base, /position:\s*sticky/);
  assert.doesNotMatch(base, /transform:/);
});
