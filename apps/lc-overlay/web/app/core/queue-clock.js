// Elapsed time and progress of the media being played. Pure, so the fast tick
// can update the queue card in place instead of rebuilding the view (which used
// to close the history and drop keyboard focus).

import { formatDuration } from './format.js';

// `pause` is the shared pause snapshot: `{ paused, remainingMs }`. While it is
// active the server froze playback on every overlay and `startedAt` still
// points at the original start, so a wall-clock reading would keep advancing.
// The frozen remaining time is authoritative: it is the position the server
// will resume from.
export function queueClock(playing, now = Date.now(), pause = null) {
  const started = playing && playing.startedAt ? new Date(playing.startedAt).getTime() : null;
  const anchored = Number.isFinite(started);
  const duration = Number(playing && playing.durationSec) || 0;
  const paused = Boolean(pause && pause.paused);
  const remainingMs = pause && pause.remainingMs;
  let elapsed;
  if (paused && Number.isFinite(remainingMs)) {
    const remaining = Math.min(duration, Math.max(0, remainingMs / 1000));
    elapsed = Math.max(0, duration - remaining);
  } else {
    elapsed = anchored ? Math.max(0, (now - started) / 1000) : 0;
  }
  const shown = Math.min(elapsed, duration);
  const ratio = duration > 0 ? Math.min(1, elapsed / duration) : 0;
  return {
    text: `${formatDuration(shown) || '0:00'} / ${formatDuration(duration) || '0:00'}`,
    ratio,
    // Exact, unrounded timings: the progress bar animates over the real
    // duration instead of stepping by whole percentage points on every tick.
    elapsed,
    duration,
    anchored,
  };
}

// On resume the server shifts `startedAt` by the paused time, but the board
// only learns it with the next fresh read. Anchor a local copy at the frozen
// position so the bar can start moving at once instead of staying still until
// the read lands and then jumping forward by the read latency.
export function resumedPlaying(playing, elapsed, now = Date.now()) {
  if (!playing || !Number.isFinite(elapsed)) {
    return playing;
  }
  return { ...playing, startedAt: new Date(now - elapsed * 1000).toISOString() };
}

// The fill is a compositor-driven linear animation: its duration is the job
// duration and its negative delay is the elapsed time, so it starts exactly
// where playback already is, even after the queue is re-rendered. The inline
// transform stays the static position, used if the animation is unavailable.
// While paused the animation must be cut: with no animation the inline
// transform is the only movement, so the bar stays frozen.
export function queueProgressStyle(clock, paused = false) {
  const position = `transform:scaleX(${clock.ratio})`;
  if (paused || !clock.anchored || clock.duration <= 0) {
    return `animation-name:none;${position}`;
  }
  return `--queue-progress-duration:${clock.duration}s;--queue-progress-delay:${-clock.elapsed}s;${position}`;
}
