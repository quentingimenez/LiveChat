// Elapsed time and progress of the media being played. Pure, so the
// one-second tick can update the queue card in place instead of rebuilding the
// view (which used to close the history and drop keyboard focus).

import { formatDuration } from './format.js';

export function queueClock(playing, now = Date.now()) {
  const started = playing && playing.startedAt ? new Date(playing.startedAt).getTime() : null;
  const duration = Number(playing && playing.durationSec) || 0;
  const elapsed = started ? Math.max(0, (now - started) / 1000) : 0;
  const shown = Math.min(elapsed, duration);
  const ratio = duration > 0 ? Math.min(1, elapsed / duration) : 0;
  return {
    text: `${formatDuration(shown) || '0:00'} / ${formatDuration(duration) || '0:00'}`,
    ratio,
  };
}
