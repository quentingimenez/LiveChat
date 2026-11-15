import assert from 'node:assert/strict';
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
