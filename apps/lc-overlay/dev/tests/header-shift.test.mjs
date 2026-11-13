// Unit tests of the stream header shift (author chip + countdown above the media).

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyMediaFrameUtils } from '../../web/dist/renderer/overlay/legacy-media-frame-utils.js';

const { computeHeaderShift } = createOverlayLegacyMediaFrameUtils();

test('a header that fits above the media is not shifted', () => {
  assert.deepEqual(
    computeHeaderShift({ mediaWidth: 600, leftWidth: 300, rightWidth: 100, minGapPx: 14 }),
    { leftShiftPx: 0, rightShiftPx: 0, unresolvedPx: 0 },
  );
});

test('the overflow is split evenly when both sides have room', () => {
  assert.deepEqual(
    computeHeaderShift({
      mediaWidth: 606,
      leftWidth: 684,
      rightWidth: 103,
      minGapPx: 14,
      leftRoomPx: 800,
      rightRoomPx: 800,
    }),
    { leftShiftPx: 98, rightShiftPx: 97, unresolvedPx: 0 },
  );
});

test('a frame anchored to the right edge pushes the whole overflow to the left', () => {
  assert.deepEqual(
    computeHeaderShift({
      mediaWidth: 606,
      leftWidth: 684,
      rightWidth: 103,
      minGapPx: 14,
      leftRoomPx: 1500,
      rightRoomPx: 52,
    }),
    { leftShiftPx: 143, rightShiftPx: 52, unresolvedPx: 0 },
  );
});

test('what no side can absorb is reported as unresolved', () => {
  assert.deepEqual(
    computeHeaderShift({
      mediaWidth: 606,
      leftWidth: 684,
      rightWidth: 103,
      minGapPx: 14,
      leftRoomPx: 40,
      rightRoomPx: -10,
    }),
    { leftShiftPx: 40, rightShiftPx: 0, unresolvedPx: 155 },
  );
});
