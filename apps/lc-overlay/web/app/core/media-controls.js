// Time maths for the details window's control bar.
//
// The bar replaces Chromium's native controls so its progress can follow the
// same linear animation as the hover preview. These helpers keep the seek
// targets inside the media and independent from the DOM: they are the part of
// the bar that can be proven in node:test, while the bar itself is pinned by
// the view and CSS tests that run with it.

// A position inside the media. An unknown duration (NaN, Infinity, 0) leaves
// the current position untouched, clamped at zero.
export function seekBy(currentTime, duration, delta) {
  const current = Number.isFinite(currentTime) ? Math.max(0, currentTime) : 0;
  const step = Number.isFinite(delta) ? delta : 0;
  const max = Number.isFinite(duration) && duration > 0 ? duration : null;
  if (max === null) {
    return current;
  }
  return Math.min(max, Math.max(0, current + step));
}

// A pointer ratio (0..1) on the track as a time inside the media. A duration
// the element has not learnt yet reads as the start.
export function seekToRatio(ratio, duration) {
  if (!Number.isFinite(duration) || duration <= 0) {
    return 0;
  }
  const bounded = Number.isFinite(ratio) ? Math.min(1, Math.max(0, ratio)) : 0;
  return bounded * duration;
}

// Where the pointer is on the track, inside 0..1. A zero-width track (the
// window still laying out) reads as the start.
export function ratioAt(clientX, rect) {
  if (!rect || !Number.isFinite(rect.left) || !Number.isFinite(rect.width) || rect.width <= 0) {
    return 0;
  }
  if (!Number.isFinite(clientX)) {
    return 0;
  }
  return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
}
