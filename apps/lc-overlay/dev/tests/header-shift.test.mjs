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

test('the countdown stays right of the author chip above an audio without artwork', () => {
  // An audio element is not rendered by the user agent (display: none), so an
  // audio-only frame lays out at zero width and both header anchors land on
  // the same point: only the shifts can keep the chip and the countdown apart.
  const shift = computeHeaderShift({
    mediaWidth: 0,
    leftWidth: 297,
    rightWidth: 103,
    minGapPx: 14,
    leftRoomPx: 1280,
    rightRoomPx: 1280,
  });

  // Frame coordinates: the frame is a single point at 0, the chip leaves from
  // that point towards the left and the countdown arrives at it from the right.
  const authorRightPx = -shift.leftShiftPx + 297;
  const countdownLeftPx = -103 + shift.rightShiftPx;

  assert.ok(
    countdownLeftPx - authorRightPx >= 14,
    `the countdown must sit at least 14px right of the author (gap ${countdownLeftPx - authorRightPx}px)`,
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
