import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  AUDIO_RATIO,
  CARD_WIDTH_PX,
  cardRatio,
  cardSpan,
  cardStyle,
  DEFAULT_RATIO,
  GAP_PX,
  MAX_RATIO,
  MIN_RATIO,
  ROW_PX,
  TITLE_LINES,
  TITLE_PX,
} from '../../web/app/core/masonry.js';

test('cardRatio: the media keeps its own shape', () => {
  assert.equal(cardRatio({ kind: 'video', width: 1280, height: 720 }), 1280 / 720);
  assert.equal(cardRatio({ kind: 'image', width: 1080, height: 1080 }), 1);
});

test('cardRatio: very tall and very wide media are clamped', () => {
  assert.equal(cardRatio({ kind: 'image', width: 500, height: 3000 }), MIN_RATIO);
  assert.equal(cardRatio({ kind: 'video', width: 4000, height: 1000 }), MAX_RATIO);
  // A 9:16 phone video is shown whole.
  assert.equal(cardRatio({ kind: 'video', width: 1080, height: 1920 }), 9 / 16);
});

test('cardRatio: unknown size and audio', () => {
  assert.equal(cardRatio({ kind: 'video', width: null, height: 720 }), DEFAULT_RATIO);
  assert.equal(cardRatio({ kind: 'image' }), DEFAULT_RATIO);
  assert.equal(cardRatio({ kind: 'audio', width: 1080, height: 1920 }), AUDIO_RATIO);
  assert.equal(cardRatio(null), DEFAULT_RATIO);
});

test('cardSpan covers the thumbnail, the title and the gap', () => {
  for (const ratio of [MIN_RATIO, 1, DEFAULT_RATIO, MAX_RATIO]) {
    const span = cardSpan(ratio);
    const needed = CARD_WIDTH_PX / ratio + TITLE_PX + GAP_PX;
    assert.ok(span * ROW_PX >= needed, `ratio ${ratio}`);
    assert.ok(span * ROW_PX < needed + ROW_PX, `ratio ${ratio}`);
  }
});

test('cardStyle sets the ratio and the span', () => {
  assert.equal(cardStyle(1), `--ratio: 1.0000; --span: ${cardSpan(1)}`);
});

// The title block is what keeps the masonry columns aligned: the CSS clamps the
// title to TITLE_LINES lines and the JS must reserve their height (13 px,
// line-height 1.3, plus the 6 px top margin). A drift makes cards overlap.
test('le titre de carte réserve exactement les lignes affichées', () => {
  assert.equal(TITLE_LINES, 2);
  const needed = TITLE_LINES * 13 * 1.3 + 6;
  assert.ok(TITLE_PX >= needed, `TITLE_PX ${TITLE_PX} < ${needed}`);
  assert.ok(TITLE_PX < needed + 8, `TITLE_PX ${TITLE_PX} réserve trop de vide`);
});

test('board.css borne le titre au même nombre de lignes que le masonry', () => {
  const css = readFileSync(new URL('../../web/app/styles/board.css', import.meta.url), 'utf8');
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  let title = null;
  while ((match = rule.exec(css)) !== null) {
    if (match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim() === '.card-title') {
      title = match[2];
    }
  }
  assert.ok(title, '.card-title doit exister dans board.css');
  assert.match(title, new RegExp(`-webkit-line-clamp:\\s*${TITLE_LINES}\\b`));
  assert.match(title, /line-height:\s*1\.3\b/);
  assert.match(title, /min-height:\s*2\.6em\b/);
});
